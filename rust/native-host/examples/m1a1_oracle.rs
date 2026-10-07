//! Differential harness for the M1-a1 rule providers: answers the same JSON
//! line protocol as `tools/m1a1-oracle` (Go) from the native compiler.
//!
//! `m1a1_oracle --root <dir> --config <file> --in cases.jsonl --out answers.jsonl`
//! compiles `<file>` with `<dir>` as the configuration root (as `-d`).
//! Not a product command; used only by the validation records.

use std::io::{BufRead, BufWriter, Write};
use std::path::PathBuf;

use mosdns_native_host::{CompiledConfig, MapperOutcome, compile_yaml_with_roots};
use serde_json::{Value, json};

fn wire(name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for label in name.trim_end_matches('.').split('.') {
        if label.is_empty() {
            continue;
        }
        out.push(u8::try_from(label.len()).unwrap_or(255));
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    out
}

fn answer(config: &CompiledConfig, request: &Value) -> Value {
    let tag = request["tag"].as_str().unwrap_or_default();
    match request["op"].as_str().unwrap_or_default() {
        "map" => {
            let mapper = config
                .domain_mappers
                .iter()
                .find(|mapper| mapper.tag == tag)
                .expect("mapper tag");
            let MapperOutcome {
                fast_flags,
                ctx_marks,
                domain_set,
                matched_rule_source,
                matched,
            } = mapper.evaluate_wire(&wire(request["name"].as_str().unwrap_or_default()));
            let flags: Vec<u32> = (0..64)
                .filter(|bit| fast_flags & (1u64 << bit) != 0)
                .collect();
            json!({"flags": flags, "marks": ctx_marks, "domain_set": domain_set, "source": matched_rule_source, "matched": matched})
        }
        "count" => {
            let mapper = config
                .domain_mappers
                .iter()
                .find(|mapper| mapper.tag == tag)
                .expect("mapper tag");
            json!({"rules": mapper.rule_count()})
        }
        "ip" => {
            let set = config
                .ip_sets
                .iter()
                .find(|set| set.tag == tag)
                .expect("ip set tag");
            let addr = request["addr"]
                .as_str()
                .unwrap_or_default()
                .parse()
                .expect("address");
            json!({"match": set.prefixes.contains(addr)})
        }
        "regexp" => {
            let pattern = request["pattern"].as_str().unwrap_or_default();
            let names: Vec<&str> = request["names"]
                .as_array()
                .map(|names| names.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            let mut matcher = mosdns_matcher_core::RegexMatcher::new();
            match matcher.add(pattern) {
                Ok(()) => json!({"valid": true, "class": "ok",
                    "matches": names.iter().map(|name| matcher.is_match(name)).collect::<Vec<_>>()}),
                Err(error) if error.is_invalid_in_go() => {
                    json!({"valid": false, "class": "invalid"})
                }
                Err(_) => json!({"valid": false, "class": "unsupported"}),
            }
        }
        other => panic!("unknown op {other}"),
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let (mut root, mut config, mut input, mut output) = (None, None, None, None);
    while let Some(flag) = args.next() {
        let value = args.next().expect("flag value");
        match flag.as_str() {
            "--root" => root = Some(PathBuf::from(value)),
            "--config" => config = Some(PathBuf::from(value)),
            "--in" => input = Some(PathBuf::from(value)),
            "--out" => output = Some(PathBuf::from(value)),
            other => panic!("unknown flag {other}"),
        }
    }
    let root = root.expect("--root");
    let config_path = config.expect("--config");
    let yaml = std::fs::read_to_string(&config_path).expect("read config");
    let base = config_path.parent().expect("config dir");
    let started = std::time::Instant::now();
    let compiled = compile_yaml_with_roots(&yaml, base, &root).unwrap_or_else(|error| {
        panic!("compile failed: {error}");
    });
    eprintln!("compiled in {:?}", started.elapsed());
    if let Ok(status) = std::fs::read_to_string("/proc/self/status") {
        for line in status
            .lines()
            .filter(|l| l.starts_with("VmRSS") || l.starts_with("VmHWM"))
        {
            eprintln!("{line}");
        }
    }
    let input = std::io::BufReader::new(std::fs::File::open(input.expect("--in")).expect("open"));
    let mut output = BufWriter::new(std::fs::File::create(output.expect("--out")).expect("create"));
    for line in input.lines() {
        let line = line.expect("line");
        let request: Value = serde_json::from_str(&line).expect("json");
        writeln!(output, "{}", answer(&compiled, &request)).expect("write");
    }
}
