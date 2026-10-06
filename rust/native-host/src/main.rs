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
            mosdns_native_host::resolve_config_paths(&config, dir.as_deref())
                .map_err(|error| error.to_string())
                .and_then(|(config, root)| {
                    mosdns_native_host::HostAssembly::from_config_file_in(&config, root.as_deref())
                        .map_err(|error| error.to_string())
                })
                .and_then(|assembly| assembly.run().map_err(|error| error.to_string()))
        }
        Err(error) => Err(error.to_string()),
    };
    if let Err(error) = result {
        eprintln!("mosdns: {error}");
        mosdns_native_host::record_fatal(&error);
        std::process::exit(2);
    }
}
