#ifndef MOSDNS_CACHE_CORE_H
#define MOSDNS_CACHE_CORE_H

#include <stdint.h>
#include <stdbool.h>

#ifdef __cplusplus
extern "C" {
#endif

#define MOSDNS_CACHE_ABI_VERSION 1u
#define MOSDNS_CACHE_CAPABILITY_LIFECYCLE (UINT64_C(1) << 0)
#define MOSDNS_CACHE_CAPABILITY_CACHE (UINT64_C(1) << 1)
#define MOSDNS_CACHE_CAPABILITY_LOOKUP_INTO (UINT64_C(1) << 2)
#define MOSDNS_CACHE_CAPABILITY_MATCHER (UINT64_C(1) << 3)
#define MOSDNS_CACHE_CAPABILITY_VALUED_MATCHER (UINT64_C(1) << 4)
#define MOSDNS_QUERY_ABI_VERSION 1u
#define MOSDNS_QUERY_RESULT_VERSION 1u
#define MOSDNS_QUERY_CAPABILITY_SNAPSHOT (UINT64_C(1) << 5)
#define MOSDNS_QUERY_CAPABILITY_INSPECT (UINT64_C(1) << 6)
#define MOSDNS_QUERY_TRANSPORT_UDP 0u
#define MOSDNS_QUERY_TRANSPORT_STREAM 1u
#define MOSDNS_QUERY_TRANSPORT_HTTP 2u
#define MOSDNS_VALUED_RULE_BATCH_VERSION 1u
#define MOSDNS_VALUED_RESULT_VERSION 1u

typedef enum MosdnsCacheStatus {
  MOSDNS_CACHE_OK = 0,
  MOSDNS_CACHE_INVALID_ARGUMENT = 1,
  MOSDNS_CACHE_CLOSED = 2,
  MOSDNS_CACHE_PANIC = 3,
  MOSDNS_CACHE_INTERNAL = 4,
  MOSDNS_CACHE_BUFFER_TOO_SMALL = 5,
} MosdnsCacheStatus;

typedef struct MosdnsCacheConfig {
  uint64_t capacity;
  uint32_t lazy_cache_ttl_secs;
  uint32_t flags;
} MosdnsCacheConfig;

typedef struct MosdnsCacheOwnedBuffer {
  uint8_t *ptr;
  uint64_t len;
} MosdnsCacheOwnedBuffer;

typedef struct MosdnsCacheBorrowedSlice {
  const uint8_t *ptr;
  uint64_t len;
} MosdnsCacheBorrowedSlice;

typedef struct MosdnsCacheWritableSlice {
  uint8_t *ptr;
  uint64_t len;
} MosdnsCacheWritableSlice;

typedef struct MosdnsValuedMatchResult {
  MosdnsCacheStatus status;
  uint32_t matched;
  uint64_t required_len;
} MosdnsValuedMatchResult;

typedef enum MosdnsCacheLookupState {
  MOSDNS_CACHE_MISS = 0,
  MOSDNS_CACHE_FRESH = 1,
  MOSDNS_CACHE_LAZY = 2,
} MosdnsCacheLookupState;

typedef struct MosdnsCacheLookupResult {
  MosdnsCacheStatus status;
  int32_t state;
  int64_t stored_at_unix;
  int64_t message_expires_at_unix;
  MosdnsCacheOwnedBuffer response;
  MosdnsCacheOwnedBuffer domain_set;
} MosdnsCacheLookupResult;

typedef struct MosdnsCacheLookupIntoResult {
  MosdnsCacheStatus status;
  int32_t state;
  int64_t stored_at_unix;
  int64_t message_expires_at_unix;
  uint64_t response_len;
  uint64_t domain_set_len;
} MosdnsCacheLookupIntoResult;

typedef struct MosdnsQuerySnapshotInput {
  uint32_t struct_size;
  uint32_t version;
  uint32_t flags;
  uint32_t reserved;
  MosdnsCacheBorrowedSlice query_wire;
  uint8_t from_udp;
  uint8_t transport_mode;
  uint16_t advertised_udp_size;
  uint32_t reserved_tail;
  uint64_t pre_fast_flags;
} MosdnsQuerySnapshotInput;

typedef struct MosdnsQueryInspectResult {
  MosdnsCacheStatus status;
  uint32_t version;
  uint32_t flags;
  uint16_t id;
  uint16_t qtype;
  uint16_t qclass;
  uint16_t advertised_udp_size;
  uint16_t edns_udp_size;
  uint16_t ecs_family;
  uint8_t ecs_source_netmask;
  uint8_t ecs_source_scope;
  uint8_t from_udp;
  uint8_t transport_mode;
  uint8_t has_opt;
  uint8_t do_bit;
  uint8_t ecs_present;
  uint8_t reserved;
  uint64_t qname_len;
  uint64_t pre_fast_flags;
  uint64_t required_len;
  uint64_t written_len;
  uint8_t ecs_address[16];
} MosdnsQueryInspectResult;

uint32_t cache_abi_version(void);
uint64_t cache_abi_capabilities(void);
uint32_t query_abi_version(void);
uint64_t query_abi_capabilities(void);
MosdnsCacheStatus query_snapshot_create(MosdnsQuerySnapshotInput input,
                                         uint64_t *out_handle);
MosdnsCacheStatus query_snapshot_required_len(uint64_t handle,
                                               uint64_t *out_len);
MosdnsCacheStatus query_snapshot_inspect(uint64_t handle,
                                         MosdnsCacheWritableSlice output,
                                         MosdnsQueryInspectResult *out_result);
MosdnsCacheStatus query_snapshot_close(uint64_t handle);
MosdnsCacheStatus cache_create(const MosdnsCacheConfig *config,
                               uint64_t *out_handle);
MosdnsCacheStatus cache_close(uint64_t handle);
MosdnsCacheStatus cache_len(uint64_t handle, uint64_t *out_len);
MosdnsCacheStatus cache_store(uint64_t handle,
                              MosdnsCacheBorrowedSlice key,
                              MosdnsCacheBorrowedSlice response,
                              MosdnsCacheBorrowedSlice domain_set,
                              int64_t stored_at_unix,
                              int64_t message_expires_at_unix,
                              int64_t cache_expires_at_unix);
MosdnsCacheStatus cache_lookup(uint64_t handle,
                               MosdnsCacheBorrowedSlice key,
                               int64_t now_unix,
                               MosdnsCacheLookupResult *out_result);
MosdnsCacheStatus cache_lookup_into(uint64_t handle,
                                    MosdnsCacheBorrowedSlice key,
                                    int64_t now_unix,
                                    MosdnsCacheWritableSlice response_out,
                                    MosdnsCacheWritableSlice domain_set_out,
                                    MosdnsCacheLookupIntoResult *out_result);
MosdnsCacheStatus cache_flush(uint64_t handle);
MosdnsCacheStatus cache_buffer_release(MosdnsCacheOwnedBuffer buffer);

// --- Domain matcher API ---
MosdnsCacheStatus domain_matcher_create(MosdnsCacheBorrowedSlice rules,
                                        uint32_t default_type,
                                        uint64_t *out_handle);
MosdnsCacheStatus domain_matcher_match(uint64_t handle,
                                       MosdnsCacheBorrowedSlice domain,
                                       bool *out_match);
MosdnsCacheStatus domain_matcher_len(uint64_t handle, uint64_t *out_len);
MosdnsCacheStatus domain_matcher_close(uint64_t handle);

// --- IP matcher API ---
MosdnsCacheStatus ip_matcher_create(MosdnsCacheBorrowedSlice data,
                                    uint64_t *out_handle);
MosdnsCacheStatus ip_matcher_match(uint64_t handle,
                                   MosdnsCacheBorrowedSlice addr,
                                   bool *out_match);
MosdnsCacheStatus ip_matcher_len(uint64_t handle, uint64_t *out_len);
MosdnsCacheStatus ip_matcher_close(uint64_t handle);

// --- Valued domain matcher API ---
// The rule batch starts with one version byte and a little-endian uint32 rule
// count. Each rule is length-prefixed UTF-8 text, a uint64 fast-mark mask, a
// uint32 context-mark count followed by uint32 marks, then length-prefixed tag
// and source strings. The result starts with one version byte, a uint64 mask,
// a uint32 context-mark count followed by uint32 marks, then length-prefixed
// tag/source strings.
// Results are written only into caller-owned output storage.
MosdnsCacheStatus valued_domain_matcher_create(MosdnsCacheBorrowedSlice rules,
                                               uint64_t *out_handle);
MosdnsCacheStatus valued_domain_matcher_match(uint64_t handle,
                                              MosdnsCacheBorrowedSlice domain,
                                              MosdnsCacheWritableSlice output,
                                              MosdnsValuedMatchResult *out_result);
MosdnsCacheStatus valued_domain_matcher_len(uint64_t handle, uint64_t *out_len);
MosdnsCacheStatus valued_domain_matcher_close(uint64_t handle);

#ifdef __cplusplus
}
#endif

#endif
