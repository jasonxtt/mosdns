//! Slice 0 deterministic model tests for the QUIC reuse owner
//! (`.trellis/tasks/09-20-rust-phase4-quic-reuse-multiplexing`).
//!
//! Every test here is pure model work: **no socket, no QUIC/H3 I/O, no
//! handshake, no loopback fixture**. The owner's physical transport is the
//! inert [`EntryTransport`] token, and every interleaving point is an explicit
//! [`ModelBarrier`] or an explicit ordered call — never a sleep and never an
//! elapsed-time poll. The only time-based wait is the deadlock guard
//! [`bounded`].
//!
//! Evidence mapping:
//!
//! * A1  — validated key isolation (`quic_reuse_key_isolates_every_dimension`);
//! * A12/R0a — the four-phase H3 cancellation decision model, model-only;
//! * A12/R0b — the pinned connection-vs-stream classification table;
//! * A6/A13 — multi-key cap, terminal-only slot accounting, same-key `Closing`,
//!   and the post-close admission race;
//! * A8/A13 — init-vs-owner-close with a late resource, and the
//!   begin_close-to-accepting=false publication race (both stage orders);
//! * A5/A13 — the aborted-at-barrier / no-surviving-caller supervised teardown;
//! * A7 — lazy idle expiry through the injected clock.
//!
//! The real pinned-stack H3 proof (one canceled request leaves the shared
//! connection, driver, and another concurrent request healthy) is Slice 2/A5 and
//! is deliberately not claimed here.

mod fixtures;

use std::error::Error as StdError;
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use h3::quic::{ConnectionErrorIncoming, StreamErrorIncoming};
use mosdns_upstream_core::quic::DoqEndpoint;
use mosdns_upstream_core::quic_reuse::{
    EntryHealth, EntryInitializer, EntryPhase, EntryTerminal, EntryTransport, InitializeFuture,
    PINNED_ERROR_CLASSIFICATION_TABLE, pinned_error_class,
};
use mosdns_upstream_core::quic_reuse::{
    H3CancellationAction, H3RequestPhase, ImmediateInitializer, MAX_CONNECTIONS_PER_OWNER,
    MAX_STREAMS_PER_CONNECTION, ManualClock, ModelBarrier, ModelSeams, QUIC_IDLE_TIMEOUT,
    QuicErrorClass, QuicErrorOutcome, QuicExchangePhase, QuicProtocol, QuicReuseKey,
    QuicReuseOwner, QuicTlsMode, classify_h3_connection_error, classify_h3_quinn_connection_error,
    classify_h3_quinn_stream_error, classify_h3_stream_error, classify_quinn_connection_error,
    classify_quinn_read_error, classify_quinn_read_to_end_error, classify_quinn_write_error,
    h3_cancellation_decision, h3_cancellation_effect, side_effect_of,
};
use mosdns_upstream_core::{
    CloseCompletion, DohEndpoint, ExchangeContext, ExchangeControl, Lifecycle, ServerIdentity,
    SideEffectState, StreamLease, TlsPolicy, TransportCancellation, UpstreamError,
};
use quinn::{
    ApplicationClose, ConnectionClose, ConnectionError, ReadError, ReadToEndError, VarInt,
    WriteError,
};

/// The upper bound on any single await: a deadlock guard only, never a
/// termination condition and never part of an assertion.
const TEST_DEADLINE: Duration = Duration::from_secs(20);

async fn bounded<F: std::future::Future>(future: F) -> F::Output {
    tokio::time::timeout(TEST_DEADLINE, future)
        .await
        .expect("operation must complete within the test deadline (deadlock guard)")
}

fn block_on<F: std::future::Future>(future: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("a current-thread runtime")
        .block_on(future)
}

fn control() -> ExchangeControl {
    ExchangeControl::new(
        ExchangeContext::new(Instant::now() + TEST_DEADLINE, TransportCancellation::new()),
        TransportCancellation::new(),
    )
}

const fn loopback(port: u16) -> SocketAddr {
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)
}

fn doq_key(port: u16) -> QuicReuseKey {
    let identity = ServerIdentity::new("dns.example").expect("a valid synthetic identity");
    let endpoint = DoqEndpoint::new(loopback(port), identity).expect("a valid DoQ endpoint");
    QuicReuseKey::from_doq(&endpoint, &TlsPolicy::insecure_skip_verify())
}

// ---------------------------------------------------------------------------
// Harness and Slice 0 model seams
// ---------------------------------------------------------------------------

/// Records every inert transport the model initializer produces, so a test can
/// prove the exact acquired resource was closed exactly once by teardown.
#[derive(Default)]
struct TransportFactory {
    next: AtomicU64,
    produced: Mutex<Vec<Arc<EntryTransport>>>,
}

impl TransportFactory {
    fn make(&self) -> Arc<EntryTransport> {
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        let transport = Arc::new(EntryTransport::new(id));
        self.produced
            .lock()
            .expect("factory lock")
            .push(Arc::clone(&transport));
        transport
    }

    fn produced(&self) -> usize {
        self.produced.lock().expect("factory lock").len()
    }

    fn last(&self) -> Arc<EntryTransport> {
        self.produced
            .lock()
            .expect("factory lock")
            .last()
            .cloned()
            .expect("the initializer produced a transport")
    }
}

/// The inert Slice 0 initializer: optionally parks on barriers around its
/// (non-)acquisition, optionally fails, and records the token it produced.
struct ModelInitializer {
    park_key: Option<QuicReuseKey>,
    before_acquire: Option<Arc<ModelBarrier>>,
    after_acquire: Option<Arc<ModelBarrier>>,
    produce: bool,
    factory: Arc<TransportFactory>,
    calls: AtomicUsize,
}

impl ModelInitializer {
    fn new(
        before_acquire: Option<Arc<ModelBarrier>>,
        after_acquire: Option<Arc<ModelBarrier>>,
        produce: bool,
        factory: Arc<TransportFactory>,
    ) -> Self {
        Self {
            park_key: None,
            before_acquire,
            after_acquire,
            produce,
            factory,
            calls: AtomicUsize::new(0),
        }
    }

    /// Parks only for this key, so a test can hold one entry's initializer while
    /// other keys initialize immediately.
    fn for_key(mut self, key: QuicReuseKey) -> Self {
        self.park_key = Some(key);
        self
    }

    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}

impl EntryInitializer for ModelInitializer {
    fn initialize(&self, key: QuicReuseKey, _generation: u64) -> InitializeFuture {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let parks = self.park_key.as_ref().is_none_or(|parked| parked == &key);
        let before = if parks {
            self.before_acquire.clone()
        } else {
            None
        };
        let after = if parks {
            self.after_acquire.clone()
        } else {
            None
        };
        let produce = self.produce;
        let factory = Arc::clone(&self.factory);
        Box::pin(async move {
            if let Some(barrier) = before {
                barrier.check().await;
            }
            let transport = if produce { Some(factory.make()) } else { None };
            if let Some(barrier) = after {
                barrier.check().await;
            }
            transport
        })
    }
}

struct Harness {
    owner: Arc<QuicReuseOwner>,
    lifecycle: Arc<Lifecycle>,
    clock: Arc<ManualClock>,
}

fn harness() -> Harness {
    harness_with(
        Arc::new(ImmediateInitializer::default()),
        ModelSeams::default(),
    )
}

fn harness_with(initializer: Arc<dyn EntryInitializer>, seams: ModelSeams) -> Harness {
    let clock = Arc::new(ManualClock::new(Instant::now()));
    let lifecycle = Arc::new(Lifecycle::new());
    let owner = Arc::new(QuicReuseOwner::with_parts(
        Arc::clone(&lifecycle),
        clock.clone(),
        initializer,
        seams,
    ));
    Harness {
        owner,
        lifecycle,
        clock,
    }
}

async fn admit_once(
    owner: &QuicReuseOwner,
    key: &QuicReuseKey,
) -> Result<StreamLease, UpstreamError> {
    let registration = owner.register().expect("the owner is open");
    let control = control();
    owner.admit(registration, key.clone(), &control).await
}

async fn admit_ok(owner: &QuicReuseOwner, key: &QuicReuseKey) -> StreamLease {
    admit_once(owner, key)
        .await
        .expect("the admission is expected to succeed")
}

fn assert_closed_not_sent(result: &Result<StreamLease, UpstreamError>) {
    match result {
        Err(UpstreamError::Closed(SideEffectState::NotSent)) => {}
        Err(other) => panic!("expected the existing Closed(NotSent) vocabulary, got {other:?}"),
        Ok(_) => panic!("expected Closed(NotSent), got a stream lease"),
    }
}

// ---------------------------------------------------------------------------
// A1 — validated key isolation
// ---------------------------------------------------------------------------

#[test]
fn quic_reuse_key_isolates_every_dimension() {
    let set = fixtures::FixtureSet::generate();
    let verified_a1 = TlsPolicy::verified(set.root_store_a()).expect("verified policy");
    // A second verified policy from the same roots: the opaque roots revision
    // differs, so the two policies must not share a connection.
    let verified_a2 = TlsPolicy::verified(set.root_store_a()).expect("verified policy");
    let verified_b = TlsPolicy::verified(set.root_store_b()).expect("verified policy");
    let insecure = TlsPolicy::insecure_skip_verify();

    let identity = ServerIdentity::new("dns.example").expect("valid identity");
    let other_identity = ServerIdentity::new("other.example").expect("valid identity");

    let doq = |identity: &ServerIdentity, port: u16, policy: &TlsPolicy| {
        QuicReuseKey::from_doq(
            &DoqEndpoint::new(loopback(port), identity.clone()).expect("valid DoQ endpoint"),
            policy,
        )
    };
    let doh3 = |url: &str, port: u16, policy: &TlsPolicy| {
        QuicReuseKey::from_doh3(
            &DohEndpoint::new(url, loopback(port)).expect("valid DoH3 endpoint"),
            policy,
        )
    };

    let base = doq(&identity, 853, &verified_a1);

    // Identical inputs and a cloned policy produce the same key.
    assert_eq!(base, doq(&identity, 853, &verified_a1));
    assert_eq!(base, doq(&identity, 853, &verified_a1.clone()));

    // Numeric dial address, including port.
    assert_ne!(base, doq(&identity, 8853, &verified_a1));

    // Canonical identity.
    assert_ne!(base, doq(&other_identity, 853, &verified_a1));

    // TLS mode and opaque roots revision.
    assert_ne!(base, doq(&identity, 853, &insecure));
    assert_ne!(base, doq(&identity, 853, &verified_a2));
    assert_ne!(base, doq(&identity, 853, &verified_b));
    assert_eq!(base.tls_mode(), QuicTlsMode::Verified);
    assert_eq!(
        doq(&identity, 853, &insecure).tls_mode(),
        QuicTlsMode::InsecureSkipVerify
    );
    assert_ne!(
        base.roots_revision(),
        doq(&identity, 853, &verified_a2).roots_revision()
    );

    // Protocol/ALPN discriminator: DoQ and DoH3 never share a key even on one
    // numeric address.
    let h3 = doh3("https://dns.example/dns-query", 853, &verified_a1);
    assert_ne!(base, h3);
    assert_eq!(base.protocol(), QuicProtocol::Doq);
    assert_eq!(h3.protocol(), QuicProtocol::Doh3);
    assert_eq!(QuicProtocol::Doq.alpn(), b"doq");
    assert_eq!(QuicProtocol::Doh3.alpn(), b"h3");
    assert_eq!(base.identity(), "dns.example");
    assert_eq!(h3.identity(), "dns.example");
    assert_eq!(base.dial(), loopback(853));
    assert_eq!(h3.dial(), loopback(853));

    // DoH3 authority is keyed, because the dial and the origin authority are
    // separate.
    assert_eq!(base.doh3_authority(), None);
    assert_eq!(h3.doh3_authority(), Some("dns.example"));
    assert_ne!(
        h3,
        doh3("https://dns.example:8443/dns-query", 853, &verified_a1)
    );
    // The path is not part of the connection identity; the authority is.
    assert_eq!(
        h3,
        doh3("https://dns.example/other-path", 853, &verified_a1)
    );

    // Hash/equality agree, so the key is safe as an owner-map key.
    let mut keys = std::collections::HashSet::new();
    keys.insert(base.clone());
    assert!(keys.contains(&doq(&identity, 853, &verified_a1)));
    assert!(!keys.contains(&doq(&identity, 8853, &verified_a1)));
}

// ---------------------------------------------------------------------------
// A12 / R0a — the four-phase cancellation decision model (model-only)
// ---------------------------------------------------------------------------

#[test]
fn r0a_four_phase_decision_model_never_selects_the_pinned_recv_stop() {
    // The frozen table. Only the phase before any request byte may actively stop
    // the send side; from "after request FIN" a read future may already own the
    // inner `Option<quinn::RecvStream>`, so the receive side is drop-only.
    let frozen = [
        (
            H3RequestPhase::BeforeSend,
            H3CancellationAction::StopSendSide,
            false,
        ),
        (
            H3RequestPhase::AfterRequestFin,
            H3CancellationAction::DropOnly,
            true,
        ),
        (
            H3RequestPhase::ResponseHead,
            H3CancellationAction::DropOnly,
            true,
        ),
        (
            H3RequestPhase::BodyRead,
            H3CancellationAction::DropOnly,
            true,
        ),
    ];

    for (phase, action, recv_in_flight) in frozen {
        let decision = h3_cancellation_decision(phase);
        assert_eq!(decision.phase(), phase);
        assert_eq!(decision.action(), action, "phase {phase:?}");
        assert_eq!(decision.recv_stream_may_be_in_flight(), recv_in_flight);
        // The pinned `h3-quinn` `RecvStream::stop_sending` unwraps the inner
        // `Option` and panics once an aborted `poll_data` left it as `None`.
        assert!(
            !decision.selects_pinned_recv_stop(),
            "phase {phase:?} must never select the pinned Option::None stop_sending path"
        );
        // The logical shared-entry effect is stream-local for every phase.
        let effect = h3_cancellation_effect(phase);
        assert_eq!(effect.entry_phase_after, EntryPhase::Active);
        assert!(effect.connection_healthy_after);
        assert!(effect.entry_leasable_after);
    }

    // The only active stop in the table is the send side; the receive-side
    // variant exists purely to name and forbid the pinned hazard.
    assert!(matches!(
        h3_cancellation_decision(H3RequestPhase::BeforeSend).action(),
        H3CancellationAction::StopSendSide
    ));
    for phase in [
        H3RequestPhase::AfterRequestFin,
        H3RequestPhase::ResponseHead,
        H3RequestPhase::BodyRead,
    ] {
        assert_eq!(
            h3_cancellation_decision(phase).action(),
            H3CancellationAction::DropOnly
        );
    }
}

#[test]
fn r0a_stream_local_failure_leaves_the_shared_entry_healthy_and_leasable() {
    block_on(async {
        let h = harness();
        let key = doq_key(9001);

        let lease = admit_ok(&h.owner, &key).await;
        let generation = lease.generation();
        let record = Arc::clone(lease.record());
        drop(lease);
        let before = h.owner.observe(&key).expect("observation");

        // One cancelled request is exactly one stream-local classification.
        assert_eq!(
            h.owner
                .apply_error_class(&key, generation, QuicErrorClass::StreamLocal),
            QuicErrorOutcome::EntryUnchanged
        );

        let after = h.owner.observe(&key).expect("observation");
        assert_eq!(after.phase, EntryPhase::Active);
        assert_eq!(after.health, EntryHealth::Healthy);
        assert_eq!(after.generation, generation);
        assert_eq!(after.permits_in_use, before.permits_in_use);
        assert_eq!(record.terminal(), None);
        assert_eq!(record.teardown_runs(), 0);

        // The same shared connection is reused by a later independent query.
        let reuse = admit_ok(&h.owner, &key).await;
        assert_eq!(reuse.generation(), generation);
        drop(reuse);
        assert_eq!(h.owner.entry_count(), 1);
    });
}

// ---------------------------------------------------------------------------
// A12 / R0b — the pinned classification table
// ---------------------------------------------------------------------------

#[test]
fn r0b_connection_level_and_stream_level_classification() {
    // quinn connection-level: every constructible variant is entry-terminal. The
    // `TransportError` variant carries a type this crate cannot name, so its
    // coverage is compiler-enforced by `classify_quinn_connection_error`'s
    // exhaustive match over the non-`#[non_exhaustive]` pinned enum.
    let connection_errors = [
        ConnectionError::VersionMismatch,
        ConnectionError::Reset,
        ConnectionError::TimedOut,
        ConnectionError::LocallyClosed,
        ConnectionError::CidsExhausted,
        ConnectionError::ApplicationClosed(ApplicationClose {
            error_code: VarInt::from_u32(0),
            reason: Vec::new().into(),
        }),
        ConnectionError::ConnectionClosed(ConnectionClose {
            error_code: quinn::TransportErrorCode::crypto(0),
            frame_type: None,
            reason: Vec::new().into(),
        }),
    ];
    for error in &connection_errors {
        assert_eq!(
            classify_quinn_connection_error(error),
            QuicErrorClass::EntryTerminal,
            "{error:?}"
        );
    }

    // quinn write: a peer stop or a closed stream is stream-local; a lost
    // connection inherits the connection class.
    assert_eq!(
        classify_quinn_write_error(&WriteError::Stopped(VarInt::from_u32(3))),
        QuicErrorClass::StreamLocal
    );
    assert_eq!(
        classify_quinn_write_error(&WriteError::ClosedStream),
        QuicErrorClass::StreamLocal
    );
    assert_eq!(
        classify_quinn_write_error(&WriteError::ConnectionLost(ConnectionError::TimedOut)),
        QuicErrorClass::EntryTerminal
    );

    // quinn read.
    assert_eq!(
        classify_quinn_read_error(&ReadError::Reset(VarInt::from_u32(3))),
        QuicErrorClass::StreamLocal
    );
    assert_eq!(
        classify_quinn_read_error(&ReadError::ClosedStream),
        QuicErrorClass::StreamLocal
    );
    assert_eq!(
        classify_quinn_read_error(&ReadError::IllegalOrderedRead),
        QuicErrorClass::StreamLocal
    );
    assert_eq!(
        classify_quinn_read_error(&ReadError::ConnectionLost(ConnectionError::Reset)),
        QuicErrorClass::EntryTerminal
    );
    assert_eq!(
        classify_quinn_read_to_end_error(&ReadToEndError::Read(ReadError::Reset(
            VarInt::from_u32(3)
        ))),
        QuicErrorClass::StreamLocal
    );
    assert_eq!(
        classify_quinn_read_to_end_error(&ReadToEndError::TooLong),
        QuicErrorClass::StreamLocal
    );

    // h3-quinn backend vocabulary: the whole `ConnectionErrorIncoming` type is
    // connection-level; only a peer stream termination is stream-local.
    let undefined: Arc<dyn StdError + Send + Sync> = Arc::new(io::Error::other("synthetic"));
    let connection_incoming = [
        ConnectionErrorIncoming::ApplicationClose { error_code: 0 },
        ConnectionErrorIncoming::Timeout,
        ConnectionErrorIncoming::InternalError(String::from("synthetic")),
        ConnectionErrorIncoming::Undefined(undefined),
    ];
    for error in &connection_incoming {
        assert_eq!(
            classify_h3_quinn_connection_error(error),
            QuicErrorClass::EntryTerminal,
            "{error:?}"
        );
    }
    assert_eq!(
        classify_h3_quinn_stream_error(&StreamErrorIncoming::ConnectionErrorIncoming {
            connection_error: ConnectionErrorIncoming::Timeout,
        }),
        QuicErrorClass::EntryTerminal
    );
    assert_eq!(
        classify_h3_quinn_stream_error(&StreamErrorIncoming::StreamTerminated { error_code: 3 }),
        QuicErrorClass::StreamLocal
    );
    assert_eq!(
        classify_h3_quinn_stream_error(&StreamErrorIncoming::Unknown(Box::new(io::Error::other(
            "synthetic"
        )))),
        QuicErrorClass::StreamLocal
    );

    // The `h3` error enums mark the enum and every variant `#[non_exhaustive]`
    // (error.rs:16-19 and per-variant cfg_attr blocks), so a downstream crate
    // can neither construct nor pattern-match any `h3` error value. Both `h3`
    // rules are therefore type-level and total; the frozen-table rows assert them
    // in `r0b_table_covers_the_pinned_vocabulary` together with the
    // non-weakening note: tuple/unit variants cannot reach the wildcard without
    // the paired connection observation, because the call sites that own a real
    // driver (Slice 2) observe the driver outcome for the same event.
    let h3_connection_rule: fn(&h3::error::ConnectionError) -> QuicErrorClass =
        classify_h3_connection_error;
    let h3_stream_rule: fn(&h3::error::StreamError) -> QuicErrorClass = classify_h3_stream_error;
    assert_eq!(
        std::mem::size_of_val(&h3_connection_rule) + std::mem::size_of_val(&h3_stream_rule),
        2 * std::mem::size_of::<usize>()
    );
}

#[test]
fn r0b_table_covers_the_pinned_vocabulary() {
    use mosdns_upstream_core::quic_reuse::QuicErrorClass::{EntryTerminal, StreamLocal};

    // Every row cites its pinned source and carries exactly one class.
    for row in PINNED_ERROR_CLASSIFICATION_TABLE {
        assert!(
            row.citation.contains(".rs:") || row.citation.contains("-0.11.18"),
            "row {} / {} has no exact source citation",
            row.vocabulary,
            row.item
        );
        assert!(matches!(row.class, EntryTerminal | StreamLocal));
    }

    // quinn connection-level vocabulary: complete, all terminal.
    for item in [
        "VersionMismatch",
        "TransportError",
        "ConnectionClosed",
        "ApplicationClosed",
        "Reset",
        "TimedOut",
        "LocallyClosed",
        "CidsExhausted",
    ] {
        assert_eq!(
            pinned_error_class("quinn", item),
            Some(EntryTerminal),
            "quinn::ConnectionError::{item}"
        );
    }

    // quinn stream-level vocabulary.
    assert_eq!(pinned_error_class("quinn", "Stopped"), Some(StreamLocal));
    assert_eq!(
        pinned_error_class("quinn", "ConnectionLost"),
        Some(EntryTerminal)
    );
    assert_eq!(
        pinned_error_class("quinn", "ClosedStream"),
        Some(StreamLocal)
    );
    assert_eq!(
        pinned_error_class("quinn", "IllegalOrderedRead"),
        Some(StreamLocal)
    );
    assert_eq!(
        pinned_error_class("quinn", "ZeroRttRejected"),
        Some(EntryTerminal)
    );
    assert_eq!(pinned_error_class("quinn", "TooLong"), Some(StreamLocal));

    // h3-quinn backend vocabulary.
    for item in ["ApplicationClose", "Timeout", "InternalError", "Undefined"] {
        assert_eq!(
            pinned_error_class("h3-quinn", item),
            Some(EntryTerminal),
            "h3::quic::ConnectionErrorIncoming::{item}"
        );
    }
    assert_eq!(
        pinned_error_class("h3-quinn", "ConnectionErrorIncoming"),
        Some(EntryTerminal)
    );
    assert_eq!(
        pinned_error_class("h3-quinn", "StreamTerminated"),
        Some(StreamLocal)
    );
    assert_eq!(pinned_error_class("h3-quinn", "Unknown"), Some(StreamLocal));

    // This crate's own framing/response-validation errors stay stream-local.
    for item in [
        "DoqProtocolTrailingResponse",
        "DoqProtocolMissingResponseFin",
        "DoqProtocolNonzeroResponseId",
        "PeerStreamTerminated",
        "ResponseHeadTooLarge",
        "IncompleteBody",
    ] {
        assert_eq!(
            pinned_error_class("mosdns-upstream-core", item),
            Some(StreamLocal),
            "{item}"
        );
    }
    assert_eq!(pinned_error_class("quinn", "NoSuchVariant"), None);
    assert_eq!(
        pinned_error_class("h3", "any request-stream outcome"),
        Some(StreamLocal)
    );
    assert_eq!(
        pinned_error_class("h3", "any driver poll_close outcome"),
        Some(EntryTerminal)
    );
}

#[test]
fn r0b_side_effect_states_are_recorded_independently_of_the_class() {
    assert_eq!(
        side_effect_of(QuicExchangePhase::PreSend),
        SideEffectState::NotSent
    );
    assert_eq!(
        side_effect_of(QuicExchangePhase::Connect),
        SideEffectState::NotSent
    );
    assert_eq!(
        side_effect_of(QuicExchangePhase::SendUncertain),
        SideEffectState::MaybeSent
    );
    assert_eq!(
        side_effect_of(QuicExchangePhase::RequestFin),
        SideEffectState::Sent
    );
    assert_eq!(
        side_effect_of(QuicExchangePhase::ResponseRead),
        SideEffectState::Sent
    );

    // Independence: an entry-terminal signal can be observed before anything was
    // sent, and a stream-local failure can be observed after a query was sent.
    assert_eq!(
        classify_quinn_connection_error(&ConnectionError::LocallyClosed),
        QuicErrorClass::EntryTerminal
    );
    assert_eq!(
        side_effect_of(QuicExchangePhase::Connect),
        SideEffectState::NotSent
    );
    assert_eq!(
        classify_quinn_read_error(&ReadError::Reset(VarInt::from_u32(3))),
        QuicErrorClass::StreamLocal
    );
    assert_eq!(
        side_effect_of(QuicExchangePhase::ResponseRead),
        SideEffectState::Sent
    );
}

#[test]
fn r0b_entry_terminal_deactivates_exactly_one_generation() {
    block_on(async {
        let h = harness();
        let key = doq_key(9002);

        let first = admit_ok(&h.owner, &key).await;
        let first_generation = first.generation();
        drop(first);
        assert_eq!(
            h.owner.observe(&key).expect("observation").phase,
            EntryPhase::Active
        );

        // A connection-level classification logically deactivates the exact
        // key+generation: immediately unleasable, still discoverable.
        assert_eq!(
            h.owner.apply_error_class(
                &key,
                first_generation,
                classify_quinn_connection_error(&ConnectionError::TimedOut)
            ),
            QuicErrorOutcome::EntryDeactivated
        );
        let closing = h.owner.observe(&key).expect("still discoverable");
        assert_eq!(closing.phase, EntryPhase::Closing);
        assert_eq!(closing.health, EntryHealth::Dead);
        assert_eq!(closing.generation, first_generation);

        // Re-applying the same class is idempotent and never a second teardown.
        assert_eq!(
            h.owner
                .apply_error_class(&key, first_generation, QuicErrorClass::EntryTerminal),
            QuicErrorOutcome::EntryDeactivated
        );
        assert_eq!(closing.record.teardown_runs(), 1);

        // A same-key admission while `Closing` is pre-send closed, with no
        // second generation and no lease of the closing slot.
        assert_closed_not_sent(&admit_once(&h.owner, &key).await);
        assert_eq!(h.owner.entry_count(), 1);
        assert_eq!(
            h.owner.observe(&key).expect("observation").generation,
            first_generation
        );

        // Removal happens only at terminal, and only then may a fresh generation
        // be admitted.
        assert_eq!(
            bounded(closing.record.wait_terminal()).await,
            EntryTerminal::Drained
        );
        assert!(h.owner.observe(&key).is_none());
        let second = admit_ok(&h.owner, &key).await;
        assert!(second.generation() > first_generation);
    });
}

// ---------------------------------------------------------------------------
// A6 / A13 — atomic multi-key admission and terminal-only slot accounting
// ---------------------------------------------------------------------------

#[test]
fn concurrent_distinct_key_admissions_never_exceed_the_owner_cap() {
    block_on(async {
        let h = harness();
        let parties = MAX_CONNECTIONS_PER_OWNER * 2;
        let gate = Arc::new(ModelBarrier::with_parties(parties));
        let mut handles = Vec::with_capacity(parties);

        for index in 0..parties {
            let owner = Arc::clone(&h.owner);
            let key = doq_key(1000 + u16::try_from(index).expect("port fits"));
            let gate = Arc::clone(&gate);
            handles.push(tokio::spawn(async move {
                let registration = owner.register().expect("the owner is open");
                let control = control();
                gate.check().await;
                let result = owner.admit(registration, key, &control).await;
                (result.is_ok(), result.err())
            }));
        }

        // Every task reaches the gate before any admission section runs.
        bounded(gate.wait_arrived()).await;
        gate.release();

        let mut leased = 0_usize;
        let mut backpressure = 0_usize;
        for handle in handles {
            let (ok, error) = bounded(handle).await.expect("the admission task joins");
            if ok {
                leased += 1;
                continue;
            }
            assert!(
                matches!(
                    error,
                    Some(UpstreamError::Backpressure(SideEffectState::NotSent))
                ),
                "excess admission must get the typed pre-send backpressure, got {error:?}"
            );
            backpressure += 1;
        }

        assert_eq!(leased, MAX_CONNECTIONS_PER_OWNER);
        assert_eq!(backpressure, MAX_CONNECTIONS_PER_OWNER);
        assert_eq!(h.owner.entry_count(), MAX_CONNECTIONS_PER_OWNER);
        assert!(h.owner.is_accepting());
    });
}

#[test]
fn same_key_concurrent_admissions_install_exactly_one_generation() {
    block_on(async {
        let init_barrier = Arc::new(ModelBarrier::new());
        let factory = Arc::new(TransportFactory::default());
        let initializer = Arc::new(ModelInitializer::new(
            Some(Arc::clone(&init_barrier)),
            None,
            true,
            Arc::clone(&factory),
        ));
        let h = harness_with(initializer.clone(), ModelSeams::default());
        let key = doq_key(1100);

        let mut handles = Vec::new();
        for _ in 0..4 {
            let owner = Arc::clone(&h.owner);
            let key = key.clone();
            handles.push(tokio::spawn(async move {
                let registration = owner.register().expect("the owner is open");
                let control = control();
                owner.admit(registration, key, &control).await
            }));
        }

        // The single reservation is installed and its one entry-owned
        // initializer is parked before acquiring: no caller has driven it.
        bounded(init_barrier.wait_arrived()).await;
        assert_eq!(h.owner.entry_count(), 1, "one key, one reservation");
        assert_eq!(initializer.calls(), 1, "one entry-owned initializer");
        assert_eq!(factory.produced(), 0, "nothing acquired yet");
        let generation = h.owner.observe(&key).expect("reservation").generation;

        init_barrier.release();
        let mut generations = Vec::new();
        for handle in handles {
            let lease = bounded(handle)
                .await
                .expect("the admission task joins")
                .expect("every same-key caller leases the published entry");
            generations.push(lease.generation());
        }
        assert!(generations.iter().all(|value| *value == generation));
        assert_eq!(h.owner.entry_count(), 1);
        assert_eq!(initializer.calls(), 1);
        assert_eq!(factory.produced(), 1);
    });
}

#[test]
fn initializing_and_closing_entries_hold_their_slot_until_terminal() {
    block_on(async {
        let init_barrier = Arc::new(ModelBarrier::new());
        let teardown_barrier = Arc::new(ModelBarrier::new());
        let factory = Arc::new(TransportFactory::default());
        let reserved_key = doq_key(1200);
        let initializer = Arc::new(
            ModelInitializer::new(
                Some(Arc::clone(&init_barrier)),
                None,
                true,
                Arc::clone(&factory),
            )
            .for_key(reserved_key.clone()),
        );
        let h = harness_with(
            initializer,
            ModelSeams {
                close_after_linearize: None,
                teardown_before_terminal: Some(Arc::clone(&teardown_barrier)),
            },
        );

        let leader = tokio::spawn({
            let owner = Arc::clone(&h.owner);
            let key = reserved_key.clone();
            async move {
                let registration = owner.register().expect("the owner is open");
                let control = control();
                owner.admit(registration, key, &control).await
            }
        });
        bounded(init_barrier.wait_arrived()).await;
        let reserved = h.owner.observe(&reserved_key).expect("reservation");
        let reserved_generation = reserved.generation;
        let reserved_record = Arc::clone(&reserved.record);
        assert_eq!(reserved.phase, EntryPhase::Initializing);

        // Fill the remaining capacity with published entries.
        let mut leases = Vec::new();
        for index in 1..MAX_CONNECTIONS_PER_OWNER {
            let key = doq_key(1200 + u16::try_from(index).expect("port fits"));
            leases.push(admit_ok(&h.owner, &key).await);
        }
        assert_eq!(h.owner.entry_count(), MAX_CONNECTIONS_PER_OWNER);

        // `Initializing` occupies its slot.
        let extra = doq_key(1300);
        assert!(matches!(
            admit_once(&h.owner, &extra).await,
            Err(UpstreamError::Backpressure(SideEffectState::NotSent))
        ));

        // Logically deactivate the reservation: `Closing` still occupies it.
        assert_eq!(
            h.owner.apply_error_class(
                &reserved_key,
                reserved_generation,
                QuicErrorClass::EntryTerminal
            ),
            QuicErrorOutcome::EntryDeactivated
        );
        assert_eq!(
            h.owner.observe(&reserved_key).expect("still there").phase,
            EntryPhase::Closing
        );
        assert!(matches!(
            admit_once(&h.owner, &extra).await,
            Err(UpstreamError::Backpressure(SideEffectState::NotSent))
        ));

        // Release the initializer: it acquires its resource after teardown was
        // requested, never publishes `Active`, and hands it to teardown.
        init_barrier.release();
        bounded(teardown_barrier.wait_arrived()).await;
        let observed = h.owner.observe(&reserved_key).expect("still discoverable");
        assert_eq!(observed.phase, EntryPhase::Closing);
        assert_eq!(observed.health, EntryHealth::Dead);
        assert_eq!(reserved_record.terminal(), None, "no terminal while parked");
        drop(leases);

        teardown_barrier.release();
        assert_eq!(
            bounded(reserved_record.wait_terminal()).await,
            EntryTerminal::Drained
        );
        assert_eq!(reserved_record.resource_closes(), 1);
        assert!(h.owner.observe(&reserved_key).is_none());

        // The original caller never leased: it observed the deactivation.
        assert_closed_not_sent(&bounded(leader).await.expect("the caller joins"));

        // The slot is reusable only now, and the fresh entry is a new generation.
        let replacement = admit_ok(&h.owner, &extra).await;
        assert!(replacement.generation() > reserved_generation);
        assert_eq!(h.owner.entry_count(), MAX_CONNECTIONS_PER_OWNER);
    });
}

#[test]
fn same_key_closing_lookup_is_closed_not_sent_without_a_drain_wait() {
    block_on(async {
        let teardown_barrier = Arc::new(ModelBarrier::new());
        let h = harness_with(
            Arc::new(ImmediateInitializer::default()),
            ModelSeams {
                close_after_linearize: None,
                teardown_before_terminal: Some(Arc::clone(&teardown_barrier)),
            },
        );
        let key = doq_key(1400);

        let lease = admit_ok(&h.owner, &key).await;
        let generation = lease.generation();
        let record = Arc::clone(lease.record());
        drop(lease);
        assert_eq!(
            h.owner
                .apply_error_class(&key, generation, QuicErrorClass::EntryTerminal),
            QuicErrorOutcome::EntryDeactivated
        );

        // The lookup finds `Closing` and returns immediately: no drain wait, no
        // second generation, no lease of the closing slot.
        assert_closed_not_sent(&admit_once(&h.owner, &key).await);
        let observed = h.owner.observe(&key).expect("still discoverable");
        assert_eq!(observed.phase, EntryPhase::Closing);
        assert_eq!(observed.generation, generation);
        assert_eq!(h.owner.entry_count(), 1);
        assert_eq!(record.teardown_runs(), 1);
        assert_eq!(record.terminal(), None, "teardown is still parked");

        // Only terminal removal frees the key for a fresh generation.
        teardown_barrier.release();
        assert_eq!(record.wait_terminal().await, EntryTerminal::Drained);
        assert!(h.owner.observe(&key).is_none());
        let fresh = admit_ok(&h.owner, &key).await;
        assert!(fresh.generation() > generation);
    });
}

#[test]
fn stage_two_only_gate_rejects_a_registered_but_unadmitted_exchange() {
    block_on(async {
        let factory = Arc::new(TransportFactory::default());
        let initializer = Arc::new(ModelInitializer::new(
            None,
            None,
            true,
            Arc::clone(&factory),
        ));
        let h = harness_with(initializer.clone(), ModelSeams::default());
        let key = doq_key(1502);

        // The exchange registers while the owner is `Open`, then parks before the
        // owner-map admission section (design §11.2 step 1).
        let registration = h.owner.register().expect("the owner is open");
        assert_eq!(h.owner.in_flight_exchanges(), 1);
        assert_eq!(initializer.calls(), 0);
        assert_eq!(h.owner.entry_count(), 0);

        // Only the map-side admission gate is withdrawn: `accepting = false` with
        // the real `Lifecycle` still `Open`, so this exchange's own registration
        // would still succeed if the gate were not checked first.
        let captured = h.owner.linearize_close();
        assert!(captured.is_empty());
        assert!(!h.owner.is_accepting());
        assert_eq!(
            h.owner.lifecycle_state(),
            mosdns_upstream_core::LifecycleState::Open
        );

        // The gate must reject before installing anything: no reservation, no
        // initializer task, no liveness or slot residue.
        let control = control();
        let rejected = h.owner.admit(registration, key.clone(), &control).await;
        assert_closed_not_sent(&rejected);
        assert_eq!(initializer.calls(), 0, "no initializer task was started");
        assert_eq!(factory.produced(), 0, "nothing was acquired");
        assert!(h.owner.observe(&key).is_none());
        assert_eq!(h.owner.entry_count(), 0);
        assert_eq!(h.owner.in_flight_exchanges(), 0, "zero liveness residue");
        assert!(!h.owner.is_accepting());
    });
}

#[test]
fn post_close_admission_race_leaves_no_residue() {
    block_on(async {
        let factory = Arc::new(TransportFactory::default());
        let initializer = Arc::new(ModelInitializer::new(
            None,
            None,
            true,
            Arc::clone(&factory),
        ));
        let h = harness_with(initializer.clone(), ModelSeams::default());

        let parked_key = doq_key(1500);
        let captured_key = doq_key(1501);

        // Exchange A completes `Lifecycle::register` while the owner is `Open`,
        // then parks before the owner-map admission section.
        let registration = h.owner.register().expect("the owner is open");
        assert_eq!(h.owner.in_flight_exchanges(), 1);

        // One entry is installed before close so stage two captures it.
        let captured_lease = admit_ok(&h.owner, &captured_key).await;
        let captured_record = Arc::clone(captured_lease.record());
        drop(captured_lease);

        // Owner close: stage one then stage two, in the frozen order.
        assert_eq!(
            h.owner.begin_close(),
            mosdns_upstream_core::CloseTransition::BeganClosing
        );
        let captured = h.owner.linearize_close();
        assert!(!h.owner.is_accepting());
        assert_eq!(captured.len(), 1);
        assert_eq!(captured[0].generation(), captured_record.generation());

        // Release A: rejected at the gate, installing nothing.
        let control = control();
        let rejected = h
            .owner
            .admit(registration, parked_key.clone(), &control)
            .await;
        assert_closed_not_sent(&rejected);
        assert!(
            h.owner.observe(&parked_key).is_none(),
            "no reservation, no initializer, no second generation"
        );
        assert_eq!(h.owner.entry_count(), 1);
        assert_eq!(initializer.calls(), 1, "no extra initializer task");
        // A's registration plus any local liveness guard were released.
        assert_eq!(h.owner.in_flight_exchanges(), 1);

        // The entry captured by close still finishes through the existing
        // `Closing -> terminal` protocol, with terminal-only removal.
        assert_eq!(
            bounded(captured_record.wait_terminal()).await,
            EntryTerminal::Drained
        );
        assert_eq!(h.owner.entry_count(), 0);
        assert_eq!(h.owner.in_flight_exchanges(), 0);
        assert_eq!(h.lifecycle.finish_close(), CloseCompletion::Closed);
        assert_eq!(
            h.owner.lifecycle_state(),
            mosdns_upstream_core::LifecycleState::Closed
        );
    });
}

// ---------------------------------------------------------------------------
// A8 / A13, design §11.1 — init-vs-owner-close with a late resource
// ---------------------------------------------------------------------------

#[test]
fn init_vs_owner_close_hands_the_late_resource_to_teardown() {
    block_on(async {
        let init_barrier = Arc::new(ModelBarrier::new());
        let teardown_barrier = Arc::new(ModelBarrier::new());
        let factory = Arc::new(TransportFactory::default());
        let initializer = Arc::new(ModelInitializer::new(
            Some(Arc::clone(&init_barrier)),
            None,
            true,
            Arc::clone(&factory),
        ));
        let h = harness_with(
            initializer.clone(),
            ModelSeams {
                close_after_linearize: None,
                teardown_before_terminal: Some(Arc::clone(&teardown_barrier)),
            },
        );
        let key = doq_key(1600);

        // The first initializer caller installs the reservation and waits; the
        // entry-owned initializer is parked before it acquires anything.
        let leader = tokio::spawn({
            let owner = Arc::clone(&h.owner);
            let key = key.clone();
            async move {
                let registration = owner.register().expect("the owner is open");
                let control = control();
                owner.admit(registration, key, &control).await
            }
        });
        bounded(init_barrier.wait_arrived()).await;
        let reserved = h.owner.observe(&key).expect("reservation");
        let record = Arc::clone(&reserved.record);

        // Abort the first initializer caller and drop every exchange waiter: the
        // initializer task is entry-owned and survives.
        leader.abort();
        assert!(bounded(leader).await.is_err(), "the leader was aborted");
        assert_eq!(h.owner.entry_count(), 1, "the reservation survives");
        assert_eq!(record.initialization_completions(), 0);

        // Close wins the shared lock first: stage one then stage two.
        assert_eq!(
            h.owner.begin_close(),
            mosdns_upstream_core::CloseTransition::BeganClosing
        );
        let captured = h.owner.linearize_close();
        assert!(!h.owner.is_accepting());
        assert_eq!(captured.len(), 1);
        assert_eq!(record.teardown_runs(), 1, "exactly one teardown");
        assert_eq!(record.terminal(), None);
        assert_eq!(
            h.owner.observe(&key).expect("discoverable").phase,
            EntryPhase::Closing
        );
        assert_eq!(
            h.owner.in_flight_exchanges(),
            1,
            "liveness held to terminal"
        );

        // The initializer completes and acquires its resource *after* close won.
        init_barrier.release();
        bounded(teardown_barrier.wait_arrived()).await;
        assert_eq!(record.initialization_completions(), 1);
        assert_eq!(factory.produced(), 1);
        let transport = factory.last();
        let observed = h.owner.observe(&key).expect("still discoverable");
        assert_eq!(observed.phase, EntryPhase::Closing, "never Active");
        assert_eq!(observed.health, EntryHealth::Dead);
        assert!(!transport.is_closed(), "teardown has not closed it yet");
        assert_eq!(record.terminal(), None, "removal is terminal-only");

        // Release teardown: the late resource is taken over and closed exactly
        // once, then the entry is removed and liveness released.
        teardown_barrier.release();
        assert_eq!(
            bounded(record.wait_terminal()).await,
            EntryTerminal::Drained
        );
        assert!(transport.is_closed());
        assert_eq!(transport.close_calls(), 1);
        assert_eq!(record.resource_closes(), 1);
        assert!(h.owner.observe(&key).is_none());
        assert_eq!(h.owner.in_flight_exchanges(), 0);
        assert_eq!(h.owner.entry_count(), 0);
        assert_eq!(factory.produced(), 1, "no orphan and no second resource");
        assert_eq!(record.teardown_runs(), 1);
    });
}

// ---------------------------------------------------------------------------
// A5 / A13, design §11.1 — aborted-at-barrier, no surviving caller
// ---------------------------------------------------------------------------

#[test]
fn aborted_at_barrier_with_no_surviving_caller_reaches_terminal() {
    block_on(async {
        let init_barrier = Arc::new(ModelBarrier::new());
        let close_barrier = Arc::new(ModelBarrier::with_parties(2));
        let teardown_barrier = Arc::new(ModelBarrier::new());
        let factory = Arc::new(TransportFactory::default());
        let initializer = Arc::new(ModelInitializer::new(
            Some(Arc::clone(&init_barrier)),
            None,
            true,
            Arc::clone(&factory),
        ));
        let h = harness_with(
            initializer,
            ModelSeams {
                close_after_linearize: Some(Arc::clone(&close_barrier)),
                teardown_before_terminal: Some(Arc::clone(&teardown_barrier)),
            },
        );
        let key = doq_key(1700);

        let leader = tokio::spawn({
            let owner = Arc::clone(&h.owner);
            let key = key.clone();
            async move {
                let registration = owner.register().expect("the owner is open");
                let control = control();
                owner.admit(registration, key, &control).await
            }
        });
        bounded(init_barrier.wait_arrived()).await;
        let reserved = h.owner.observe(&key).expect("reservation");
        let record = Arc::clone(&reserved.record);
        leader.abort();
        assert!(bounded(leader).await.is_err());

        // Two concurrent close callers linearize admission, then park before
        // waiting on any terminal completion.
        let mut close_tasks = Vec::new();
        for _ in 0..2 {
            let owner = Arc::clone(&h.owner);
            close_tasks.push(tokio::spawn(async move { owner.close().await }));
        }
        bounded(close_barrier.wait_arrived()).await;

        // Held-barrier assertions: `Closing` is discoverable, the reservation was
        // not removed, no completion exists, and the `Lifecycle` has not drained.
        let closing = h.owner.observe(&key).expect("discoverable while Closing");
        assert_eq!(closing.phase, EntryPhase::Closing);
        assert_eq!(closing.generation, reserved.generation);
        assert_eq!(record.teardown_runs(), 1);
        assert_eq!(record.terminal(), None);
        assert_eq!(
            h.owner.lifecycle_state(),
            mosdns_upstream_core::LifecycleState::Closing
        );
        assert_eq!(h.owner.in_flight_exchanges(), 1);
        assert!(!h.owner.is_accepting());

        // Abort the first close waiter and then all of them: teardown is owned by
        // the supervised task and still progresses.
        close_tasks[0].abort();
        close_tasks[1].abort();
        for task in close_tasks {
            assert!(
                bounded(task).await.is_err(),
                "both close waiters were aborted"
            );
        }

        init_barrier.release();
        bounded(teardown_barrier.wait_arrived()).await;
        assert_eq!(record.initialization_completions(), 1);
        assert_eq!(
            h.owner.observe(&key).expect("discoverable").phase,
            EntryPhase::Closing
        );

        teardown_barrier.release();
        assert_eq!(
            bounded(record.wait_terminal()).await,
            EntryTerminal::Drained
        );
        assert_eq!(record.resource_closes(), 1);
        assert_eq!(factory.produced(), 1);
        assert!(h.owner.observe(&key).is_none(), "removal happened");
        assert_eq!(
            h.owner.in_flight_exchanges(),
            0,
            "liveness released at terminal"
        );
        assert_eq!(h.lifecycle.finish_close(), CloseCompletion::Closed);
    });
}

// ---------------------------------------------------------------------------
// A8 / A13, design §11.3 — the publication gate in both stage orders
// ---------------------------------------------------------------------------

#[test]
fn begin_close_without_stage_two_already_fails_publication() {
    block_on(async {
        let init_barrier = Arc::new(ModelBarrier::new());
        let teardown_barrier = Arc::new(ModelBarrier::new());
        let factory = Arc::new(TransportFactory::default());
        let initializer = Arc::new(ModelInitializer::new(
            None,
            Some(Arc::clone(&init_barrier)),
            true,
            Arc::clone(&factory),
        ));
        let h = harness_with(
            initializer,
            ModelSeams {
                close_after_linearize: None,
                teardown_before_terminal: Some(Arc::clone(&teardown_barrier)),
            },
        );
        let key = doq_key(1800);

        let leader = tokio::spawn({
            let owner = Arc::clone(&h.owner);
            let key = key.clone();
            async move {
                let registration = owner.register().expect("the owner is open");
                let control = control();
                owner.admit(registration, key, &control).await
            }
        });

        // The entry-owned initializer has acquired its resource and is parked
        // immediately before its publication attempt.
        bounded(init_barrier.wait_arrived()).await;
        assert_eq!(
            h.owner.observe(&key).expect("reservation").phase,
            EntryPhase::Initializing
        );
        assert_eq!(factory.produced(), 1);

        // Only stage one of the two-stage close runs.
        assert_eq!(
            h.owner.begin_close(),
            mosdns_upstream_core::CloseTransition::BeganClosing
        );
        assert_eq!(
            h.owner.lifecycle_state(),
            mosdns_upstream_core::LifecycleState::Closing
        );
        assert!(h.owner.is_accepting(), "stage two has not linearized yet");

        // The publication attempt now fails the three-way condition: the real
        // `Lifecycle` is no longer `Open`, so `Active` is never published and the
        // retained resource takes the supervised-teardown path.
        init_barrier.release();
        bounded(teardown_barrier.wait_arrived()).await;
        let observed = h.owner.observe(&key).expect("discoverable");
        assert_eq!(observed.phase, EntryPhase::Closing);
        assert_eq!(observed.health, EntryHealth::Dead);
        let record = Arc::clone(&observed.record);
        assert_eq!(record.initialization_completions(), 1);
        assert_eq!(record.teardown_runs(), 1);
        assert_eq!(record.terminal(), None);

        // Stage two now runs: the entry is already `Closing`, so no second
        // teardown is started and the outcome is unchanged.
        let captured = h.owner.linearize_close();
        assert!(!h.owner.is_accepting());
        assert_eq!(captured.len(), 1);
        assert_eq!(record.teardown_runs(), 1);

        teardown_barrier.release();
        assert_eq!(
            bounded(record.wait_terminal()).await,
            EntryTerminal::Drained
        );
        assert_eq!(record.resource_closes(), 1);
        assert!(h.owner.observe(&key).is_none());
        assert_closed_not_sent(&bounded(leader).await.expect("leader joins"));
    });
}

#[test]
fn stage_two_without_stage_one_also_fails_publication() {
    block_on(async {
        let init_barrier = Arc::new(ModelBarrier::new());
        let teardown_barrier = Arc::new(ModelBarrier::new());
        let factory = Arc::new(TransportFactory::default());
        let initializer = Arc::new(ModelInitializer::new(
            None,
            Some(Arc::clone(&init_barrier)),
            true,
            Arc::clone(&factory),
        ));
        let h = harness_with(
            initializer,
            ModelSeams {
                close_after_linearize: None,
                teardown_before_terminal: Some(Arc::clone(&teardown_barrier)),
            },
        );
        let key = doq_key(1801);

        let leader = tokio::spawn({
            let owner = Arc::clone(&h.owner);
            let key = key.clone();
            async move {
                let registration = owner.register().expect("the owner is open");
                let control = control();
                owner.admit(registration, key, &control).await
            }
        });
        bounded(init_barrier.wait_arrived()).await;
        assert_eq!(
            h.owner.observe(&key).expect("reservation").phase,
            EntryPhase::Initializing
        );

        // The inverse order: the map admission gate closes while the real
        // `Lifecycle` is still `Open`. Neither check alone is sufficient.
        let captured = h.owner.linearize_close();
        assert!(!h.owner.is_accepting());
        assert_eq!(
            h.owner.lifecycle_state(),
            mosdns_upstream_core::LifecycleState::Open
        );
        assert_eq!(captured.len(), 1);

        init_barrier.release();
        bounded(teardown_barrier.wait_arrived()).await;
        let observed = h.owner.observe(&key).expect("discoverable");
        assert_eq!(observed.phase, EntryPhase::Closing, "never Active");
        let record = Arc::clone(&observed.record);
        assert_eq!(record.teardown_runs(), 1, "exactly one teardown");
        assert_eq!(record.initialization_completions(), 1);

        teardown_barrier.release();
        assert_eq!(
            bounded(record.wait_terminal()).await,
            EntryTerminal::Drained
        );
        assert_eq!(record.resource_closes(), 1);
        assert_eq!(factory.produced(), 1);
        assert!(h.owner.observe(&key).is_none());
        assert_eq!(h.owner.in_flight_exchanges(), 0);
        assert_closed_not_sent(&bounded(leader).await.expect("leader joins"));
    });
}

// ---------------------------------------------------------------------------
// A7 — lazy idle expiry through the injected clock
// ---------------------------------------------------------------------------

#[test]
fn idle_expiry_marks_closing_without_removal() {
    block_on(async {
        let h = harness();
        let key = doq_key(1900);

        let lease = admit_ok(&h.owner, &key).await;
        let generation = lease.generation();
        let record = Arc::clone(lease.record());
        drop(lease);

        // Used before expiry: still reusable on the same generation.
        h.clock.advance(
            QUIC_IDLE_TIMEOUT
                .checked_sub(Duration::from_secs(1))
                .expect("the idle timeout is at least one second"),
        );
        let reuse = admit_ok(&h.owner, &key).await;
        assert_eq!(reuse.generation(), generation);
        drop(reuse);

        // Idle past the timeout: `Closing`, discoverable, unleasable.
        h.clock.advance(QUIC_IDLE_TIMEOUT + Duration::from_secs(1));
        h.owner.maintain();
        let expired = h.owner.observe(&key).expect("stays discoverable");
        assert_eq!(expired.phase, EntryPhase::Closing);
        assert_eq!(expired.health, EntryHealth::Dead);
        assert_eq!(expired.generation, generation);
        assert_eq!(record.teardown_runs(), 1);
        assert_closed_not_sent(&admit_once(&h.owner, &key).await);

        // Removed only at terminal.
        assert_eq!(
            bounded(record.wait_terminal()).await,
            EntryTerminal::Drained
        );
        assert!(h.owner.observe(&key).is_none());
        let fresh = admit_ok(&h.owner, &key).await;
        assert!(fresh.generation() > generation);
    });
}

#[test]
fn stale_generation_callbacks_never_touch_a_newer_generation() {
    block_on(async {
        let h = harness();
        let key = doq_key(2000);

        let first = admit_ok(&h.owner, &key).await;
        let first_generation = first.generation();
        drop(first);
        assert_eq!(
            h.owner
                .apply_error_class(&key, first_generation, QuicErrorClass::EntryTerminal),
            QuicErrorOutcome::EntryDeactivated
        );
        h.owner.maintain_and_drain().await;
        assert!(h.owner.observe(&key).is_none());

        let second = admit_ok(&h.owner, &key).await;
        let second_generation = second.generation();
        drop(second);
        assert!(second_generation > first_generation);

        // A stale callback for the old generation must not touch the new one.
        assert_eq!(
            h.owner
                .apply_error_class(&key, first_generation, QuicErrorClass::EntryTerminal),
            QuicErrorOutcome::NoMatchingEntry
        );
        let observed = h.owner.observe(&key).expect("observation");
        assert_eq!(observed.generation, second_generation);
        assert_eq!(observed.phase, EntryPhase::Active);
        assert_eq!(observed.health, EntryHealth::Healthy);
        // And a stream-local class for the live generation is still a no-op.
        assert_eq!(
            h.owner
                .apply_error_class(&key, second_generation, QuicErrorClass::StreamLocal),
            QuicErrorOutcome::EntryUnchanged
        );
    });
}

// ---------------------------------------------------------------------------
// A6 — local stream-slot backpressure, permit release, no queue growth
// ---------------------------------------------------------------------------

#[test]
fn stream_slot_backpressure_and_permit_release() {
    block_on(async {
        let h = harness();
        let key = doq_key(2100);

        let mut leases = Vec::new();
        for _ in 0..MAX_STREAMS_PER_CONNECTION {
            leases.push(admit_ok(&h.owner, &key).await);
        }
        assert_eq!(
            h.owner.observe(&key).expect("observation").permits_in_use,
            MAX_STREAMS_PER_CONNECTION
        );

        // Exhausted local slots: typed pre-send backpressure, no queue, and no
        // second connection for the same key.
        assert!(matches!(
            admit_once(&h.owner, &key).await,
            Err(UpstreamError::Backpressure(SideEffectState::NotSent))
        ));
        assert_eq!(
            h.owner.observe(&key).expect("observation").permits_in_use,
            MAX_STREAMS_PER_CONNECTION
        );
        assert_eq!(h.owner.entry_count(), 1);

        // Releasing exactly one permit admits exactly one more lease.
        let released = leases.pop().expect("a lease");
        let generation = released.generation();
        drop(released);
        assert_eq!(
            h.owner.observe(&key).expect("observation").permits_in_use,
            MAX_STREAMS_PER_CONNECTION - 1
        );
        let extra = admit_ok(&h.owner, &key).await;
        assert_eq!(extra.generation(), generation);
        drop(extra);

        // Dropping every lease returns the slot count to zero exactly once.
        drop(leases);
        assert_eq!(
            h.owner.observe(&key).expect("observation").permits_in_use,
            0
        );
        assert_eq!(h.owner.entry_count(), 1);
    });
}

#[test]
fn owner_close_cancels_streams_and_waits_for_exchange_registrations() {
    block_on(async {
        let teardown_barrier = Arc::new(ModelBarrier::new());
        let factory = Arc::new(TransportFactory::default());
        let initializer = Arc::new(ModelInitializer::new(
            None,
            None,
            true,
            Arc::clone(&factory),
        ));
        let h = harness_with(
            initializer,
            ModelSeams {
                close_after_linearize: None,
                teardown_before_terminal: Some(Arc::clone(&teardown_barrier)),
            },
        );
        let key = doq_key(2200);

        let lease = admit_ok(&h.owner, &key).await;
        let generation = lease.generation();
        let record = Arc::clone(lease.record());
        assert_eq!(
            h.owner.in_flight_exchanges(),
            2,
            "entry liveness plus the lease"
        );

        assert_eq!(
            h.owner.begin_close(),
            mosdns_upstream_core::CloseTransition::BeganClosing
        );
        let captured = h.owner.linearize_close();
        assert_eq!(captured.len(), 1);

        // Held barrier: the entry is `Closing` and discoverable, teardown has
        // already cancelled the generation's outstanding request streams, and the
        // acquired transport is owned by the supervised task but not yet closed.
        bounded(teardown_barrier.wait_arrived()).await;
        let closing = h.owner.observe(&key).expect("discoverable while Closing");
        assert_eq!(closing.phase, EntryPhase::Closing);
        assert_eq!(closing.generation, generation);
        assert_eq!(
            closing.permits_in_use, 0,
            "teardown cancelled the stream permit"
        );
        assert_eq!(record.streams_cancelled(), 1);
        assert_eq!(record.resource_closes(), 0);
        assert!(!factory.last().is_closed());
        assert_eq!(record.terminal(), None, "removal is terminal-only");

        // Releasing an already-cancelled lease is a no-op: no permit underflow, no
        // resurrected slot, no early drain.
        drop(lease);
        assert_eq!(
            h.owner.observe(&key).expect("still there").permits_in_use,
            0
        );
        assert_eq!(
            h.owner.in_flight_exchanges(),
            1,
            "the entry's liveness remains"
        );

        teardown_barrier.release();
        assert_eq!(
            bounded(record.wait_terminal()).await,
            EntryTerminal::Drained
        );
        assert_eq!(record.resource_closes(), 1);
        assert!(factory.last().is_closed());
        assert!(h.owner.observe(&key).is_none(), "terminal-only removal");
        assert_eq!(h.owner.in_flight_exchanges(), 0);
        assert_eq!(h.lifecycle.finish_close(), CloseCompletion::Closed);
    });
}
