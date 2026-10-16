//! Compact, lossless storage for terminal audit records.

use std::cmp::Ordering;
use std::collections::hash_map::RandomState;
use std::hash::{BuildHasher, Hash};
use std::net::SocketAddr;
use std::sync::{Arc, Weak};
use std::time::{Duration, SystemTime};

use super::{AuditRecord, ResponseDetails};

const SHARING_SLOTS: usize = 4096;

/// A fixed-size weak cache. Collisions replace only the cache hint; existing
/// records/snapshots keep their own strong references. It never owns strings
/// after their final record has been released.
struct SharingCache<T> {
    slots: Vec<Option<Weak<T>>>,
    hash: RandomState,
}

impl<T> Default for SharingCache<T> {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            hash: RandomState::new(),
        }
    }
}

impl<T: Hash + Eq> SharingCache<T> {
    fn share(&mut self, mut value: T, compact: impl FnOnce(&mut T)) -> Arc<T> {
        if self.slots.is_empty() {
            if self.slots.try_reserve_exact(SHARING_SLOTS).is_err() {
                // Sharing is optional: a recoverable hint-table allocation
                // failure must not become a DNS failure or lose this record.
                compact(&mut value);
                return Arc::new(value);
            }
            self.slots.resize_with(SHARING_SLOTS, || None);
        }
        let slot = usize::try_from(self.hash.hash_one(&value) % SHARING_SLOTS as u64)
            .expect("sharing slot fits usize");
        if let Some(existing) = self.slots[slot].as_ref().and_then(Weak::upgrade) {
            if *existing == value {
                return existing;
            }
        }
        compact(&mut value);
        let shared = Arc::new(value);
        self.slots[slot] = Some(Arc::downgrade(&shared));
        shared
    }
}

#[derive(Default)]
pub(super) struct AuditRetention {
    templates: SharingCache<AuditRecord>,
    responses: SharingCache<ResponseDetails>,
}

/// The original record is the shared template, with only its per-row fields
/// replaced by empty placeholders. Filters/ranks explicitly read common facts
/// without allocating a full record for every inspected row.
pub(crate) struct RetainedAuditRecord {
    pub timestamp: SystemTime,
    pub client_addr: SocketAddr,
    pub qname: Box<str>,
    pub trace_id: RetainedTraceId,
    pub elapsed: Duration,
    pub response_details: Arc<ResponseDetails>,
    template: Arc<AuditRecord>,
}

impl RetainedAuditRecord {
    /// Common facts only. Per-row fields must be read from the retained record;
    /// expansion is required before passing a full AuditRecord to a projector.
    pub(crate) fn common(&self) -> &AuditRecord {
        &self.template
    }

    pub(crate) fn expand(&self) -> AuditRecord {
        AuditRecord {
            timestamp: self.timestamp,
            client_addr: self.client_addr,
            qname: self.qname.to_string(),
            trace_id: self.trace_id.to_string(),
            elapsed: self.elapsed,
            response_details: self.response_details.as_ref().clone(),
            ..self.template.as_ref().clone()
        }
    }
}

impl AuditRetention {
    pub(super) fn retain(&mut self, mut record: AuditRecord) -> RetainedAuditRecord {
        let response =
            std::mem::replace(&mut record.response_details, ResponseDetails::no_response());
        RetainedAuditRecord {
            timestamp: std::mem::replace(&mut record.timestamp, SystemTime::UNIX_EPOCH),
            client_addr: std::mem::replace(&mut record.client_addr, ([0, 0, 0, 0], 0).into()),
            qname: std::mem::take(&mut record.qname).into_boxed_str(),
            trace_id: RetainedTraceId::from_string(std::mem::take(&mut record.trace_id)),
            elapsed: std::mem::replace(&mut record.elapsed, Duration::ZERO),
            response_details: self.responses.share(response, |response| {
                response.answers.shrink_to_fit();
                for answer in &mut response.answers {
                    answer.data.shrink_to_fit();
                }
                if let Some(error) = &mut response.answer_decode_error {
                    error.shrink_to_fit();
                }
            }),
            template: self.templates.share(record, |_| {}),
        }
    }
}

#[derive(Eq, PartialEq)]
pub(crate) enum RetainedTraceId {
    Native { nonce: u128, counter: u64 },
    Text(Box<str>),
}

impl RetainedTraceId {
    fn from_string(value: String) -> Self {
        let parsed = value.strip_prefix("n-").and_then(|text| {
            let (nonce, counter) = text.split_once('-')?;
            if nonce.len() != 32
                || counter.len() != 16
                || !nonce
                    .bytes()
                    .chain(counter.bytes())
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            {
                return None;
            }
            Some(Self::Native {
                nonce: u128::from_str_radix(nonce, 16).ok()?,
                counter: u64::from_str_radix(counter, 16).ok()?,
            })
        });
        parsed.unwrap_or_else(|| Self::Text(value.into_boxed_str()))
    }
}

impl std::fmt::Display for RetainedTraceId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Native { nonce, counter } => write!(formatter, "n-{nonce:032x}-{counter:016x}"),
            Self::Text(value) => formatter.write_str(value),
        }
    }
}

impl Ord for RetainedTraceId {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (
                Self::Native {
                    nonce: left,
                    counter: left_counter,
                },
                Self::Native {
                    nonce: right,
                    counter: right_counter,
                },
            ) => (left, left_counter).cmp(&(right, right_counter)),
            _ => self.to_string().cmp(&other.to_string()),
        }
    }
}

impl PartialOrd for RetainedTraceId {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[cfg(test)]
mod tests {
    use std::hash::Hash;
    use std::sync::Arc;

    use super::{RetainedTraceId, SHARING_SLOTS, SharingCache};

    #[derive(Eq, PartialEq, Debug)]
    struct Colliding(u64);

    impl Hash for Colliding {
        fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
            // Deliberately valid hash collisions exercise full equality.
            0_u8.hash(state);
        }
    }

    #[test]
    fn sharing_cache_collisions_never_change_live_records_or_retain_payloads() {
        let mut cache = SharingCache::default();
        let first = cache.share(Colliding(1), |_| {});
        let weak = Arc::downgrade(&first);
        let second = cache.share(Colliding(2), |_| {});
        assert_eq!(first.0, 1);
        assert_eq!(second.0, 2);
        let again = cache.share(Colliding(2), |_| {
            panic!("shared payload must not be compacted again")
        });
        assert!(Arc::ptr_eq(&second, &again));
        drop(first);
        assert!(weak.upgrade().is_none());
        for value in 3..10000 {
            assert_eq!(cache.share(Colliding(value), |_| {}).0, value);
        }
        assert_eq!(cache.slots.len(), SHARING_SLOTS);
        assert_eq!(second.0, 2);
    }

    #[test]
    fn trace_id_representation_preserves_all_bytes_and_lexical_order() {
        let mut values = vec![
            "n-11111111111111111111111111111111-0000000000000001".to_owned(),
            "n-11111111111111111111111111111111-ffffffffffffffff".to_owned(),
            "n-11111111111111111111111111111111-0000000000000000".to_owned(),
            "n-00000000000000000000000000000000-ffffffffffffffff".to_owned(),
            "n-ABCDEFABCDEFABCDEFABCDEFABCDEFAB-0000000000000001".to_owned(),
            "n-+1111111111111111111111111111111-0000000000000001".to_owned(),
            "legacy trace 零".to_owned(),
            String::new(),
        ];
        let mut compact: Vec<_> = values
            .iter()
            .cloned()
            .map(RetainedTraceId::from_string)
            .collect();
        assert_eq!(
            values,
            compact.iter().map(ToString::to_string).collect::<Vec<_>>()
        );
        values.sort();
        compact.sort();
        assert_eq!(
            values,
            compact.iter().map(ToString::to_string).collect::<Vec<_>>()
        );
    }

    #[test]
    fn old_generation_strings_live_only_until_the_last_snapshot_reference() {
        let mut cache = SharingCache::default();
        let old_record = cache.share("generation-one rule source".to_owned(), |_| {});
        let weak = Arc::downgrade(&old_record);
        let read_snapshot = Arc::clone(&old_record);
        for generation in 2..10000 {
            let _ = cache.share(format!("generation-{generation} rule source"), |_| {});
        }
        drop(old_record);
        assert!(weak.upgrade().is_some());
        drop(read_snapshot);
        assert!(weak.upgrade().is_none());
        assert_eq!(cache.slots.len(), SHARING_SLOTS);
    }
}
