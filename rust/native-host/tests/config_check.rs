use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use mosdns_native_host::{CheckFindingKind, check_config};

const GAPS: &str = "tests/fixtures/check/gaps/config.yaml";
const SUPPORTED: &str = "tests/fixtures/check/supported/config.yaml";

fn run_binary(args: &[&str]) -> (i32, String, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_mosdns"))
        .args(args)
        .output()
        .expect("run mosdns");
    (
        output.status.code().expect("exit code"),
        String::from_utf8(output.stdout).expect("utf-8 stdout"),
        String::from_utf8(output.stderr).expect("utf-8 stderr"),
    )
}

#[test]
fn supported_configuration_has_no_findings() {
    let report = check_config(Path::new(SUPPORTED)).expect("check runs");
    assert!(report.findings.is_empty(), "{report}");
    assert_eq!(report.definitions, 3);
    assert_eq!(report.exit_code(), 0);
}

#[test]
fn every_gap_is_listed_with_location_and_classification() {
    let report = check_config(Path::new(GAPS)).expect("check runs");
    let expected = format!(
        "\
GLOBAL {GAPS}:log.production  unsupported field
GLOBAL {GAPS}:log.encoding  unsupported field
GAP no_args domain_output {GAPS}:plugins[3].args  required field is missing
GAP light_set sd_set sub/rules.yaml:plugins[0].type  unsupported plugin type `sd_set`
GAP rewrite_x rewrite {GAPS}:plugins[1].type  unsupported plugin type `rewrite`
GAP two_bad_params cache {GAPS}:plugins[2].args.first_unknown  unsupported field
GAP udp_in udp_server {GAPS}:plugins[5].args.listen  only a numeric IP SocketAddr with a nonzero port is supported
CASCADE entry sequence {GAPS}:plugins[4].args[0].exec  references removed `rewrite_x`; unknown executable reference `$rewrite_x`
CASCADE entry sequence {GAPS}:plugins[4].args[1].matches[0]  references removed `light_set`; unknown domain_set reference `$light_set`
GAP entry sequence {GAPS}:plugins[4].args[1].matches[1]  unsupported matcher `qclass`
GAP entry sequence {GAPS}:plugins[4].args[1].exec  unsupported executable `debug_print`
CASCADE entry sequence {GAPS}:plugins[4].args[2].exec[0]  references removed `rewrite_x`; unknown executable reference `$rewrite_x`
GAP entry sequence {GAPS}:plugins[4].args[2].exec[1]  unsupported executable `black_hole`
CASCADE entry sequence {GAPS}:plugins[4].args[4].exec  references removed `no_args`; unknown executable reference `$no_args`
SKIPPED managed-store open/recovery and pending-transaction gate (start-only; writes state)
NOTE first-error-per-definition; rerun after fixing to see further gaps
NOTE sequences are checked per matcher/exec/rule item; the failing item is neutralized and checking continues
SUMMARY gaps=8 cascades=4 globals=2 definitions=8
"
    );
    assert_eq!(report.to_string(), expected);
    assert_eq!(report.exit_code(), 1);
}

#[test]
fn cascade_names_the_removed_definition() {
    let report = check_config(Path::new(GAPS)).expect("check runs");
    let cascades: Vec<_> = report
        .findings
        .iter()
        .filter_map(|finding| match &finding.kind {
            CheckFindingKind::Cascade { removed } => Some(removed.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(cascades, ["rewrite_x", "light_set", "rewrite_x", "no_args"]);
}

#[test]
fn only_the_first_unsupported_parameter_of_a_definition_is_reported() {
    let report = check_config(Path::new(GAPS)).expect("check runs");
    let cache: Vec<_> = report
        .findings
        .iter()
        .filter(|finding| finding.tag.as_deref() == Some("two_bad_params"))
        .collect();
    assert_eq!(cache.len(), 1);
    assert!(cache[0].field.ends_with("args.first_unknown"));
}

#[test]
fn a_reference_to_a_never_declared_tag_is_a_direct_gap() {
    let root = TempDir::new("undeclared");
    let config = root.path().join("config.yaml");
    std::fs::write(
        &config,
        "log:\n  level: error\nplugins:\n  - tag: fwd\n    type: forward\n    args:\n      \
         upstreams:\n        - addr: \"udp://127.0.0.1:15453\"\n  - tag: entry\n    type: \
         sequence\n    args:\n      - exec: $typo\n      - exec: $fwd\n  - tag: udp_in\n    \
         type: udp_server\n    args:\n      entry: entry\n      listen: \"127.0.0.1:15353\"\n      \
         enable_audit: false\n",
    )
    .unwrap();
    let report = check_config(&config).expect("check runs");
    assert_eq!(report.findings.len(), 1, "{report}");
    assert_eq!(report.findings[0].kind, CheckFindingKind::Gap);
    assert_eq!(report.findings[0].field, "plugins[1].args[0].exec");
}

#[test]
fn every_item_of_a_long_exec_list_is_checked() {
    let root = TempDir::new("long-exec");
    let config = root.path().join("config.yaml");
    let items = "debug_print, ".repeat(12);
    std::fs::write(
        &config,
        format!(
            "log:\n  level: error\nplugins:\n  - tag: fwd\n    type: forward\n    args:\n      \
             upstreams:\n        - addr: \"udp://127.0.0.1:15453\"\n  - tag: entry\n    \
             type: sequence\n    args:\n      - exec: [{items}$fwd]\n  - tag: udp_in\n    \
             type: udp_server\n    args:\n      entry: entry\n      listen: \
             \"127.0.0.1:15353\"\n      enable_audit: false\n"
        ),
    )
    .unwrap();
    let report = check_config(&config).expect("check runs");
    let fields: Vec<_> = report.findings.iter().map(|f| f.field.as_str()).collect();
    let expected: Vec<_> = (0..12)
        .map(|index| format!("plugins[1].args[0].exec[{index}]"))
        .collect();
    assert_eq!(fields, expected, "{report}");
    assert!(
        report
            .findings
            .iter()
            .all(|finding| finding.kind == CheckFindingKind::Gap)
    );
}

#[test]
fn unreadable_or_unparsable_configuration_is_an_error() {
    for path in [
        "tests/fixtures/check/yaml_error/config.yaml",
        "tests/fixtures/check/missing_include/config.yaml",
        "tests/fixtures/check/absent/config.yaml",
    ] {
        assert!(check_config(Path::new(path)).is_err(), "{path}");
    }
}

#[test]
fn parsed_structural_rejections_are_findings_not_errors() {
    let valid = "log:\n  level: error\n";
    let cases: [(&str, Option<&str>, &str); 5] = [
        (
            "- not a mapping\n",
            None,
            "config.yaml:  top level must be a mapping",
        ),
        (
            "log:\n  level: error\ninclude: sub.yaml\nplugins: []\n",
            None,
            "config.yaml:include  expected a sequence",
        ),
        (
            "log:\n  level: error\ninclude:\n  - [1]\nplugins: []\n",
            None,
            "config.yaml:include[0]  expected a string",
        ),
        (
            valid,
            None,
            "config.yaml:plugins  required field is missing",
        ),
        (
            "log:\n  level: error\ninclude:\n  - sub.yaml\nplugins: []\n",
            Some("plugins: []\nextra: 1\n"),
            "GLOBAL sub.yaml:extra  unsupported field",
        ),
    ];
    for (index, (config, include, expected)) in cases.iter().enumerate() {
        let root = TempDir::new(&format!("structural-{index}"));
        std::fs::write(root.path().join("config.yaml"), config).unwrap();
        if let Some(include) = include {
            std::fs::write(root.path().join("sub.yaml"), include).unwrap();
        }
        let output = Command::new(env!("CARGO_BIN_EXE_mosdns"))
            .args(["check", "-c", "config.yaml"])
            .current_dir(root.path())
            .output()
            .expect("run mosdns");
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert_eq!(output.status.code(), Some(1), "case {index}: {stdout}");
        assert!(stdout.contains(expected), "case {index}: {stdout}");
        assert!(stdout.contains("\nSKIPPED "), "case {index}: {stdout}");
        assert!(stdout.contains("\nSUMMARY "), "case {index}: {stdout}");
        assert!(output.stderr.is_empty(), "case {index}");
    }
}

#[test]
fn binary_exit_codes_and_both_config_spellings() {
    let (code, stdout, _) = run_binary(&["check", "-c", SUPPORTED]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("SUMMARY gaps=0 cascades=0 globals=0 definitions=3"));

    let (code, stdout, _) = run_binary(&["check", "--config", GAPS]);
    assert_eq!(code, 1);
    assert!(stdout.contains("SUMMARY gaps=8 cascades=4 globals=2 definitions=8"));

    let (code, stdout, stderr) =
        run_binary(&["check", "-c", "tests/fixtures/check/yaml_error/config.yaml"]);
    assert_eq!(code, 2);
    assert!(stdout.is_empty());
    assert!(stderr.contains("invalid YAML"), "{stderr}");

    let (code, _, stderr) = run_binary(&[
        "check",
        "--config",
        "tests/fixtures/check/missing_include/config.yaml",
    ]);
    assert_eq!(code, 2);
    assert!(stderr.contains("cannot read included file"), "{stderr}");

    let (code, _, stderr) = run_binary(&["check", "-c"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("usage:"), "{stderr}");
}

#[test]
fn start_still_fails_fast_on_the_first_rejection() {
    let (code, stdout, stderr) = run_binary(&["start", "-c", GAPS]);
    assert_eq!(code, 2);
    assert!(stdout.is_empty());
    assert_eq!(stderr, "mosdns: $.log.production: unsupported field\n");
}

/// Recursive content and metadata snapshot (atime excluded).
fn snapshot(root: &Path) -> BTreeMap<PathBuf, (bool, u64, std::time::SystemTime, u32, Vec<u8>)> {
    use std::os::unix::fs::PermissionsExt;
    let mut entries = BTreeMap::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        let content = if metadata.is_file() {
            std::fs::read(&path).unwrap()
        } else {
            Vec::new()
        };
        if metadata.is_dir() {
            for entry in std::fs::read_dir(&path).unwrap() {
                pending.push(entry.unwrap().path());
            }
        }
        entries.insert(
            path,
            (
                metadata.is_dir(),
                metadata.len(),
                metadata.modified().unwrap(),
                metadata.permissions().mode(),
                content,
            ),
        );
    }
    entries
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn check_has_no_side_effects() {
    let root = TempDir::new("side-effects");
    let dns_port = free_port();
    let api_port = free_port();
    let template =
        std::fs::read_to_string("tests/fixtures/check/side_effects/config.yaml").unwrap();
    let config = root.path().join("config.yaml");
    std::fs::write(
        &config,
        template
            .replace("{DNS_PORT}", &dns_port.to_string())
            .replace("{API_PORT}", &api_port.to_string()),
    )
    .unwrap();
    // Existing state and dump artifacts must stay byte- and mtime-identical.
    std::fs::create_dir_all(root.path().join("state")).unwrap();
    std::fs::write(root.path().join("state/switch1.txt"), "B").unwrap();
    std::fs::create_dir_all(root.path().join("cache")).unwrap();
    std::fs::write(root.path().join("cache/cache.dump"), b"opaque dump").unwrap();
    let before = snapshot(root.path());

    let report = check_config(&config).expect("check runs");
    assert!(report.findings.is_empty(), "{report}");
    let (code, _, _) = run_binary(&["check", "-c", config.to_str().unwrap()]);
    assert_eq!(code, 0);

    assert_eq!(snapshot(root.path()), before);
    assert!(!root.path().join("webinfo").exists(), "webinfo was created");
    let dns = format!("127.0.0.1:{dns_port}");
    std::net::UdpSocket::bind(&dns).expect("dns port is free after check");
    std::net::TcpListener::bind(&dns).expect("dns tcp port is free after check");
    std::net::TcpListener::bind(format!("127.0.0.1:{api_port}"))
        .expect("api port is free after check");
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("config-check-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
