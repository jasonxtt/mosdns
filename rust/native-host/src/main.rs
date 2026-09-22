fn main() {
    if let Err(error) = mosdns_native_host::prepare_from_args(std::env::args_os()) {
        eprintln!("mosdns: {error}");
        std::process::exit(2);
    }
}
