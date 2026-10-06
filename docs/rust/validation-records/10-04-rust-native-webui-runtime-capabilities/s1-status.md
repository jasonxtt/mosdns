# S1 — shared identity and readiness

Inline authorized implementation on rust, preserving inherited dirty source. CLI version branches before config/runtime construction. Shared product identity is MOSDNS_BUILD_VERSION or dev; Cargo tracks environment changes. Supervisor start and RuntimeControl admission/recovery/shutdown provide one coherent health state; health bypasses ordinary admission only for the exact read-only route and never resumes or shuts down the host.

VM: /root/mosdns-rust-webui-20261004/source on SSH mosdns-rust. Real HTTP test covers starting/ready/applying/reopen/stopping/recovery_required and nullable schemas, methods and probe side-effect exclusion. Real supervisor HTTP test confirms shared version, ready and listener closure; existing failed-bind/rebind/fault tests retained. CLI integration proves config-independent version and extra-argument refusal. Default dev and injected s1-webui-proof both tested.

Validation: lifecycle 1 PASS; CLI 1 PASS; scoped HTTP regression 11 PASS; fmt and native-host all-target strict Clippy PASS. Initial missing-health and missing-version red tests and first Clippy format-argument failure retained in evidence. No performance/full-migration claims. Exact local/VM file hashes in s1-tested-source.json.

C2C exact-source review pending. No S2 code begun, ordinary commit, push, deployment, default switch or archive.
