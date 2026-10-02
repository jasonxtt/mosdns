use mosdns_native_host::{ResponsePolicy, TtlPolicy, compile_yaml};
fn config(plugins: &str, rules: &str) -> String {
    format!(
        "log: {{level: error}}\nplugins:\n{plugins}\n  - tag: upstream\n    type: forward\n    args:\n      upstreams:\n        - addr: udp://127.0.0.1:19000\n  - tag: main\n    type: sequence\n    args:\n{rules}\n  - tag: listener\n    type: udp_server\n    args:\n      entry: main\n      listen: 127.0.0.1:19100\n      enable_audit: false\n"
    )
}
#[test]
fn named_response_policies_and_ttl_compile_through_public_yaml() {
    let yaml = config(
        "  - tag: local\n    type: hosts\n    args:\n      entries: ['Case.example 192.0.2.1 ::1']\n  - tag: rewrite\n    type: redirect\n    args:\n      rules: ['a.example b.example']\n  - tag: networks\n    type: ip_set\n    args:\n      ips: ['192.0.2.0/24', '2001:db8::/32']",
        "      - exec: $local\n      - exec: ttl 300-10\n      - exec: $rewrite\n      - exec: $upstream",
    );
    let error = compile_yaml(&yaml).err();
    assert!(
        error.is_none(),
        "approved named/quick policy grammar must compile: {error:?}"
    );
}
#[test]
fn text_ip_references_and_or_expression_compile_before_execution() {
    let yaml = config(
        "  - tag: networks\n    type: ip_set\n    args:\n      ips: ['::1', '192.0.2.0/24']",
        "      - matches: resp_ip $networks 2001:db8::/32\n        exec: accept\n      - exec: $upstream",
    );
    let error = compile_yaml(&yaml).err();
    assert!(
        error.is_none(),
        "typed IP expression must compile without runtime file reads: {error:?}"
    );
}

struct Scratch(std::path::PathBuf);
impl Scratch {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "mosdns-policy-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("owned temporary directory");
        Self(path)
    }
    fn write(&self, name: &str, content: impl AsRef<[u8]>) {
        std::fs::write(self.0.join(name), content).expect("fixture");
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn domain_payload_precedence_replacement_and_startup_snapshot_are_observable() {
    let dir = Scratch::new();
    dir.write(
        "hosts.txt",
        "# comment\nfull:case.example 192.0.2.9\nregexp:.*example 192.0.2.8\n",
    );
    let yaml = config(
        "  - tag: local\n    type: hosts\n    args:\n      entries: ['keyword:example 192.0.2.1', 'regexp:.*example 192.0.2.2', 'regexp:.* 192.0.2.3', 'domain:example 192.0.2.4', 'domain:deep.example 192.0.2.5', 'case.example 192.0.2.6']\n      files: ['hosts.txt']",
        "      - exec: $local\n      - exec: $upstream",
    );
    let compiled =
        mosdns_native_host::compile_yaml_with_base(&yaml, &dir.0).expect("payload snapshot");
    let ResponsePolicy::Hosts(rules) = &compiled.response_policies[0].policy else {
        panic!("hosts")
    };
    assert_eq!(
        rules.lookup("CASE.EXAMPLE.").unwrap().ipv4[0].octets(),
        [192, 0, 2, 9]
    );
    assert_eq!(
        rules.lookup("sub.deep.example").unwrap().ipv4[0].octets(),
        [192, 0, 2, 5]
    );
    assert_eq!(
        rules.lookup("sub.example").unwrap().ipv4[0].octets(),
        [192, 0, 2, 4]
    );
    assert_eq!(
        rules.lookup("otherexample").unwrap().ipv4[0].octets(),
        [192, 0, 2, 8]
    );
    assert_eq!(
        rules.len(),
        6,
        "replacement uses normalized identity and does not reorder regex priority"
    );
    dir.write("hosts.txt", "case.example 192.0.2.100\n");
    assert_eq!(
        rules.lookup("case.example").unwrap().ipv4[0].octets(),
        [192, 0, 2, 9],
        "snapshot never rereads a changed file"
    );
}
#[test]
fn ip_files_have_empty_missing_bad_and_binary_startup_outcomes() {
    let dir = Scratch::new();
    dir.write("empty.txt", "# only comments\n\n");
    let yaml = config(
        "  - tag: networks\n    type: ip_set\n    args:\n      ips: ['::1']\n      files: ['missing.txt', 'empty.txt']",
        "      - matches: resp_ip $networks &empty.txt\n        exec: accept\n      - exec: $upstream",
    );
    let compiled = mosdns_native_host::compile_yaml_with_base(&yaml, &dir.0)
        .expect("missing skips, empty valid");
    assert!(
        compiled.ip_sets[0]
            .prefixes
            .contains("::1".parse().unwrap())
    );
    assert!(
        !compiled.ip_sets[0]
            .prefixes
            .contains("192.0.2.1".parse().unwrap())
    );
    for bytes in [
        b"192.0.2.1\nwrong\n".as_slice(),
        b"SRS\0binary",
        b"\x1f\x8bgarbage",
        b"\x78\x9cgarbage",
        b"\xff",
    ] {
        dir.write("empty.txt", bytes);
        let error = mosdns_native_host::compile_yaml_with_base(&yaml, &dir.0)
            .err()
            .expect("bad source rejected entirely");
        assert!(error.path.contains("empty.txt:1") || error.path.contains("empty.txt:2"));
    }
    let hosts = yaml
        .replace("type: ip_set", "type: hosts")
        .replace("ips: ['::1']", "entries: ['x ::1']")
        .replace("resp_ip $networks &empty.txt", "_true");
    assert!(
        mosdns_native_host::compile_yaml_with_base(&hosts, &dir.0).is_err(),
        "missing hosts file fails startup"
    );
}
#[test]
fn unsupported_shapes_and_ttl_bounds_fail_at_config_boundary() {
    for plugins in [
        "  - tag: x\n    type: ip_set\n    args: { sets: ['$other'] }",
        "  - tag: x\n    type: ttl\n    args: {}",
        "  - tag: x\n    type: hosts\n    args: {entries: ['x invalid']}",
        "  - tag: x\n    type: redirect\n    args: {rules: ['x y z']}",
        "  - tag: x\n    type: hosts\n    args: {bogus: []}",
    ] {
        assert!(compile_yaml(&config(plugins, "      - exec: $upstream")).is_err());
    }
    for expression in [
        "ttl",
        "ttl -1",
        "ttl +1",
        "ttl 4294967296",
        "ttl 1 2",
        "ttl 1-2-3",
        "resp_ip ::1/129",
        "resp_ip $upstream",
        "resp_ip &",
    ] {
        let rule = if expression.starts_with("ttl") {
            format!("      - exec: {expression}\n      - exec: $upstream")
        } else {
            format!("      - matches: {expression}\n        exec: accept\n      - exec: $upstream")
        };
        assert!(compile_yaml(&config("", &rule)).is_err(), "{expression}");
    }
    for (input, expected) in [
        ("ttl 0", TtlPolicy::Fixed(0)),
        ("ttl 4294967295", TtlPolicy::Fixed(u32::MAX)),
        ("ttl 300-10", TtlPolicy::Range { min: 300, max: 10 }),
        ("ttl 0-0", TtlPolicy::Range { min: 0, max: 0 }),
    ] {
        let cfg = compile_yaml(&config(
            "",
            &format!("      - exec: {input}\n      - exec: $upstream"),
        ))
        .expect("valid uint32 descriptor");
        let ResponsePolicy::Ttl(actual) = cfg.response_policies[0].policy else {
            panic!("typed TTL")
        };
        assert_eq!(actual, expected);
    }
}
#[test]
fn included_policy_file_paths_belong_to_declaring_yaml() {
    let dir = Scratch::new();
    std::fs::create_dir(dir.0.join("included")).unwrap();
    dir.write(
        "included/ips.txt",
        "192.0.2.0/24 comment ignored\n2001:db8::/32\n",
    );
    dir.write("included/defs.yaml","plugins:\n  - tag: networks\n    type: ip_set\n    args: {files: ['ips.txt']}\n  - tag: local\n    type: hosts\n    args: {entries: ['x 192.0.2.1 ::1']}\n");
    let yaml = format!(
        "include: ['included/defs.yaml']\n{}",
        config(
            "",
            "      - exec: $local\n      - matches: resp_ip $networks\n        exec: accept\n      - exec: $upstream"
        )
    );
    let cfg =
        mosdns_native_host::compile_yaml_with_base(&yaml, &dir.0).expect("included source base");
    assert!(
        cfg.ip_sets[0]
            .prefixes
            .contains("192.0.2.255".parse().unwrap())
    );
    assert!(
        cfg.ip_sets[0]
            .prefixes
            .contains("2001:db8::1".parse().unwrap())
    );
    assert!(
        !cfg.ip_sets[0]
            .prefixes
            .contains("2001:db9::1".parse().unwrap())
    );
}
#[test]
fn text_loader_limits_are_enforced_before_snapshot_publication() {
    use std::io::Write;
    let dir = Scratch::new();
    let yaml = config(
        "  - tag: networks\n    type: ip_set\n    args: {files: ['large.txt']}",
        "      - exec: $upstream",
    );
    dir.write(
        "large.txt",
        vec![b'#'; mosdns_native_host::POLICY_LINE_LIMIT + 1],
    );
    let check = || {
        mosdns_native_host::compile_yaml_with_base(&yaml, &dir.0)
            .err()
            .expect("limit error")
    };
    assert!(check().reason.contains("line limit"));
    {
        let mut out =
            std::io::BufWriter::new(std::fs::File::create(dir.0.join("large.txt")).unwrap());
        let comment = format!("#{}\n", "x".repeat(65534));
        for _ in 0..1025 {
            out.write_all(comment.as_bytes()).unwrap();
        }
    }
    assert!(check().reason.contains("byte limit"));
    {
        let mut out =
            std::io::BufWriter::new(std::fs::File::create(dir.0.join("large.txt")).unwrap());
        for _ in 0..=mosdns_native_host::POLICY_RULE_LIMIT {
            out.write_all(b"::1\n").unwrap();
        }
    }
    assert!(check().reason.contains("rule limit"));
}

#[test]
fn descriptors_cannot_start_an_incomplete_runtime() {
    let yaml = config(
        "  - tag: local\n    type: hosts\n    args: {entries: ['x 192.0.2.1']}",
        "      - exec: $local\n      - exec: $upstream",
    );
    let compiled = compile_yaml(&yaml).expect("typed compile supported in S1");
    assert!(
        mosdns_native_host::HostAssembly::from_config(compiled).is_err(),
        "S1 fails closed before listener bind rather than ignoring policy"
    );
}

#[test]
fn empty_policy_defaults_and_mapped_hosts_keep_typed_family() {
    let cfg=compile_yaml(&config("  - tag: local\n    type: hosts\n  - tag: rewrite\n    type: redirect\n    args: null\n  - tag: networks\n    type: ip_set\n    args: {ips: null, files: null, sets: []}","      - exec: $upstream")).expect("empty defaults");
    assert!(cfg.ip_sets[0].prefixes.is_empty());
    let cfg = compile_yaml(&config(
        "  - tag: local\n    type: hosts\n    args: {entries: ['x ::ffff:192.0.2.1']}",
        "      - exec: $local\n      - exec: $upstream",
    ))
    .expect("mapped IPv6 hosts");
    let mosdns_native_host::ResponsePolicy::Hosts(rules) = &cfg.response_policies[0].policy else {
        panic!("hosts")
    };
    assert!(rules.lookup("x").unwrap().ipv4.is_empty());
    assert_eq!(rules.lookup("x").unwrap().ipv6.len(), 1);
    assert!(
        compile_yaml(&config(
            "  - tag: networks\n    type: ip_set\n    args: {ips: ['192.0.2.1 invalid']}",
            "      - exec: $upstream"
        ))
        .is_err()
    );
}
