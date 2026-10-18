//! Product build identity shared by the standalone CLI and HTTP health.
//!
//! A release is built with `MOSDNS_BUILD_VERSION=rust-vX.Y.Z` (the release tag); any other
//! build reports the crate version in the same form, so every build has a parseable version.
pub const VERSION: &str = match option_env!("MOSDNS_BUILD_VERSION") {
    Some(version) => version,
    None => concat!("rust-v", env!("CARGO_PKG_VERSION")),
};
