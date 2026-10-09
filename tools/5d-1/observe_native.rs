//! Supplementary observer launcher; not the candidate performance binary.
//! Copy into native-host/examples/ in the disposable VM source and build offline
//! with the same release target/env. Uses the existing public host API only.
use std::io::Write;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mosdns_native_host::CliCommand::Start { config, dir } =
        mosdns_native_host::parse_args(std::env::args_os())?
    else {
        return Err("observer launcher requires start".into());
    };
    mosdns_native_host::configure_process_memory();
    let (config, root) = mosdns_native_host::resolve_config_paths(&config, dir.as_deref())?;
    let host = mosdns_native_host::HostAssembly::from_config_file_in(&config, root.as_deref())?;
    let mut output = std::fs::File::create(std::env::var("FIVE_D_OBSERVE_PATH")?)?;
    let began = std::time::Instant::now();
    host.block_on(async {
        let serving = host.serve_host();
        tokio::pin!(serving);
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let terminal = tokio::select! {
                result = &mut serving => Some(result),
                _ = interval.tick() => None,
            };
            let metrics = host.metrics_snapshot();
            writeln!(output, "{}", serde_json::json!({
                "elapsed_s": began.elapsed().as_secs_f64(),
                "epoch_s": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs_f64(),
                "in_flight": metrics.in_flight,
                "admitted": metrics.admitted_total,
                "completed": metrics.completed_total,
                "after_shutdown": terminal.is_some()
            }))?;
            output.flush()?;
            if let Some(result) = terminal {
                result?;
                return Ok::<_, Box<dyn std::error::Error>>(());
            }
        }
    })
}
