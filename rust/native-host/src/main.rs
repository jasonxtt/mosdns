fn main() {
    let command = mosdns_native_host::parse_args(std::env::args_os());
    let result = match command {
        Ok(mosdns_native_host::CliCommand::Version) => {
            println!("{}", mosdns_native_host::build_identity::VERSION);
            return;
        }
        Ok(mosdns_native_host::CliCommand::Check { config, dir }) => {
            match mosdns_native_host::resolve_config_paths(&config, dir.as_deref())
                .map_err(|error| error.to_string())
                .and_then(|(config, root)| {
                    mosdns_native_host::check_config_in(&config, root.as_deref())
                        .map_err(|error| error.to_string())
                }) {
                Ok(report) => {
                    print!("{report}");
                    std::process::exit(report.exit_code());
                }
                Err(error) => Err(error),
            }
        }
        Ok(mosdns_native_host::CliCommand::Start { config, dir }) => {
            // Before anything can replace the executable on disk.
            mosdns_native_host::capture_restart_target();
            mosdns_native_host::configure_process_memory();
            serve(&config, dir.as_deref())
        }
        Err(error) => Err(error.to_string()),
    };
    if let Err(error) = result {
        eprintln!("mosdns: {error}");
        mosdns_native_host::record_fatal(&error);
        std::process::exit(2);
    }
}

/// Runs the host until it is told to stop. A restart requested through
/// `POST /api/v1/system/restart` ends the run exactly like `SIGTERM` (drained, saved,
/// closed) and then replaces this process image with the same executable, keeping the
/// process id, so the service manager never sees the service stop. If the replacement
/// fails, the host is started again in this process and keeps serving.
fn serve(config: &std::path::Path, dir: Option<&std::path::Path>) -> Result<(), String> {
    loop {
        let (config_path, root) = mosdns_native_host::resolve_config_paths(config, dir)
            .map_err(|error| error.to_string())?;
        let assembly =
            mosdns_native_host::HostAssembly::from_config_file_in(&config_path, root.as_deref())
                .map_err(|error| error.to_string())?;
        let control = assembly.control();
        let result = assembly.run().map_err(|error| error.to_string());
        let restart = control.restart_requested();
        // Ends the runtime and every thread it owns before the process image is replaced.
        drop(control);
        drop(assembly);
        if !restart {
            return result;
        }
        let Some(target) = mosdns_native_host::restart_target() else {
            return result;
        };
        // A failed final save (a cache dump that cannot be written, say) must not turn a
        // requested restart into an exit that leaves the service down for the service
        // manager's restart delay: it is reported and the restart goes ahead.
        if let Err(error) = &result {
            eprintln!("mosdns: {error} (restarting anyway)");
        }
        eprintln!("mosdns: restarting in place");
        let error = target.exec();
        eprintln!("mosdns: restart failed ({error}); starting again in this process");
    }
}
