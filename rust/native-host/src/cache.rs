use std::cell::Cell;
use std::rc::Rc;
use std::time::Instant;

use mosdns_cache_core::{CacheConfig, CacheError, NativeCache};
use mosdns_dns_core::{
    QueryError, ResponseError, ResponseMetadata, ResponseQuestion, observe_response_metadata,
    patch_response_id_ra, validate_response,
};

const CACHE_CAPACITY: u64 = 64;
const CACHE_LAZY_TTL_SECS: u32 = 0;
const CLASS_IN: u16 = 1;
const RCODE_NOERROR: u16 = 0;
const RCODE_SERVFAIL: u16 = 2;
const RCODE_NXDOMAIN: u16 = 3;
const RETENTION_FALLBACK_SECS: u32 = 5;
const RETENTION_NXDOMAIN_SECS: u32 = 30;
const RETENTION_MAX_NOERROR_SECS: u32 = 300;

/// A monotonic clock expressed in the same elapsed-second epoch for every
/// operation on one host-owned cache.
pub trait CacheClock {
    /// Returns elapsed whole seconds since the clock's private epoch.
    fn now_seconds(&self) -> Result<i64, CacheAdapterError>;
}

#[derive(Clone)]
pub(crate) struct MonotonicCacheClock {
    epoch: Instant,
}

impl MonotonicCacheClock {
    pub(crate) fn new() -> Self {
        Self {
            epoch: Instant::now(),
        }
    }
}

impl CacheClock for MonotonicCacheClock {
    fn now_seconds(&self) -> Result<i64, CacheAdapterError> {
        i64::try_from(self.epoch.elapsed().as_secs()).map_err(|_| CacheAdapterError::ClockOverflow)
    }
}

/// A deterministic whole-second clock for adapter and request-driver tests.
#[derive(Clone)]
pub struct CacheTestClock {
    seconds: Rc<Cell<u64>>,
}

impl CacheTestClock {
    #[must_use]
    pub fn new(seconds: u64) -> Self {
        Self {
            seconds: Rc::new(Cell::new(seconds)),
        }
    }

    pub fn set(&self, seconds: u64) {
        self.seconds.set(seconds);
    }

    pub fn advance(&self, seconds: u64) {
        self.seconds.set(self.seconds.get().saturating_add(seconds));
    }
}

impl CacheClock for CacheTestClock {
    fn now_seconds(&self) -> Result<i64, CacheAdapterError> {
        i64::try_from(self.seconds.get()).map_err(|_| CacheAdapterError::ClockOverflow)
    }
}

/// Errors from the native adapter boundary. A malformed query is a caller
/// error; malformed responses are admission misses rather than a reason to
/// alter the W1 forwarding result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CacheAdapterError {
    InvalidQuery,
    Cache(CacheError),
    ClockOverflow,
    ExpiryOverflow,
}

impl std::fmt::Display for CacheAdapterError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidQuery => formatter.write_str("invalid cache query"),
            Self::Cache(error) => error.fmt(formatter),
            Self::ClockOverflow => formatter.write_str("cache clock overflow"),
            Self::ExpiryOverflow => formatter.write_str("cache expiry overflow"),
        }
    }
}

impl std::error::Error for CacheAdapterError {}

impl From<CacheError> for CacheAdapterError {
    fn from(error: CacheError) -> Self {
        Self::Cache(error)
    }
}

/// One host-owned adapter around one owned cache-core store.
#[derive(Clone)]
pub struct NativeCacheAdapter {
    cache: Rc<NativeCache>,
    clock: Rc<dyn CacheClock>,
}

impl NativeCacheAdapter {
    /// Creates the native Phase 5A cache with the reviewed fixed W2 capacity.
    pub fn new() -> Result<Self, CacheAdapterError> {
        Self::with_clock(Rc::new(MonotonicCacheClock::new()))
    }

    /// Creates a cache with an injected clock. The clock is private to this
    /// cache instance, so stored and lookup timestamps cannot mix epochs.
    pub fn with_clock(clock: Rc<dyn CacheClock>) -> Result<Self, CacheAdapterError> {
        let cache = NativeCache::new(CacheConfig {
            capacity: CACHE_CAPACITY,
            lazy_cache_ttl_secs: CACHE_LAZY_TTL_SECS,
            flags: 0,
        })?;
        Ok(Self {
            cache: Rc::new(cache),
            clock,
        })
    }

    /// Test constructor using a deterministic clock and a fresh store.
    pub fn for_test(clock: CacheTestClock) -> Result<Self, CacheAdapterError> {
        Self::with_clock(Rc::new(clock))
    }

    /// Returns the key bytes used by the in-memory native cache.
    pub fn key_for_query(&self, query: &[u8]) -> Result<Option<Vec<u8>>, CacheAdapterError> {
        key_for_query(query)
    }

    /// Looks up an eligible query and patches the response copy for its ID.
    pub fn lookup(&self, query: &[u8]) -> Result<Option<Vec<u8>>, CacheAdapterError> {
        let Some((key, request_id)) = query_key_and_id(query)? else {
            return Ok(None);
        };
        let now = self.clock.now_seconds()?;
        let Some(lookup) = self.cache.lookup(&key, now)? else {
            return Ok(None);
        };
        let response = patch_response_id_ra(&lookup.response, request_id)
            .map_err(|_| CacheAdapterError::InvalidQuery)?;
        Ok(Some(response))
    }

    /// Arms one request-local publication token after an eligible query miss.
    pub fn begin_store(&self, query: &[u8]) -> Result<Option<PendingStore>, CacheAdapterError> {
        let Some((key, _, question)) = query_key_question(query)? else {
            return Ok(None);
        };
        Ok(Some(PendingStore {
            adapter: self.clone(),
            key,
            question,
        }))
    }

    /// Returns the underlying store count after cache-core maintenance.
    #[must_use]
    pub fn len(&self) -> u64 {
        self.cache.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A request-owned cache publication token. Dropping it is the normal path
/// for cancellation, failed execution, malformed responses, and local errors.
pub struct PendingStore {
    adapter: NativeCacheAdapter,
    key: Vec<u8>,
    question: ResponseQuestion,
}

impl PendingStore {
    /// Validates and publishes one successful upstream response. Consuming the
    /// token makes duplicate publication impossible at the API boundary.
    pub fn publish(self, response: &[u8]) -> Result<bool, CacheAdapterError> {
        let Some(metadata) = response_metadata(response) else {
            return Ok(false);
        };
        if !eligible_response(&metadata, &self.question) {
            return Ok(false);
        }
        if validate_response(response).is_err() {
            return Ok(false);
        }

        let ttl = mosdns_dns_core::observe_response_ttl(response)
            .map_err(|_| CacheAdapterError::InvalidQuery)?;
        let retention = retention_seconds(metadata.rcode, ttl.minimal_ttl, ttl.record_count);
        let stored_at = self.adapter.clock.now_seconds()?;
        let expires = stored_at
            .checked_add(i64::from(retention))
            .ok_or(CacheAdapterError::ExpiryOverflow)?;
        self.adapter
            .cache
            .store(&self.key, response, &[], stored_at, expires, expires)?;
        Ok(true)
    }
}

fn query_key_and_id(query: &[u8]) -> Result<Option<(Vec<u8>, u16)>, CacheAdapterError> {
    let Some((key, header, _)) = query_key_question(query)? else {
        return Ok(None);
    };
    Ok(Some((key, header.id)))
}

fn query_key_question(
    query: &[u8],
) -> Result<Option<(Vec<u8>, mosdns_dns_core::QueryHeader, ResponseQuestion)>, CacheAdapterError> {
    let (header, question) = mosdns_dns_core::parse_query(query).map_err(|error| match error {
        QueryError::Parse(_) | QueryError::Unsupported(_) => CacheAdapterError::InvalidQuery,
    })?;
    if header.arcount != 0 || question.qclass != CLASS_IN {
        return Ok(None);
    }
    let key = encode_key(
        query,
        &header,
        &question.qname_wire,
        question.qtype,
        question.qclass,
    );
    Ok(Some((
        key,
        header,
        ResponseQuestion {
            qname_wire: question.qname_wire,
            qtype: question.qtype,
            qclass: question.qclass,
        },
    )))
}

fn key_for_query(query: &[u8]) -> Result<Option<Vec<u8>>, CacheAdapterError> {
    Ok(query_key_question(query)?.map(|(key, _, _)| key))
}

fn encode_key(
    query: &[u8],
    header: &mosdns_dns_core::QueryHeader,
    qname_wire: &[u8],
    qtype: u16,
    qclass: u16,
) -> Vec<u8> {
    let flags = u16::from_be_bytes([query[2], query[3]]);
    let security_flags = flags & 0x0030; // AD and CD are independent key dimensions.
    let mut key = Vec::with_capacity(8 + qname_wire.len());
    key.extend_from_slice(b"mosdns-cache-v1\0");
    key.extend_from_slice(&(u16::try_from(qname_wire.len()).unwrap_or(u16::MAX)).to_be_bytes());
    key.extend_from_slice(qname_wire);
    key.extend_from_slice(&qtype.to_be_bytes());
    key.extend_from_slice(&qclass.to_be_bytes());
    key.extend_from_slice(&security_flags.to_be_bytes());
    // Keep the header argument semantically used at the key boundary; ID is
    // deliberately absent, while these fields prove the parsed shape.
    let _ = header.qdcount;
    key
}

fn response_metadata(response: &[u8]) -> Option<ResponseMetadata> {
    observe_response_metadata(response).ok()
}

fn eligible_response(metadata: &ResponseMetadata, question: &ResponseQuestion) -> bool {
    metadata.opcode == 0
        && !metadata.truncated
        && !metadata.has_opt
        && metadata.question.as_ref() == Some(question)
}

fn retention_seconds(rcode: u16, minimal_ttl: u32, record_count: u32) -> u32 {
    match rcode {
        RCODE_NXDOMAIN => RETENTION_NXDOMAIN_SECS,
        RCODE_SERVFAIL => RETENTION_FALLBACK_SECS,
        RCODE_NOERROR if record_count > 0 && minimal_ttl > 0 => {
            minimal_ttl.min(RETENTION_MAX_NOERROR_SECS)
        }
        _ => RETENTION_FALLBACK_SECS,
    }
}

impl From<ResponseError> for CacheAdapterError {
    fn from(_: ResponseError) -> Self {
        Self::InvalidQuery
    }
}
