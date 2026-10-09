//! `mosdns-settings-migrate --from <go-config-dir> [-d <root>] -c <config> --out <dir>`:
//! one-time migration of the reference host's override files (M1-f2).

use std::path::PathBuf;

const USAGE: &str = "usage: mosdns-settings-migrate --from <reference-config-dir> [-d <native-root>] -c <native-config> --out <output-dir>\n\
writes <output-dir>/settings.json and migration-report.txt; never changes the inputs.\n\
exit code: 0 everything mapped, 1 some item needs a person (see the report), 2 error";

fn main() {
    let mut from = None;
    let mut config = None;
    let mut dir = None;
    let mut out = None;
    let mut args = std::env::args_os().skip(1);
    while let Some(flag) = args.next() {
        let slot = match flag.to_str() {
            Some("--from") => &mut from,
            Some("-c" | "--config") => &mut config,
            Some("-d" | "--dir") => &mut dir,
            Some("--out") => &mut out,
            _ => {
                eprintln!("{USAGE}");
                std::process::exit(2);
            }
        };
        let Some(value) = args.next() else {
            eprintln!("{USAGE}");
            std::process::exit(2);
        };
        *slot = Some(PathBuf::from(value));
    }
    let (Some(from), Some(config), Some(out)) = (from, config, out) else {
        eprintln!("{USAGE}");
        std::process::exit(2);
    };
    let options = mosdns_native_host::settings_migrate::Options {
        from,
        config,
        dir,
        out,
    };
    match mosdns_native_host::settings_migrate::run(&options) {
        Ok(outcome) => {
            print!("{}", outcome.report);
            std::process::exit(outcome.exit_code);
        }
        Err(error) => {
            eprintln!("mosdns-settings-migrate: {error}");
            std::process::exit(2);
        }
    }
}
