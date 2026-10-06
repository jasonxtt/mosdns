//! Read-only artifact inspection for controlled validation. No resources bind.
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: inspect_managed_config CONFIG")?;
    let config = mosdns_native_host::load_and_compile(Path::new(&path))?;
    let profile = config
        .managed_profile
        .as_ref()
        .ok_or("native management is disabled")?;
    println!(
        "{}",
        serde_json::json!({
            "generated_yaml":profile.generated_yaml,
            "sha256":profile.generated_sha256,
            "listener_count":config.listeners.len(),
            "group_count":profile.groups.len(),
            "router_sequence":config.managed_router.as_ref().map(|router| router.sequence.index())
        })
    );
    Ok(())
}
