//! Controlled connection reuse for the plain-TCP and secure upstreams.
//!
//! This module adds a **reuse owner layer above the existing primitives**. It
//! does not replace them and does not reimplement anything they already own:
//! framing, the control race, response validation, lifecycle admission, and the
//! response-commit linearization point all stay in their existing homes.
//!
//! ## What reuse is, and is not
//!
//! Reuse means "keep one idle connection for a key so the next sequential
//! exchange on that key does not pay connect (and TLS handshake) again". It is
//! deliberately **serial per connection**: a reused connection carries at most
//! one outstanding query, so concurrency across callers comes from having
//! several connections rather than from multiplexing one.
//!
//! There is no pending map, no response-reordering buffer, and no ID
//! demultiplexing. Those belong to a separate task that must first establish an
//! unambiguous correlation policy without rewriting DNS IDs.
//!
//! ## Not in this module
//!
//! No QUIC/HTTP3, no UDP pooling (UDP is connectionless), no socket policy, no
//! protocol fallback, no listener, and no host/config/API wiring.

use std::collections::HashMap;
use std::fmt;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::net::TcpStream;

#[cfg(test)]
use crate::CommitPause;
use crate::secure::{
    DohEndpoint, DotEndpoint, SecureError, SecureResponse, ServerIdentity, TlsPolicy,
};
use crate::{
    CloseCompletion, CloseResult, CloseTransition, Endpoint, ExchangeContext, ExchangeControl,
    ExchangeRequest, ExchangeResponse, Lifecycle, LifecycleState, RequestError, SideEffectState,
    Transport, TransportCancellation, UpstreamError,
};

/// The revision recorded by the policy-agnostic [`SecureKey::new`] /
/// [`SecureKey::try_new`] constructors, which take no [`TlsPolicy`].
///
/// It mirrors [`crate::secure::TlsPolicy::insecure_skip_verify`]'s sentinel: a
/// key built this way identifies the *mode* but no particular root store. Owners
/// holding a real policy use [`SecureKey::from_policy`] and record its actual
/// revision instead.
const NO_ROOTS_REVISION: u64 = 0;

// ---------------------------------------------------------------------------
// Bound constants (confirmed for this task; deliberately not configuration)
// ---------------------------------------------------------------------------

/// Retained idle connections for one [`ReuseKey`].
///
/// One is enough: while a connection serves at most one outstanding query, a
/// second idle entry for the same key buys nothing.
pub const MAX_IDLE_PER_KEY: usize = 1;

/// Retained idle connections across all keys.
///
/// Bounds descriptors for a host with many configured upstreams without
/// introducing a global budget policy.
pub const MAX_IDLE_TOTAL: usize = 8;

/// Maximum age of an idle entry.
///
/// Close to the common peer idle-close interval, so a stale entry is evicted by
/// the lazy check at checkout/insert before its first use would fail. Evaluated
/// from the injected [`Clock`]; no timer task exists.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(10);

/// Concurrent outstanding queries on one reused connection.
///
/// Fixed at one: serial per connection is the settled policy, not a default.
pub const MAX_PENDING_PER_CONNECTION: usize = 1;

/// The injected time source, so idle expiry is deterministic under test.
///
/// This reuses the resolver's existing injected-clock trait rather than
/// introducing a second time abstraction: the reuse owner never reads the wall
/// clock directly.
pub use crate::resolver::{Clock, SystemClock};

// ---------------------------------------------------------------------------
// Reuse key
// ---------------------------------------------------------------------------

/// Which secure protocol a pooled connection speaks.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SecureKind {
    /// DNS-over-TLS.
    Dot,
    /// DNS-over-HTTPS.
    Doh,
}

/// The secure half of a [`ReuseKey`].
///
/// Every field that can change the trust or framing contract of an established
/// connection is represented here, so a connection authenticated for one
/// service is never handed to another.
///
/// The identity is stored as its normalized text rather than as a
/// [`ServerIdentity`] so the key is hashable: `ServerIdentity` is deliberately
/// not `Hash` in the reviewed endpoint contract, and hashing its canonical text
/// is equivalent for keying while keeping that type unchanged.
///
/// ## Trust policy is identified by mode *and* roots revision
///
/// `insecure_skip_verify` alone is not enough to identify a trust configuration:
/// it separates the insecure mode from the verified one, but two policies
/// verified against **different** root stores would compare equal and could then
/// share a connection. `roots_revision` closes that gap with the opaque ordinal
/// [`TlsPolicy::roots_revision`] mints, so a connection authenticated under one
/// trust configuration is never reused under another. No root material enters
/// the key.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct SecureKey {
    kind: SecureKind,
    identity: String,
    authority: Option<String>,
    insecure_skip_verify: bool,
    roots_revision: u64,
}

impl SecureKey {
    /// Builds a secure key from a service identity.
    ///
    /// The identity is normalized by [`ServerIdentity`], which never resolves
    /// it. Use [`ReuseKey::plain_secure_endpoint`] to receive a typed error for
    /// an unusable identity.
    ///
    /// This is the policy-agnostic constructor: it records only whether
    /// verification is skipped, and carries the no-roots revision. Owners that
    /// hold a real [`TlsPolicy`] use [`Self::from_policy`] instead, which also
    /// records that policy's roots revision.
    ///
    /// # Panics
    ///
    /// Panics when `identity` is not a valid DNS name or IP literal. Use
    /// [`Self::try_new`] for a fallible construction.
    #[must_use]
    pub fn new(
        kind: SecureKind,
        identity: &str,
        authority: Option<&str>,
        insecure_skip_verify: bool,
    ) -> Self {
        Self::try_new(kind, identity, authority, insecure_skip_verify)
            .expect("a secure key needs a valid service identity")
    }

    /// Builds a secure key, rejecting an unusable identity.
    ///
    /// See [`Self::new`] for the policy-agnostic semantics of this form.
    ///
    /// # Errors
    ///
    /// Returns [`UpstreamError::InvalidRequest`] when `identity` is neither a
    /// valid DNS name nor an IP literal.
    pub fn try_new(
        kind: SecureKind,
        identity: &str,
        authority: Option<&str>,
        insecure_skip_verify: bool,
    ) -> Result<Self, UpstreamError> {
        Self::try_new_with_revision(
            kind,
            identity,
            authority,
            insecure_skip_verify,
            NO_ROOTS_REVISION,
        )
    }

    /// Builds a secure key from a service identity and a real TLS policy.
    ///
    /// The policy supplies both the verification mode and its roots revision, so
    /// two policies verified against different root stores produce different
    /// keys and can never share a pooled connection. The revision is an opaque
    /// ordinal; no root material is copied into the key.
    ///
    /// # Panics
    ///
    /// Panics when `identity` is not a valid DNS name or IP literal.
    #[must_use]
    pub(crate) fn from_policy(
        kind: SecureKind,
        identity: &str,
        authority: Option<&str>,
        policy: &TlsPolicy,
    ) -> Self {
        Self::try_new_with_revision(
            kind,
            identity,
            authority,
            policy.is_insecure_skip_verify(),
            policy.roots_revision(),
        )
        .expect("a secure key needs a valid service identity")
    }

    fn try_new_with_revision(
        kind: SecureKind,
        identity: &str,
        authority: Option<&str>,
        insecure_skip_verify: bool,
        roots_revision: u64,
    ) -> Result<Self, UpstreamError> {
        let identity = ServerIdentity::new(identity)
            .map_err(|_| UpstreamError::InvalidRequest(RequestError::Malformed))?;
        Ok(Self {
            kind,
            identity: identity.as_str().to_owned(),
            authority: authority.map(str::to_owned),
            insecure_skip_verify,
            roots_revision,
        })
    }

    /// The secure protocol this key describes.
    #[must_use]
    pub const fn kind(&self) -> SecureKind {
        self.kind
    }

    /// The normalized TLS service identity text.
    #[must_use]
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// The DoH URL authority, when the service is HTTPS.
    #[must_use]
    pub fn authority(&self) -> Option<&str> {
        self.authority.as_deref()
    }

    /// Whether the connection was opened under an insecure-skip-verify policy.
    #[must_use]
    pub const fn is_insecure_skip_verify(&self) -> bool {
        self.insecure_skip_verify
    }
}

/// The identity of one reusable connection.
///
/// Built only from a **validated numeric endpoint** plus the secure identity
/// material, so:
///
/// * a hostname never appears here — a resolver refresh changes future dials
///   only, and an established connection stays valid for the address it was
///   opened to;
/// * the numeric dial is the destination, never the service identity;
/// * the negotiated protocol is part of the key, so an HTTP/2 connection is
///   never served to an HTTP/1.1 request.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ReuseKey {
    pub(crate) dial: SocketAddr,
    /// The transport discriminant, hashed locally so the reviewed [`Transport`]
    /// enum keeps its exact public shape.
    pub(crate) transport: TransportDiscriminant,
    pub(crate) secure: Option<SecureKey>,
    negotiated: Option<String>,
}

/// A hashable stand-in for [`Transport`].
///
/// `Transport` is part of the reviewed Phase 4 boundary and is not `Hash`, so
/// changing it would be an unnecessary API change. This local mirror carries the
/// same discrimination for keying purposes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TransportDiscriminant {
    Udp,
    Tcp,
}

impl From<Transport> for TransportDiscriminant {
    fn from(transport: Transport) -> Self {
        match transport {
            Transport::Udp => Self::Udp,
            Transport::Tcp => Self::Tcp,
        }
    }
}

impl ReuseKey {
    /// Builds a key for a validated numeric endpoint.
    #[must_use]
    pub fn from_endpoint(endpoint: Endpoint) -> Self {
        Self {
            dial: endpoint.address(),
            transport: endpoint.transport().into(),
            secure: None,
            negotiated: None,
        }
    }

    /// Builds a key for a validated numeric endpoint, rejecting UDP.
    ///
    /// UDP is connectionless, so there is no connection to reuse.
    ///
    /// # Errors
    ///
    /// Returns [`UpstreamError::InvalidRequest`] for a UDP endpoint.
    pub fn try_from_endpoint(endpoint: Endpoint) -> Result<Self, UpstreamError> {
        if endpoint.transport() == Transport::Udp {
            return Err(UpstreamError::InvalidRequest(RequestError::Malformed));
        }
        Ok(Self::from_endpoint(endpoint))
    }

    /// Builds a provisional key for a secure endpoint.
    ///
    /// The key is *provisional* until a handshake completes: the negotiated
    /// protocol is unknown before then, so the returned key carries none and is
    /// refined with [`Self::with_negotiated_protocol`] once the handshake
    /// reports one. A provisional key can only ever miss a lookup, which is the
    /// safe direction.
    ///
    /// # Errors
    ///
    /// Returns [`UpstreamError::InvalidRequest`] when the identity cannot be
    /// represented as a TLS service identity.
    pub fn plain_secure_endpoint(
        dial: SocketAddr,
        secure: SecureKey,
    ) -> Result<Self, UpstreamError> {
        let _ = secure.identity();
        Ok(Self {
            dial,
            transport: Transport::Tcp.into(),
            secure: Some(secure),
            negotiated: None,
        })
    }

    /// Refines this key with the protocol a handshake actually negotiated.
    #[must_use]
    pub fn with_negotiated_protocol(mut self, negotiated: Option<String>) -> Self {
        self.negotiated = negotiated;
        self
    }

    /// The numeric destination the connection was opened to.
    #[must_use]
    pub const fn dial(&self) -> SocketAddr {
        self.dial
    }

    /// The transport this connection speaks.
    #[must_use]
    pub const fn transport(&self) -> Transport {
        match self.transport {
            TransportDiscriminant::Udp => Transport::Udp,
            TransportDiscriminant::Tcp => Transport::Tcp,
        }
    }

    /// The secure half of the key, or `None` for plain TCP.
    #[must_use]
    pub const fn secure(&self) -> Option<&SecureKey> {
        self.secure.as_ref()
    }

    /// The protocol negotiated on the connection, if any.
    #[must_use]
    pub fn negotiated_protocol(&self) -> Option<&str> {
        self.negotiated.as_deref()
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// A typed connection-reuse failure.
///
/// These variants sit alongside the existing transport errors and never relabel
/// one: a pool failure is never reported as a protocol failure and vice versa.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PoolError {
    /// The key already has an outstanding exchange.
    ///
    /// The caller may use its own fresh connection; this owner never queues,
    /// because a queue would convert backpressure into latency.
    Busy,
    /// The owner is closing or closed.
    Closed,
    /// A connectionless endpoint cannot be pooled.
    NotReusable,
}

impl fmt::Display for PoolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Busy => "this key already has an outstanding exchange",
            Self::Closed => "the reuse owner is closed",
            Self::NotReusable => "a connectionless endpoint cannot be pooled",
        })
    }
}

impl std::error::Error for PoolError {}

impl From<PoolError> for UpstreamError {
    fn from(error: PoolError) -> Self {
        match error {
            // A busy key has written nothing, so the side-effect state is
            // `NotSent`. It is reported through the existing closed-style
            // transport error rather than a new transport variant, keeping the
            // transport error set closed.
            PoolError::Busy => Self::Runtime(SideEffectState::NotSent),
            PoolError::Closed => Self::Closed(SideEffectState::NotSent),
            PoolError::NotReusable => Self::InvalidRequest(RequestError::Malformed),
        }
    }
}

// ---------------------------------------------------------------------------
// Idle entries
// ---------------------------------------------------------------------------

/// A connection that is idle and available for reuse.
#[derive(Debug)]
struct IdleConnection {
    stream: TcpStream,
    idle_since: Instant,
}

/// The owner's short-lock state.
#[derive(Debug, Default)]
struct PoolInner {
    idle: HashMap<ReuseKey, IdleConnection>,
    /// Connections currently leased to a caller.
    leased: usize,
    closed: bool,
}

// ---------------------------------------------------------------------------
// Reuse owner
// ---------------------------------------------------------------------------

/// Owns the reusable connections for one numeric endpoint.
///
/// ## Serial per connection
///
/// A reused connection carries **at most one** outstanding query
/// ([`MAX_PENDING_PER_CONNECTION`]). RFC 7766 §7 permits out-of-order responses,
/// so serving several outstanding queries on one connection would require
/// demultiplexing replies back to the right caller; doing that by the caller's
/// original ID requires pairwise-distinct IDs, and doing it otherwise would
/// require rewriting IDs. Rewriting is unavailable: [`ExchangeRequest`] records
/// the caller's original ID and never rewrites the caller's bytes. So this owner
/// is serial, which is provably unambiguous, and a second concurrent request for
/// a busy key is rejected with [`PoolError::Busy`] rather than queued.
///
/// ## Deadline, cancellation, and close
///
/// Every exchange uses the caller's original [`ExchangeContext`] unchanged, so a
/// reuse hit never extends a caller's budget. Owner close transitions through the
/// existing [`Lifecycle`], refuses further checkouts, waits for every leased
/// connection to return, and drops every idle entry. A connection returning
/// after `Closing` began is discarded, never re-inserted.
///
/// ## Rebuild policy
///
/// A retained connection that the peer has already closed is detected by a
/// non-destructive liveness probe **before** any DNS byte is written. That is
/// the `NotSent` case and the only case in which this owner dials a replacement
/// — once. Any failure after the query was written is terminal: the bytes may
/// have reached the peer, so retrying could double-send it.
pub struct ReuseOwner {
    endpoint: Endpoint,
    lifecycle: Arc<Lifecycle>,
    cancellation: TransportCancellation,
    inner: Arc<Mutex<PoolInner>>,
    clock: Arc<dyn Clock>,
}

impl fmt::Debug for ReuseOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ReuseOwner")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl ReuseOwner {
    /// Creates an owner for one validated numeric endpoint.
    ///
    /// # Panics
    ///
    /// Panics for a UDP endpoint, which has no connection to reuse. Use
    /// [`Self::try_new`] for a fallible construction.
    #[must_use]
    pub fn new(endpoint: Endpoint) -> Self {
        Self::try_new(endpoint).expect("a UDP endpoint has no reusable connection")
    }

    /// Creates an owner for one validated numeric endpoint.
    ///
    /// # Errors
    ///
    /// Returns [`PoolError::NotReusable`] for a UDP endpoint.
    pub fn try_new(endpoint: Endpoint) -> Result<Self, PoolError> {
        if endpoint.transport() == Transport::Udp {
            return Err(PoolError::NotReusable);
        }
        Ok(Self {
            endpoint,
            lifecycle: Arc::new(Lifecycle::new()),
            cancellation: TransportCancellation::new(),
            inner: Arc::new(Mutex::new(PoolInner::default())),
            clock: Arc::new(SystemClock),
        })
    }

    /// Installs an injected clock so idle expiry is deterministic under test.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// The numeric endpoint this owner reuses connections for.
    #[must_use]
    pub const fn endpoint(&self) -> Endpoint {
        self.endpoint
    }

    /// The owner lifecycle state.
    #[must_use]
    pub fn lifecycle_state(&self) -> LifecycleState {
        self.lifecycle.state()
    }

    /// The number of connections currently leased to callers.
    #[must_use]
    pub fn leased_connections(&self) -> usize {
        self.lock().leased
    }

    /// The number of idle (reusable) connections retained.
    #[must_use]
    pub fn idle_connections(&self) -> usize {
        self.lock().idle.len()
    }

    /// The number of in-flight exchanges registered with the lifecycle.
    #[must_use]
    pub fn in_flight_exchanges(&self) -> usize {
        self.lifecycle.in_flight()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, PoolInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Begins owner shutdown and cancels owned work.
    #[must_use]
    pub fn begin_close(&self) -> CloseTransition {
        let transition = self.lifecycle.begin_close();
        if transition == CloseTransition::BeganClosing {
            self.cancellation.cancel();
            // Idle connections carry no registration, so nothing else will
            // release them: drop them here.
            self.lock().idle.clear();
        }
        transition
    }

    /// Retains a connection for `key` through the production release path.
    ///
    /// This is the in-crate seam the pool-bound tests use to drive more than one
    /// key, which the single-key public owner API cannot express. It calls the
    /// same `release` the exchange path calls, so the bounds under test are the
    /// real ones.
    #[cfg(test)]
    fn release_for_test(&self, key: ReuseKey, stream: TcpStream, reusable: bool) {
        self.release(key, stream, reusable);
    }

    /// Takes a live idle connection for `key` through the production checkout.
    #[cfg(test)]
    fn checkout_for_test(&self, key: &ReuseKey) -> Option<TcpStream> {
        self.checkout_live(key)
    }

    /// Begins close, drains every leased connection, then completes shutdown.
    ///
    /// Idempotent and convergent: repeated calls reach the same `Closed` owner.
    pub async fn close(&self) -> CloseResult {
        match self.begin_close() {
            CloseTransition::AlreadyClosed => return CloseResult::AlreadyClosed,
            CloseTransition::BeganClosing | CloseTransition::AlreadyClosing => {}
        }
        self.lifecycle.drain().await;
        self.lock().closed = true;
        match self.lifecycle.finish_close() {
            CloseCompletion::Closed | CloseCompletion::AlreadyClosed => CloseResult::Closed,
            CloseCompletion::NotClosing | CloseCompletion::InFlight => CloseResult::AlreadyClosing,
        }
    }

    /// Claims the single serial slot for the plain-TCP key of this endpoint.
    ///
    /// Returns `None` when the key is already busy or the owner is closed; the
    /// caller maps that to a typed error. No queue is involved.
    fn admit_lease(&self) -> Option<LeaseGuard> {
        let mut inner = self.lock();
        if inner.closed || self.lifecycle.state() != LifecycleState::Open {
            return None;
        }
        if inner.leased >= MAX_PENDING_PER_CONNECTION {
            return None;
        }
        inner.leased += 1;
        Some(LeaseGuard {
            inner: Arc::clone(&self.inner),
        })
    }

    /// Takes a live idle connection for `key`, discarding a dead or stale one.
    ///
    /// An entry older than [`IDLE_TIMEOUT`] is dropped. A retained connection
    /// the peer has already closed is detected here by a non-destructive
    /// readiness probe, before any DNS byte is written, so the caller can safely
    /// dial a replacement.
    fn checkout_live(&self, key: &ReuseKey) -> Option<TcpStream> {
        let now = self.clock.now();
        let entry = {
            let mut inner = self.lock();
            inner.idle.remove(key)?
        };
        if now.saturating_duration_since(entry.idle_since) >= IDLE_TIMEOUT {
            return None;
        }
        if is_peer_closed(&entry.stream) {
            return None;
        }
        Some(entry.stream)
    }

    /// Returns a completed connection to the idle set.
    ///
    /// A connection is retained only when the exchange completed cleanly, the
    /// owner is still `Open`, and the pool is not closed — so a closed pool can
    /// never be repopulated.
    fn release(&self, key: ReuseKey, stream: TcpStream, reusable: bool) {
        let now = self.clock.now();
        let mut inner = self.lock();
        if inner.leased > 0 {
            inner.leased -= 1;
        }
        if !reusable || inner.closed || self.lifecycle.state() != LifecycleState::Open {
            return;
        }
        // Per-key bound: `MAX_IDLE_PER_KEY` is one, so a newer entry replaces
        // the older one for this key.
        if !inner.idle.contains_key(&key) {
            // Global bound: evict the least-recently-returned entry when full.
            if inner.idle.len() >= MAX_IDLE_TOTAL {
                if let Some(oldest) = inner
                    .idle
                    .iter()
                    .min_by_key(|(_, entry)| entry.idle_since)
                    .map(|(key, _)| key.clone())
                {
                    inner.idle.remove(&oldest);
                }
            }
        }
        inner.idle.insert(
            key,
            IdleConnection {
                stream,
                idle_since: now,
            },
        );
    }

    /// Runs one exchange, reusing an idle connection for this endpoint's key.
    ///
    /// # Errors
    ///
    /// Returns the existing typed [`UpstreamError`] for every transport outcome.
    /// A busy key is [`UpstreamError::Runtime`] with `NotSent`; a closing owner
    /// is [`UpstreamError::Closed`].
    pub async fn exchange(
        &self,
        request: ExchangeRequest<'_>,
        context: ExchangeContext,
    ) -> Result<ExchangeResponse, UpstreamError> {
        // Registration is the admission step: it is serialized with
        // `Open -> Closing`, so close can never observe a zero count while this
        // exchange is admitting itself.
        let _in_flight = self.lifecycle.register()?;

        let key = ReuseKey::from_endpoint(self.endpoint);
        let request_id = request.request_id();
        let query = request.query();
        let control = ExchangeControl::new(context, self.cancellation.clone());

        control.check_at(Instant::now(), SideEffectState::NotSent)?;

        // Serial admission for this key: at most one outstanding exchange.
        let lease = self
            .admit_lease()
            .ok_or_else(|| UpstreamError::from(PoolError::Busy))?;

        // Try a retained connection first. `checkout_live` has already rejected a
        // dead or stale entry, so anything returned here is worth a write.
        if let Some(stream) = self.checkout_live(&key) {
            match self.run_framed(stream, query, request_id, &control).await {
                Attempt::Success(response, stream) => {
                    self.release(key, stream, true);
                    drop(lease);
                    return Ok(response);
                }
                // The peer closed between the probe and the write, and no DNS
                // byte was written: replacing the connection cannot double-send.
                Attempt::Unwritten(stream, _error) => drop(stream),
                // Terminal: the connection may be in an unknown state, so it is
                // discarded rather than returned to the idle set.
                Attempt::Terminal(stream, error) => {
                    drop(stream);
                    drop(lease);
                    return Err(error);
                }
            }
        }

        // Dial fresh. This is either the first exchange for the key or the single
        // permitted replacement after an unwritten idle-connection failure.
        let stream = race_io(
            &control,
            SideEffectState::NotSent,
            control.context().deadline(),
            async {
                TcpStream::connect(self.endpoint.address())
                    .await
                    .map_err(|_| UpstreamError::Connect)
            },
        )
        .await?;

        // Connect is an async wake: re-apply the full control before writing.
        // Nothing has been written, so this is still `NotSent`.
        control.check_at(Instant::now(), SideEffectState::NotSent)?;

        match self.run_framed(stream, query, request_id, &control).await {
            Attempt::Success(response, stream) => {
                self.release(key, stream, true);
                drop(lease);
                Ok(response)
            }
            // A fresh connection is never rebuilt: its connect either succeeded
            // (so a failure is terminal and may have sent bytes) or it did not.
            Attempt::Unwritten(stream, error) => {
                drop(stream);
                drop(lease);
                Err(error)
            }
            Attempt::Terminal(stream, error) => {
                drop(stream);
                drop(lease);
                Err(error)
            }
        }
    }

    /// Writes one framed query and reads one framed, validated response.
    ///
    /// All framing and the control race come from [`crate::tcp`], so this module
    /// adds no second framing implementation and no second control state
    /// machine.
    async fn run_framed(
        &self,
        mut stream: TcpStream,
        query: &[u8],
        request_id: u16,
        control: &ExchangeControl,
    ) -> Attempt {
        let deadline = control.context().deadline();

        // The write races the same absolute deadline plus owner close and caller
        // cancellation. `write_frame` maps a short write to `Send(MaybeSent)`
        // because part of the frame may already have been accepted.
        if let Err(error) = race_io(control, SideEffectState::MaybeSent, deadline, async {
            crate::tcp::write_frame(&mut stream, query).await
        })
        .await
        {
            // A `NotSent` write failure provably left no byte with the peer.
            if matches!(error, UpstreamError::Send(SideEffectState::NotSent)) {
                return Attempt::Unwritten(stream, error);
            }
            return Attempt::Terminal(Some(stream), error);
        }

        let body = match race_io(control, SideEffectState::Sent, deadline, async {
            crate::tcp::read_frame(&mut stream).await
        })
        .await
        {
            Ok(body) => body,
            Err(error) => return Attempt::Terminal(Some(stream), error),
        };

        // A response shorter than the header or with QR clear cannot be
        // attributed to this exchange as a response.
        let header = match mosdns_dns_core::inspect_response_header(&body) {
            Ok(header) => header,
            Err(_) => {
                return Attempt::Terminal(Some(stream), UpstreamError::MalformedResponse);
            }
        };
        // The accepted response must answer this request, preserving the
        // caller's original DNS ID.
        if header.id != request_id {
            return Attempt::Terminal(Some(stream), UpstreamError::ResponseMismatch);
        }
        // Only a complete, dns-core-valid response may be returned.
        if mosdns_dns_core::validate_response(&body).is_err() {
            return Attempt::Terminal(Some(stream), UpstreamError::MalformedResponse);
        }

        // The single control-aware commit point, shared with the existing
        // transports: owner close first, then caller cancellation, then the
        // original absolute deadline, then success. It starts no timer and
        // resets no deadline.
        let caller_cancellation = control.caller_cancellation();
        if let Err(error) = self.lifecycle.commit_final_response(
            &caller_cancellation,
            deadline,
            SideEffectState::Sent,
        ) {
            return Attempt::Terminal(Some(stream), error);
        }

        Attempt::Success(
            ExchangeResponse::new(body, request_id, header.id, Transport::Tcp, false),
            stream,
        )
    }
}

/// The outcome of one framed attempt, carrying the stream so the owner decides
/// whether to retain it.
enum Attempt {
    /// The exchange completed and the connection may be retained.
    Success(ExchangeResponse, TcpStream),
    /// The attempt failed with no DNS byte written, so one replacement dial is
    /// allowed.
    Unwritten(TcpStream, UpstreamError),
    /// The attempt failed terminally; the connection must be discarded.
    Terminal(Option<TcpStream>, UpstreamError),
}

/// Reports whether the peer has already closed this connection.
///
/// A half-closed idle connection cannot be detected by writing: the local send
/// may be accepted into the socket buffer and only the read then fails, which
/// would look like a sent-then-failed exchange. This probe instead asks the
/// socket, without consuming DNS data and without blocking, whether it is
/// readable-at-EOF:
///
/// * `Ok(0)` is end-of-stream, so the peer is gone;
/// * `WouldBlock` means no data is pending, which is the healthy idle case;
/// * any other result (unexpected bytes on a serial connection, or a failure)
///   means the connection is not trustworthy for a fresh query.
///
/// The probe is deterministic and takes no timeout, so no wall-clock wait is
/// involved.
fn is_peer_closed(stream: &TcpStream) -> bool {
    let mut probe = [0u8; 1];
    match stream.try_read(&mut probe) {
        // EOF: the peer closed the connection.
        Ok(0) => true,
        // Unsolicited bytes on a connection with no outstanding query: this
        // owner never produces them, so the stream is not in a known-good state.
        Ok(_) => true,
        Err(error) => error.kind() != std::io::ErrorKind::WouldBlock,
    }
}

/// RAII lease over one outstanding exchange.
///
/// Dropping the guard always releases the slot, so an aborted caller future
/// cannot leak one.
struct LeaseGuard {
    inner: Arc<Mutex<PoolInner>>,
}
impl Drop for LeaseGuard {
    fn drop(&mut self) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if inner.leased > 0 {
            inner.leased -= 1;
        }
    }
}

/// Races one I/O future against owner close, caller cancellation, and the
/// exchange's single absolute deadline.
///
/// A thin wrapper over the reviewed [`crate::tcp::race_control`] helper, so the
/// crate keeps exactly one control-race implementation.
async fn race_io<F, T>(
    control: &ExchangeControl,
    side_effect: SideEffectState,
    deadline: Instant,
    io: F,
) -> Result<T, UpstreamError>
where
    F: std::future::Future<Output = Result<T, UpstreamError>>,
{
    crate::tcp::race_control(control, side_effect, deadline, io).await
}

// ---------------------------------------------------------------------------
// Secure reuse owner (DoT)
// ---------------------------------------------------------------------------

/// Owns the reusable, authenticated `DoT` session for one endpoint.
///
/// ## Identity is part of the connection, not decoration
///
/// A pooled `DoT` session has completed a TLS handshake against **one** service
/// identity. Reuse therefore keys on that identity, the endpoint's numeric dial
/// address, and the TLS policy discriminant, so a session authenticated as one
/// service is never handed to another. The owner holds exactly one endpoint, so
/// the key is fixed for its lifetime; this is the same single-key shape as
/// [`ReuseOwner`] and keeps the cache key explicit rather than global.
///
/// `DoT` advertises no ALPN (RFC 7858 defines none), so the negotiated-protocol
/// component of the key is not used on this path — the handshake itself is the
/// only protocol negotiation that exists.
///
/// ## Serial, one deadline, and close
///
/// One session carries at most one outstanding exchange
/// ([`MAX_PENDING_PER_CONNECTION`]), exhausted queries are never demultiplexed,
/// and every exchange uses the caller's original absolute deadline unchanged. A
/// session the peer has already closed is detected by a non-destructive probe
/// before any DNS byte is written; only then does the owner dial a replacement,
/// once. `close()` drops idle sessions and drains registrations.
pub struct SecureReuseOwner {
    endpoint: DotEndpoint,
    tls: TlsPolicy,
    lifecycle: Arc<Lifecycle>,
    cancellation: TransportCancellation,
    inner: Arc<Mutex<SecurePoolInner>>,
    clock: Arc<dyn Clock>,
    /// Deterministic parking seam immediately before this owner's pooled-response
    /// commit gate. Compiled for tests only; see [`CommitPause`].
    #[cfg(test)]
    commit_pause: Mutex<Option<Arc<CommitPause>>>,
}

impl fmt::Debug for SecureReuseOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecureReuseOwner")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

/// The secure owner's short-lock state.
struct SecurePoolInner {
    idle: Option<crate::secure::PooledDotSession>,
    idle_since: Instant,
    leased: usize,
    closed: bool,
}

impl fmt::Debug for SecurePoolInner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("SecurePoolInner")
            .field("has_idle", &self.idle.is_some())
            .field("leased", &self.leased)
            .field("closed", &self.closed)
            .finish()
    }
}

impl SecureReuseOwner {
    /// Creates an owner for one validated secure endpoint and policy.
    ///
    /// The policy is validated here, exactly as [`crate::secure::DotUpstream`]
    /// does, so an unusable policy is rejected before any exchange rather than
    /// silently accepted.
    ///
    /// # Errors
    ///
    /// Returns the typed [`SecureError::TlsConfig`] when the policy cannot
    /// produce a usable client configuration.
    pub fn new(endpoint: DotEndpoint, tls: TlsPolicy) -> Result<Self, SecureError> {
        tls.client_config()?;
        Ok(Self {
            endpoint,
            tls,
            lifecycle: Arc::new(Lifecycle::new()),
            cancellation: TransportCancellation::new(),
            inner: Arc::new(Mutex::new(SecurePoolInner {
                idle: None,
                idle_since: Instant::now(),
                leased: 0,
                closed: false,
            })),
            clock: Arc::new(SystemClock),
            #[cfg(test)]
            commit_pause: Mutex::new(None),
        })
    }

    /// Installs an injected clock so idle expiry is deterministic under test.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// The secure endpoint this owner reuses sessions for.
    #[must_use]
    pub const fn endpoint(&self) -> &DotEndpoint {
        &self.endpoint
    }

    /// The reuse key for this owner's fixed endpoint and policy.
    ///
    /// The key carries the numeric dial, the secure kind, the normalized service
    /// identity, and the TLS policy discriminant — never a hostname in place of
    /// the dial address, and never the resolver snapshot.
    ///
    /// # Errors
    ///
    /// Returns the typed error when the identity cannot form a key, which cannot
    /// happen for an endpoint that was already validated.
    pub fn reuse_key(&self) -> Result<ReuseKey, UpstreamError> {
        ReuseKey::plain_secure_endpoint(
            self.endpoint.dial(),
            SecureKey::from_policy(
                SecureKind::Dot,
                self.endpoint.identity().as_str(),
                None,
                &self.tls,
            ),
        )
    }

    /// The owner lifecycle state.
    #[must_use]
    pub fn lifecycle_state(&self) -> LifecycleState {
        self.lifecycle.state()
    }

    /// The number of authenticated sessions currently leased to callers.
    #[must_use]
    pub fn leased_connections(&self) -> usize {
        self.secure_lock().leased
    }

    /// Whether an idle authenticated session is retained.
    #[must_use]
    pub fn idle_connections(&self) -> usize {
        usize::from(self.secure_lock().idle.is_some())
    }

    /// The number of in-flight exchanges registered with the lifecycle.
    #[must_use]
    pub fn in_flight_exchanges(&self) -> usize {
        self.lifecycle.in_flight()
    }

    fn secure_lock(&self) -> std::sync::MutexGuard<'_, SecurePoolInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Begins owner shutdown and cancels owned work.
    #[must_use]
    pub fn begin_close(&self) -> CloseTransition {
        let transition = self.lifecycle.begin_close();
        if transition == CloseTransition::BeganClosing {
            self.cancellation.cancel();
            // The idle session carries no registration, so nothing else will
            // release it.
            self.secure_lock().idle = None;
        }
        transition
    }

    /// Begins close, drains every leased session, then completes shutdown.
    ///
    /// Idempotent and convergent.
    pub async fn close(&self) -> CloseResult {
        match self.begin_close() {
            CloseTransition::AlreadyClosed => return CloseResult::AlreadyClosed,
            CloseTransition::BeganClosing | CloseTransition::AlreadyClosing => {}
        }
        self.lifecycle.drain().await;
        self.secure_lock().closed = true;
        match self.lifecycle.finish_close() {
            CloseCompletion::Closed | CloseCompletion::AlreadyClosed => CloseResult::Closed,
            CloseCompletion::NotClosing | CloseCompletion::InFlight => CloseResult::AlreadyClosing,
        }
    }

    /// Claims the single serial slot.
    fn admit_lease(&self) -> Option<SecureLeaseGuard> {
        let mut inner = self.secure_lock();
        if inner.closed || self.lifecycle.state() != LifecycleState::Open {
            return None;
        }
        if inner.leased >= MAX_PENDING_PER_CONNECTION {
            return None;
        }
        inner.leased += 1;
        Some(SecureLeaseGuard {
            inner: Arc::clone(&self.inner),
        })
    }

    /// Takes a live idle session, discarding a dead or stale one.
    fn checkout_live(&self) -> Option<crate::secure::PooledDotSession> {
        let session = {
            let mut inner = self.secure_lock();
            inner.idle.take()?
        };
        let idle_since = self.secure_lock().idle_since;
        if self.clock.now().saturating_duration_since(idle_since) >= IDLE_TIMEOUT {
            return None;
        }
        if session.is_peer_closed() {
            return None;
        }
        Some(session)
    }

    /// Retains a completed session, unless the owner may no longer reuse it.
    fn release(&self, session: crate::secure::PooledDotSession) {
        let now = self.clock.now();
        let mut inner = self.secure_lock();
        if inner.leased > 0 {
            inner.leased -= 1;
        }
        if inner.closed || self.lifecycle.state() != LifecycleState::Open {
            return;
        }
        inner.idle = Some(session);
        inner.idle_since = now;
    }

    /// The final commit linearization point for one pooled `DoT` response.
    ///
    /// A pooled session validates its response but does not own an owner
    /// lifecycle, so the owner must apply the same commit the fresh path applies
    /// ([`Lifecycle::commit_final_response`]) **before** the session is returned
    /// to the pool. Without it, an owner close, caller cancellation, or deadline
    /// landing between response validation and the owner's return would let a
    /// pooled exchange report success where a fresh one would fail — the exact
    /// window the final-commit contract exists to close.
    ///
    /// The request has already been sent by this point, so the side effect is
    /// `Sent`. On failure the caller must **discard** the session rather than
    /// re-pool it, because the response was never committed.
    fn commit_pooled_response(
        &self,
        control: &ExchangeControl,
        deadline: Instant,
    ) -> Result<(), SecureError> {
        self.lifecycle
            .commit_final_response(
                &control.caller_cancellation(),
                deadline,
                SideEffectState::Sent,
            )
            .map_err(SecureError::from)
    }

    #[cfg(test)]
    fn install_commit_pause(&self, pause: Arc<CommitPause>) {
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

    /// Runs one authenticated `DoT` exchange, reusing an idle session.
    ///
    /// # Errors
    ///
    /// Returns the existing typed [`SecureError`] for every transport outcome,
    /// with the same `NotSent`/`Sent` classification as the fresh DoT path.
    pub async fn exchange(
        &self,
        request: ExchangeRequest<'_>,
        context: ExchangeContext,
    ) -> Result<SecureResponse, SecureError> {
        let _in_flight = self.lifecycle.register()?;
        let control = ExchangeControl::new(context, self.cancellation.clone());
        let deadline = control.context().deadline();

        control.check_at(Instant::now(), SideEffectState::NotSent)?;

        let lease = self
            .admit_lease()
            .ok_or_else(|| SecureError::from(UpstreamError::from(PoolError::Busy)))?;

        // Try a retained session first. `checkout_live` already rejected a dead
        // or stale one, so anything returned here is worth a query.
        if let Some(session) = self.checkout_live() {
            match session.exchange(&request, &control, deadline).await {
                Ok((response, session)) => {
                    // Commit before re-pooling: a close/cancel/deadline that
                    // landed during the exchange must still fail this response,
                    // and a failed commit must never leave the session reusable.
                    #[cfg(test)]
                    self.reach_commit_gate().await;
                    if let Err(error) = self.commit_pooled_response(&control, deadline) {
                        drop(session);
                        drop(lease);
                        return Err(error);
                    }
                    self.release(session);
                    drop(lease);
                    return Ok(response);
                }
                Err(outcome) => {
                    if !outcome.rebuildable {
                        // Terminal: the session is discarded, never re-pooled.
                        drop(outcome.session);
                        drop(lease);
                        return Err(outcome.error);
                    }
                    // Unwritten: replacing the session cannot double-send.
                    drop(outcome.session);
                }
            }
        }

        // Dial and authenticate a fresh session. This is either the first
        // exchange or the single permitted replacement.
        let session =
            crate::secure::PooledDotSession::connect(&self.endpoint, &self.tls, &control, deadline)
                .await?;

        // Connect/handshake are an async wake: re-apply the full control before
        // the first write. No DNS byte has been sent, so this is `NotSent`.
        control.check_at(Instant::now(), SideEffectState::NotSent)?;

        match session.exchange(&request, &control, deadline).await {
            Ok((response, session)) => {
                // Same commit as the reused path: the first pooled exchange has
                // exactly the same final-commit obligation.
                #[cfg(test)]
                self.reach_commit_gate().await;
                if let Err(error) = self.commit_pooled_response(&control, deadline) {
                    drop(session);
                    drop(lease);
                    return Err(error);
                }
                self.release(session);
                drop(lease);
                Ok(response)
            }
            Err(outcome) => {
                drop(outcome.session);
                drop(lease);
                Err(outcome.error)
            }
        }
    }
}

/// RAII lease over one outstanding secure exchange.
struct SecureLeaseGuard {
    inner: Arc<Mutex<SecurePoolInner>>,
}

impl Drop for SecureLeaseGuard {
    fn drop(&mut self) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if inner.leased > 0 {
            inner.leased -= 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Secure reuse owner (DoH)
// ---------------------------------------------------------------------------

/// Owns the reusable, authenticated `DoH` session for one endpoint.
///
/// ## Identity and protocol are part of the session
///
/// A pooled `DoH` session completed a TLS handshake against one service URL
/// identity, negotiated one HTTP version, and sent requests to one authority and
/// path. The reuse key therefore carries the numeric dial, the service identity,
/// the authority, the TLS-policy discriminant, and the negotiated protocol, so a
/// session established for one service is never handed to another. Because the
/// owner holds exactly one endpoint, the key is fixed for its lifetime.
///
/// A checkout compares that key **without** the negotiated protocol, because the
/// retained session itself is the authority on which protocol it speaks. It
/// re-stamps the key with the protocol the session actually negotiated when it
/// returns the session to the pool. A mismatch can therefore only cause a miss,
/// which is the safe direction: an HTTP/1.1 session can never serve a request
/// expecting HTTP/2 or the reverse.
///
/// ## Both negotiated protocols are pooled
///
/// A session is retained and reused whichever HTTP version it negotiated.
/// HTTP/1.1 retains its `SendRequest` sender plus its connection driver.
/// HTTP/2 retains the sender, the driver, **and** the crate-internal
/// `H2ScopeLease` whose executor Hyper dispatches its children to.
///
/// Two details make the HTTP/2 case work:
///
/// * The pooled scope holds **no owner registration** and **no caller token**.
///   Holding a registration for the session's whole life would keep the owner
///   permanently non-drained so `close()` could never converge, and capturing the
///   opening request's caller token would tie the long-lived driver to that one
///   caller — cancelling it later would abort the driver and break every
///   subsequent reuse.
/// * There is no between-exchange settle step. The connection driver is itself a
///   tracked child that lives as long as the session is usable, so the tracked
///   child count never reaches zero while the session remains reusable; the scope
///   is sealed, and its children aborted, only when the session is dropped or
///   discarded.
///
/// Reuse on a retained HTTP/2 session is an ordinary new stream on the same
/// connection, so the accept count stays at one.
///
/// ## Serial, one deadline, and close
///
/// One session carries at most one outstanding exchange
/// ([`MAX_PENDING_PER_CONNECTION`]); there is no pending map and no
/// demultiplexing. Every exchange uses the caller's original absolute deadline
/// unchanged, and `close()` drops the idle session and drains registrations.
pub struct DohReuseOwner {
    endpoint: DohEndpoint,
    tls: TlsPolicy,
    lifecycle: Arc<Lifecycle>,
    cancellation: TransportCancellation,
    inner: Arc<Mutex<DohPoolInner>>,
    clock: Arc<dyn Clock>,
    /// Deterministic parking seam immediately before this owner's pooled-response
    /// commit gate. Compiled for tests only; see [`CommitPause`].
    #[cfg(test)]
    commit_pause: Mutex<Option<Arc<CommitPause>>>,
}

impl fmt::Debug for DohReuseOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DohReuseOwner")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

/// The `DoH` owner's short-lock state.
///
/// The idle entry stores the session and the key it was established under, so a
/// reuse can verify the negotiated protocol matches before handing it out.
struct DohPoolInner {
    idle: Option<(ReuseKey, crate::secure::PooledDohSession)>,
    idle_since: Instant,
    leased: usize,
    closed: bool,
}

impl fmt::Debug for DohPoolInner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DohPoolInner")
            .field("has_idle", &self.idle.is_some())
            .field("leased", &self.leased)
            .field("closed", &self.closed)
            .finish()
    }
}

impl DohReuseOwner {
    /// Creates an owner for one validated `DoH` endpoint and policy.
    ///
    /// # Errors
    ///
    /// Returns [`SecureError::TlsConfig`] when the policy cannot produce a usable
    /// client configuration.
    pub fn new(endpoint: DohEndpoint, tls: TlsPolicy) -> Result<Self, SecureError> {
        tls.client_config()?;
        Ok(Self {
            endpoint,
            tls,
            lifecycle: Arc::new(Lifecycle::new()),
            cancellation: TransportCancellation::new(),
            inner: Arc::new(Mutex::new(DohPoolInner {
                idle: None,
                idle_since: Instant::now(),
                leased: 0,
                closed: false,
            })),
            clock: Arc::new(SystemClock),
            #[cfg(test)]
            commit_pause: Mutex::new(None),
        })
    }

    /// Installs an injected clock so idle expiry is deterministic under test.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// The endpoint this owner reuses sessions for.
    #[must_use]
    pub const fn endpoint(&self) -> &DohEndpoint {
        &self.endpoint
    }

    /// The owner lifecycle state.
    #[must_use]
    pub fn lifecycle_state(&self) -> LifecycleState {
        self.lifecycle.state()
    }

    /// Whether an idle authenticated session is retained.
    #[must_use]
    pub fn idle_connections(&self) -> usize {
        usize::from(self.doh_lock().idle.is_some())
    }

    /// The number of in-flight exchanges registered with the lifecycle.
    #[must_use]
    pub fn in_flight_exchanges(&self) -> usize {
        self.lifecycle.in_flight()
    }

    fn doh_lock(&self) -> std::sync::MutexGuard<'_, DohPoolInner> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Begins owner shutdown and cancels owned work.
    ///
    /// The retained session is deliberately **not** dropped here. Dropping can
    /// only seal and abort an HTTP/2 scope; it cannot wait for the tracked
    /// children to finish, and `begin_close` is synchronous. The session is
    /// therefore left in place for [`Self::close`], which awaits its explicit
    /// teardown. Lease admission is already refused once the lifecycle leaves
    /// `Open`, so a session parked here can never be handed out again.
    #[must_use]
    pub fn begin_close(&self) -> CloseTransition {
        let transition = self.lifecycle.begin_close();
        if transition == CloseTransition::BeganClosing {
            self.cancellation.cancel();
        }
        transition
    }

    /// Begins close, drains every leased session, then completes shutdown.
    ///
    /// Close does not merely drop the retained session: a pooled HTTP/2 session
    /// owns tracked child futures, and the design requires that no tracked child
    /// outlives its connection and that close returns `Closed` only after
    /// draining has finished. `begin_close` leaves the idle session parked
    /// because it is synchronous; this takes that session and awaits its
    /// explicit teardown. A session still leased when close begins discards
    /// itself on return and awaits the same teardown before its guard drops, and
    /// `drain()` above waits for that guard — so `Closed` is only reachable once
    /// the children are gone.
    pub async fn close(&self) -> CloseResult {
        match self.begin_close() {
            CloseTransition::AlreadyClosed => return CloseResult::AlreadyClosed,
            CloseTransition::BeganClosing | CloseTransition::AlreadyClosing => {}
        }
        self.lifecycle.drain().await;

        // Await the retained session's teardown outside the lifecycle gate.
        let idle = self.doh_lock().idle.take();
        if let Some((_key, session)) = idle {
            session.shutdown().await;
        }

        self.doh_lock().closed = true;
        match self.lifecycle.finish_close() {
            CloseCompletion::Closed | CloseCompletion::AlreadyClosed => CloseResult::Closed,
            CloseCompletion::NotClosing | CloseCompletion::InFlight => CloseResult::AlreadyClosing,
        }
    }

    /// Claims the single serial slot.
    fn admit_lease(&self) -> Option<DohLeaseGuard> {
        let mut inner = self.doh_lock();
        if inner.closed || self.lifecycle.state() != LifecycleState::Open {
            return None;
        }
        if inner.leased >= MAX_PENDING_PER_CONNECTION {
            return None;
        }
        inner.leased += 1;
        Some(DohLeaseGuard {
            inner: Arc::clone(&self.inner),
        })
    }

    /// Takes a live idle session serving the same service as `base`.
    ///
    /// The comparison deliberately covers the numeric dial, the transport, and
    /// the whole secure key (kind, identity, authority, TLS-policy discriminant)
    /// but **not** the negotiated protocol: the connection's protocol is a
    /// property of the connection, and the retained session itself is the
    /// authority on which protocol it speaks. A session established for a
    /// different service can therefore never be handed out.
    async fn checkout_live(&self, base: &ReuseKey) -> Option<crate::secure::PooledDohSession> {
        let (stored_key, mut session) = {
            let mut inner = self.doh_lock();
            inner.idle.take()?
        };
        let idle_since = self.doh_lock().idle_since;
        if self.clock.now().saturating_duration_since(idle_since) >= IDLE_TIMEOUT {
            session.shutdown().await;
            return None;
        }
        let same_service = stored_key.dial == base.dial
            && stored_key.transport == base.transport
            && stored_key.secure == base.secure;
        if !same_service || session.is_closed() {
            session.shutdown().await;
            return None;
        }
        Some(session)
    }

    /// Retains a completed session for a later exchange.
    ///
    /// Both HTTP/1.1 and HTTP/2 sessions are retained. A pooled HTTP/2 session
    /// owns its own child-tracking scope and holds no owner registration, so
    /// retaining it does not prevent `close()` from draining.
    async fn release(&self, key: ReuseKey, session: crate::secure::PooledDohSession) {
        let now = self.clock.now();
        let mut session = Some(session);
        let discard = {
            let mut inner = self.doh_lock();
            if inner.leased > 0 {
                inner.leased -= 1;
            }
            if inner.closed || self.lifecycle.state() != LifecycleState::Open {
                true
            } else {
                inner.idle = Some((
                    key,
                    session
                        .take()
                        .expect("the session is present before retention"),
                ));
                inner.idle_since = now;
                false
            }
        };
        if discard {
            session
                .take()
                .expect("the discarded session is present")
                .shutdown()
                .await;
        }
    }

    /// The final commit linearization point for one pooled `DoH` response.
    ///
    /// A pooled session validates its response but does not own an owner
    /// lifecycle, so the owner must apply the same commit the fresh DoH path
    /// applies ([`Lifecycle::commit_final_response`]) **before** the session is
    /// returned to the pool. Without it, an owner close, caller cancellation, or
    /// deadline landing between response validation and the owner's return would
    /// let a pooled exchange report success where a fresh one would fail — the
    /// exact window the final-commit contract exists to close.
    ///
    /// The request has already been sent by this point, so the side effect is
    /// `Sent`. On failure the caller must **discard** the session rather than
    /// re-pool it, because the response was never committed.
    fn commit_pooled_response(
        &self,
        control: &ExchangeControl,
        deadline: Instant,
    ) -> Result<(), SecureError> {
        self.lifecycle
            .commit_final_response(
                &control.caller_cancellation(),
                deadline,
                SideEffectState::Sent,
            )
            .map_err(SecureError::from)
    }

    /// The live tracked-child count of the retained session's HTTP/2 scope, or
    /// `None` when nothing is retained. Test-only.
    #[cfg(test)]
    fn active_children_for_test(&self) -> Option<usize> {
        let inner = self.doh_lock();
        let (_key, session) = inner.idle.as_ref()?;
        session.active_children_for_test()
    }

    /// Installs a teardown barrier on the retained session's HTTP/2 scope.
    ///
    /// Returns `false` when no session is retained or the retained one is
    /// HTTP/1.1 (which has no tracked children). Test-only.
    #[cfg(test)]
    fn install_teardown_pause_for_test(&self, pause: Arc<crate::secure::H2TeardownPause>) -> bool {
        let inner = self.doh_lock();
        let Some((_key, session)) = inner.idle.as_ref() else {
            return false;
        };
        session.install_teardown_pause_for_test(pause).is_some()
    }

    #[cfg(test)]
    fn install_commit_pause(&self, pause: Arc<CommitPause>) {
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

    /// Runs one authenticated `DoH` exchange, reusing an idle session.
    ///
    /// # Errors
    ///
    /// Returns the existing typed [`SecureError`] for every transport outcome.
    pub async fn exchange(
        &self,
        request: ExchangeRequest<'_>,
        context: ExchangeContext,
    ) -> Result<SecureResponse, SecureError> {
        let _in_flight = self.lifecycle.register()?;
        let control = ExchangeControl::new(context, self.cancellation.clone());
        let deadline = control.context().deadline();

        control.check_at(Instant::now(), SideEffectState::NotSent)?;

        // The request target is built from the endpoint's own path and the
        // caller's query, exactly as the fresh path does, so reuse cannot change
        // the requested URL.
        let target = self.endpoint.get_request_target(request)?;
        let authority = self.endpoint.authority();
        let request_id = request.request_id();

        let lease = self
            .admit_lease()
            .ok_or_else(|| SecureError::from(UpstreamError::from(PoolError::Busy)))?;

        // Lookup is by the key *without* a negotiated protocol. A stored entry
        // always carries the protocol its session actually negotiated, so this
        // can only miss — the safe direction, because the session's own protocol
        // is what decides whether it may serve this request.
        if let Some(session) = self.checkout_live(&self.service_key()?).await {
            let negotiated = session.protocol_name().to_owned();
            match session
                .exchange(&target, &authority, request_id, &control, deadline)
                .await
            {
                Ok((response, session)) => {
                    // Commit before re-pooling: a close/cancel/deadline that
                    // landed during the exchange must still fail this response,
                    // and a failed commit must never leave the session reusable.
                    #[cfg(test)]
                    self.reach_commit_gate().await;
                    if let Err(error) = self.commit_pooled_response(&control, deadline) {
                        session.shutdown().await;
                        drop(lease);
                        return Err(error);
                    }
                    self.release(self.confirmed_key(&negotiated)?, session)
                        .await;
                    drop(lease);
                    return Ok(response);
                }
                Err(outcome) => {
                    if !outcome.rebuildable {
                        if let Some(session) = outcome.session {
                            session.shutdown().await;
                        }
                        drop(lease);
                        return Err(outcome.error);
                    }
                    if let Some(session) = outcome.session {
                        session.shutdown().await;
                    }
                }
            }
        }

        // Dial and authenticate a fresh session. This is either the first
        // exchange or the single permitted replacement.
        // Only the owner token is handed over: the session is retained across
        // exchanges, so it must not be bound to this caller's cancellation. The
        // `control` below still carries this exchange's caller token.
        let session = crate::secure::PooledDohSession::connect(
            &self.endpoint,
            &self.tls,
            self.cancellation.clone(),
            &control,
            deadline,
        )
        .await?;

        // Connect/handshake are an async wake: re-apply the control before the
        // request. No DNS byte has been sent, so this is `NotSent`.
        control.check_at(Instant::now(), SideEffectState::NotSent)?;

        let negotiated = session.protocol_name().to_owned();
        match session
            .exchange(&target, &authority, request_id, &control, deadline)
            .await
        {
            Ok((response, session)) => {
                // Same commit as the reused path: the first pooled exchange has
                // exactly the same final-commit obligation.
                #[cfg(test)]
                self.reach_commit_gate().await;
                if let Err(error) = self.commit_pooled_response(&control, deadline) {
                    session.shutdown().await;
                    drop(lease);
                    return Err(error);
                }
                self.release(self.confirmed_key(&negotiated)?, session)
                    .await;
                drop(lease);
                Ok(response)
            }
            Err(outcome) => {
                if let Some(session) = outcome.session {
                    session.shutdown().await;
                }
                drop(lease);
                Err(outcome.error)
            }
        }
    }

    /// The service key: dial, identity, authority, and policy, with no
    /// negotiated protocol.
    ///
    /// This is what a lookup compares against; the negotiated protocol is a
    /// property of the retained session, not of the request.
    fn service_key(&self) -> Result<ReuseKey, SecureError> {
        ReuseKey::plain_secure_endpoint(
            self.endpoint.dial(),
            SecureKey::from_policy(
                SecureKind::Doh,
                self.endpoint.identity().as_str(),
                Some(&self.endpoint.authority()),
                &self.tls,
            ),
        )
        .map_err(SecureError::from)
    }

    /// The key confirmed with the protocol a session actually negotiated.
    fn confirmed_key(&self, negotiated: &str) -> Result<ReuseKey, SecureError> {
        Ok(ReuseKey::plain_secure_endpoint(
            self.endpoint.dial(),
            SecureKey::from_policy(
                SecureKind::Doh,
                self.endpoint.identity().as_str(),
                Some(&self.endpoint.authority()),
                &self.tls,
            ),
        )
        .map_err(SecureError::from)?
        .with_negotiated_protocol(Some(negotiated.to_owned())))
    }
}

/// RAII lease over one outstanding `DoH` exchange.
struct DohLeaseGuard {
    inner: Arc<Mutex<DohPoolInner>>,
}

impl Drop for DohLeaseGuard {
    fn drop(&mut self) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if inner.leased > 0 {
            inner.leased -= 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DohReuseOwner, IDLE_TIMEOUT, MAX_IDLE_PER_KEY, MAX_IDLE_TOTAL, ReuseKey, ReuseOwner,
        SecureKey, SecureKind, SecureReuseOwner,
    };
    use crate::secure::TlsPolicy;
    use crate::{
        CloseTransition, CommitPause, Endpoint, ExchangeContext, ExchangeRequest, LifecycleState,
        SideEffectState, Transport, TransportCancellation, UpstreamError,
    };
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    /// A clock the test advances by hand, so idle expiry is deterministic.
    #[derive(Debug)]
    struct ManualClock(Mutex<Instant>);

    impl ManualClock {
        fn new() -> Arc<Self> {
            Arc::new(Self(Mutex::new(Instant::now())))
        }

        fn advance(&self, seconds: u64) {
            let mut now = self.0.lock().expect("clock");
            *now += Duration::from_secs(seconds);
        }
    }

    impl super::Clock for ManualClock {
        fn now(&self) -> Instant {
            *self.0.lock().expect("clock")
        }
    }

    fn tcp_key(port: u16) -> ReuseKey {
        ReuseKey::from_endpoint(
            Endpoint::new(
                SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port),
                Transport::Tcp,
            )
            .expect("endpoint"),
        )
    }

    /// A connected `tokio::net::TcpStream` plus its listener, so a retained entry
    /// owns a real socket.
    ///
    /// The listener is returned so the caller keeps the peer alive: a dropped
    /// listener would close the connection and make the entry look peer-closed.
    /// Adopting a std stream as a tokio stream registers it with the current
    /// runtime's reactor, so this must be called inside a runtime context.
    fn connected_stream() -> (tokio::net::TcpStream, std::net::TcpListener) {
        let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind");
        let std_stream =
            std::net::TcpStream::connect(listener.local_addr().expect("addr")).expect("connect");
        std_stream.set_nonblocking(true).expect("non-blocking");
        let stream = tokio::net::TcpStream::try_from(std_stream).expect("adopt the stream");
        (stream, listener)
    }

    /// Runs a pool-bound body inside a runtime, because adopting a socket needs a
    /// reactor. The runtime is current-thread and never blocks on I/O: the pool
    /// only probes readiness.
    fn with_runtime<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime")
            .block_on(future)
    }

    /// The multi-key eviction contract for the **global** idle bound.
    ///
    /// This drives the production pool directly with more than one key, which the
    /// single-key owner API cannot do: it inserts `MAX_IDLE_TOTAL + 1` distinct
    /// keys in return order and proves the oldest is evicted, that the bound is
    /// never exceeded, and that each remaining key still holds at most
    /// `MAX_IDLE_PER_KEY`.
    #[test]
    fn the_global_idle_bound_evicts_the_oldest_entry_across_keys() {
        with_runtime(async {
            let clock = ManualClock::new();
            let owner = ReuseOwner::new(
                Endpoint::new(
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 853),
                    Transport::Tcp,
                )
                .expect("endpoint"),
            )
            .with_clock(Arc::clone(&clock) as Arc<dyn super::Clock>);

            // Keep every listener alive so each retained stream stays connected
            // and cannot be discarded as peer-closed.
            let mut listeners = Vec::new();
            let mut keys = Vec::new();

            for index in 0..(MAX_IDLE_TOTAL + 1) {
                let (stream, listener) = connected_stream();
                listeners.push(listener);
                let key = tcp_key(9000 + u16::try_from(index).expect("port"));
                // Distinct insertion times, so "oldest" is unambiguous.
                clock.advance(1);
                owner.release_for_test(key.clone(), stream, true);
                keys.push(key);
            }

            assert_eq!(
                owner.idle_connections(),
                MAX_IDLE_TOTAL,
                "the global bound is never exceeded"
            );
            // The first-inserted key is the oldest and must have been evicted.
            assert!(
                owner.checkout_for_test(&keys[0]).is_none(),
                "the oldest idle entry is evicted once the global bound is reached"
            );
            // Later keys are still present.
            assert!(
                owner.checkout_for_test(&keys[MAX_IDLE_TOTAL]).is_some(),
                "the most recently returned entry is retained"
            );
            drop(listeners);
        });
    }

    /// Re-inserting the same key twice keeps exactly one entry for it.
    #[test]
    fn the_per_key_bound_keeps_exactly_one_entry() {
        with_runtime(async {
            let clock = ManualClock::new();
            let owner = ReuseOwner::new(
                Endpoint::new(
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 853),
                    Transport::Tcp,
                )
                .expect("endpoint"),
            )
            .with_clock(Arc::clone(&clock) as Arc<dyn super::Clock>);

            let key = tcp_key(9100);
            let mut listeners = Vec::new();
            for _ in 0..3 {
                let (stream, listener) = connected_stream();
                listeners.push(listener);
                clock.advance(1);
                owner.release_for_test(key.clone(), stream, true);
            }
            assert_eq!(
                owner.idle_connections(),
                MAX_IDLE_PER_KEY,
                "a key never holds more than the per-key bound"
            );
            drop(listeners);
        });
    }

    /// An entry older than `IDLE_TIMEOUT` is not handed out.
    #[test]
    fn a_stale_entry_past_the_idle_timeout_is_not_handed_out() {
        with_runtime(async {
            let clock = ManualClock::new();
            let owner = ReuseOwner::new(
                Endpoint::new(
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 853),
                    Transport::Tcp,
                )
                .expect("endpoint"),
            )
            .with_clock(Arc::clone(&clock) as Arc<dyn super::Clock>);

            let key = tcp_key(9200);
            let (stream, listener) = connected_stream();
            owner.release_for_test(key.clone(), stream, true);
            assert!(
                owner.checkout_for_test(&key).is_some(),
                "fresh entry is live"
            );

            clock.advance(IDLE_TIMEOUT.as_secs() + 1);
            assert!(
                owner.checkout_for_test(&key).is_none(),
                "an entry past IDLE_TIMEOUT is discarded, not handed out"
            );
            drop(listener);
        });
    }

    /// A closed owner never accepts a retained connection.
    #[test]
    fn a_closed_owner_discards_returned_connections() {
        with_runtime(async {
            let owner = ReuseOwner::new(
                Endpoint::new(
                    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 853),
                    Transport::Tcp,
                )
                .expect("endpoint"),
            )
            .with_clock(ManualClock::new() as Arc<dyn super::Clock>);

            let _ = owner.begin_close();
            let (stream, listener) = connected_stream();
            owner.release_for_test(tcp_key(9300), stream, false);
            assert_eq!(
                owner.idle_connections(),
                0,
                "a non-reusable or closed-pool return is discarded"
            );
            drop(listener);
        });
    }

    /// Proves the DoH secure key discriminates authority and policy.
    #[test]
    fn a_doh_secure_key_discriminates_authority_and_policy() {
        let dial = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 443);
        let key = |authority: &str, insecure: bool| {
            ReuseKey::plain_secure_endpoint(
                dial,
                SecureKey::new(SecureKind::Doh, "dns.example", Some(authority), insecure),
            )
            .expect("key")
        };
        assert_ne!(
            key("dns.example", false),
            key("dns.example:8443", false),
            "a different authority is a different key"
        );
        assert_ne!(
            key("dns.example", false),
            key("dns.example", true),
            "a different TLS policy is a different key"
        );
    }

    /// A verified policy over a fresh single-anchor root store.
    ///
    /// Each call mints a new policy, so each is a distinct trust configuration
    /// even when the anchor material is identical.
    fn verified_policy(identity: &CommitIdentity) -> TlsPolicy {
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(identity.ca_der.clone())
            .expect("the generated CA parses as a trust anchor");
        TlsPolicy::verified(roots).expect("verified policy")
    }

    /// The core of the discriminator gap: two *separately constructed* verified
    /// policies must not produce interchangeable keys, or a connection
    /// authenticated under one trust configuration could be handed to a request
    /// configured with another.
    ///
    /// The roots here are byte-identical, which is the strictest form of the
    /// test: even then the two policies are distinct configurations, so reuse
    /// must not bridge them.
    #[test]
    fn two_separately_constructed_verified_policies_do_not_share_a_key() {
        let identity = commit_identity();
        let dial = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 853);

        let first_policy = verified_policy(&identity);
        let second_policy = verified_policy(&identity);

        let first = ReuseKey::plain_secure_endpoint(
            dial,
            SecureKey::from_policy(SecureKind::Dot, "dns.example", None, &first_policy),
        )
        .expect("first key");
        let second = ReuseKey::plain_secure_endpoint(
            dial,
            SecureKey::from_policy(SecureKind::Dot, "dns.example", None, &second_policy),
        )
        .expect("second key");

        assert_ne!(
            first, second,
            "two independently constructed verified policies are different \
             trust configurations and must not share a connection"
        );

        // The same policy must still be self-consistent: an owner checks its
        // own key on every exchange, so a policy that disagreed with itself
        // would never reuse anything.
        let again = ReuseKey::plain_secure_endpoint(
            dial,
            SecureKey::from_policy(SecureKind::Dot, "dns.example", None, &first_policy),
        )
        .expect("key from the same policy");
        assert_eq!(first, again, "one policy must always produce the same key");

        // A clone is the same configuration, so it must stay interchangeable
        // with the original. This is what lets an owner be cloned or the policy
        // be passed by value without silently breaking its own reuse.
        let cloned = first_policy.clone();
        let from_clone = ReuseKey::plain_secure_endpoint(
            dial,
            SecureKey::from_policy(SecureKind::Dot, "dns.example", None, &cloned),
        )
        .expect("key from the cloned policy");
        assert_eq!(
            first, from_clone,
            "cloning a policy must preserve its roots revision"
        );

        // The insecure mode stays in its own class and never collides with a
        // verified policy.
        let insecure = ReuseKey::plain_secure_endpoint(
            dial,
            SecureKey::from_policy(
                SecureKind::Dot,
                "dns.example",
                None,
                &TlsPolicy::insecure_skip_verify(),
            ),
        )
        .expect("insecure key");
        assert_ne!(first, insecure, "insecure must differ from verified");
    }

    /// The policy-agnostic compatibility path must keep working, and must stay
    /// distinct from a policy-bearing key for the *verified* mode.
    ///
    /// `SecureKey::new(.., false)` records no root store, so it cannot be
    /// interchangeable with a key built from a specific verified policy.
    #[test]
    fn the_policy_agnostic_key_stays_compatible_and_distinct() {
        let identity = commit_identity();
        let dial = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 853);

        let legacy = ReuseKey::plain_secure_endpoint(
            dial,
            SecureKey::new(SecureKind::Dot, "dns.example", None, false),
        )
        .expect("legacy verified key");
        let same_again = ReuseKey::plain_secure_endpoint(
            dial,
            SecureKey::new(SecureKind::Dot, "dns.example", None, false),
        )
        .expect("legacy verified key again");
        assert_eq!(legacy, same_again, "the compatibility path is stable");

        let policy = verified_policy(&identity);
        let from_policy = ReuseKey::plain_secure_endpoint(
            dial,
            SecureKey::from_policy(SecureKind::Dot, "dns.example", None, &policy),
        )
        .expect("policy key");
        assert_ne!(
            legacy, from_policy,
            "a key that names no root store must not be interchangeable with one \
             that names a specific verified policy"
        );

        // The insecure compatibility path still agrees with the insecure policy,
        // because neither identifies a root store.
        let legacy_insecure = ReuseKey::plain_secure_endpoint(
            dial,
            SecureKey::new(SecureKind::Dot, "dns.example", None, true),
        )
        .expect("legacy insecure key");
        let insecure_policy_key = ReuseKey::plain_secure_endpoint(
            dial,
            SecureKey::from_policy(
                SecureKind::Dot,
                "dns.example",
                None,
                &TlsPolicy::insecure_skip_verify(),
            ),
        )
        .expect("insecure policy key");
        assert_eq!(
            legacy_insecure, insecure_policy_key,
            "both insecure forms describe the same configuration"
        );
    }

    // -----------------------------------------------------------------------
    // Pooled final-commit contract
    // -----------------------------------------------------------------------
    //
    // A pooled session validates its response but owns no lifecycle, so the
    // *owner* must apply `commit_final_response` before the session is re-pooled.
    // These tests run a real loopback DoT peer so the owner genuinely reaches its
    // commit gate with a validated response, park it on the existing
    // `CommitPause` seam, then make the commit lose. They assert both the typed
    // error and that the session was discarded rather than returned to the pool.

    /// A generated trust anchor plus a `dns.example` leaf, as the DoT contract
    /// tests use. The keys exist only in this process and are never written out.
    struct CommitIdentity {
        ca_der: rustls::pki_types::CertificateDer<'static>,
        leaf_der: rustls::pki_types::CertificateDer<'static>,
        leaf_key: rcgen::KeyPair,
    }

    fn commit_identity() -> CommitIdentity {
        use rcgen::{
            BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair,
            KeyUsagePurpose, SanType, date_time_ymd,
        };

        let ca_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("ca key");
        let mut ca_params = CertificateParams::default();
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "mosdns-reuse-commit-root");
        ca_params.not_before = date_time_ymd(2024, 1, 1);
        ca_params.not_after = date_time_ymd(2036, 1, 1);
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        let ca_cert = ca_params.self_signed(&ca_key).expect("self-signed CA");

        let leaf_key = KeyPair::generate_for(&rcgen::PKCS_ECDSA_P256_SHA256).expect("leaf key");
        let mut leaf_params = CertificateParams::default();
        leaf_params
            .distinguished_name
            .push(DnType::CommonName, "dns.example");
        leaf_params.subject_alt_names = vec![SanType::DnsName(
            "dns.example".try_into().expect("valid synthetic DNS name"),
        )];
        leaf_params.not_before = date_time_ymd(2024, 1, 1);
        leaf_params.not_after = date_time_ymd(2036, 1, 1);
        leaf_params.is_ca = IsCa::ExplicitNoCa;
        leaf_params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        leaf_params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];

        let issuer = rcgen::Issuer::from_params(&ca_params, &ca_key);
        let leaf_cert = leaf_params
            .signed_by(&leaf_key, &issuer)
            .expect("signed leaf");

        CommitIdentity {
            ca_der: ca_cert.der().clone(),
            leaf_der: leaf_cert.der().clone(),
            leaf_key,
        }
    }

    fn commit_policy(identity: &CommitIdentity) -> crate::secure::TlsPolicy {
        let mut roots = rustls::RootCertStore::empty();
        roots
            .add(identity.ca_der.clone())
            .expect("generated CA parses");
        crate::secure::TlsPolicy::verified(roots).expect("verified policy")
    }

    fn commit_dot_endpoint(address: SocketAddr) -> crate::secure::DotEndpoint {
        crate::secure::DotEndpoint::new(
            address,
            crate::secure::ServerIdentity::new("dns.example").expect("identity"),
        )
        .expect("dot endpoint")
    }

    fn test_query(id: u16) -> Vec<u8> {
        let mut wire = Vec::new();
        wire.extend_from_slice(&id.to_be_bytes());
        wire.extend_from_slice(&[0x01, 0x00]);
        wire.extend_from_slice(&1u16.to_be_bytes());
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire.extend_from_slice(&0u16.to_be_bytes());
        wire.extend_from_slice(&[0x07, b'e', b'x', b'a', b'm', b'p', b'l', b'e']);
        wire.extend_from_slice(&[0x03, b'o', b'r', b'g', 0x00]);
        wire.extend_from_slice(&[0x00, 0x01, 0x00, 0x01]);
        wire
    }

    fn test_response(id: u16) -> Vec<u8> {
        let mut wire = test_query(id);
        wire[2] = 0x81;
        wire[3] = 0x80;
        wire[6..8].copy_from_slice(&1u16.to_be_bytes());
        wire.extend_from_slice(&[0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01]);
        wire.extend_from_slice(&60u32.to_be_bytes());
        wire.extend_from_slice(&[0x00, 0x04, 192, 0, 2, 77]);
        wire
    }

    /// A one-shot loopback DoT peer: accepts one connection, answers one framed
    /// query, then waits for the client to close. Every wait is bounded.
    struct CommitPeer {
        address: SocketAddr,
        handle: std::thread::JoinHandle<()>,
    }

    impl CommitPeer {
        fn start(identity: &CommitIdentity, id: u16) -> Self {
            use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
            use tokio::net::TcpListener as AsyncTcpListener;
            use tokio::time::timeout;
            use tokio_rustls::TlsAcceptor;

            let listener =
                std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind loopback");
            let address = listener.local_addr().expect("address");
            listener.set_nonblocking(true).expect("non-blocking");

            let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .expect("safe protocol versions")
            .with_no_client_auth()
            .with_single_cert(
                vec![identity.leaf_der.clone(), identity.ca_der.clone()],
                rustls::pki_types::PrivateKeyDer::try_from(identity.leaf_key.serialize_der())
                    .expect("valid PKCS#8 key"),
            )
            .expect("consistent certificate and key");
            config.alpn_protocols = vec![b"dot".to_vec()];
            let config = Arc::new(config);
            let response = test_response(id);

            let handle = std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("server runtime");
                runtime.block_on(async move {
                    let listener =
                        AsyncTcpListener::from_std(listener).expect("adopt the listener");
                    let Ok(Ok((stream, _))) = timeout(POOL_TEST_TIMEOUT, listener.accept()).await
                    else {
                        return;
                    };
                    let acceptor = TlsAcceptor::from(config);
                    let Ok(Ok(mut tls)) = timeout(POOL_TEST_TIMEOUT, acceptor.accept(stream)).await
                    else {
                        return;
                    };
                    // Read one framed query.
                    let mut prefix = [0u8; 2];
                    if timeout(POOL_TEST_TIMEOUT, tls.read_exact(&mut prefix))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let length = usize::from(u16::from_be_bytes(prefix));
                    let mut body = vec![0u8; length];
                    if timeout(POOL_TEST_TIMEOUT, tls.read_exact(&mut body))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let mut framed = Vec::with_capacity(response.len() + 2);
                    framed.extend_from_slice(
                        &u16::try_from(response.len())
                            .expect("response fits a DNS frame")
                            .to_be_bytes(),
                    );
                    framed.extend_from_slice(&response);
                    if timeout(POOL_TEST_TIMEOUT, tls.write_all(&framed))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let _ = timeout(POOL_TEST_TIMEOUT, tls.flush()).await;
                    // Drain until the client closes, so joining cannot hang: the
                    // failing-commit path drops the session, which closes this.
                    // Exactly one bounded read per iteration — a second unbounded
                    // read here would hang the server thread and therefore
                    // `join`, which was a real bug in the first draft.
                    let mut scratch = [0u8; 64];
                    loop {
                        match timeout(POOL_TEST_TIMEOUT, tls.read(&mut scratch)).await {
                            Ok(Ok(0) | Err(_)) | Err(_) => return,
                            Ok(Ok(_)) => {}
                        }
                    }
                });
            });

            Self { address, handle }
        }

        fn join(self) {
            let _ = self.handle.join();
        }
    }

    const POOL_TEST_TIMEOUT: Duration = Duration::from_secs(10);

    /// Waits for a parked owner to reach its commit gate, bounded.
    ///
    /// An unbounded wait here would hang the whole test if the exchange failed
    /// before the gate — which was a real bug in the first draft of these tests.
    async fn await_gate(pause: &CommitPause) {
        tokio::time::timeout(POOL_TEST_TIMEOUT, pause.arrived())
            .await
            .expect("the exchange must reach the pooled commit gate");
    }

    /// Joins a spawned exchange, bounded.
    async fn join_exchange<T>(task: tokio::task::JoinHandle<T>) -> T {
        tokio::time::timeout(POOL_TEST_TIMEOUT, task)
            .await
            .expect("the exchange must finish within the test bound")
            .expect("exchange task joined")
    }

    /// Extracts the error from an exchange outcome.
    ///
    /// `SecureResponse` deliberately has no `Debug`, so `expect_err` is unusable
    /// here; this makes the failure message explicit instead.
    fn expect_pool_error(
        outcome: Result<crate::secure::SecureResponse, crate::secure::SecureError>,
        message: &str,
    ) -> crate::secure::SecureError {
        match outcome {
            Ok(_) => panic!("{message}"),
            Err(error) => error,
        }
    }

    /// A one-shot loopback DoH peer speaking HTTP/1.1 over TLS.
    ///
    /// It answers exactly one `GET /dns-query?dns=...` with a valid
    /// `application/dns-message` body, then drains until the client closes.
    struct CommitDohPeer {
        address: SocketAddr,
        handle: std::thread::JoinHandle<()>,
    }

    impl CommitDohPeer {
        fn start(identity: &CommitIdentity) -> Self {
            use base64::Engine as _;
            use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
            use tokio::net::TcpListener as AsyncTcpListener;
            use tokio::time::timeout;
            use tokio_rustls::TlsAcceptor;

            let listener =
                std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind loopback");
            let address = listener.local_addr().expect("address");
            listener.set_nonblocking(true).expect("non-blocking");

            let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .expect("safe protocol versions")
            .with_no_client_auth()
            .with_single_cert(
                vec![identity.leaf_der.clone(), identity.ca_der.clone()],
                rustls::pki_types::PrivateKeyDer::try_from(identity.leaf_key.serialize_der())
                    .expect("valid PKCS#8 key"),
            )
            .expect("consistent certificate and key");
            // The pooled H1 path: no h2 offered, so the session is HTTP/1.1.
            config.alpn_protocols = vec![b"http/1.1".to_vec()];
            let config = Arc::new(config);

            let handle = std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("server runtime");
                runtime.block_on(async move {
                    let listener =
                        AsyncTcpListener::from_std(listener).expect("adopt the listener");
                    let Ok(Ok((stream, _))) = timeout(POOL_TEST_TIMEOUT, listener.accept()).await
                    else {
                        return;
                    };
                    let acceptor = TlsAcceptor::from(config);
                    let Ok(Ok(mut tls)) = timeout(POOL_TEST_TIMEOUT, acceptor.accept(stream)).await
                    else {
                        return;
                    };
                    // Read the request head, which carries the base64url query.
                    let mut buffer = Vec::new();
                    let mut chunk = [0u8; 512];
                    loop {
                        let Ok(Ok(read)) = timeout(POOL_TEST_TIMEOUT, tls.read(&mut chunk)).await
                        else {
                            return;
                        };
                        if read == 0 {
                            return;
                        }
                        buffer.extend_from_slice(&chunk[..read]);
                        if buffer.windows(4).any(|window| window == b"\r\n\r\n") {
                            break;
                        }
                    }
                    let head = String::from_utf8_lossy(&buffer).into_owned();
                    // The DoH GET rewrites the DNS ID to 0 for cacheability
                    // (RFC 8484), so the peer echoes the ID it actually received
                    // rather than the caller's original one; the pooled path
                    // restores the caller's ID on the way back.
                    let Some(target) = head
                        .lines()
                        .next()
                        .and_then(|line| line.split(' ').nth(1))
                        .and_then(|target| target.split("dns=").nth(1))
                    else {
                        panic!("the peer must receive a dns= query parameter");
                    };
                    let Ok(query) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(target)
                    else {
                        panic!("the peer must receive a base64url query");
                    };
                    let received_id = u16::from_be_bytes([query[0], query[1]]);
                    let reply_body = test_response(received_id);
                    let mut reply = Vec::new();
                    reply.extend_from_slice(b"HTTP/1.1 200 OK\r\n");
                    reply.extend_from_slice(b"content-type: application/dns-message\r\n");
                    reply.extend_from_slice(
                        format!("content-length: {}\r\n", reply_body.len()).as_bytes(),
                    );
                    reply.extend_from_slice(b"\r\n");
                    reply.extend_from_slice(&reply_body);
                    if timeout(POOL_TEST_TIMEOUT, tls.write_all(&reply))
                        .await
                        .is_err()
                    {
                        return;
                    }
                    let _ = timeout(POOL_TEST_TIMEOUT, tls.flush()).await;
                    // Drain until the client closes; the failing-commit path drops
                    // the session, which is what ends this. One bounded read per
                    // iteration, so this can never hang `join`.
                    let mut scratch = [0u8; 64];
                    loop {
                        match timeout(POOL_TEST_TIMEOUT, tls.read(&mut scratch)).await {
                            Ok(Ok(0) | Err(_)) | Err(_) => return,
                            Ok(Ok(_)) => {}
                        }
                    }
                });
            });

            Self { address, handle }
        }

        fn join(self) {
            let _ = self.handle.join();
        }
    }

    /// The DoH twin of the DoT owner-close case: the pooled DoH owner has exactly
    /// the same final-commit obligation and must fail and discard on close.
    #[test]
    fn a_pooled_doh_response_loses_the_commit_gate_to_owner_close() {
        let identity = commit_identity();
        let id = 0xd003;
        let peer = CommitDohPeer::start(&identity);
        let endpoint =
            crate::secure::DohEndpoint::new("https://dns.example/dns-query", peer.address)
                .expect("doh endpoint");
        let owner =
            Arc::new(DohReuseOwner::new(endpoint, commit_policy(&identity)).expect("owner"));
        let pause = Arc::new(CommitPause::new());
        owner.install_commit_pause(Arc::clone(&pause));

        with_runtime(async {
            let request = test_query(id);
            let exchange = {
                let owner = Arc::clone(&owner);
                tokio::spawn(async move {
                    owner
                        .exchange(
                            ExchangeRequest::new(&request).expect("request"),
                            ExchangeContext::new(
                                Instant::now() + POOL_TEST_TIMEOUT,
                                TransportCancellation::new(),
                            ),
                        )
                        .await
                })
            };

            // The HTTP/1.1 exchange has completed and validated its response; the
            // owner is parked immediately before its own commit.
            await_gate(&pause).await;
            assert_eq!(owner.begin_close(), CloseTransition::BeganClosing);

            pause.release();
            let outcome = join_exchange(exchange).await;
            let error = expect_pool_error(outcome, "owner close must fail the response");
            assert_eq!(
                error,
                crate::secure::SecureError::Transport(UpstreamError::Closed(SideEffectState::Sent)),
                "a pooled DoH response must lose to owner close at the gate"
            );
            assert_eq!(
                owner.idle_connections(),
                0,
                "a DoH response that lost the gate must never be re-pooled"
            );
            assert_eq!(owner.close().await, crate::CloseResult::Closed);
            assert_eq!(
                owner.in_flight_exchanges(),
                0,
                "the failed commit must still have released its registration"
            );
        });
        peer.join();
    }

    /// Owner close must not report `Closed` while a pooled HTTP/2 session still
    /// has live tracked children.
    ///
    /// A pooled h2 session owns Hyper's per-request send/pipe futures and its
    /// connection driver through `H2ScopeLease`. `Drop` can only seal and abort;
    /// it cannot wait. Without the explicit async teardown, `close()` would
    /// return `Closed` while those children were still alive, which is exactly
    /// the "tracked child outliving its connection" the design forbids.
    ///
    /// This drives a real pooled h2 session against a loopback peer, parks its
    /// teardown on the existing `H2TeardownPause` barrier, and asserts that
    /// `close()` does not complete until the barrier is released.
    #[test]
    fn a_pooled_h2_close_waits_for_tracked_children_to_drain() {
        let identity = commit_identity();
        let peer = CommitH2Peer::start(&identity);
        let endpoint =
            crate::secure::DohEndpoint::new("https://dns.example/dns-query", peer.address)
                .expect("doh endpoint");
        let owner =
            Arc::new(DohReuseOwner::new(endpoint, commit_policy(&identity)).expect("owner"));

        with_runtime(async {
            // One successful exchange leaves a retained, negotiated h2 session
            // whose scope has live tracked children (the connection driver).
            let query = test_query(0xd010);
            let request = ExchangeRequest::new(&query).expect("request");
            bounded_exchange(&owner, request, POOL_TEST_TIMEOUT)
                .await
                .expect("the h2 exchange succeeds");
            assert_eq!(owner.idle_connections(), 1, "an h2 session is retained");

            // Park the next teardown of that session's scope.
            let pause = Arc::new(crate::secure::H2TeardownPause::new());
            assert!(
                owner.install_teardown_pause_for_test(Arc::clone(&pause)),
                "the retained session must be HTTP/2 for this test"
            );

            let closer = {
                let owner = Arc::clone(&owner);
                tokio::spawn(async move { owner.close().await })
            };

            // Close must reach the barrier rather than completing immediately.
            tokio::time::timeout(POOL_TEST_TIMEOUT, pause.arrived())
                .await
                .expect("close reaches the pooled h2 teardown barrier");

            // While the barrier is held, the children have not drained, so close
            // must not have finished.
            let mut closer = closer;
            tokio::select! {
                biased;
                result = &mut closer => panic!(
                    "close returned {result:?} while pooled h2 children were still live"
                ),
                () = tokio::task::yield_now() => {}
            }

            // Releasing the barrier lets the children drain and close converge.
            pause.release();
            let outcome = tokio::time::timeout(POOL_TEST_TIMEOUT, closer)
                .await
                .expect("close finishes once children drain")
                .expect("close task joins");
            assert_eq!(outcome, crate::CloseResult::Closed);
            assert_eq!(
                owner.idle_connections(),
                0,
                "close drops the retained session"
            );
            assert_eq!(owner.in_flight_exchanges(), 0);
            assert_eq!(
                owner.active_children_for_test(),
                None,
                "the drained session is gone, so its scope is gone with it"
            );
        });
        peer.join();
    }

    /// Runs one pooled exchange under a bound, failing loudly instead of hanging.
    async fn bounded_exchange(
        owner: &DohReuseOwner,
        request: ExchangeRequest<'_>,
        bound: Duration,
    ) -> Result<crate::secure::SecureResponse, crate::secure::SecureError> {
        tokio::time::timeout(
            bound,
            Box::pin(owner.exchange(
                request,
                ExchangeContext::new(Instant::now() + bound, TransportCancellation::new()),
            )),
        )
        .await
        .expect("the exchange must finish within the bound")
    }

    /// A one-shot loopback DoH peer speaking HTTP/2 over TLS.
    struct CommitH2Peer {
        address: SocketAddr,
        handle: std::thread::JoinHandle<()>,
    }

    impl CommitH2Peer {
        fn start(identity: &CommitIdentity) -> Self {
            use base64::Engine as _;
            use tokio::net::TcpListener as AsyncTcpListener;
            use tokio::time::timeout;
            use tokio_rustls::TlsAcceptor;

            let listener =
                std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind loopback");
            let address = listener.local_addr().expect("address");
            listener.set_nonblocking(true).expect("non-blocking");

            let mut config = rustls::ServerConfig::builder_with_provider(Arc::new(
                rustls::crypto::ring::default_provider(),
            ))
            .with_safe_default_protocol_versions()
            .expect("safe protocol versions")
            .with_no_client_auth()
            .with_single_cert(
                vec![identity.leaf_der.clone(), identity.ca_der.clone()],
                rustls::pki_types::PrivateKeyDer::try_from(identity.leaf_key.serialize_der())
                    .expect("valid PKCS#8 key"),
            )
            .expect("consistent certificate and key");
            config.alpn_protocols = vec![b"h2".to_vec()];
            let config = Arc::new(config);

            let handle = std::thread::spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("server runtime");
                runtime.block_on(async move {
                    let listener =
                        AsyncTcpListener::from_std(listener).expect("adopt the listener");
                    let Ok(Ok((stream, _))) = timeout(POOL_TEST_TIMEOUT, listener.accept()).await
                    else {
                        return;
                    };
                    let acceptor = TlsAcceptor::from(config);
                    let Ok(Ok(tls)) = timeout(POOL_TEST_TIMEOUT, acceptor.accept(stream)).await
                    else {
                        return;
                    };
                    let Ok(mut connection) = h2::server::handshake(tls).await else {
                        return;
                    };
                    // Serve every stream on this connection so the pooled session
                    // stays usable and its driver stays alive.
                    loop {
                        let next = timeout(POOL_TEST_TIMEOUT, connection.accept()).await;
                        let Ok(Some(Ok((request, mut respond)))) = next else {
                            return;
                        };
                        let Some(target) = request.uri().path_and_query() else {
                            return;
                        };
                        let Some(encoded) = target.as_str().split("dns=").nth(1) else {
                            return;
                        };
                        let Ok(query) =
                            base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(encoded)
                        else {
                            return;
                        };
                        let received_id = u16::from_be_bytes([query[0], query[1]]);
                        let reply = test_response(received_id);
                        let head = hyper::Response::builder()
                            .status(200)
                            .header("content-type", "application/dns-message")
                            .body(())
                            .expect("h2 response head");
                        let Ok(mut send) = respond.send_response(head, false) else {
                            return;
                        };
                        if send
                            .send_data(hyper::body::Bytes::from(reply), true)
                            .is_err()
                        {
                            return;
                        }
                    }
                });
            });

            Self { address, handle }
        }

        fn join(self) {
            let _ = self.handle.join();
        }
    }

    /// Parks an owner on its pooled-response commit gate and returns the handle.
    fn install_pause(owner: &SecureReuseOwner) -> Arc<CommitPause> {
        let pause = Arc::new(CommitPause::new());
        owner.install_commit_pause(Arc::clone(&pause));
        pause
    }

    /// Owner close reaching the pooled commit gate first must fail the exchange
    /// and must not re-pool the session.
    #[test]
    fn a_pooled_dot_response_loses_the_commit_gate_to_owner_close() {
        let identity = commit_identity();
        let id = 0xd001;
        let peer = CommitPeer::start(&identity, id);
        let owner = Arc::new(
            SecureReuseOwner::new(commit_dot_endpoint(peer.address), commit_policy(&identity))
                .expect("owner"),
        );
        let pause = install_pause(&owner);

        with_runtime(async {
            let request = test_query(id);
            let exchange = {
                let owner = Arc::clone(&owner);
                tokio::spawn(async move {
                    owner
                        .exchange(
                            ExchangeRequest::new(&request).expect("request"),
                            ExchangeContext::new(
                                Instant::now() + POOL_TEST_TIMEOUT,
                                TransportCancellation::new(),
                            ),
                        )
                        .await
                })
            };

            // The real DoT exchange has completed and validated its response; the
            // owner is now parked immediately before its own commit, which is
            // exactly the window that previously returned success.
            await_gate(&pause).await;

            // Owner close wins the gate.
            assert_eq!(owner.begin_close(), CloseTransition::BeganClosing);
            assert_eq!(owner.lifecycle_state(), LifecycleState::Closing);

            pause.release();
            let outcome = join_exchange(exchange).await;
            let error = expect_pool_error(outcome, "owner close must fail the response");
            assert_eq!(
                error,
                crate::secure::SecureError::Transport(UpstreamError::Closed(SideEffectState::Sent)),
                "a pooled response must lose to owner close at the commit gate"
            );
            assert_eq!(
                owner.idle_connections(),
                0,
                "a response that lost the commit gate must never be re-pooled"
            );

            assert_eq!(
                owner.close().await,
                crate::CloseResult::Closed,
                "the begun close still converges"
            );
            assert_eq!(owner.in_flight_exchanges(), 0);
            assert_eq!(owner.leased_connections(), 0, "the lease was released");
        });
        peer.join();
    }

    /// Caller cancellation reaching the pooled commit gate first must fail the
    /// exchange and must not re-pool the session.
    #[test]
    fn a_pooled_dot_response_loses_the_commit_gate_to_caller_cancellation() {
        let identity = commit_identity();
        let id = 0xd002;
        let peer = CommitPeer::start(&identity, id);
        let owner = Arc::new(
            SecureReuseOwner::new(commit_dot_endpoint(peer.address), commit_policy(&identity))
                .expect("owner"),
        );
        let pause = install_pause(&owner);
        let token = TransportCancellation::new();

        with_runtime(async {
            let request = test_query(id);
            let exchange = {
                let owner = Arc::clone(&owner);
                let token = token.clone();
                tokio::spawn(async move {
                    owner
                        .exchange(
                            ExchangeRequest::new(&request).expect("request"),
                            ExchangeContext::new(Instant::now() + POOL_TEST_TIMEOUT, token),
                        )
                        .await
                })
            };

            await_gate(&pause).await;

            // The caller gives up while the owner is parked before its commit.
            token.cancel();
            pause.release();

            let outcome = join_exchange(exchange).await;
            let error = expect_pool_error(outcome, "caller cancellation must fail");
            assert_eq!(
                error,
                crate::secure::SecureError::Transport(UpstreamError::Cancelled(
                    SideEffectState::Sent
                )),
                "a pooled response must lose to caller cancellation at the gate"
            );
            assert_eq!(
                owner.idle_connections(),
                0,
                "a cancelled pooled response must never be re-pooled"
            );
            assert_eq!(owner.lifecycle_state(), LifecycleState::Open);
            assert_eq!(owner.leased_connections(), 0);
        });
        peer.join();
    }
}
