fn main() {
    let command = mosdns_native_host::parse_args(std::env::args_os());
    let result = match command {
        Ok(mosdns_native_host::CliCommand::Version) => {
            println!("{}", mosdns_native_host::build_identity::VERSION);
            return;
        }
        Ok(mosdns_native_host::CliCommand::Start { config }) => {
            mosdns_native_host::HostAssembly::from_config_file(&config)
                .map_err(|error| error.to_string())
                .and_then(|assembly| assembly.run().map_err(|error| error.to_string()))
        }
        Err(error) => Err(error.to_string()),
    };
    if let Err(error) = result {
        eprintln!("mosdns: {error}");
        std::process::exit(2);
    }
}
