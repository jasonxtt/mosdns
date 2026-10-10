//! Bounded DoH reuse, revised 2026-10-09 after 5D-1's slow-upstream evidence.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Instant;

use tokio::sync::{Notify, Semaphore};

use super::{Clock, IDLE_TIMEOUT, SystemClock};
use crate::secure::{
    DohEndpoint, H2DrainHandle, PooledDohSession, SecureError, SecureResponse, TlsPolicy,
};
use crate::{
    CloseCompletion, CloseResult, CloseTransition, ExchangeContext, ExchangeControl,
    ExchangeRequest, Lifecycle, LifecycleState, SideEffectState, TransportCancellation,
    UpstreamError,
};

/// Connecting, busy, idle and retired-but-not-drained connections per owner.
pub const MAX_DOH_CONNECTIONS: usize = 12;
/// Local exchange leases, including waits for a peer's HTTP/2 stream allowance.
pub const MAX_DOH_STREAMS: usize = 64;
/// Total admitted calls, including bounded pool/stream waiters.
pub const MAX_DOH_CALLS: usize = 256;

/// Internal diagnostic counters; no public HTTP API is added.
#[derive(Clone, Copy, Debug, Default)]
pub struct DohReuseStats {
    pub connect_attempts: usize,
    pub reused: usize,
    pub active_leases: usize,
    pub peak_leases: usize,
    pub connections: usize,
}

/// Immutable endpoint/trust/route owner. Connection resources remain accounted
/// for until their last lease and tracked child scope have actually drained.
pub struct DohReuseOwner {
    endpoint: DohEndpoint,
    tls: TlsPolicy,
    lifecycle: Arc<Lifecycle>,
    cancellation: TransportCancellation,
    pool: Arc<Pool>,
    admission: Arc<Semaphore>,
    streams: Arc<Semaphore>,
    clock: Arc<dyn Clock>,
    #[cfg(test)]
    commit_pause: Mutex<Option<Arc<crate::CommitPause>>>,
}

struct Pool {
    state: Mutex<State>,
    changed: Notify,
    opened: AtomicUsize,
}

#[derive(Default)]
struct State {
    idle: Vec<Connection>,
    closing: Vec<Connection>,
    scopes: Vec<Scope>,
    dialing: usize,
    discovered: bool,
    h2: bool,
    stats: DohReuseStats,
}

struct Connection {
    session: PooledDohSession,
    slot: Arc<Slot>,
    idle_since: Instant,
}

struct Scope {
    handle: H2DrainHandle,
    // Keep the descriptor budget reserved until child teardown finishes.
    slot: Arc<Slot>,
}

struct Slot(Weak<Pool>);

impl Drop for Slot {
    fn drop(&mut self) {
        if let Some(pool) = self.0.upgrade() {
            pool.opened.fetch_sub(1, Ordering::SeqCst);
            pool.changed.notify_waiters();
        }
    }
}

impl Pool {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

struct Lease {
    pool: Arc<Pool>,
    connection: Option<Connection>,
    slot: Arc<Slot>,
    connecting: bool,
}

impl Drop for Lease {
    fn drop(&mut self) {
        if self.connecting {
            self.pool.lock().dialing -= 1;
        }
        // H1 abort/drop discards its socket. The slot's last drop wakes waiters.
        self.pool.changed.notify_waiters();
    }
}

struct ActiveLease(Arc<Pool>);

impl Drop for ActiveLease {
    fn drop(&mut self) {
        self.0.lock().stats.active_leases -= 1;
    }
}

impl std::fmt::Debug for DohReuseOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DohReuseOwner")
            .field("endpoint", &self.endpoint)
            .field("stats", &self.stats())
            .finish_non_exhaustive()
    }
}

impl DohReuseOwner {
    /// Validate TLS policy before any I/O.
    pub fn new(endpoint: DohEndpoint, tls: TlsPolicy) -> Result<Self, SecureError> {
        tls.client_config()?;
        Ok(Self {
            endpoint,
            tls,
            lifecycle: Arc::new(Lifecycle::new()),
            cancellation: TransportCancellation::new(),
            pool: Arc::new(Pool {
                state: Mutex::new(State::default()),
                changed: Notify::new(),
                opened: AtomicUsize::new(0),
            }),
            admission: Arc::new(Semaphore::new(MAX_DOH_CALLS)),
            streams: Arc::new(Semaphore::new(MAX_DOH_STREAMS)),
            clock: Arc::new(SystemClock),
            #[cfg(test)]
            commit_pause: Mutex::new(None),
        })
    }

    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }
    #[must_use]
    pub const fn endpoint(&self) -> &DohEndpoint {
        &self.endpoint
    }
    #[must_use]
    pub fn lifecycle_state(&self) -> LifecycleState {
        self.lifecycle.state()
    }
    #[must_use]
    pub fn in_flight_exchanges(&self) -> usize {
        self.lifecycle.in_flight()
    }
    #[must_use]
    pub fn idle_connections(&self) -> usize {
        self.pool.lock().idle.len()
    }
    #[must_use]
    pub fn stats(&self) -> DohReuseStats {
        let mut stats = self.pool.lock().stats;
        stats.connections = self.pool.opened.load(Ordering::SeqCst);
        stats
    }

    #[must_use]
    pub fn begin_close(&self) -> CloseTransition {
        let result = self.lifecycle.begin_close();
        if result == CloseTransition::BeganClosing {
            self.cancellation.cancel();
            self.pool.changed.notify_waiters();
        }
        result
    }

    /// Park retained sessions in shared state across every cancellable await.
    /// Concurrent/aborted closers can still see and drain all owned resources.
    pub async fn close(&self) -> CloseResult {
        if self.begin_close() == CloseTransition::AlreadyClosed {
            return CloseResult::AlreadyClosed;
        }
        self.lifecycle.drain().await;
        {
            let mut state = self.pool.lock();
            let idle = std::mem::take(&mut state.idle);
            state.closing.extend(idle);
        }
        self.drain_scopes(true).await;
        self.pool.lock().closing.clear();
        match self.lifecycle.finish_close() {
            CloseCompletion::Closed | CloseCompletion::AlreadyClosed => CloseResult::Closed,
            CloseCompletion::NotClosing | CloseCompletion::InFlight => CloseResult::AlreadyClosing,
        }
    }

    fn publish_scope(&self, handle: H2DrainHandle, slot: Arc<Slot>) {
        let mut state = self.pool.lock();
        if !state.scopes.iter().any(|s| s.handle.is_same_scope(&handle)) {
            state.scopes.push(Scope { handle, slot });
        }
    }

    async fn drain_scopes(&self, all: bool) {
        let handles: Vec<_> = self
            .pool
            .lock()
            .scopes
            .iter()
            .filter(|scope| all || Arc::strong_count(&scope.slot) == 1)
            .map(|scope| scope.handle.clone())
            .collect();
        for handle in handles {
            handle.finish().await;
            self.pool
                .lock()
                .scopes
                .retain(|s| !s.handle.is_same_scope(&handle));
        }
    }

    async fn checkout(&self, control: &ExchangeControl) -> Result<Lease, SecureError> {
        loop {
            control.check_at(Instant::now(), SideEffectState::NotSent)?;
            let notified = self.pool.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut state = self.pool.lock();
                let now = self.clock.now();
                state.idle.retain_mut(|conn| {
                    let busy_h2 =
                        conn.session.protocol_name() == "h2" && Arc::strong_count(&conn.slot) > 2;
                    (busy_h2 || now.saturating_duration_since(conn.idle_since) < IDLE_TIMEOUT)
                        && !conn.session.is_closed()
                });
                if let Some(conn) = state
                    .idle
                    .iter_mut()
                    .find(|c| c.session.protocol_name() == "h2")
                {
                    let connection = Connection {
                        session: conn.session.clone_h2().expect("h2 sender"),
                        slot: Arc::clone(&conn.slot),
                        idle_since: now,
                    };
                    state.stats.reused += 1;
                    return Ok(Lease {
                        pool: Arc::clone(&self.pool),
                        slot: Arc::clone(&connection.slot),
                        connection: Some(connection),
                        connecting: false,
                    });
                }
                if let Some(connection) = state.idle.pop() {
                    state.stats.reused += 1;
                    return Ok(Lease {
                        pool: Arc::clone(&self.pool),
                        slot: Arc::clone(&connection.slot),
                        connection: Some(connection),
                        connecting: false,
                    });
                }
            }
            // A retired scope's slot cannot be reused until actual child drain.
            crate::tcp::race_control(
                control,
                SideEffectState::NotSent,
                control.context().deadline(),
                async {
                    self.drain_scopes(false).await;
                    Ok::<(), SecureError>(())
                },
            )
            .await?;
            {
                let mut state = self.pool.lock();
                let opened = self.pool.opened.load(Ordering::SeqCst);
                if opened < MAX_DOH_CONNECTIONS
                    && ((!state.h2 && state.discovered) || state.dialing == 0)
                {
                    self.pool.opened.fetch_add(1, Ordering::SeqCst);
                    state.dialing += 1;
                    state.stats.connect_attempts += 1;
                    return Ok(Lease {
                        pool: Arc::clone(&self.pool),
                        connection: None,
                        slot: Arc::new(Slot(Arc::downgrade(&self.pool))),
                        connecting: true,
                    });
                }
            }
            crate::tcp::race_control(
                control,
                SideEffectState::NotSent,
                control.context().deadline(),
                async {
                    notified.await;
                    Ok::<(), SecureError>(())
                },
            )
            .await?;
        }
    }

    async fn connect(
        &self,
        lease: &mut Lease,
        control: &ExchangeControl,
    ) -> Result<(), SecureError> {
        let session = PooledDohSession::connect(
            &self.endpoint,
            &self.tls,
            self.cancellation.clone(),
            control,
            control.context().deadline(),
            |handle| self.publish_scope(handle, Arc::clone(&lease.slot)),
        )
        .await?;
        let h2 = session.protocol_name() == "h2";
        let shared = session.clone_h2();
        lease.connection = Some(Connection {
            session,
            slot: Arc::clone(&lease.slot),
            idle_since: self.clock.now(),
        });
        {
            let mut state = self.pool.lock();
            state.dialing -= 1;
            state.discovered = true;
            state.h2 = h2;
            if let Some(session) = shared {
                state.idle.push(Connection {
                    session,
                    slot: Arc::clone(&lease.slot),
                    idle_since: self.clock.now(),
                });
            }
        }
        lease.connecting = false;
        self.pool.changed.notify_waiters();
        Ok(())
    }

    fn retain(&self, lease: &mut Lease, session: PooledDohSession) {
        if self.lifecycle.state() == LifecycleState::Open {
            let mut state = self.pool.lock();
            if session.protocol_name() == "h2" {
                if let Some(conn) = state
                    .idle
                    .iter_mut()
                    .find(|c| Arc::ptr_eq(&c.slot, &lease.slot))
                {
                    conn.idle_since = self.clock.now();
                }
            } else {
                state.idle.push(Connection {
                    session,
                    slot: Arc::clone(&lease.slot),
                    idle_since: self.clock.now(),
                });
            }
        }
        self.pool.changed.notify_waiters();
    }

    /// Original deadline covers stream admission, pool wait and every I/O phase.
    pub async fn exchange(
        &self,
        request: ExchangeRequest<'_>,
        context: ExchangeContext,
    ) -> Result<SecureResponse, SecureError> {
        let _registered = self.lifecycle.register()?;
        let control = ExchangeControl::new(context, self.cancellation.clone());
        let deadline = control.context().deadline();
        control.check_at(Instant::now(), SideEffectState::NotSent)?;
        let _admitted = Arc::clone(&self.admission)
            .try_acquire_owned()
            .map_err(|_| UpstreamError::Backpressure(SideEffectState::NotSent))?;
        let _stream =
            crate::tcp::race_control(&control, SideEffectState::NotSent, deadline, async {
                Arc::clone(&self.streams)
                    .acquire_owned()
                    .await
                    .map_err(|_| SecureError::from(UpstreamError::Closed(SideEffectState::NotSent)))
            })
            .await?;
        {
            let mut state = self.pool.lock();
            state.stats.active_leases += 1;
            state.stats.peak_leases = state.stats.peak_leases.max(state.stats.active_leases);
        }
        let _active = ActiveLease(Arc::clone(&self.pool));
        let target = self.endpoint.get_request_target(request)?;
        let authority = self.endpoint.authority();
        // Only a reused, provably NotSent transport failure gets one replacement.
        for replacement in 0..2 {
            let mut lease = self.checkout(&control).await?;
            let reused = lease.connection.is_some();
            if !reused {
                self.connect(&mut lease, &control).await?;
            }
            control.check_at(Instant::now(), SideEffectState::NotSent)?;
            let connection = lease.connection.take().expect("connected session");
            match connection
                .session
                .exchange(
                    &target,
                    &authority,
                    request.request_id(),
                    &control,
                    deadline,
                )
                .await
            {
                Ok((response, session)) => {
                    #[cfg(test)]
                    self.reach_commit_gate().await;
                    if let Err(error) = self.lifecycle.commit_final_response(
                        &control.caller_cancellation(),
                        deadline,
                        SideEffectState::Sent,
                    ) {
                        if session.protocol_name() == "http/1.1" {
                            session.shutdown().await;
                        }
                        return Err(error.into());
                    }
                    self.retain(&mut lease, session);
                    return Ok(response);
                }
                Err(outcome) => {
                    let retry = reused
                        && replacement == 0
                        && outcome.rebuildable
                        && matches!(
                            outcome.error,
                            SecureError::Transport(
                                UpstreamError::Send(SideEffectState::NotSent)
                                    | UpstreamError::Receive(SideEffectState::NotSent)
                            )
                        );
                    if let Some(mut session) = outcome.session {
                        if session.protocol_name() == "http/1.1" {
                            session.shutdown().await;
                        } else if session.is_closed() {
                            self.pool
                                .lock()
                                .idle
                                .retain(|c| !Arc::ptr_eq(&c.slot, &lease.slot));
                        }
                    }
                    drop(lease);
                    // Interrupted cleanup remains registered for the next
                    // caller/close; it cannot extend this request's deadline.
                    let _ = crate::tcp::race_control(
                        &control,
                        outcome.error.side_effect(),
                        deadline,
                        async {
                            self.drain_scopes(false).await;
                            Ok::<(), SecureError>(())
                        },
                    )
                    .await;
                    if !retry {
                        return Err(outcome.error);
                    }
                }
            }
        }
        unreachable!("two bounded attempts return an outcome")
    }

    #[cfg(test)]
    pub(super) fn install_commit_pause(&self, pause: Arc<crate::CommitPause>) {
        *self
            .commit_pause
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(pause);
    }
    #[cfg(test)]
    async fn reach_commit_gate(&self) {
        let pause = self
            .commit_pause
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(pause) = pause {
            pause.pause().await;
        }
    }
    #[cfg(test)]
    pub(super) fn registered_scope_count(&self) -> usize {
        self.pool.lock().scopes.len()
    }
    #[cfg(test)]
    pub(super) fn active_children_for_test(&self) -> Option<usize> {
        self.pool
            .lock()
            .idle
            .first()?
            .session
            .active_children_for_test()
    }
    #[cfg(test)]
    pub(super) fn install_teardown_pause_for_test(
        &self,
        pause: Arc<crate::secure::H2TeardownPause>,
    ) -> bool {
        self.pool
            .lock()
            .idle
            .first()
            .and_then(|c| c.session.install_teardown_pause_for_test(pause))
            .is_some()
    }
    #[cfg(test)]
    pub(super) fn install_teardown_pause_on_scope_for_test(
        &self,
        index: usize,
        pause: Arc<crate::secure::H2TeardownPause>,
    ) -> bool {
        let state = self.pool.lock();
        if let Some(scope) = state.scopes.get(index) {
            scope.handle.install_teardown_pause(pause);
            true
        } else {
            false
        }
    }
    #[cfg(test)]
    pub(super) fn scope_children_for_test(&self, index: usize) -> Option<usize> {
        self.pool
            .lock()
            .scopes
            .get(index)
            .map(|s| s.handle.active_children())
    }
    #[cfg(test)]
    pub(super) async fn drain_stale_scopes(&self, _keep: Option<&H2DrainHandle>) {
        self.drain_scopes(true).await;
    }
}
