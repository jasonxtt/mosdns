# S4 validation — configured-tag HTTP and capability inventory

Status: implementation green; cumulative C2C delta review PASS.

The real `slice6_management_http` integration test exercises a configured tag
containing a space, URI decoding, exact show/post bodies, JSON `value`, legacy
`Value`, empty form values, case-insensitive media types, UTF-8 charset, invalid
JSON/form values, unsupported media, malformed/stale generation headers, 405,
404, and a request over the 1 MiB bound returning 413 without changing the
state file. Capability discovery asserts the schema-1 type/tag/read/write
inventory and generation string.

Observed focused result:

```text
cargo test --manifest-path rust/Cargo.toml -p mosdns-native-host --test slice6_management_http
14 passed, 0 failed
```

Generation preconditions are now parsed only after a configured native switch
route is identified; unrelated plugin and non-plugin requests retain their
normal route semantics even with malformed generation headers.

The broader native-host library suite also covers state-file admission,
durable replacement, owner serialization, query snapshot capture, runtime
rebind, disconnect/shutdown draining, and crash-boundary recovery; see S6 for
the ordered run and its environment caveat. The final post-drain delta review
returned `FINAL: PASS` on BASE `115b62cd19e86957b85200ba30fc70f72add8a70` to
HEAD `22d15b20e1314f655c43a8d9affae277e15653d3`, TREE
`f11c8267aad09e96c824393c3ced96685a561757`, covering 9 paths and 15,043 bytes
in the fresh C2C conversation
https://chatgpt.com/c/6ac45d74-daf4-83ee-8b11-4d70ae7db541.
