//! M1-f2: the one-time migration of the reference host's override files to the
//! native `webinfo/settings.json`. Runs the tool's library entry on a small
//! native configuration and hand-written reference files.

use std::path::{Path, PathBuf};

use mosdns_native_host::settings_migrate::{Options, run};

struct Tree(PathBuf);

impl Tree {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("m1-f2-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("native")).unwrap();
        std::fs::create_dir_all(root.join("go/webinfo")).unwrap();
        std::fs::write(root.join("native/config.yaml"), CONFIG).unwrap();
        Self(root)
    }

    fn go(&self, name: &str, text: &str) {
        std::fs::write(self.0.join("go/webinfo").join(name), text).unwrap();
    }

    fn options(&self) -> Options {
        Options {
            from: self.0.join("go"),
            config: PathBuf::from("config.yaml"),
            dir: Some(self.0.join("native")),
            out: self.0.join("out"),
        }
    }

    fn settings(&self) -> serde_json::Value {
        let text = std::fs::read_to_string(self.0.join("out/settings.json")).unwrap();
        serde_json::from_str(&text).unwrap()
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const CONFIG: &str = r#"
log: {level: error}
plugins:
  - tag: main_proxy
    type: socks5_proxy
    args: {addr: ""}
  - tag: client_ecs
    type: ecs_address
    args: {addr: "2408:8888::8"}
  - tag: domestic
    type: aliapi
    args:
      concurrent: 2
      proxy: $main_proxy
      upstreams:
        - {tag: ali, addr: "udp://223.5.5.5"}
  - tag: foreign
    type: aliapi
    args:
      concurrent: 2
      proxy: $main_proxy
      upstreams:
        - {tag: google, addr: "https://dns.google/dns-query", dial_addr: "8.8.8.8", use_proxy: true}
        - {tag: cloudflare, addr: "https://1.1.1.1/dns-query", use_proxy: true}
  - tag: foreignecs
    type: aliapi
    args:
      concurrent: 1
      proxy: $main_proxy
      upstreams:
        - {tag: google-foreignecs, addr: "https://dns.google/dns-query", dial_addr: "8.8.8.8", use_proxy: true}
  - tag: my_realiplist
    type: domain_output
    args: {file_stat: gen/real.txt, dump_interval: 3600}
  - tag: entry
    type: sequence
    args:
      - exec: ecs $client_ecs
      - exec: $foreign
  - tag: listener
    type: udp_server
    args: {entry: entry, listen: "127.0.0.1:15353"}
"#;

fn snapshot(directory: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut files: Vec<(PathBuf, Vec<u8>)> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let bytes = std::fs::read(&path).unwrap();
            (path, bytes)
        })
        .collect();
    files.sort();
    files
}

#[test]
fn reference_files_become_the_differences_only() {
    let tree = Tree::new("ok");
    tree.go(
        "config_overrides.json",
        r#"{"socks5":"10.0.0.1:1080","ecs":"198.51.100.5","replacements":[]}"#,
    );
    tree.go(
        "upstream_overrides.json",
        r#"{
          "foreign": [
            {"tag":"google","enabled":true,"protocol":"https","addr":"https://dns.google/dns-query","dial_addr":"8.8.8.8"},
            {"tag":"cloudflare","enabled":true,"protocol":"https","addr":"https://1.1.1.1/dns-query"}],
          "foreignecs": [
            {"tag":"google","enabled":true,"protocol":"https","addr":"https://dns.google/dns-query","dial_addr":"8.8.8.8"}],
          "domestic": [
            {"tag":"ali","enabled":true,"protocol":"udp","addr":"223.5.5.5"},
            {"tag":"isp","enabled":true,"protocol":"udp","addr":"114.114.114.114"},
            {"tag":"tcp","enabled":false,"protocol":"tcp","addr":"tcp://223.5.5.5"}]}"#,
    );
    tree.go("domain_generation_settings.json", r#"{"enabled":true,"remember_direct":false,"remember_proxy":true,"no_v4":true,"no_v6":true}"#);
    let before = snapshot(&tree.0.join("go/webinfo"));

    let outcome = run(&tree.options()).expect("migrates");
    assert_eq!(outcome.exit_code, 0, "{}", outcome.report);
    let settings = tree.settings();
    assert_eq!(settings["version"], 1);
    assert_eq!(settings["proxies"]["main_proxy"]["addr"], "10.0.0.1:1080");
    assert_eq!(settings["ecs"]["client_ecs"]["addr"], "198.51.100.5");
    // `foreign`/`foreignecs` equal the YAML once Go's proxy fallback is explicit.
    assert!(settings["upstreams"].get("foreign").is_none(), "{settings}");
    assert!(
        settings["upstreams"].get("foreignecs").is_none(),
        "{settings}"
    );
    let domestic = settings["upstreams"]["domestic"].as_array().unwrap();
    assert_eq!(domestic.len(), 3);
    assert_eq!(domestic[1]["tag"], "isp");
    assert_eq!(domestic[2]["enabled"], false);
    assert_eq!(
        settings["domain_generation"]["plugins"]["my_realiplist"]["enabled"],
        false
    );
    assert!(settings["domain_generation"].get("enabled").is_none());
    assert!(
        outcome.report.contains("[改名] foreignecs"),
        "{}",
        outcome.report
    );
    assert!(outcome.report.contains("生效值汇总"), "{}", outcome.report);
    assert!(tree.0.join("out/migration-report.txt").is_file());
    assert_eq!(
        snapshot(&tree.0.join("go/webinfo")),
        before,
        "inputs are never changed"
    );
    assert!(
        !tree.0.join("native/webinfo/settings.json").exists(),
        "the native root is not written"
    );

    // A second run does not overwrite the first result.
    assert!(run(&tree.options()).unwrap_err().contains("exists"));
}

#[test]
fn what_cannot_be_mapped_is_listed_and_the_exit_code_says_so() {
    let tree = Tree::new("unmapped");
    tree.go(
        "config_overrides.json",
        r#"{"socks5":"","ecs":"","replacements":[{"original":"a.example","new":"b.example","comment":""}]}"#,
    );
    tree.go(
        "upstream_overrides.json",
        r#"{
          "domestic": [
            {"tag":"ali","enabled":true,"protocol":"udp","addr":"223.6.6.6"},
            {"tag":"signed","enabled":true,"protocol":"aliapi","server_addr":"223.5.5.5"},
            {"tag":"marked","enabled":true,"protocol":"udp","addr":"9.9.9.9","so_mark":7}],
          "special_upstream_x": [{"tag":"s","enabled":true,"protocol":"udp","addr":"1.1.1.1"}]}"#,
    );
    let outcome = run(&tree.options()).expect("still produces its outputs");
    assert_eq!(outcome.exit_code, 1, "{}", outcome.report);
    // `foreign` and `foreignecs` are not in the file: the reference ran its own
    // YAML lists for them, which the tool cannot see.
    for needle in [
        "replacements",
        "signed",
        "marked",
        "special_upstream_x",
        "foreign：",
        "foreignecs：",
    ] {
        assert!(
            outcome.report.contains(needle),
            "{needle} not in\n{}",
            outcome.report
        );
    }
    assert!(
        outcome.report.starts_with("迁移报告：6 项无法映射"),
        "{}",
        outcome.report
    );
    // The mappable entry is kept; nothing was invented for the others.
    let domestic = tree.settings()["upstreams"]["domestic"].clone();
    assert_eq!(domestic.as_array().unwrap().len(), 1);
}

#[test]
fn a_failed_write_leaves_no_partial_result() {
    let tree = Tree::new("writefail");
    tree.go(
        "config_overrides.json",
        r#"{"socks5":"10.0.0.1:1080","ecs":"","replacements":[]}"#,
    );
    // The report cannot be put in place: a directory has its name.
    std::fs::create_dir_all(tree.0.join("out/migration-report.txt/inner")).unwrap();
    let error = run(&tree.options()).expect_err("must fail");
    assert!(error.contains("out"), "{error}");
    for name in [
        "settings.json",
        ".settings.json.tmp",
        ".migration-report.txt.tmp",
    ] {
        assert!(
            !tree.0.join("out").join(name).exists(),
            "{name} was left behind"
        );
    }
}

#[test]
fn settings_that_do_not_load_stop_the_run_without_output() {
    let tree = Tree::new("badproxy");
    tree.go(
        "config_overrides.json",
        r#"{"socks5":"not-an-address","ecs":"","replacements":[]}"#,
    );
    let error = run(&tree.options()).expect_err("must fail");
    assert!(error.contains("host:port"), "{error}");
    assert!(!tree.0.join("out/settings.json").exists());
}
