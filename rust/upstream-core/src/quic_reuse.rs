//! Phase 4 QUIC reuse/multiplexing — Slice 0 (model, R0 gates, no I/O).
//!
//! This module is the QUIC-specific shared-connection owner for the future
//! Rust-native host. It is deliberately **not** a specialization of the serial
//! TCP [`ReuseOwner`](crate::ReuseOwner): one validated [`QuicReuseKey`] maps to
//! one physical QUIC connection, concurrency is stream-level and bounded, and
//! the entry lifecycle is an explicit
//! `Initializing -> Active -> Closing -> Drained | Failed` state machine whose
//! terminal is the only removal point.
//!
//! # Slice 0 boundary (no socket, no QUIC/H3 I/O)
//!
//! Slice 0 implements and exercises the *model*: validated key isolation, the
//! two R0 classification contracts, atomic `accepting`-gated multi-key
//! admission, the single initialization crossing/handoff protocol with an
//! entry-owned cancellation-safe initializer, and the entry-owned supervised
//! teardown. **No code in this slice opens a socket, dials, handshakes, or
//! drives an H3 connection.** The physical transport is represented by
//! [`EntryTransport`], an inert token that only records that it was acquired
//! and later closed by the supervised teardown.
//!
//! ## R0a — H3 request-stream cancellation is a per-phase decision model
//!
//! Pinned hazard (`h3-quinn 0.0.10`, locked local registry source):
//! `RecvStream::poll_data` *takes* the inner `Option<quinn::RecvStream>` into its
//! in-flight `read_chunk_fut` (`h3-quinn-0.0.10/src/lib.rs:376`) and restores it
//! only when the read completes (`:383-384`); `RecvStream::stop_sending` then
//! does `self.stream.as_mut().unwrap()` (`:391-394`), so a local control
//! decision that drops the read leaves `None` and the later receive-side stop
//! panics. `recv_id()` unwraps the same option (`:399-404`).
//!
//! [`h3_cancellation_decision`] freezes the per-phase answer: only the phase
//! before any request byte is written may actively stop the **send** side
//! (`RequestStream::stop_stream`, `h3-0.0.8/src/client/stream.rs:251`); every
//! phase from "after request FIN" onward is **drop-only** — the unchanged typed
//! control error is returned and the `RequestStream` is dropped, letting
//! Quinn's `RecvStream::Drop` stop unread data with code zero
//! (`quinn-0.11.7/src/recv_stream.rs:500-515`).
//!
//! The decision model never selects the pinned receive-side stop
//! ([`H3CancellationAction::StopRecvSide`] exists only so the model can name and
//! forbid it), and a local request cancellation is stream-local: it leaves the
//! logical shared entry `Active`, healthy, and leasable
//! ([`QuicReuseOwner::apply_error_class`] with [`QuicErrorClass::StreamLocal`]).
//!
//! **This is model evidence only.** The real pinned-stack proof that cancelling
//! one actual H3 request leaves the shared connection, the driver, and another
//! concurrent request healthy is Slice 2/A5 and is not claimed here.
//!
//! ## R0b — connection-level versus stream-level classification
//!
//! [`QuicErrorClass`] has exactly two values and the `classify_*` functions are
//! the single authoritative mapping over the pinned `h3`/`h3-quinn`/`quinn`
//! vocabulary. Later slices must classify through them rather than at a call
//! site:
//!
//! * connection-level signals (any `quinn::ConnectionError`, the
//!   `h3::quic::ConnectionErrorIncoming` type, any `h3::error::ConnectionError`,
//!   `read`/`write` `ConnectionLost`, and any `h3::error::StreamError` produced
//!   by a connection error or a peer GOAWAY) are **entry-terminal**: they
//!   logically deactivate the exact key+generation `Active -> Closing` through
//!   [`QuicReuseOwner::apply_error_class`], which is immediately unleasable and
//!   removed from the map only at the terminal `Drained`/`Failed`;
//! * stream-local signals are the per-request outcomes the pinned h3 layer proves
//!   without any connection error: the `StreamError{..}` / `RemoteTerminate{..}` /
//!   `HeaderTooBig{..}` variants from `h3 0.0.8` (`error.rs:79-136`), plus the
//!   request-stream piece of the `h3-quinn` backend vocabulary
//!   (`StreamTerminated` / `Unknown`, where only `ClosedStream` is reachable and
//!   `ConnectionLost` is routed through its connection arm), framing and
//!   response-validation bounds, and `ClosedStream`. They keep a healthy entry
//!   reusable because a pinned connection error, when one exists, is recorded
//!   on the same shared state and reported by the driver's `poll_close` outcome.
//!
//! The one exception to "name everything by variant" is forced by the pinned API
//! itself: `h3 0.0.8` marks the whole `StreamError` enum **and** every one of its
//! variants `#[non_exhaustive]` with no opt-out feature, so a downstream crate
//! can only pattern-match the struct-shaped variants and can never construct or
//! match the tuple/unit ones (`ConnectionError(_)`, `RemoteClosing`,
//! `Undefined(_)`). For exactly this reason the R0b evidence records a second,
//! equivalent, classification rule that needs no variant name (see
//! [`classify_h3_stream_error`]): **a per-request `h3::error::StreamError`
//! reaches our call sites only together with the driver's error-shared state,
//! and the same pinned code that produces a connection-carrying stream error
//! (`ConnectionError(_, reason)`) or refuses new work (`RemoteClosing`) also
//! records a connection error on that state, so the independently terminal
//! [`classify_h3_connection_error`] (the driver's `poll_close` outcome) or
//! [`classify_h3_quinn_connection_error`] (the backend `ConnectionErrorIncoming`)
//! fires for the same event.** No variant is folded into the stream-local arm,
//! and no future `#[non_exhaustive]` variant can slip through: any tuple/unit
//! outcome reaches `StreamLocal` only after the shared-state rule has had its
//! say in a later slice, whose call sites must pair the stream error with the
//! driver observation.
//!
//! A `SendRequest`-level failure on its own proves neither health nor death, so
//! it is stream-local until the connection layer independently reports terminal.
//! [`side_effect_of`] records the `NotSent`/`MaybeSent`/`Sent` side-effect state
//! separately, because the two axes are independent.
//!
//! ## R0c — pinned API assumptions
//!
//! Every API fact this module relies on is verified against the locked local
//! registry source with an exact file/line citation in the task's
//! `research/slice0-r0-evidence.md`. The pinned graph is unchanged:
//! `quinn 0.11.7`, `h3 0.0.8`, `h3-quinn 0.0.10`. A pinned assumption that does
//! not hold stops the task for re-planning; adding, removing, or bumping a
//! dependency is never the remedy.
//!
//! ## Frozen contracts implemented here
//!
//! * validated key isolation (numeric dial, closed protocol/ALPN, canonical
//!   identity, TLS mode plus opaque roots revision, DoH3 authority);
//! * one no-`await` owner-map critical section that **checks `accepting` first**,
//!   looks the key up, marks dead/idle-expired entries `Closing` without
//!   removing them, capacity-checks counting `Initializing`/`Closing`/`Active`
//!   until terminal, and then reuses/joins/installs;
//! * one entry-owned initializer task per `Initializing` reservation whose
//!   completion is delivered exactly once, even if the initializer panics or its
//!   first caller is aborted;
//! * `Initializing -> Active` publication that requires `Lifecycle == Open`,
//!   `accepting == true`, and this exact generation still `Initializing` under
//!   that one lock; any miss hands the whole result (including a late-acquired
//!   resource) to the supervised teardown;
//! * two-stage owner close: stage one [`QuicReuseOwner::begin_close`] (the real
//!   `Lifecycle` `Open -> Closing` plus owner-token cancellation, never inside
//!   the map lock) then stage two [`QuicReuseOwner::linearize_close`] (the sole
//!   map-side admission-vs-close linearization);
//! * exactly one entry-owned supervised teardown per generation, which awaits the
//!   initializer handoff, cancels outstanding stream permits, closes the
//!   resource, and alone performs terminal removal plus slot/liveness release;
//! * same-key `Closing` lookup -> `UpstreamError::Closed(SideEffectState::NotSent)`
//!   with no drain wait, no second generation, and no lease of the `Closing` slot.
//!
//! ## Model seams (deterministic tests only)
//!
//! Slice 0 has no I/O to interleave with, so determinism is provided by explicit
//! seams rather than by sleeps: [`OwnerClock`]/[`ManualClock`] for idle expiry,
//! [`EntryInitializer`] for the entry-owned initializer body, and
//! [`ModelSeams::teardown_before_terminal`] plus [`ModelBarrier`] for the
//! close-vs-publication and aborted-at-barrier orderings. Slice 1/2 construct the
//! owner with [`ModelSeams::default`], which installs no barrier at all.

use std::collections::HashMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use tokio::sync::Notify;
use tokio::task::JoinHandle;

use crate::secure::{DohEndpoint, TlsPolicy};
use crate::tcp::race_control;
use crate::{
    CloseCompletion, CloseResult, CloseTransition, ExchangeContext, ExchangeControl, Lifecycle,
    LifecycleState, SharedInFlightGuard, SideEffectState, TransportCancellation, UpstreamError,
};

/// The exact DoQ ALPN, reusing the frozen constant rather than duplicating it.
pub use crate::quic::DOQ_ALPN;
/// The exact DoH3 ALPN, reusing the frozen constant.
pub use crate::quic::H3_ALPN;

/// The `h3` request-stream error code a safe active send-side cancellation uses
/// (RFC 9114 §7.2.3 `H3_REQUEST_CANCELLED`).
pub const H3_REQUEST_CANCELLED: u64 = 0x010c;

// ---------------------------------------------------------------------------
// Task-local bounds (frozen, non-configurable, not product behavior)
// ---------------------------------------------------------------------------

/// Maximum concurrent stream permits one QUIC connection entry hands out.
///
/// Task-local calibration only: implementation evidence may revise this value
/// without changing the public contract, but it stays finite and the
/// backpressure semantics stay explicit.
pub const MAX_STREAMS_PER_CONNECTION: usize = 32;

/// Maximum live QUIC connection entries one owner may hold across all keys.
///
/// `Initializing`, `Closing`, and `Active` entries all count against this bound
/// until their terminal `Drained`/`Failed` removal.
pub const MAX_CONNECTIONS_PER_OWNER: usize = 8;

/// Lazy idle expiry for an unused, healthy entry with no outstanding permit.
///
/// There is no background reaper and no timer: expiry is evaluated under the
/// owner-map lock by [`QuicReuseOwner::maintain`] and by the admission scan.
pub const QUIC_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// The closed protocol discriminator and the validated reuse key
// ---------------------------------------------------------------------------

/// The closed QUIC protocol/ALPN discriminator.
///
/// The set is closed on purpose: a caller cannot offer an arbitrary ALPN, so a
/// connection built for one protocol can never be reused for another.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum QuicProtocol {
    /// DNS-over-QUIC (RFC 9250), ALPN [`DOQ_ALPN`].
    Doq,
    /// DNS-over-HTTP/3 (RFC 9114), ALPN [`H3_ALPN`].
    Doh3,
}

impl QuicProtocol {
    /// The exact ALPN this protocol offers, and the only one.
    #[must_use]
    pub const fn alpn(self) -> &'static [u8] {
        match self {
            Self::Doq => DOQ_ALPN,
            Self::Doh3 => H3_ALPN,
        }
    }
}

/// The TLS trust mode of a reuse key.
///
/// The mode is one half of the trust identity; the opaque roots revision is the
/// other. No certificate, key, or digest material enters the key.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum QuicTlsMode {
    /// Certificate chain, name, and validity verification against explicit roots.
    Verified,
    /// The explicit opt-in policy that skips verification.
    InsecureSkipVerify,
}

impl QuicTlsMode {
    fn of(policy: &TlsPolicy) -> Self {
        if policy.is_insecure_skip_verify() {
            Self::InsecureSkipVerify
        } else {
            Self::Verified
        }
    }
}

/// A validated QUIC connection-reuse key.
///
/// Constructed only from an already-validated endpoint and [`TlsPolicy`], so an
/// invalid dial port or identity cannot reach the key. Two keys are equal only
/// when every isolation dimension agrees:
///
/// * the numeric dial address including port (never a hostname);
/// * the closed [`QuicProtocol`]/ALPN discriminator;
/// * the canonical service identity text;
/// * the TLS [`QuicTlsMode`] and the policy's opaque roots revision;
/// * the validated DoH3 HTTP authority, where it is distinct from the identity
///   (DoQ has no HTTP authority).
///
/// A resolver A/AAAA selection change therefore produces a new key for a new
/// dial without rewriting identity, authority, or an established connection.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct QuicReuseKey {
    dial: SocketAddr,
    protocol: QuicProtocol,
    identity: String,
    tls_mode: QuicTlsMode,
    roots_revision: u64,
    doh3_authority: Option<String>,
}

impl QuicReuseKey {
    /// Builds the DoQ reuse key for a validated endpoint and TLS policy.
    #[must_use]
    pub fn from_doq(endpoint: &crate::quic::DoqEndpoint, policy: &TlsPolicy) -> Self {
        Self {
            dial: endpoint.dial(),
            protocol: QuicProtocol::Doq,
            identity: endpoint.identity().as_str().to_owned(),
            tls_mode: QuicTlsMode::of(policy),
            roots_revision: policy.roots_revision(),
            doh3_authority: None,
        }
    }

    /// Builds the DoH3 reuse key for a validated endpoint and TLS policy.
    ///
    /// `DohEndpoint` separates the origin authority from the numeric dial, so
    /// the authority is part of the key: a connection authenticated for one
    /// origin must never silently serve a different one.
    #[must_use]
    pub fn from_doh3(endpoint: &DohEndpoint, policy: &TlsPolicy) -> Self {
        Self {
            dial: endpoint.dial(),
            protocol: QuicProtocol::Doh3,
            identity: endpoint.identity().as_str().to_owned(),
            tls_mode: QuicTlsMode::of(policy),
            roots_revision: policy.roots_revision(),
            doh3_authority: Some(endpoint.authority()),
        }
    }

    /// The numeric destination this key opens exactly one connection to.
    #[must_use]
    pub const fn dial(&self) -> SocketAddr {
        self.dial
    }

    /// The closed protocol/ALPN discriminator.
    #[must_use]
    pub const fn protocol(&self) -> QuicProtocol {
        self.protocol
    }

    /// The canonical service identity text.
    #[must_use]
    pub fn identity(&self) -> &str {
        &self.identity
    }

    /// The TLS trust mode.
    #[must_use]
    pub const fn tls_mode(&self) -> QuicTlsMode {
        self.tls_mode
    }

    /// The opaque roots revision identifying the exact trust configuration.
    #[must_use]
    pub const fn roots_revision(&self) -> u64 {
        self.roots_revision
    }

    /// The validated DoH3 HTTP authority, absent for DoQ.
    #[must_use]
    pub fn doh3_authority(&self) -> Option<&str> {
        self.doh3_authority.as_deref()
    }
}

// ---------------------------------------------------------------------------
// Entry lifecycle vocabulary
// ---------------------------------------------------------------------------

/// The stored phase of one connection entry.
///
/// `Drained` and `Failed` are terminal outcomes, not stored phases: terminal is
/// the only point at which the entry is removed from the owner map, so
/// [`EntryTerminal`] is recorded on the surviving [`EntryRecord`] instead.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryPhase {
    /// Reserved placeholder: one key+generation, one entry-owned initializer, no
    /// leases.
    Initializing,
    /// Published and leasable; reachable only from `Initializing`.
    Active,
    /// Unleasable but still in the map and discoverable until terminal.
    Closing,
}

/// The terminal outcome of one entry generation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryTerminal {
    /// The initializer handoff was observed, outstanding stream permits were
    /// cancelled, and the acquired transport (if any) was closed by the
    /// supervised teardown.
    Drained,
    /// Teardown could not complete gracefully, or nothing was left to drain.
    Failed,
}

/// The leaseability health of an entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryHealth {
    /// Reusable: a connection-level failure has not been classified for this
    /// exact key+generation.
    Healthy,
    /// Logically deactivated: never leasable again, removed only at terminal.
    Dead,
}

// ---------------------------------------------------------------------------
// R0a — per-phase H3 request-stream cancellation decision model
// ---------------------------------------------------------------------------

/// The four phases of one H3 request stream at which a local control decision
/// can win. The phase is a property of the caller's own progress, never a guess
/// from an error shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum H3RequestPhase {
    /// The stream exists (or is being opened) but no request byte has been
    /// written.
    BeforeSend,
    /// The request side reached STREAM FIN; only the receive side remains.
    AfterRequestFin,
    /// The response head is being read.
    ResponseHead,
    /// The response body is being read.
    BodyRead,
}

/// The safe local cancellation action for one H3 request phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum H3CancellationAction {
    /// Safely abort the **send** side with [`H3_REQUEST_CANCELLED`]
    /// (`RequestStream::stop_stream`) and then drop the stream.
    StopSendSide,
    /// Return the unchanged typed control error and drop the `RequestStream`.
    /// The receive side is stopped only by Quinn's `RecvStream::Drop`.
    DropOnly,
    /// The pinned `h3-quinn` receive-side stop (`RecvStream::stop_sending`),
    /// which panics when an aborted `poll_data` left the inner stream as `None`.
    ///
    /// **Never selected.** The variant exists so the decision model can name,
    /// document, and forbid the pinned hazard rather than silently omit it.
    StopRecvSide,
}

/// The frozen decision for one H3 request phase.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct H3CancellationDecision {
    phase: H3RequestPhase,
    action: H3CancellationAction,
    recv_stream_may_be_in_flight: bool,
}

impl H3CancellationDecision {
    /// The phase this decision was made for.
    #[must_use]
    pub const fn phase(self) -> H3RequestPhase {
        self.phase
    }

    /// The safe action for this phase.
    #[must_use]
    pub const fn action(self) -> H3CancellationAction {
        self.action
    }

    /// Whether a `h3-quinn` read future may already hold the inner
    /// `Option<quinn::RecvStream>` in its in-flight `read_chunk_fut`.
    ///
    /// `true` means an explicit receive-side stop is the pinned panic path.
    #[must_use]
    pub const fn recv_stream_may_be_in_flight(self) -> bool {
        self.recv_stream_may_be_in_flight
    }

    /// Whether this decision selects the pinned receive-side stop.
    ///
    /// Must be `false` for every phase.
    #[must_use]
    pub const fn selects_pinned_recv_stop(self) -> bool {
        matches!(self.action, H3CancellationAction::StopRecvSide)
    }
}

/// The frozen per-phase local cancellation decision for one H3 request stream.
///
/// The pinned `h3-quinn` receive stream is only provably still owned by the
/// caller *before* the response phase begins: from "after request FIN" the
/// caller races its control decision against the response read, so a read future
/// may already have taken the inner `Option`. Every such phase is therefore
/// drop-only.
#[must_use]
pub const fn h3_cancellation_decision(phase: H3RequestPhase) -> H3CancellationDecision {
    match phase {
        H3RequestPhase::BeforeSend => H3CancellationDecision {
            phase,
            action: H3CancellationAction::StopSendSide,
            recv_stream_may_be_in_flight: false,
        },
        H3RequestPhase::AfterRequestFin
        | H3RequestPhase::ResponseHead
        | H3RequestPhase::BodyRead => H3CancellationDecision {
            phase,
            action: H3CancellationAction::DropOnly,
            recv_stream_may_be_in_flight: true,
        },
    }
}

/// The frozen logical effect of one local request-stream cancellation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct H3CancellationEffect {
    /// The shared entry's phase after the cancellation. Always
    /// [`EntryPhase::Active`] for a published entry: a request cancellation is
    /// stream-local and never deactivates the shared connection.
    pub entry_phase_after: EntryPhase,
    /// Whether the shared connection is still healthy afterwards.
    pub connection_healthy_after: bool,
    /// Whether the entry can still be leased for a later independent query.
    pub entry_leasable_after: bool,
}

/// The frozen effect of a local request-stream cancellation on the shared entry.
///
/// This is deliberately independent of the phase: the phase decides *how* the
/// stream is torn down, never *whether* the shared entry survives.
#[must_use]
pub const fn h3_cancellation_effect(_phase: H3RequestPhase) -> H3CancellationEffect {
    H3CancellationEffect {
        entry_phase_after: EntryPhase::Active,
        connection_healthy_after: true,
        entry_leasable_after: true,
    }
}

// ---------------------------------------------------------------------------
// R0b — connection-level versus stream-level error classification
// ---------------------------------------------------------------------------

/// The authoritative classification of one pinned QUIC/H3 error.
///
/// Every error is exactly one class.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuicErrorClass {
    /// The physical connection is proven terminal. The exact key+generation is
    /// logically deactivated (`Active -> Closing`, immediately unleasable) and
    /// removed only at the terminal `Drained`/`Failed`.
    EntryTerminal,
    /// Only this request stream failed. A healthy entry stays reusable, and the
    /// query's bytes are never replayed.
    StreamLocal,
}

/// The recorded effect of applying one [`QuicErrorClass`] to the owner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuicErrorOutcome {
    /// Stream-local: the entry is unchanged, healthy, and reusable.
    EntryUnchanged,
    /// Entry-terminal: the exact key+generation is now `Closing` (immediately
    /// unleasable); physical removal happens only at terminal.
    EntryDeactivated,
    /// No entry with this exact key+generation: it already reached terminal, or
    /// this is a stale callback that must never touch a newer generation.
    NoMatchingEntry,
}

/// One row of the authoritative R0b classification table.
#[derive(Clone, Copy, Debug)]
pub struct PinnedErrorClassRow {
    /// The pinned vocabulary this row belongs to: `quinn`, `h3-quinn`, `h3`, or
    /// this crate for its own response-validation errors.
    pub vocabulary: &'static str,
    /// The pinned error type name.
    pub error_type: &'static str,
    /// The variant or outcome name.
    pub item: &'static str,
    /// The exact locked-source citation the row is derived from.
    pub citation: &'static str,
    /// The single class this item maps to.
    pub class: QuicErrorClass,
}

/// The authoritative connection-level versus stream-level classification table
/// (R0b).
///
/// This is the only source later slices use for logical deactivation. Every row
/// is derived from the pinned source cited in the row, not from a `Result` shape
/// and not from a rendered reason string.
pub const PINNED_ERROR_CLASSIFICATION_TABLE: &[PinnedErrorClassRow] = &[
    // ---- quinn 0.11.7 -----------------------------------------------------
    PinnedErrorClassRow {
        vocabulary: "quinn",
        error_type: "quinn::ConnectionError",
        item: "VersionMismatch",
        citation: "quinn-0.11.7/src/lib.rs:62 re-exports quinn-proto-0.11.18/src/connection/mod.rs:3909-3941",
        class: QuicErrorClass::EntryTerminal,
    },
    PinnedErrorClassRow {
        vocabulary: "quinn",
        error_type: "quinn::ConnectionError",
        item: "TransportError",
        citation: "quinn-proto-0.11.18/src/connection/mod.rs:3909-3941",
        class: QuicErrorClass::EntryTerminal,
    },
    PinnedErrorClassRow {
        vocabulary: "quinn",
        error_type: "quinn::ConnectionError",
        item: "ConnectionClosed",
        citation: "quinn-proto-0.11.18/src/connection/mod.rs:3909-3941",
        class: QuicErrorClass::EntryTerminal,
    },
    PinnedErrorClassRow {
        vocabulary: "quinn",
        error_type: "quinn::ConnectionError",
        item: "ApplicationClosed",
        citation: "quinn-proto-0.11.18/src/connection/mod.rs:3909-3941",
        class: QuicErrorClass::EntryTerminal,
    },
    PinnedErrorClassRow {
        vocabulary: "quinn",
        error_type: "quinn::ConnectionError",
        item: "Reset",
        citation: "quinn-proto-0.11.18/src/connection/mod.rs:3909-3941",
        class: QuicErrorClass::EntryTerminal,
    },
    PinnedErrorClassRow {
        vocabulary: "quinn",
        error_type: "quinn::ConnectionError",
        item: "TimedOut",
        citation: "quinn-proto-0.11.18/src/connection/mod.rs:3909-3941",
        class: QuicErrorClass::EntryTerminal,
    },
    PinnedErrorClassRow {
        vocabulary: "quinn",
        error_type: "quinn::ConnectionError",
        item: "LocallyClosed",
        citation: "quinn-proto-0.11.18/src/connection/mod.rs:3909-3941",
        class: QuicErrorClass::EntryTerminal,
    },
    PinnedErrorClassRow {
        vocabulary: "quinn",
        error_type: "quinn::ConnectionError",
        item: "CidsExhausted",
        citation: "quinn-proto-0.11.18/src/connection/mod.rs:3909-3941",
        class: QuicErrorClass::EntryTerminal,
    },
    PinnedErrorClassRow {
        vocabulary: "quinn",
        error_type: "quinn::WriteError",
        item: "Stopped",
        citation: "quinn-0.11.7/src/send_stream.rs:423-440",
        class: QuicErrorClass::StreamLocal,
    },
    PinnedErrorClassRow {
        vocabulary: "quinn",
        error_type: "quinn::WriteError",
        item: "ConnectionLost",
        citation: "quinn-0.11.7/src/send_stream.rs:423-440",
        class: QuicErrorClass::EntryTerminal,
    },
    PinnedErrorClassRow {
        vocabulary: "quinn",
        error_type: "quinn::WriteError",
        item: "ClosedStream",
        citation: "quinn-0.11.7/src/send_stream.rs:423-440",
        class: QuicErrorClass::StreamLocal,
    },
    PinnedErrorClassRow {
        vocabulary: "quinn",
        error_type: "quinn::WriteError",
        item: "ZeroRttRejected",
        citation: "quinn-0.11.7/src/send_stream.rs:423-440",
        class: QuicErrorClass::EntryTerminal,
    },
    PinnedErrorClassRow {
        vocabulary: "quinn",
        error_type: "quinn::ReadError",
        item: "Reset",
        citation: "quinn-0.11.7/src/recv_stream.rs:520-547",
        class: QuicErrorClass::StreamLocal,
    },
    PinnedErrorClassRow {
        vocabulary: "quinn",
        error_type: "quinn::ReadError",
        item: "ConnectionLost",
        citation: "quinn-0.11.7/src/recv_stream.rs:520-547",
        class: QuicErrorClass::EntryTerminal,
    },
    PinnedErrorClassRow {
        vocabulary: "quinn",
        error_type: "quinn::ReadError",
        item: "ClosedStream",
        citation: "quinn-0.11.7/src/recv_stream.rs:520-547",
        class: QuicErrorClass::StreamLocal,
    },
    PinnedErrorClassRow {
        vocabulary: "quinn",
        error_type: "quinn::ReadError",
        item: "IllegalOrderedRead",
        citation: "quinn-0.11.7/src/recv_stream.rs:520-547",
        class: QuicErrorClass::StreamLocal,
    },
    PinnedErrorClassRow {
        vocabulary: "quinn",
        error_type: "quinn::ReadError",
        item: "ZeroRttRejected",
        citation: "quinn-0.11.7/src/recv_stream.rs:520-547",
        class: QuicErrorClass::EntryTerminal,
    },
    PinnedErrorClassRow {
        vocabulary: "quinn",
        error_type: "quinn::ReadToEndError",
        item: "Read",
        citation: "quinn-0.11.7/src/recv_stream.rs:467-475",
        class: QuicErrorClass::StreamLocal,
    },
    PinnedErrorClassRow {
        vocabulary: "quinn",
        error_type: "quinn::ReadToEndError",
        item: "TooLong",
        citation: "quinn-0.11.7/src/recv_stream.rs:467-475",
        class: QuicErrorClass::StreamLocal,
    },
    // ---- h3-quinn 0.0.10 backend vocabulary -------------------------------
    PinnedErrorClassRow {
        vocabulary: "h3-quinn",
        error_type: "h3::quic::ConnectionErrorIncoming",
        item: "ApplicationClose",
        citation: "h3-0.0.8/src/quic.rs:20-33; h3-quinn-0.0.10/src/lib.rs:109-126",
        class: QuicErrorClass::EntryTerminal,
    },
    PinnedErrorClassRow {
        vocabulary: "h3-quinn",
        error_type: "h3::quic::ConnectionErrorIncoming",
        item: "Timeout",
        citation: "h3-0.0.8/src/quic.rs:20-33; h3-quinn-0.0.10/src/lib.rs:109-126",
        class: QuicErrorClass::EntryTerminal,
    },
    PinnedErrorClassRow {
        vocabulary: "h3-quinn",
        error_type: "h3::quic::ConnectionErrorIncoming",
        item: "InternalError",
        citation: "h3-0.0.8/src/quic.rs:20-33",
        class: QuicErrorClass::EntryTerminal,
    },
    PinnedErrorClassRow {
        vocabulary: "h3-quinn",
        error_type: "h3::quic::ConnectionErrorIncoming",
        item: "Undefined",
        citation: "h3-0.0.8/src/quic.rs:20-33; h3-quinn-0.0.10/src/lib.rs:109-126",
        class: QuicErrorClass::EntryTerminal,
    },
    PinnedErrorClassRow {
        vocabulary: "h3-quinn",
        error_type: "h3::quic::StreamErrorIncoming",
        item: "ConnectionErrorIncoming",
        citation: "h3-0.0.8/src/quic.rs:57-71",
        class: QuicErrorClass::EntryTerminal,
    },
    PinnedErrorClassRow {
        vocabulary: "h3-quinn",
        error_type: "h3::quic::StreamErrorIncoming",
        item: "StreamTerminated",
        citation: "h3-0.0.8/src/quic.rs:57-71",
        class: QuicErrorClass::StreamLocal,
    },
    PinnedErrorClassRow {
        vocabulary: "h3-quinn",
        error_type: "h3::quic::StreamErrorIncoming",
        item: "Unknown",
        citation: "h3-0.0.8/src/quic.rs:57-71; h3-quinn-0.0.10/src/lib.rs:407-435",
        class: QuicErrorClass::StreamLocal,
    },
    // ---- h3 0.0.8 driver and request stream -------------------------------
    //
    // R0b non-weakening note: `h3::error::StreamError` marks the enum and every
    // variant `#[non_exhaustive]` with no opt-out (`error.rs:16-19` and each
    // variant's `cfg_attr`), so a downstream crate cannot match
    // `ConnectionError(_)`, `RemoteClosing`, or `Undefined(_)` at all. Those
    // outcomes are connection-carrying by pinned construction — a connection
    // error is recorded on the shared `Arc<SharedState>` that the entry's driver
    // polls (`connection_error_creators.rs:22-27, 110-123`), with `poll_close`
    // surfacing it ahead of control frames (`connection.rs:514`) — and they
    // reach the entry as the independently-terminal
    // `h3::error::ConnectionError` (driver outcome) or the backend
    // `h3::quic::StreamErrorIncoming::ConnectionErrorIncoming { .. }` arm, never
    // folded into the stream-local wildcard. No pinned variant is silently
    // reclassified.
    PinnedErrorClassRow {
        vocabulary: "h3",
        error_type: "h3::error::ConnectionError",
        item: "any driver poll_close outcome",
        citation: "h3-0.0.8/src/error/error.rs:13-31; h3-0.0.8/src/client/connection.rs:397-400",
        class: QuicErrorClass::EntryTerminal,
    },
    PinnedErrorClassRow {
        vocabulary: "h3",
        error_type: "h3::error::StreamError",
        item: "any request-stream outcome",
        citation: "h3-0.0.8/src/error/error.rs:79-136; h3-0.0.8/src/client/stream.rs:225-260",
        class: QuicErrorClass::StreamLocal,
    },
    // ---- this crate's own response/framing validation ---------------------
    PinnedErrorClassRow {
        vocabulary: "mosdns-upstream-core",
        error_type: "SecureError",
        item: "DoqProtocolTrailingResponse",
        citation: "rust/upstream-core/src/secure/error.rs:478",
        class: QuicErrorClass::StreamLocal,
    },
    PinnedErrorClassRow {
        vocabulary: "mosdns-upstream-core",
        error_type: "SecureError",
        item: "DoqProtocolMissingResponseFin",
        citation: "rust/upstream-core/src/secure/error.rs:490",
        class: QuicErrorClass::StreamLocal,
    },
    PinnedErrorClassRow {
        vocabulary: "mosdns-upstream-core",
        error_type: "SecureError",
        item: "DoqProtocolNonzeroResponseId",
        citation: "rust/upstream-core/src/secure/error.rs:501",
        class: QuicErrorClass::StreamLocal,
    },
    PinnedErrorClassRow {
        vocabulary: "mosdns-upstream-core",
        error_type: "DohProtocolError",
        item: "PeerStreamTerminated",
        citation: "rust/upstream-core/src/secure/error.rs:359",
        class: QuicErrorClass::StreamLocal,
    },
    PinnedErrorClassRow {
        vocabulary: "mosdns-upstream-core",
        error_type: "DohProtocolError",
        item: "ResponseHeadTooLarge",
        citation: "rust/upstream-core/src/secure/error.rs:321",
        class: QuicErrorClass::StreamLocal,
    },
    PinnedErrorClassRow {
        vocabulary: "mosdns-upstream-core",
        error_type: "DohProtocolError",
        item: "IncompleteBody",
        citation: "rust/upstream-core/src/secure/error.rs:334",
        class: QuicErrorClass::StreamLocal,
    },
];

/// Looks up one frozen row of [`PINNED_ERROR_CLASSIFICATION_TABLE`].
#[must_use]
pub fn pinned_error_class(vocabulary: &str, item: &str) -> Option<QuicErrorClass> {
    PINNED_ERROR_CLASSIFICATION_TABLE
        .iter()
        .find(|row| row.vocabulary == vocabulary && row.item == item)
        .map(|row| row.class)
}

/// Classifies one Quinn connection-level error.
///
/// Every `quinn::ConnectionError` variant proves the physical connection is lost
/// or unusable, so all of them are [`QuicErrorClass::EntryTerminal`].
#[must_use]
pub fn classify_quinn_connection_error(error: &quinn::ConnectionError) -> QuicErrorClass {
    match error {
        quinn::ConnectionError::VersionMismatch
        | quinn::ConnectionError::TransportError(_)
        | quinn::ConnectionError::ConnectionClosed(_)
        | quinn::ConnectionError::ApplicationClosed(_)
        | quinn::ConnectionError::Reset
        | quinn::ConnectionError::TimedOut
        | quinn::ConnectionError::LocallyClosed
        | quinn::ConnectionError::CidsExhausted => QuicErrorClass::EntryTerminal,
    }
}

/// Classifies one Quinn stream write error.
///
/// `Stopped` and `ClosedStream` terminate only this stream; `ConnectionLost`
/// inherits the connection-level class. `ZeroRttRejected` is unreachable under
/// this task's no-0-RTT contract and is conservatively terminal: an error that
/// proves neither health nor death must never keep a connection for reuse.
#[must_use]
pub fn classify_quinn_write_error(error: &quinn::WriteError) -> QuicErrorClass {
    match error {
        quinn::WriteError::Stopped(_) | quinn::WriteError::ClosedStream => {
            QuicErrorClass::StreamLocal
        }
        quinn::WriteError::ConnectionLost(connection) => {
            classify_quinn_connection_error(connection)
        }
        quinn::WriteError::ZeroRttRejected => QuicErrorClass::EntryTerminal,
    }
}

/// Classifies one Quinn stream read error.
///
/// `Reset` terminates only this stream and `ClosedStream` is a local stream
/// state; `IllegalOrderedRead` is a local read-discipline fault on one stream,
/// not connection evidence. `ConnectionLost` inherits the connection class.
#[must_use]
pub fn classify_quinn_read_error(error: &quinn::ReadError) -> QuicErrorClass {
    match error {
        quinn::ReadError::Reset(_)
        | quinn::ReadError::ClosedStream
        | quinn::ReadError::IllegalOrderedRead => QuicErrorClass::StreamLocal,
        quinn::ReadError::ConnectionLost(connection) => classify_quinn_connection_error(connection),
        quinn::ReadError::ZeroRttRejected => QuicErrorClass::EntryTerminal,
    }
}

/// Classifies one `RecvStream::read_to_end` error.
#[must_use]
pub fn classify_quinn_read_to_end_error(error: &quinn::ReadToEndError) -> QuicErrorClass {
    match error {
        quinn::ReadToEndError::Read(read) => classify_quinn_read_error(read),
        quinn::ReadToEndError::TooLong => QuicErrorClass::StreamLocal,
    }
}

/// Classifies the `h3` backend-level connection error vocabulary
/// ([`h3::quic::ConnectionErrorIncoming`]).
///
/// The whole type is connection-level by construction: `h3-quinn` maps exactly
/// the terminal Quinn connection errors into `Undefined`
/// (`h3-quinn-0.0.10/src/lib.rs:109-126`), an application close is a closed H3
/// connection, a timeout is terminal, and an internal error makes h3 close the
/// connection.
#[must_use]
pub fn classify_h3_quinn_connection_error(
    error: &h3::quic::ConnectionErrorIncoming,
) -> QuicErrorClass {
    match error {
        h3::quic::ConnectionErrorIncoming::ApplicationClose { .. }
        | h3::quic::ConnectionErrorIncoming::Timeout
        | h3::quic::ConnectionErrorIncoming::InternalError(_)
        | h3::quic::ConnectionErrorIncoming::Undefined(_) => QuicErrorClass::EntryTerminal,
    }
}

/// Classifies the `h3` backend-level stream error vocabulary
/// ([`h3::quic::StreamErrorIncoming`]).
///
/// `ConnectionErrorIncoming` inherits the connection class. `StreamTerminated`
/// is a peer reset/stop of this one stream. `Unknown` is produced by
/// `h3-quinn` only from `ReadError::ClosedStream`, `ReadError::ZeroRttRejected`,
/// `WriteError::ClosedStream`, and `WriteError::ZeroRttRejected`
/// (`h3-quinn-0.0.10/src/lib.rs:409-435`); the only reachable producer is
/// `ClosedStream`, which is stream-local, so the connection is not proven dead.
#[must_use]
pub fn classify_h3_quinn_stream_error(error: &h3::quic::StreamErrorIncoming) -> QuicErrorClass {
    match error {
        h3::quic::StreamErrorIncoming::ConnectionErrorIncoming { connection_error } => {
            classify_h3_quinn_connection_error(connection_error)
        }
        h3::quic::StreamErrorIncoming::StreamTerminated { .. }
        | h3::quic::StreamErrorIncoming::Unknown(_) => QuicErrorClass::StreamLocal,
    }
}

/// Classifies the `h3` client connection error vocabulary
/// ([`h3::error::ConnectionError`]).
///
/// This is the H3 driver's `poll_close` terminal outcome. Every variant — a
/// local close, a remote close/timeout/internal/undefined, or the bare
/// `Timeout` — means the H3 connection is closed, so the entry must never be
/// reused. A clean `H3_NO_ERROR` close is still a closed connection.
///
/// The classification is deliberately **type-level**: `h3` 0.0.8 marks both the
/// enum and each variant `#[non_exhaustive]`, so a downstream crate cannot name
/// the variants in a pattern. That is harmless here because the type itself
/// already carries the connection-level meaning.
#[must_use]
pub fn classify_h3_connection_error(_error: &h3::error::ConnectionError) -> QuicErrorClass {
    QuicErrorClass::EntryTerminal
}

/// Classifies the `h3` request-stream error vocabulary
/// ([`h3::error::StreamError`]).
///
/// Two independent facts from the pinned source drive this rule:
///
/// 1. The `h3` request-stream failures that never touch the connection state are
///    exactly the struct-shaped variants: `StreamError{..}`, `RemoteTerminate{..}`
///    (a reset on one stream direction), and `HeaderTooBig{..}` — all
///    [`QuicErrorClass::StreamLocal`].
/// 2. `h3 0.0.8` marks the enum and every variant `#[non_exhaustive]`
///    (`error.rs:16-19, 84-87, 94-99, 105-110, 114-121, 126-131, 134-139`) with
///    no opt-out, so the tuple/unit variants `ConnectionError(_)`,
///    `RemoteClosing`, and `Undefined(_)` **cannot be matched downstream**. The
///    same pinned code that produces those variants also records the connection
///    condition on the shared `Arc<SharedState>` that the entry's long-lived
///    driver polls: a stream error produced by a connection error calls
///    `set_conn_error_and_wake` on the way out
///    (`connection_error_creators.rs:22-27, 110-123`), and `poll_close` surfaces
///    any recorded connection error ahead of control frames
///    (`connection.rs:514`). Therefore any `RemoteClosing`-refusing state (its
///    producer `check_peer_connection_closing` reads only the shared
///    `is_closing` flag, `connection_error_creators.rs:133-139`) is paired with
///    the driver's independently-terminal [`classify_h3_connection_error`]
///    outcome, and with the backend [`classify_h3_quinn_connection_error`]
///    outcome for errors that cross the `h3-quinn` trait boundary
///    (`StreamErrorIncoming::ConnectionErrorIncoming { .. }`).
///
/// The wildcard arm below is `StreamLocal` only because the wildcard is
/// unreachable without the paired connection observation: a call site that
/// routes an `h3::error::StreamError` to this function must route its entry
/// through the driver observation first (Slice 2 owns the binding, where a real
/// driver exists). If the pinned API ever allowed matching the tuple/unit
/// variants, those arms would classify `ConnectionError(_)` via
/// [`classify_h3_connection_error`] and `RemoteClosing` as terminal — neither
/// would reach `StreamLocal`.
///
/// # Safety (R0b non-weakening proof)
///
/// Struct-shaped variants are exhaustively covered in the first arm. The
/// wildcard can only observe the tuple/unit variants listed above. It must not
/// be extended to classify new nameable variants: Rust's exhaustiveness checker
/// will flag genuinely new matchable variants at compile time because they are
/// not part of the wildcard's matched set.
#[must_use]
pub fn classify_h3_stream_error(error: &h3::error::StreamError) -> QuicErrorClass {
    match error {
        h3::error::StreamError::StreamError { .. }
        | h3::error::StreamError::RemoteTerminate { .. }
        | h3::error::StreamError::HeaderTooBig { .. } => QuicErrorClass::StreamLocal,
        _ => QuicErrorClass::StreamLocal,
    }
}

/// The exchange phase in which a local error was observed.
///
/// The side-effect state is derived from the phase, never from an error shape,
/// and is independent of [`QuicErrorClass`]: a stream-local failure can still
/// have sent a query, and a `NotSent` failure still keeps the entry healthy.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuicExchangePhase {
    /// Before any request byte or stream exists (key validation, permit).
    PreSend,
    /// Connection handshake / single-flight initialization.
    Connect,
    /// Stream open or request write has begun and its completion is unknown.
    SendUncertain,
    /// The request side reached STREAM FIN.
    RequestFin,
    /// A response byte was read or validated.
    ResponseRead,
}

/// The exact recorded side-effect state for one exchange phase.
#[must_use]
pub const fn side_effect_of(phase: QuicExchangePhase) -> SideEffectState {
    match phase {
        QuicExchangePhase::PreSend | QuicExchangePhase::Connect => SideEffectState::NotSent,
        QuicExchangePhase::SendUncertain => SideEffectState::MaybeSent,
        QuicExchangePhase::RequestFin | QuicExchangePhase::ResponseRead => SideEffectState::Sent,
    }
}

// ---------------------------------------------------------------------------
// Injected clock and deterministic model seams
// ---------------------------------------------------------------------------

/// The injected clock used by the owner's lazy idle-expiry scan.
///
/// The owner never reads the wall clock directly and never starts a timer, so
/// idle expiry is deterministic under [`ManualClock`].
pub trait OwnerClock: Send + Sync + 'static {
    /// The current instant according to this clock.
    fn now(&self) -> Instant;
}

/// The production clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemOwnerClock;

impl OwnerClock for SystemOwnerClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// A test-controlled clock: the owner's lazy maintenance only ever compares
/// instants, so advancing this clock is enough to expire an entry.
#[derive(Debug)]
pub struct ManualClock {
    now: Mutex<Instant>,
}

impl ManualClock {
    /// Creates a clock reading `origin` until it is advanced.
    #[must_use]
    pub const fn new(origin: Instant) -> Self {
        Self {
            now: Mutex::new(origin),
        }
    }

    /// Moves the clock forward by `by`.
    pub fn advance(&self, by: Duration) {
        let mut now = self.lock();
        *now += by;
    }

    /// Sets the clock to an absolute instant.
    pub fn set(&self, at: Instant) {
        *self.lock() = at;
    }

    fn lock(&self) -> MutexGuard<'_, Instant> {
        self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl OwnerClock for ManualClock {
    fn now(&self) -> Instant {
        *self.lock()
    }
}

/// A deterministic model barrier.
///
/// `check()` announces arrival and then parks until `release()`; `wait_arrived()`
/// waits until every party has arrived. It uses no timer and no sleep, so the
/// model tests it enables are fully deterministic. It is a model-only seam: the
/// production owner never gains or loses correctness from it.
#[derive(Debug)]
pub struct ModelBarrier {
    parties: usize,
    arrived: AtomicUsize,
    released: AtomicBool,
    arrived_notify: Notify,
    released_notify: Notify,
}

impl ModelBarrier {
    /// A one-party barrier.
    #[must_use]
    pub const fn new() -> Self {
        Self::with_parties(1)
    }

    /// A barrier that waits for `parties` arrivals.
    #[must_use]
    pub const fn with_parties(parties: usize) -> Self {
        Self {
            parties,
            arrived: AtomicUsize::new(0),
            released: AtomicBool::new(false),
            arrived_notify: Notify::const_new(),
            released_notify: Notify::const_new(),
        }
    }

    /// Announces arrival and parks until [`Self::release`].
    pub async fn check(&self) {
        self.arrive();
        loop {
            let notified = self.released_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.released.load(Ordering::SeqCst) {
                return;
            }
            notified.await;
        }
    }

    /// Waits until every party has arrived.
    pub async fn wait_arrived(&self) {
        loop {
            let notified = self.arrived_notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if self.arrived.load(Ordering::SeqCst) >= self.parties {
                return;
            }
            notified.await;
        }
    }

    /// Releases every parked party. Idempotent.
    pub fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
        self.released_notify.notify_waiters();
    }

    /// How many parties have arrived so far.
    #[must_use]
    pub fn arrived_count(&self) -> usize {
        self.arrived.load(Ordering::SeqCst)
    }

    fn arrive(&self) {
        if self.arrived.fetch_add(1, Ordering::SeqCst) + 1 >= self.parties {
            self.arrived_notify.notify_waiters();
        }
    }
}

impl Default for ModelBarrier {
    fn default() -> Self {
        Self::new()
    }
}

/// Deterministic interleaving seams installed into an owner.
///
/// These exist so the Slice 0 model tests can hold a chosen point without any
/// sleep or time-based wait. Slice 1/2 construct the owner with
/// [`ModelSeams::default`], which installs nothing.
#[derive(Clone, Debug, Default)]
pub struct ModelSeams {
    /// Parked inside [`QuicReuseOwner::close`] immediately after owner-close
    /// stage two linearized and before the terminal completions are awaited.
    pub close_after_linearize: Option<Arc<ModelBarrier>>,
    /// Parked inside the entry-owned supervised teardown after the initializer
    /// hand-off and before the terminal record and removal.
    pub teardown_before_terminal: Option<Arc<ModelBarrier>>,
}

// ---------------------------------------------------------------------------
// The entry-owned transport, its initializer, and the shared entry record
// ---------------------------------------------------------------------------

/// Slice 0's inert model of one entry-owned physical transport.
///
/// It performs **no** socket or QUIC/H3 I/O. It exists so the model can prove
/// that exactly one resource is handed from the entry-owned initializer to the
/// supervised teardown and closed exactly once — including a resource acquired
/// after teardown was already requested. Slice 1/2 replace it with the real
/// Quinn connection and H3 driver handle.
#[derive(Debug)]
pub struct EntryTransport {
    id: u64,
    closed: AtomicBool,
    close_calls: AtomicUsize,
}

impl EntryTransport {
    /// Creates an inert transport token carrying an identifying `id`.
    #[must_use]
    pub const fn new(id: u64) -> Self {
        Self {
            id,
            closed: AtomicBool::new(false),
            close_calls: AtomicUsize::new(0),
        }
    }

    /// The token's identity, for deterministic test assertions.
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// Whether the supervised teardown has closed this token.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// How many times teardown closed this token. Must never exceed one.
    #[must_use]
    pub fn close_calls(&self) -> usize {
        self.close_calls.load(Ordering::SeqCst)
    }

    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
        self.close_calls.fetch_add(1, Ordering::SeqCst);
    }
}

/// The boxed future one entry-owned initializer returns.
pub type InitializeFuture =
    Pin<Box<dyn Future<Output = Option<Arc<EntryTransport>>> + Send + 'static>>;

/// Builds one entry's physical transport **outside** the owner-map lock, on the
/// entry-owned initializer task.
///
/// Slice 0 has no I/O, so its tests supply inert implementations (for example
/// one that parks on a [`ModelBarrier`] and then acquires a token). Slice 1/2
/// supply the real Quinn/H3 builders. Returning `None` is an initialization
/// failure and still delivers exactly one completion to the supervised teardown.
pub trait EntryInitializer: Send + Sync + 'static {
    /// Builds the transport for `key`/`generation`.
    fn initialize(&self, key: QuicReuseKey, generation: u64) -> InitializeFuture;
}

/// The Slice 0 default initializer: acquires one inert token immediately and
/// never fails. It performs no I/O and is replaced by Slice 1/2.
#[derive(Debug, Default)]
pub struct ImmediateInitializer {
    next: AtomicU64,
}

impl EntryInitializer for ImmediateInitializer {
    fn initialize(&self, _key: QuicReuseKey, _generation: u64) -> InitializeFuture {
        let id = self.next.fetch_add(1, Ordering::SeqCst) + 1;
        Box::pin(async move { Some(Arc::new(EntryTransport::new(id))) })
    }
}

/// The per-generation record that outlives the map entry.
///
/// It carries the exact key+generation identity and the deterministic evidence
/// the model tests assert on: the single initialization completion, whether a
/// teardown ran exactly once, the resource close, the cancelled stream permits,
/// and the terminal outcome. A caller obtains it from an admission, a
/// [`StreamLease`], or [`QuicReuseOwner::observe`].
#[derive(Debug)]
pub struct EntryRecord {
    key: QuicReuseKey,
    generation: u64,
    terminal: Mutex<Option<EntryTerminal>>,
    initialization_completions: AtomicUsize,
    teardown_runs: AtomicUsize,
    resource_closes: AtomicUsize,
    streams_cancelled: AtomicUsize,
    changed: Notify,
}

impl EntryRecord {
    fn new(key: QuicReuseKey, generation: u64) -> Self {
        Self {
            key,
            generation,
            terminal: Mutex::new(None),
            initialization_completions: AtomicUsize::new(0),
            teardown_runs: AtomicUsize::new(0),
            resource_closes: AtomicUsize::new(0),
            streams_cancelled: AtomicUsize::new(0),
            changed: Notify::const_new(),
        }
    }

    /// The exact key this record belongs to.
    #[must_use]
    pub const fn key(&self) -> &QuicReuseKey {
        &self.key
    }

    /// The exact generation this record belongs to.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// The terminal outcome, once the supervised teardown reached it.
    #[must_use]
    pub fn terminal(&self) -> Option<EntryTerminal> {
        *self.terminal.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// How many initialization completions were delivered. Exactly one.
    #[must_use]
    pub fn initialization_completions(&self) -> usize {
        self.initialization_completions.load(Ordering::SeqCst)
    }

    /// How many supervised teardowns were started. At most one.
    #[must_use]
    pub fn teardown_runs(&self) -> usize {
        self.teardown_runs.load(Ordering::SeqCst)
    }

    /// How many times the supervised teardown closed an acquired transport. At
    /// most one.
    #[must_use]
    pub fn resource_closes(&self) -> usize {
        self.resource_closes.load(Ordering::SeqCst)
    }

    /// How many outstanding stream permits the teardown cancelled.
    #[must_use]
    pub fn streams_cancelled(&self) -> usize {
        self.streams_cancelled.load(Ordering::SeqCst)
    }

    /// Awaits the terminal outcome. Observing the latch performs no teardown
    /// work: it only waits for the entry-owned supervised task.
    pub async fn wait_terminal(&self) -> EntryTerminal {
        loop {
            let notified = self.changed.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(terminal) = self.terminal() {
                return terminal;
            }
            notified.await;
        }
    }

    fn note_initialization_completion(&self) {
        self.initialization_completions
            .fetch_add(1, Ordering::SeqCst);
    }

    fn note_teardown_run(&self) {
        self.teardown_runs.fetch_add(1, Ordering::SeqCst);
    }

    fn note_resource_close(&self) {
        self.resource_closes.fetch_add(1, Ordering::SeqCst);
    }

    fn note_streams_cancelled(&self, count: usize) {
        self.streams_cancelled.store(count, Ordering::SeqCst);
    }

    fn set_terminal(&self, terminal: EntryTerminal) {
        {
            let mut slot = self.terminal.lock().unwrap_or_else(PoisonError::into_inner);
            *slot = Some(terminal);
        }
        self.changed.notify_waiters();
    }

    fn notify_changed(&self) {
        self.changed.notify_waiters();
    }
}

/// A read-only observation of the current stored state of one entry.
#[derive(Clone, Debug)]
pub struct EntryObservation {
    /// The exact key.
    pub key: QuicReuseKey,
    /// The exact generation.
    pub generation: u64,
    /// The stored phase (`Initializing`, `Active`, or `Closing`).
    pub phase: EntryPhase,
    /// The leaseability health.
    pub health: EntryHealth,
    /// Whether the entry-owned initializer delivered its single completion.
    pub initialized: bool,
    /// Whether teardown was requested for this generation.
    pub teardown_requested: bool,
    /// Stream permits currently leased.
    pub permits_in_use: usize,
    /// The last instant this entry was leased or published.
    pub last_used: Instant,
    /// The surviving per-generation record.
    pub record: Arc<EntryRecord>,
}

// ---------------------------------------------------------------------------
// The QUIC-specific reuse owner
// ---------------------------------------------------------------------------

struct OwnerState {
    /// The model-only admission-vs-close gate. `false` once owner-close stage
    /// two has linearized; set only inside this lock.
    accepting: bool,
    entries: HashMap<QuicReuseKey, Entry>,
    next_generation: u64,
}

struct Entry {
    generation: u64,
    phase: EntryPhase,
    health: EntryHealth,
    teardown_requested: bool,
    initialized: bool,
    transport: Option<Arc<EntryTransport>>,
    permits_in_use: usize,
    last_used: Instant,
    liveness: Option<SharedInFlightGuard>,
    initializer_task: Option<JoinHandle<()>>,
    record: Arc<EntryRecord>,
}

struct OwnerShared {
    lifecycle: Arc<Lifecycle>,
    owner_cancellation: TransportCancellation,
    clock: Arc<dyn OwnerClock>,
    initializer: Arc<dyn EntryInitializer>,
    seams: ModelSeams,
    state: Mutex<OwnerState>,
}

impl OwnerShared {
    fn lock(&self) -> MutexGuard<'_, OwnerState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Removes the entry's transport (if any) and cancels every outstanding
    /// stream permit of this generation. Terminal-only.
    fn take_transport_for_teardown(
        &self,
        key: &QuicReuseKey,
        generation: u64,
    ) -> (Option<Arc<EntryTransport>>, usize) {
        let mut state = self.lock();
        let Some(entry) = state.entries.get_mut(key) else {
            return (None, 0);
        };
        if entry.generation != generation {
            return (None, 0);
        }
        let cancelled = entry.permits_in_use;
        entry.permits_in_use = 0;
        (entry.transport.take(), cancelled)
    }

    /// The only entry-removal path: terminal `Drained`/`Failed` removes the
    /// exact key+generation and releases slot and liveness.
    fn finish_teardown(&self, key: &QuicReuseKey, generation: u64, terminal: EntryTerminal) {
        let mut state = self.lock();
        let removed = match state.entries.get(key) {
            Some(entry) if entry.generation == generation => state.entries.remove(key),
            _ => None,
        };
        if let Some(entry) = removed {
            // Dropping the entry releases its slot and its `Lifecycle`
            // registration; this is the only place that happens.
            drop(entry.liveness);
            entry.record.set_terminal(terminal);
        }
        drop(state);
    }

    fn release_permit(&self, key: &QuicReuseKey, generation: u64) {
        let mut state = self.lock();
        let Some(entry) = state.entries.get_mut(key) else {
            return;
        };
        if entry.generation != generation || entry.permits_in_use == 0 {
            return;
        }
        entry.permits_in_use -= 1;
        if entry.permits_in_use == 0 && entry.health == EntryHealth::Healthy {
            entry.last_used = self.clock.now();
        }
    }
}

/// The single owner-owned initialization completion delivered to teardown.
///
/// Delivery happens exactly once, on the explicit path or in `Drop`. That is
/// what makes an aborted/panicking initializer unable to strand an
/// `Initializing`/`Closing` entry.
struct InitializationGuard {
    shared: Arc<OwnerShared>,
    key: QuicReuseKey,
    generation: u64,
    delivered: bool,
}

impl InitializationGuard {
    fn new(shared: Arc<OwnerShared>, key: QuicReuseKey, generation: u64) -> Self {
        Self {
            shared,
            key,
            generation,
            delivered: false,
        }
    }

    fn deliver(mut self, transport: Option<Arc<EntryTransport>>) {
        self.delivered = true;
        complete_initialization(&self.shared, &self.key, self.generation, transport);
    }
}

impl Drop for InitializationGuard {
    fn drop(&mut self) {
        if !self.delivered {
            complete_initialization(&self.shared, &self.key, self.generation, None);
        }
    }
}

/// The single publication/handoff point for one entry generation.
///
/// `Active` is published only when `Lifecycle == Open`, `accepting == true`, and
/// this exact generation is still `Initializing` all hold under the one map
/// lock. Any miss hands the whole result — including a late-acquired resource —
/// to the supervised teardown, which is started exactly once.
fn complete_initialization(
    shared: &Arc<OwnerShared>,
    key: &QuicReuseKey,
    generation: u64,
    transport: Option<Arc<EntryTransport>>,
) {
    let mut state = shared.lock();
    // The single publication point: the real `Lifecycle` state and the map
    // admission gate are independent, so neither substitutes for the other, and
    // the exact generation must still be `Initializing`.
    let accepting = state.accepting;
    let lifecycle_open = shared.lifecycle.state() == LifecycleState::Open;

    let (record, publish) = {
        let Some(entry) = state.entries.get_mut(key) else {
            // The entry already reached terminal; this generation has no
            // surviving state to publish into.
            return;
        };
        if entry.generation != generation {
            // A stale initializer must never touch a newer generation.
            return;
        }
        entry.record.note_initialization_completion();
        entry.initialized = true;
        let publish = entry.phase == EntryPhase::Initializing && accepting && lifecycle_open;
        if publish {
            if let Some(transport) = transport {
                entry.transport = Some(transport);
            }
            entry.phase = EntryPhase::Active;
            entry.health = EntryHealth::Healthy;
            entry.last_used = shared.clock.now();
        } else {
            // Never publish `Active`. The whole result — including a resource
            // built after teardown was already requested — goes to the
            // supervised teardown.
            if let Some(transport) = transport {
                entry.transport = Some(transport);
            }
            entry.phase = EntryPhase::Closing;
            entry.health = EntryHealth::Dead;
            entry.teardown_requested = true;
        }
        (Arc::clone(&entry.record), publish)
    };
    if !publish {
        mark_started_teardown(shared, &mut state, key);
    }
    drop(state);
    record.notify_changed();
}

/// Marks one entry `Closing`/`TeardownRequested` without removing it, then
/// starts its exactly-once supervised teardown — all inside the caller's single
/// locked section.
fn mark_closing_and_start(shared: &Arc<OwnerShared>, state: &mut OwnerState, key: &QuicReuseKey) {
    if let Some(entry) = state.entries.get_mut(key) {
        if entry.phase != EntryPhase::Closing {
            entry.phase = EntryPhase::Closing;
            entry.health = EntryHealth::Dead;
            entry.teardown_requested = true;
        }
    }
    mark_started_teardown(shared, state, key);
}

/// Starts the entry-owned supervised teardown at most once per generation.
fn mark_started_teardown(shared: &Arc<OwnerShared>, state: &mut OwnerState, key: &QuicReuseKey) {
    let Some(entry) = state.entries.get_mut(key) else {
        return;
    };
    if entry.record.teardown_runs() != 0 {
        return;
    }
    entry.record.note_teardown_run();
    let generation = entry.generation;
    let record = Arc::clone(&entry.record);
    let initializer = entry.initializer_task.take();
    let task_shared = Arc::clone(shared);
    let task_key = key.clone();
    tokio::spawn(async move {
        if let Some(handle) = initializer {
            // Awaiting the entry-owned task is safe even when this teardown was
            // started from inside that same task: the task finishes publishing
            // and returns without needing the map lock again.
            let _ = handle.await;
        }
        // Cancel this generation's outstanding request streams and take the
        // acquired transport. Both happen before the model seam, so the
        // `Closing` window a test holds is already post-cancellation.
        let (transport, cancelled) = task_shared.take_transport_for_teardown(&task_key, generation);
        record.note_streams_cancelled(cancelled);
        let barrier = task_shared.seams.teardown_before_terminal.clone();
        if let Some(barrier) = barrier {
            barrier.check().await;
        }
        let terminal = match transport {
            Some(transport) => {
                transport.close();
                record.note_resource_close();
                EntryTerminal::Drained
            }
            None => EntryTerminal::Failed,
        };
        task_shared.finish_teardown(&task_key, generation, terminal);
    });
}

/// Spawns the entry-owned initializer task for a freshly installed reservation.
fn spawn_initializer(
    shared: &Arc<OwnerShared>,
    key: &QuicReuseKey,
    generation: u64,
) -> JoinHandle<()> {
    let task_shared = Arc::clone(shared);
    let task_key = key.clone();
    tokio::spawn(async move {
        let guard =
            InitializationGuard::new(Arc::clone(&task_shared), task_key.clone(), generation);
        let transport = task_shared
            .initializer
            .initialize(task_key, generation)
            .await;
        guard.deliver(transport);
    })
}

/// Waits until the entry for `key`/`generation` is no longer `Initializing`.
///
/// Interest in the entry's transition notify is registered *before* the state is
/// re-checked, so a transition between the check and the wait cannot be missed.
async fn wait_for_entry_change(shared: &Arc<OwnerShared>, key: &QuicReuseKey, generation: u64) {
    loop {
        let record = {
            let state = shared.lock();
            match state.entries.get(key) {
                Some(entry)
                    if entry.generation == generation
                        && entry.phase == EntryPhase::Initializing =>
                {
                    Arc::clone(&entry.record)
                }
                _ => return,
            }
        };
        let notified = record.changed.notified();
        tokio::pin!(notified);
        notified.as_mut().enable();
        let still_initializing = {
            let state = shared.lock();
            matches!(
                state.entries.get(key),
                Some(entry) if entry.generation == generation
                    && entry.phase == EntryPhase::Initializing
            )
        };
        if !still_initializing {
            return;
        }
        notified.await;
    }
}

/// One in-flight exchange registration on the owner's real [`Lifecycle`].
///
/// Held from `register()` until the exchange either leases a stream permit or is
/// rejected; dropping it releases the registration. A rejected post-close
/// admission therefore leaves zero liveness residue.
pub struct ExchangeRegistration {
    _guard: SharedInFlightGuard,
}

/// One leased stream permit on one `Active` entry generation.
///
/// Dropping it releases exactly one permit once; after the supervised teardown
/// has cancelled the generation's permits, the drop is a no-op because the
/// entry's counter was already zeroed.
pub struct StreamLease {
    shared: Arc<OwnerShared>,
    key: QuicReuseKey,
    generation: u64,
    record: Arc<EntryRecord>,
    _registration: ExchangeRegistration,
}

impl StreamLease {
    /// The exact key this lease belongs to.
    #[must_use]
    pub const fn key(&self) -> &QuicReuseKey {
        &self.key
    }

    /// The exact generation this lease belongs to.
    #[must_use]
    pub const fn generation(&self) -> u64 {
        self.generation
    }

    /// The surviving per-generation record.
    #[must_use]
    pub const fn record(&self) -> &Arc<EntryRecord> {
        &self.record
    }
}

impl Drop for StreamLease {
    fn drop(&mut self) {
        self.shared.release_permit(&self.key, self.generation);
    }
}

enum Admission {
    Rejected(UpstreamError),
    Leased {
        record: Arc<EntryRecord>,
        generation: u64,
    },
    Joined {
        generation: u64,
    },
}

/// The QUIC-specific shared-connection owner.
///
/// It owns one physical connection per validated [`QuicReuseKey`], bounded
/// cross-key admission, bounded per-connection stream permits, lazy idle expiry,
/// and the two-stage close plus entry-owned supervised teardown. It never
/// generalizes the serial TCP reuse pool and never hands an entry across keys.
///
/// Every operation that may start an entry-owned task (admission of a new
/// `Initializing` reservation, `linearize_close`, `maintain`, logical
/// deactivation) must be called from within the caller's Tokio runtime context,
/// exactly like the crate's other transports; the owner still creates no
/// runtime of its own.
pub struct QuicReuseOwner {
    shared: Arc<OwnerShared>,
}

impl QuicReuseOwner {
    /// Creates an owner with the system clock, the inert Slice 0 initializer,
    /// and no model seam.
    #[must_use]
    pub fn new(lifecycle: Arc<Lifecycle>) -> Self {
        Self::with_parts(
            lifecycle,
            Arc::new(SystemOwnerClock),
            Arc::new(ImmediateInitializer::default()),
            ModelSeams::default(),
        )
    }

    /// Creates an owner from explicit parts.
    ///
    /// Slice 1/2 pass their real [`EntryInitializer`] and `ModelSeams::default()`.
    #[must_use]
    pub fn with_parts(
        lifecycle: Arc<Lifecycle>,
        clock: Arc<dyn OwnerClock>,
        initializer: Arc<dyn EntryInitializer>,
        seams: ModelSeams,
    ) -> Self {
        Self {
            shared: Arc::new(OwnerShared {
                lifecycle,
                owner_cancellation: TransportCancellation::new(),
                clock,
                initializer,
                seams,
                state: Mutex::new(OwnerState {
                    accepting: true,
                    entries: HashMap::new(),
                    next_generation: 0,
                }),
            }),
        }
    }

    /// The shared lifecycle gate this owner registers exchanges with.
    #[must_use]
    pub fn lifecycle(&self) -> &Arc<Lifecycle> {
        &self.shared.lifecycle
    }

    /// Builds the per-exchange control carrying this owner's cancellation token.
    #[must_use]
    pub fn exchange_control(&self, context: ExchangeContext) -> ExchangeControl {
        ExchangeControl::new(context, self.shared.owner_cancellation.clone())
    }

    /// The current real `Lifecycle` state.
    #[must_use]
    pub fn lifecycle_state(&self) -> LifecycleState {
        self.shared.lifecycle.state()
    }

    /// The number of `Lifecycle` registrations currently outstanding.
    #[must_use]
    pub fn in_flight_exchanges(&self) -> usize {
        self.shared.lifecycle.in_flight()
    }

    /// Whether owner-map admission is still accepting new reservations.
    #[must_use]
    pub fn is_accepting(&self) -> bool {
        self.shared.lock().accepting
    }

    /// The number of live entries, including `Initializing` and `Closing` ones.
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.shared.lock().entries.len()
    }

    /// A read-only observation of the stored entry for `key`, if any.
    #[must_use]
    pub fn observe(&self, key: &QuicReuseKey) -> Option<EntryObservation> {
        let state = self.shared.lock();
        let entry = state.entries.get(key)?;
        Some(EntryObservation {
            key: key.clone(),
            generation: entry.generation,
            phase: entry.phase,
            health: entry.health,
            initialized: entry.initialized,
            teardown_requested: entry.teardown_requested,
            permits_in_use: entry.permits_in_use,
            last_used: entry.last_used,
            record: Arc::clone(&entry.record),
        })
    }

    /// Admission step one: registers one exchange under the real `Lifecycle`.
    ///
    /// A rejection is `UpstreamError::Closed(SideEffectState::NotSent)` and
    /// happens before any network operation.
    ///
    /// # Errors
    ///
    /// Returns `Closed(NotSent)` when the owner is already closing or closed.
    pub fn register(&self) -> Result<ExchangeRegistration, UpstreamError> {
        let guard = self.shared.lifecycle.register_owned()?;
        Ok(ExchangeRegistration { _guard: guard })
    }

    /// Admission step two: the atomic, no-`await` owner-map critical section,
    /// then the caller-owned wait for an `Initializing` entry to resolve.
    ///
    /// The critical section checks `accepting` first, looks the key up, marks
    /// dead/idle-expired entries `Closing` without removing them, capacity-checks
    /// counting `Initializing`/`Closing`/`Active` entries alike, and then reuses,
    /// joins, or installs. No `await` happens inside it.
    ///
    /// # Errors
    ///
    /// * `Closed(NotSent)` at the `accepting` gate, for a same-key `Closing`
    ///   entry (no drain wait, no second generation), for a dead entry, or when
    ///   close wins while joining an `Initializing` entry;
    /// * `Backpressure(NotSent)` when the owner-entry bound or this connection's
    ///   stream-slot bound is exhausted (never an unbounded queue);
    /// * the caller's own `Closed`/`Cancelled`/`DeadlineExceeded` when its
    ///   control wins the join race. That ends only this caller's wait; the
    ///   entry-owned initializer keeps running.
    pub async fn admit(
        &self,
        registration: ExchangeRegistration,
        key: QuicReuseKey,
        control: &ExchangeControl,
    ) -> Result<StreamLease, UpstreamError> {
        let now = self.shared.clock.now();
        let mut admission = self.atomic_admission(&key, now);
        loop {
            match admission {
                Admission::Rejected(error) => return Err(error),
                Admission::Leased { record, generation } => {
                    return Ok(StreamLease {
                        shared: Arc::clone(&self.shared),
                        key,
                        generation,
                        record,
                        _registration: registration,
                    });
                }
                Admission::Joined { generation } => {
                    let shared = Arc::clone(&self.shared);
                    let wait_key = key.clone();
                    let io = async move {
                        wait_for_entry_change(&shared, &wait_key, generation).await;
                        Ok::<(), UpstreamError>(())
                    };
                    race_control(
                        control,
                        SideEffectState::NotSent,
                        control.context().deadline(),
                        io,
                    )
                    .await?;
                    admission = self.after_join(&key, generation);
                }
            }
        }
    }

    /// The atomic multi-key admission section and its `accepting` gate.
    fn atomic_admission(&self, key: &QuicReuseKey, now: Instant) -> Admission {
        let mut state = self.shared.lock();

        // (1) The gate is checked first: install nothing, start no initializer,
        //     and touch no entry.
        if !state.accepting {
            return Admission::Rejected(UpstreamError::Closed(SideEffectState::NotSent));
        }
        // The real `Lifecycle` state is an independent check; neither the map
        // gate nor the lifecycle state substitutes for the other.
        if self.shared.lifecycle.state() != LifecycleState::Open {
            return Admission::Rejected(UpstreamError::Closed(SideEffectState::NotSent));
        }

        // (2)+(3) Look the key up and mark dead/idle-expired entries `Closing`
        //         without removing them (they stay discoverable until terminal).
        self.expire_idle_locked(&mut state, now);
        if let Some(entry) = state.entries.get_mut(key) {
            match entry.phase {
                EntryPhase::Active => {
                    if entry.health != EntryHealth::Healthy {
                        return Admission::Rejected(UpstreamError::Closed(
                            SideEffectState::NotSent,
                        ));
                    }
                    if entry.permits_in_use >= MAX_STREAMS_PER_CONNECTION {
                        return Admission::Rejected(UpstreamError::Backpressure(
                            SideEffectState::NotSent,
                        ));
                    }
                    entry.permits_in_use += 1;
                    return Admission::Leased {
                        record: Arc::clone(&entry.record),
                        generation: entry.generation,
                    };
                }
                EntryPhase::Initializing => {
                    return Admission::Joined {
                        generation: entry.generation,
                    };
                }
                // A same-key `Closing` lookup: the existing typed pre-send
                // closed result, with no drain wait and no second generation.
                EntryPhase::Closing => {
                    return Admission::Rejected(UpstreamError::Closed(SideEffectState::NotSent));
                }
            }
        }

        // (4) Capacity counts `Initializing`/`Closing`/`Active` alike: a slot
        //     frees only at the terminal removal.
        if state.entries.len() >= MAX_CONNECTIONS_PER_OWNER {
            return Admission::Rejected(UpstreamError::Backpressure(SideEffectState::NotSent));
        }

        // (5) Install a fresh-generation `Initializing` reservation together
        //     with its entry-owned initializer task, still under this lock.
        state.next_generation += 1;
        let generation = state.next_generation;
        let Ok(liveness) = self.shared.lifecycle.register_owned() else {
            // The owner began closing between the gate read and this
            // registration; install nothing and leave no residue.
            return Admission::Rejected(UpstreamError::Closed(SideEffectState::NotSent));
        };
        let record = Arc::new(EntryRecord::new(key.clone(), generation));
        let initializer_task = spawn_initializer(&self.shared, key, generation);
        state.entries.insert(
            key.clone(),
            Entry {
                generation,
                phase: EntryPhase::Initializing,
                health: EntryHealth::Healthy,
                teardown_requested: false,
                initialized: false,
                transport: None,
                permits_in_use: 0,
                last_used: now,
                liveness: Some(liveness),
                initializer_task: Some(initializer_task),
                record: Arc::clone(&record),
            },
        );
        Admission::Joined { generation }
    }

    /// Re-checks the entry after an `Initializing` join wait.
    fn after_join(&self, key: &QuicReuseKey, generation: u64) -> Admission {
        let mut state = self.shared.lock();
        let Some(entry) = state.entries.get_mut(key) else {
            return Admission::Rejected(UpstreamError::Closed(SideEffectState::NotSent));
        };
        if entry.generation != generation {
            return Admission::Rejected(UpstreamError::Closed(SideEffectState::NotSent));
        }
        match entry.phase {
            EntryPhase::Active if entry.health == EntryHealth::Healthy => {
                if entry.permits_in_use >= MAX_STREAMS_PER_CONNECTION {
                    return Admission::Rejected(UpstreamError::Backpressure(
                        SideEffectState::NotSent,
                    ));
                }
                entry.permits_in_use += 1;
                Admission::Leased {
                    record: Arc::clone(&entry.record),
                    generation,
                }
            }
            EntryPhase::Initializing => Admission::Joined { generation },
            // Close won the shared lock, or the entry was logically deactivated:
            // the typed pre-send closed result, never a late lease.
            EntryPhase::Active | EntryPhase::Closing => {
                Admission::Rejected(UpstreamError::Closed(SideEffectState::NotSent))
            }
        }
    }

    /// Marks healthy, unleased, idle-expired entries `Closing` under the map
    /// lock without removing them, starting their supervised teardown once.
    fn expire_idle_locked(&self, state: &mut OwnerState, now: Instant) {
        let expired: Vec<QuicReuseKey> = state
            .entries
            .iter()
            .filter(|(_, entry)| {
                matches!(entry.phase, EntryPhase::Initializing | EntryPhase::Active)
                    && entry.health == EntryHealth::Healthy
                    && entry.permits_in_use == 0
                    && now.saturating_duration_since(entry.last_used) >= QUIC_IDLE_TIMEOUT
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in &expired {
            mark_closing_and_start(&self.shared, state, key);
        }
    }

    /// Lazy maintenance: expires idle entries using the injected clock.
    ///
    /// It never removes an entry, never awaits a drain, and starts no timer. An
    /// expired entry becomes `Closing` — discoverable but unleasable — until its
    /// supervised teardown reaches terminal.
    pub fn maintain(&self) {
        let now = self.shared.clock.now();
        let mut state = self.shared.lock();
        self.expire_idle_locked(&mut state, now);
    }

    /// The optional deterministic drain for tests: expires idle entries and
    /// awaits each captured generation's shared terminal completion.
    ///
    /// The await is confined to maintenance and is never part of admission.
    pub async fn maintain_and_drain(&self) {
        self.maintain();
        let records: Vec<Arc<EntryRecord>> = {
            let state = self.shared.lock();
            state
                .entries
                .values()
                .filter(|entry| entry.phase == EntryPhase::Closing)
                .map(|entry| Arc::clone(&entry.record))
                .collect()
        };
        for record in records {
            record.wait_terminal().await;
        }
    }

    /// Applies one R0b classification to the exact key+generation.
    ///
    /// This is the only logical-deactivation path: `EntryTerminal` moves
    /// `Active -> Closing` under the map lock (immediately unleasable) and starts
    /// the same supervised teardown as every other close path, while the physical
    /// removal stays a terminal concern. `StreamLocal` never changes the entry.
    /// A stale key+generation is a no-op.
    pub fn apply_error_class(
        &self,
        key: &QuicReuseKey,
        generation: u64,
        class: QuicErrorClass,
    ) -> QuicErrorOutcome {
        let mut state = self.shared.lock();
        let Some(entry) = state.entries.get(key) else {
            return QuicErrorOutcome::NoMatchingEntry;
        };
        if entry.generation != generation {
            return QuicErrorOutcome::NoMatchingEntry;
        }
        if class == QuicErrorClass::StreamLocal {
            return QuicErrorOutcome::EntryUnchanged;
        }
        if entry.phase == EntryPhase::Closing {
            // Already logically deactivated by an earlier terminal signal or
            // close; idempotent, and never a second teardown.
            return QuicErrorOutcome::EntryDeactivated;
        }
        mark_closing_and_start(&self.shared, &mut state, key);
        QuicErrorOutcome::EntryDeactivated
    }

    /// Owner-close stage one: the real `Lifecycle` `Open -> Closing` plus the
    /// owner token cancellation, before any map lock is taken.
    #[must_use]
    pub fn begin_close(&self) -> CloseTransition {
        let transition = self.shared.lifecycle.begin_close();
        if transition == CloseTransition::BeganClosing {
            self.shared.owner_cancellation.cancel();
        }
        transition
    }

    /// Owner-close stage two: the sole map-side admission-vs-close
    /// linearization.
    ///
    /// In one no-`await` section it sets `accepting = false`, marks every
    /// `Initializing`/`Active` entry `Closing`/`TeardownRequested` without
    /// removing its reservation, and starts each entry's exactly-once supervised
    /// teardown. It returns the captured records so a caller can await their
    /// shared terminal completion. [`Self::close`] runs it automatically right
    /// after stage one; it is exposed so the deterministic model test can order
    /// the two stages explicitly.
    pub fn linearize_close(&self) -> Vec<Arc<EntryRecord>> {
        let mut state = self.shared.lock();
        state.accepting = false;
        let keys: Vec<QuicReuseKey> = state.entries.keys().cloned().collect();
        for key in &keys {
            mark_closing_and_start(&self.shared, &mut state, key);
        }
        keys.iter()
            .filter_map(|key| {
                state
                    .entries
                    .get(key)
                    .map(|entry| Arc::clone(&entry.record))
            })
            .collect()
    }

    /// The complete two-stage owner close.
    ///
    /// Stage one turns the real `Lifecycle` `Open -> Closing` and cancels the
    /// owner token; stage two is the map-side linearization; then every captured
    /// entry's shared terminal completion is awaited and the `Lifecycle` drains
    /// before shutdown completes. Repeated and concurrent calls are idempotent.
    pub async fn close(&self) -> CloseResult {
        match self.begin_close() {
            CloseTransition::AlreadyClosed => return CloseResult::AlreadyClosed,
            CloseTransition::BeganClosing | CloseTransition::AlreadyClosing => {}
        }
        let captured = self.linearize_close();
        let barrier = self.shared.seams.close_after_linearize.clone();
        if let Some(barrier) = barrier {
            barrier.check().await;
        }
        for record in &captured {
            record.wait_terminal().await;
        }
        self.shared.lifecycle.drain().await;
        match self.shared.lifecycle.finish_close() {
            CloseCompletion::Closed | CloseCompletion::AlreadyClosed => CloseResult::Closed,
            CloseCompletion::NotClosing | CloseCompletion::InFlight => CloseResult::AlreadyClosing,
        }
    }
}
