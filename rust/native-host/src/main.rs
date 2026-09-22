fn main() {
    match mosdns_native_host::prepare_from_args(std::env::args_os()) {
        Ok(assembly) => {
            if let Err(error) = assembly.run() {
                eprintln!("mosdns: {error}");
                std::process::exit(2);
            }
        }
        Err(error) => {
            eprintln!("mosdns: {error}");
            std::process::exit(2);
        }
    }
}
