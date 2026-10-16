//! Diagnostic launcher only; never use this binary for acceptance timing.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mosdns_native_host::CliCommand::Start { config, dir } =
        mosdns_native_host::parse_args(std::env::args_os())?
    else {
        return Err("memory launcher requires start".into());
    };
    mosdns_native_host::configure_process_memory();
    let (config, root) = mosdns_native_host::resolve_config_paths(&config, dir.as_deref())?;
    let host = mosdns_native_host::HostAssembly::from_config_file_in(&config, root.as_deref())?;
    host.block_on(host.serve_host())?;
    // The diagnostic-only BASE observer hook measures original allocations
    // before the public snapshot deep-clones them.
    let _ = host.audit_snapshot();
    Ok(())
}
