//! Product build identity shared by the standalone CLI and HTTP health.
pub const VERSION: &str = match option_env!("MOSDNS_BUILD_VERSION") {
    Some(version) => version,
    None => "dev",
};
