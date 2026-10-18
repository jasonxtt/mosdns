//! `MosDNS` Rust cache core: the safe [`NativeCache`] used by the native host.
//! (The former C ABI and its handle registry were removed with the Go bridge in Phase 6a.)

#![allow(clippy::missing_errors_doc)]

use bytes::Bytes;
use moka::sync::Cache as MokaCache;
use std::sync::Arc;

mod wire;

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct CacheConfig {
    pub capacity: u64,
    pub lazy_cache_ttl_secs: u32,
    pub flags: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LookupState {
    Miss = 0,
    Fresh = 1,
    Lazy = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CacheError {
    InvalidConfig,
    InvalidKey,
    InvalidResponse,
    InvalidExpiry,
    Internal,
}

impl std::fmt::Display for CacheError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::InvalidConfig => "invalid cache configuration",
            Self::InvalidKey => "invalid cache key",
            Self::InvalidResponse => "invalid cache response",
            Self::InvalidExpiry => "invalid cache expiry",
            Self::Internal => "cache operation failed",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for CacheError {}

struct CacheState {
    config: CacheConfig,
    entries: MokaCache<Bytes, Arc<CacheEntry>>,
}

/// An owned, safe cache instance for Rust-native callers.
///
/// This object owns one Moka store and accepts ordinary Rust slices.
pub struct NativeCache {
    state: CacheState,
}

/// An optional native-only immutable attachment. It shares the exact entry's
/// replacement/eviction/flush lifetime. Native snapshots retain it; codecs choose
/// supported typed metadata.
#[derive(Clone)]
pub struct NativeAttachment(Arc<dyn std::any::Any + Send + Sync>);
impl NativeAttachment {
    pub fn new<T: std::any::Any + Send + Sync>(value: T) -> Self {
        Self(Arc::new(value))
    }
    #[must_use]
    pub fn downcast_ref<T: std::any::Any>(&self) -> Option<&T> {
        self.0.downcast_ref()
    }
}
impl std::fmt::Debug for NativeAttachment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NativeAttachment")
    }
}
impl PartialEq for NativeAttachment {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}
impl Eq for NativeAttachment {}

#[derive(Debug)]
struct CacheEntry {
    attachment: Option<NativeAttachment>,
    response: Bytes,
    domain_set: Bytes,
    stored_at_unix: i64,
    message_expires_at_unix: i64,
    cache_expires_at_unix: i64,
    wall_times: Option<[i64; 3]>,
}

#[derive(Debug, Eq, PartialEq)]
pub struct NativeLookup {
    pub attachment: Option<NativeAttachment>,
    pub state: LookupState,
    pub stored_at_unix: i64,
    pub message_expires_at_unix: i64,
    pub response: Vec<u8>,
    pub domain_set: Vec<u8>,
}

/// An owned native snapshot; runtime timestamps use the caller's monotonic epoch.
#[derive(Clone, Debug)]
pub struct NativeSnapshotEntry {
    pub attachment: Option<NativeAttachment>,
    pub key: Vec<u8>,
    pub response: Vec<u8>,
    pub domain_set: Vec<u8>,
    pub times: [i64; 3],
    pub wall_times: Option<[i64; 3]>,
}

/// Fully validated, owned native insertion prepared before a management commit.
pub struct PreparedNativeEntry {
    key: Bytes,
    entry: Arc<CacheEntry>,
}
impl PreparedNativeEntry {
    pub fn new(item: NativeSnapshotEntry) -> Result<Self, CacheError> {
        validate_store(
            &item.key,
            &item.response,
            item.times[0],
            item.times[1],
            item.times[2],
        )?;
        Ok(Self {
            key: Bytes::from(item.key),
            entry: Arc::new(CacheEntry {
                attachment: item.attachment,
                response: Bytes::from(item.response),
                domain_set: Bytes::from(item.domain_set),
                stored_at_unix: item.times[0],
                message_expires_at_unix: item.times[1],
                cache_expires_at_unix: item.times[2],
                wall_times: item.wall_times,
            }),
        })
    }
}

fn validate_store(
    key: &[u8],
    response: &[u8],
    stored_at_unix: i64,
    message_expires_at_unix: i64,
    cache_expires_at_unix: i64,
) -> Result<(), CacheError> {
    if key.is_empty() {
        return Err(CacheError::InvalidKey);
    }
    if response.is_empty() || wire::validate_response(response).is_err() {
        return Err(CacheError::InvalidResponse);
    }
    if message_expires_at_unix < stored_at_unix || cache_expires_at_unix < stored_at_unix {
        return Err(CacheError::InvalidExpiry);
    }
    Ok(())
}

fn lookup_data(
    cache: &NativeCache,
    key: &[u8],
    now_unix: i64,
) -> Result<Option<NativeLookup>, CacheError> {
    let Some(entry) = cache.state.entries.get(key) else {
        return Ok(None);
    };
    if now_unix >= entry.cache_expires_at_unix {
        cache.state.entries.invalidate(key);
        cache.state.entries.run_pending_tasks();
        return Ok(None);
    }

    let (lookup_state, response) = if now_unix < entry.message_expires_at_unix {
        let elapsed = now_unix
            .saturating_sub(entry.stored_at_unix)
            .clamp(0, i64::from(u32::MAX));
        let elapsed = u32::try_from(elapsed).unwrap_or(u32::MAX);
        (LookupState::Fresh, wire::age_ttls(&entry.response, elapsed))
    } else if cache.state.config.lazy_cache_ttl_secs > 0 {
        (LookupState::Lazy, wire::set_ttls(&entry.response, 5))
    } else {
        return Ok(None);
    };
    let response = response.map_err(|_| {
        cache.state.entries.invalidate(key);
        CacheError::Internal
    })?;
    Ok(Some(NativeLookup {
        attachment: entry.attachment.clone(),
        state: lookup_state,
        stored_at_unix: entry.stored_at_unix,
        message_expires_at_unix: entry.message_expires_at_unix,
        response,
        domain_set: entry.domain_set.to_vec(),
    }))
}

impl NativeCache {
    pub fn new(config: CacheConfig) -> Result<Self, CacheError> {
        if config.capacity == 0 {
            return Err(CacheError::InvalidConfig);
        }
        Ok(Self {
            state: CacheState {
                config,
                entries: MokaCache::builder().max_capacity(config.capacity).build(),
            },
        })
    }

    pub fn store(
        &self,
        key: &[u8],
        response: &[u8],
        domain_set: &[u8],
        stored_at_unix: i64,
        message_expires_at_unix: i64,
        cache_expires_at_unix: i64,
    ) -> Result<(), CacheError> {
        self.store_timed(
            key,
            response,
            domain_set,
            [
                stored_at_unix,
                message_expires_at_unix,
                cache_expires_at_unix,
            ],
            None,
        )
    }

    /// Dual-clock insertion.
    pub fn store_timed(
        &self,
        key: &[u8],
        response: &[u8],
        domain_set: &[u8],
        times: [i64; 3],
        wall_times: Option<[i64; 3]>,
    ) -> Result<(), CacheError> {
        self.store_timed_with_attachment(key, response, domain_set, times, wall_times, None)
    }

    pub fn store_timed_with_attachment(
        &self,
        key: &[u8],
        response: &[u8],
        domain_set: &[u8],
        times: [i64; 3],
        wall_times: Option<[i64; 3]>,
        attachment: Option<NativeAttachment>,
    ) -> Result<(), CacheError> {
        let [
            stored_at_unix,
            message_expires_at_unix,
            cache_expires_at_unix,
        ] = times;
        validate_store(
            key,
            response,
            stored_at_unix,
            message_expires_at_unix,
            cache_expires_at_unix,
        )?;
        self.state.entries.insert(
            Bytes::copy_from_slice(key),
            Arc::new(CacheEntry {
                attachment,
                response: Bytes::copy_from_slice(response),
                domain_set: Bytes::copy_from_slice(domain_set),
                stored_at_unix,
                message_expires_at_unix,
                cache_expires_at_unix,
                wall_times,
            }),
        );
        Ok(())
    }

    /// Commits prevalidated entries without a recoverable partial-merge error.
    pub fn merge_prepared(&self, entries: Vec<PreparedNativeEntry>) {
        for item in entries {
            self.state.entries.insert(item.key, item.entry);
        }
    }

    /// Captures only entries whose complete retention window is still live.
    #[must_use]
    pub fn snapshot(&self, now: i64) -> Vec<NativeSnapshotEntry> {
        self.state.entries.run_pending_tasks();
        self.state
            .entries
            .iter()
            .filter_map(|(key, entry)| {
                (entry.cache_expires_at_unix > now).then(|| NativeSnapshotEntry {
                    attachment: entry.attachment.clone(),
                    key: key.to_vec(),
                    response: entry.response.to_vec(),
                    domain_set: entry.domain_set.to_vec(),
                    times: [
                        entry.stored_at_unix,
                        entry.message_expires_at_unix,
                        entry.cache_expires_at_unix,
                    ],
                    wall_times: entry.wall_times,
                })
            })
            .collect()
    }

    /// Rejects oversized snapshots before duplicating live entry payloads.
    pub fn snapshot_bounded(
        &self,
        now: i64,
        max_entries: usize,
        max_bytes: usize,
    ) -> Result<Vec<NativeSnapshotEntry>, CacheError> {
        self.state.entries.run_pending_tasks();
        let mut snapshot = Vec::new();
        let mut bytes = 0_usize;
        for (key, entry) in &self.state.entries {
            if entry.cache_expires_at_unix <= now {
                continue;
            }
            bytes = bytes
                .checked_add(key.len())
                .and_then(|n| n.checked_add(entry.response.len()))
                .and_then(|n| n.checked_add(entry.domain_set.len()))
                .ok_or(CacheError::InvalidConfig)?;
            if snapshot.len() >= max_entries || bytes > max_bytes {
                return Err(CacheError::InvalidConfig);
            }
            snapshot.push(NativeSnapshotEntry {
                attachment: entry.attachment.clone(),
                key: key.to_vec(),
                response: entry.response.to_vec(),
                domain_set: entry.domain_set.to_vec(),
                times: [
                    entry.stored_at_unix,
                    entry.message_expires_at_unix,
                    entry.cache_expires_at_unix,
                ],
                wall_times: entry.wall_times,
            });
        }
        Ok(snapshot)
    }

    pub fn lookup(&self, key: &[u8], now_unix: i64) -> Result<Option<NativeLookup>, CacheError> {
        if key.is_empty() {
            return Err(CacheError::InvalidKey);
        }
        lookup_data(self, key, now_unix)
    }

    #[must_use]
    pub fn len(&self) -> u64 {
        self.state.entries.run_pending_tasks();
        self.state.entries.entry_count()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    #[must_use]
    pub fn live_len(&self, now: i64) -> usize {
        self.state.entries.run_pending_tasks();
        self.state
            .entries
            .iter()
            .filter(|(_, entry)| entry.cache_expires_at_unix > now)
            .count()
    }

    pub fn flush(&self) {
        self.state.entries.invalidate_all();
        self.state.entries.run_pending_tasks();
    }
}

#[cfg(test)]
mod tests {
    use super::{CacheConfig, LookupState, NativeCache};

    const CONFIG: CacheConfig = CacheConfig {
        capacity: 4,
        lazy_cache_ttl_secs: 0,
        flags: 0,
    };

    fn response(ttl: u32, address: [u8; 4]) -> Vec<u8> {
        let mut packet = vec![
            0x12, 0x34, 0x81, 0x80, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x07, b'e',
            b'x', b'a', b'm', b'p', b'l', b'e', 0x03, b'o', b'r', b'g', 0x00, 0x00, 0x01, 0x00,
            0x01, 0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01, 0, 0, 0, 0, 0x00, 0x04,
        ];
        packet.extend_from_slice(&address);
        packet[35..39].copy_from_slice(&ttl.to_be_bytes());
        packet
    }

    fn answer_ttl(packet: &[u8]) -> u32 {
        u32::from_be_bytes(packet[35..39].try_into().expect("answer TTL"))
    }

    #[test]
    fn native_attachment_tracks_entry_replacement_flush_and_import() {
        let cache = NativeCache::new(CONFIG).unwrap();
        let wire = response(60, [192, 0, 2, 1]);
        cache
            .store_timed_with_attachment(
                b"key",
                &wire,
                b"rules",
                [100, 160, 200],
                None,
                Some(super::NativeAttachment::new(7_u64)),
            )
            .unwrap();
        assert_eq!(
            cache
                .lookup(b"key", 101)
                .unwrap()
                .unwrap()
                .attachment
                .unwrap()
                .downcast_ref::<u64>(),
            Some(&7)
        );
        let snapshot = cache.snapshot(101);
        cache.merge_prepared(
            snapshot
                .into_iter()
                .map(|item| super::PreparedNativeEntry::new(item).unwrap())
                .collect(),
        );
        assert_eq!(
            cache
                .lookup(b"key", 101)
                .unwrap()
                .unwrap()
                .attachment
                .unwrap()
                .downcast_ref::<u64>(),
            Some(&7),
            "native snapshots preserve attachment"
        );
        cache
            .store_timed_with_attachment(
                b"key",
                &wire,
                b"rules",
                [100, 160, 200],
                None,
                Some(super::NativeAttachment::new(8_u64)),
            )
            .unwrap();
        cache.store(b"key", &wire, b"", 100, 160, 200).unwrap();
        assert!(
            cache
                .lookup(b"key", 101)
                .unwrap()
                .unwrap()
                .attachment
                .is_none()
        );
        cache.flush();
        assert!(cache.lookup(b"key", 101).unwrap().is_none());
    }

    #[test]
    fn native_cache_objects_are_isolated_and_return_owned_payloads() {
        let first = NativeCache::new(CONFIG).expect("first cache");
        let second = NativeCache::new(CONFIG).expect("second cache");
        let packet = response(60, [192, 0, 2, 1]);

        first
            .store(b"key", &packet, b"domain-a", 100, 160, 200)
            .expect("store");
        assert!(
            second
                .lookup(b"key", 101)
                .expect("isolated lookup")
                .is_none()
        );

        let mut hit = first.lookup(b"key", 101).expect("lookup").expect("hit");
        assert_eq!(hit.state, LookupState::Fresh);
        assert_eq!(hit.domain_set, b"domain-a");
        hit.response[0] = 0;
        hit.domain_set[0] = b'X';

        let fresh = first
            .lookup(b"key", 102)
            .expect("second lookup")
            .expect("hit");
        assert_eq!(answer_ttl(&fresh.response), 58);
        assert_eq!(fresh.response[41..45], packet[41..45]);
        assert_eq!(answer_ttl(&packet), 60);
        assert_eq!(fresh.domain_set, b"domain-a");
    }

    #[test]
    fn native_cache_overwrite_and_expiry_are_exact_without_cumulative_ttl_age() {
        let cache = NativeCache::new(CONFIG).expect("cache");
        let first = response(60, [192, 0, 2, 1]);
        let replacement = response(30, [192, 0, 2, 2]);

        cache
            .store(b"key", &first, &[], 100, 160, 200)
            .expect("first store");
        let first_hit = cache
            .lookup(b"key", 110)
            .expect("first lookup")
            .expect("hit");
        assert_eq!(answer_ttl(&first_hit.response), 50);
        let second_hit = cache
            .lookup(b"key", 111)
            .expect("second lookup")
            .expect("hit");
        assert_eq!(answer_ttl(&second_hit.response), 49);

        cache
            .store(b"key", &replacement, &[], 120, 150, 200)
            .expect("overwrite");
        let overwritten = cache
            .lookup(b"key", 121)
            .expect("overwrite lookup")
            .expect("hit");
        assert_eq!(overwritten.response[41..45], [192, 0, 2, 2]);
        assert_eq!(answer_ttl(&overwritten.response), 29);
        assert!(
            cache
                .lookup(b"key", 149)
                .expect("message expiry boundary predecessor")
                .is_some()
        );
        assert!(
            cache
                .lookup(b"key", 150)
                .expect("message expiry boundary")
                .is_none()
        );

        cache
            .store(b"cache-expiry", &replacement, &[], 200, 260, 205)
            .expect("cache expiry store");
        assert!(
            cache
                .lookup(b"cache-expiry", 204)
                .expect("cache expiry boundary predecessor")
                .is_some()
        );
        assert!(
            cache
                .lookup(b"cache-expiry", 205)
                .expect("cache expiry boundary")
                .is_none()
        );
    }

    #[test]
    fn native_cache_rejects_malformed_wire_and_keeps_capacity_bounded() {
        let cache = NativeCache::new(CONFIG).expect("cache");
        let packet = response(60, [192, 0, 2, 3]);
        let mut malformed = packet.clone();
        malformed.truncate(20);
        assert_eq!(
            cache.store(b"bad", &malformed, &[], 100, 160, 200),
            Err(super::CacheError::InvalidResponse)
        );

        for (key, address) in [
            (b"key-0", [192, 0, 2, 4]),
            (b"key-1", [192, 0, 2, 5]),
            (b"key-2", [192, 0, 2, 6]),
            (b"key-3", [192, 0, 2, 7]),
            (b"key-4", [192, 0, 2, 8]),
        ] {
            cache
                .store(key, &response(60, address), &[], 100, 160, 200)
                .expect("store");
        }
        assert!(cache.len() <= CONFIG.capacity);
    }
}
