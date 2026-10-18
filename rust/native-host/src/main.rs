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
        Ok(mosdns_native_host::CliCommand::UpdateGuard { transaction }) => {
            // Replaces this process; reaching the end means even the final exec failed.
            Err(mosdns_native_host::run_update_guard(&transaction))
        }
        Ok(mosdns_native_host::CliCommand::Start { config, dir }) => {
            // Before anything can replace the executable on disk.
            mosdns_native_host::capture_restart_target();
            // A candidate the update guard started dies with the guard.
            mosdns_native_host::arm_update_candidate();
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
    recover_interrupted_update(config, dir)?;
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
        // An update hands over to its guard instead of this program again.
        let pending = mosdns_native_host::take_pending();
        let Some(target) = pending.as_ref().or(mosdns_native_host::restart_target()) else {
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
        mosdns_native_host::abandon_failed_update_exec(target, &error.to_string());
    }
}

/// Before the configuration is loaded: an update that was interrupted (power loss, a killed
/// guard) is rolled back to the old program and configuration, and the old program is run.
fn recover_interrupted_update(
    config: &std::path::Path,
    dir: Option<&std::path::Path>,
) -> Result<(), String> {
    let (config_path, root) =
        mosdns_native_host::resolve_config_paths(config, dir).map_err(|error| error.to_string())?;
    let root = root.unwrap_or_else(|| {
        config_path
            .parent()
            .filter(|path| !path.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."))
            .to_path_buf()
    });
    let recovered = mosdns_native_host::recover_interrupted_update(&root)
        .map_err(|error| format!("update recovery failed: {error}"))?;
    if let Some((executable, args)) = recovered {
        eprintln!("mosdns: an interrupted update was rolled back; starting the previous version");
        let mut command = std::process::Command::new(&executable);
        if let Some(argv0) = args.first() {
            std::os::unix::process::CommandExt::arg0(&mut command, argv0);
        }
        let error = std::os::unix::process::CommandExt::exec(command.args(args.iter().skip(1)));
        return Err(format!(
            "cannot start the previous version after the rollback: {error}"
        ));
    }
    Ok(())
}
