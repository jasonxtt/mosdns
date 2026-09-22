//! Pure Rust DNS wire foundation for the experimental MosDNS query core.
//!
//! This crate mirror of the frozen Go oracle contract in
//! `pkg/query_context/rust_bridge` (Slice 0 of the Phase 3 task) and the
//! existing `cache-core` wire walk. It contains no cgo, Go types, server
//! state, plugin registry, or network I/O, and is dependency-light so the
//! later ABI/Go-adapter slices can link it as an `rlib`.
//!
//! The public surface is deliberately small and pure:
//!
//! - `query`: strict query header/question validation (QR clear, QUERY
//!   opcode, exactly one question) and name/compression bounds;
//! - `edns`: OPT presence, UDP size, DO bit, and first ECS option with family,
//!   source netmask/scope, and fixed-width address;
//! - `response`: response validation and TTL observation/aging/replacement
//!   over declared counts, skipping OPT records;
//! - `header`: response ID/RA patching and pure UDP/stream/HTTP framing
//!   helpers;
//! - `resolver`: bootstrap query encoding and response selection for endpoint
//!   resolution, still without sockets, clocks, caches, or a runtime.
//!
//! Typed errors classify wire defects versus unsupported-but-legal input, and
//! every inspection allocates nothing or returns caller-owned values; malformed
//! input never produces partial output or mutates its input. `resolver`'s query
//! builder is the one exception to "allocates nothing": it returns a new
//! caller-owned query buffer and never retains or mutates its inputs.

#![forbid(unsafe_op_in_unsafe_fn)]
// Pedantic is denied at workspace level; like the matcher-core/runtime
// crates, this pure-logic crate opts out of pedantic while keeping the
// workspace `all` lints (correctness) as hard errors.
#![allow(clippy::pedantic)]

pub mod edns;
pub mod header;
pub mod query;
pub mod resolver;
pub mod response;

// Re-exports mirror the Go oracle's public atoms so callers depend on this
// crate, not on module paths, when the wire layer is later consumed by the ABI
// slice.
pub use edns::{EcsInfo, EdnsInfo, EdnsParseError, OPT_RDLEN, extract_edns, extract_edns_at};
pub use header::{
    FrameMode, FramingError, HeaderError, ResponseBuildError, ResponseHeader, frame_response,
    inspect_response_header, patch_response_id_ra, synthesize_response,
};
pub use query::{
    QueryError, QueryHeader, QueryParseError, QueryUnsupportedError, QuestionInfo, parse_query,
    parse_question,
};
pub use resolver::{
    AddressFamily, CnameChainPolicy, QueryIdSource, RESOLVER_DEFAULT_MAX_CNAME_LINKS,
    RESOLVER_MAX_CNAME_LINKS, RESOLVER_UDP_PAYLOAD_SIZE, ResolverWireError, SelectedAddress,
    build_resolver_query, parse_resolver_response,
};
pub use response::{
    ResponseError, ResponseMetadata, ResponseQuestion, TtlInfo, age_response_ttls,
    observe_response_metadata, observe_response_ttl, replace_response_ttls, validate_response,
};
