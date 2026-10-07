//! M1-a1: read-only rule providers (`domain_set_light`, `sd_set_light`,
//! `adguard_rule`, `si_set`), `domain_mapper` and the `mark` matcher.
//!
//! The fixture tree under `tests/fixtures/m1_a1/tree` mirrors the production
//! layout (`rule/`, `srs/`, `adguard/`); its SRS files were written by the
//! reference sing library (`tools/m1a1-oracle/mksrs`). Expected answers are
//! the ones the reference providers give for the same tree (see the
//! differential record).

use std::path::{Path, PathBuf};
use std::time::Duration;

use mosdns_native_host::{
    CheckFindingKind, CompiledConfig, HostAssembly, MapperOutcome, check_config, compile_yaml,
    load_and_compile,
};
use mosdns_upstream_core::TransportCancellation;

const TREE: &str = "tests/fixtures/m1_a1/tree";

struct TempTree(PathBuf);

impl TempTree {
    /// Copies the fixture tree and fills the listener port.
    fn new(name: &str, port: u16) -> Self {
        let root = std::env::temp_dir().join(format!("m1-a1-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        copy_dir(Path::new(TREE), &root);
        let config = root.join("config.yaml");
        let text = std::fs::read_to_string(&config).unwrap();
        std::fs::write(&config, text.replace("__PORT__", &port.to_string())).unwrap();
        Self(root)
    }

    fn config(&self) -> PathBuf {
        self.0.join("config.yaml")
    }
}

impl Drop for TempTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

fn free_port() -> u16 {
    std::net::UdpSocket::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn wire(name: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for label in name.trim_end_matches('.').split('.') {
        out.push(u8::try_from(label.len()).unwrap());
        out.extend_from_slice(label.as_bytes());
    }
    out.push(0);
    out
}

fn evaluate(config: &CompiledConfig, tag: &str, name: &str) -> MapperOutcome {
    config
        .domain_mappers
        .iter()
        .find(|mapper| mapper.tag == tag)
        .expect("mapper")
        .evaluate_wire(&wire(name))
}

fn flags(outcome: &MapperOutcome) -> Vec<u32> {
    (0..64)
        .filter(|bit| outcome.fast_flags & (1u64 << bit) != 0)
        .collect()
}

/// (name, fast marks, ctx marks, output tags, rule sources); `None` = default.
type Expected<'a> = (&'a str, &'a [u32], &'a [u32], &'a str, &'a str);

#[test]
#[allow(clippy::too_many_lines)] // one table of reference answers
fn providers_and_mapper_give_the_reference_answers() {
    let tree = TempTree::new("answers", 15353);
    let config = load_and_compile(&tree.config()).expect("tree compiles");
    let cases: &[Expected<'_>] = &[
        // domain_set_light: text rules, inline exps, trimming, case-insensitive full.
        (
            "blocked.example.",
            &[1, 3],
            &[],
            "black|black2",
            "blocklist",
        ),
        (
            "www.BLOCKED.example.",
            &[1, 3],
            &[],
            "black|black2",
            "blocklist",
        ),
        (
            "_http._tcp.blocked.example.",
            &[1, 3],
            &[],
            "black|black2",
            "blocklist",
        ),
        (
            "exact.block.example.",
            &[1, 3],
            &[],
            "black|black2",
            "blocklist",
        ),
        (
            "EXACT.BLOCK.EXAMPLE.",
            &[1, 3],
            &[],
            "black|black2",
            "blocklist",
        ),
        (
            "x.adtrack.y.test.",
            &[1, 3],
            &[],
            "black|black2",
            "blocklist",
        ),
        ("ban42.example.", &[1, 3], &[], "black|black2", "blocklist"),
        // Go regexp escapes and flags run with Go semantics.
        (
            "12.digits.example.",
            &[1, 3],
            &[],
            "black|black2",
            "blocklist",
        ),
        (
            "CASEFOLD.example.",
            &[1, 3],
            &[],
            "black|black2",
            "blocklist",
        ),
        ("inline.example.", &[1, 3], &[], "black|black2", "blocklist"),
        ("spaced.example.", &[1, 3], &[], "black|black2", "blocklist"),
        // sd_set_light: SRS domain items; regexp only for the source that enables it.
        (
            "exact.srs.example.",
            &[2],
            &[77],
            "sub",
            "srs_main|srs_noregex",
        ),
        (
            "a.suffix.srs.example.",
            &[2],
            &[77],
            "sub",
            "srs_main|srs_noregex",
        ),
        ("a.example.cn.", &[2], &[77], "sub", "srs_main|srs_noregex"),
        ("my.kwsrs.test.", &[2], &[77], "sub", "srs_main|srs_noregex"),
        ("re12.srs.example.", &[2], &[77], "sub", "srs_main"),
        // adguard_rule: block rules only, `@@` lines are not exported.
        ("ads.guard.example.", &[5], &[], "ads", "adguard"),
        ("x.wild.guard.example.", &[5], &[], "ads", "adguard"),
        ("trkabc.guard.example.", &[5], &[], "ads", "adguard"),
        ("pat7.guard.example.", &[5], &[], "ads", "adguard"),
        ("7.dig.guard.example.", &[5], &[], "ads", "adguard"),
        ("allowed.guard.example.", &[5], &[], "ads", "adguard"),
        ("plain.guard.example.", &[5], &[], "ads", "adguard"),
        // A list stops at the reference scanner's line limit, keeping what
        // came before.
        ("pre.guard.example.", &[5], &[], "ads", "adguard"),
    ];
    for (name, marks, ctx, tags, sources) in cases {
        let outcome = evaluate(&config, "unified", name);
        assert!(outcome.matched, "{name} must match");
        let expected_flags: Vec<u32> = marks.to_vec();
        assert_eq!(flags(&outcome), expected_flags, "{name} fast marks");
        assert_eq!(&outcome.ctx_marks, ctx, "{name} ctx marks");
        assert_eq!(outcome.domain_set.as_deref(), Some(*tags), "{name} tags");
        assert_eq!(
            outcome.matched_rule_source.as_deref(),
            Some(*sources),
            "{name} sources"
        );
    }
    // Not matched: the mapper's default applies.
    for name in [
        "nothing.test.",
        "sub.exact.block.example.",
        "ban.example.",
        "nocolon.example.",
        "type.example.",
        "sub.plain.guard.example.",
        "hosts.guard.example.",
        "disabled.guard.example.",
        "re12.srs.example.test.",
        "x.digits.example.",
        "post.guard.example.",
    ] {
        let outcome = evaluate(&config, "unified", name);
        assert!(!outcome.matched, "{name} must not match");
        assert_eq!(flags(&outcome), [17], "{name}");
        assert_eq!(outcome.domain_set.as_deref(), Some("none"), "{name}");
        assert_eq!(outcome.matched_rule_source, None, "{name}");
    }
    // A mapper without rules and defaults changes nothing.
    let quiet = evaluate(&config, "special_matcher", "blocked.example.");
    assert_eq!(quiet, MapperOutcome::default());
    assert!(
        config
            .domain_mappers
            .iter()
            .find(|mapper| mapper.tag == "unified")
            .unwrap()
            .rule_count()
            > 10
    );
}

#[test]
fn si_set_reads_the_enabled_sources_and_skips_the_rest() {
    let tree = TempTree::new("ipset", 15353);
    let config = load_and_compile(&tree.config()).expect("tree compiles");
    let set = config
        .ip_sets
        .iter()
        .find(|set| set.tag == "geoip")
        .expect("geoip");
    for (address, expected) in [
        ("10.0.0.0", true),
        ("10.0.0.255", true),
        ("10.0.1.0", false),
        ("192.0.2.4", false),
        ("192.0.2.5", true),
        ("192.0.2.9", true),
        ("192.0.2.10", false),
        ("2001:db8::1", true),
        ("2001:db8:1::", false),
        ("8.8.8.8", false),
    ] {
        assert_eq!(
            set.prefixes.contains(address.parse().unwrap()),
            expected,
            "{address}"
        );
    }
}

/// Sends one A query over UDP and returns the response's rcode.
async fn rcode(target: std::net::SocketAddr, name: &str, id: u16) -> u8 {
    let mut query = id.to_be_bytes().to_vec();
    query.extend([1, 0, 0, 1, 0, 0, 0, 0, 0, 0]);
    query.extend(wire(name));
    query.extend([0, 1, 0, 1]);
    let socket = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
    socket.send_to(&query, target).await.unwrap();
    let mut packet = vec![0; 1500];
    let (length, _) = tokio::time::timeout(Duration::from_secs(3), socket.recv_from(&mut packet))
        .await
        .expect("response in time")
        .unwrap();
    assert!(length >= 12);
    packet[3] & 15
}

#[test]
fn a_real_query_routes_through_the_mapper_marks() {
    let port = free_port();
    let tree = TempTree::new("udp", port);
    let host = HostAssembly::from_config_file(&tree.config()).expect("assembles");
    host.block_on(async {
        let bound = host.bind_host().await.expect("binds");
        let shutdown = TransportCancellation::new();
        let serving = tokio::task::spawn_local(bound.serve(shutdown.clone()));
        let target: std::net::SocketAddr = ([127, 0, 0, 1], port).into();
        // fast mark 1 → NXDOMAIN, fast mark 5 → REFUSED, ctx mark 77 →
        // SERVFAIL (checked first), default mark 17 → NOERROR.
        for (name, expected) in [
            ("blocked.example.", 3),
            ("EXACT.block.example.", 3),
            // `qname $adguard` is the reference direct matcher: exception
            // rules win, so the allow-listed name falls through to the
            // mapper (which still marks it, as the reference export does).
            ("ads.guard.example.", 1),
            ("x.wild.guard.example.", 1),
            ("allowed.guard.example.", 5),
            ("hosts.guard.example.", 0),
            ("7.dig.guard.example.", 1),
            // The direct matcher sees the reference presentation-form name.
            ("_http._tcp.ads.guard.example.", 1),
            ("pre.guard.example.", 1),
            ("post.guard.example.", 0),
            ("plain.guard.example.", 1),
            ("sub.plain.guard.example.", 0),
            ("PAT7.guard.example.", 1),
            ("exact.srs.example.", 2),
            ("nothing.test.", 0),
            // Trailing/leading dots in rules (answers taken from the reference
            // `qname $adguard` matcher): `@@||tr2.guard.example...^` does not
            // exempt `tr2.guard.example.` (three dots are not the plain name),
            // and `||..dots.guard.example^` still blocks it.
            ("tr2.guard.example.", 1),
            ("dots.guard.example.", 1),
            ("x.dots.guard.example.", 1),
        ] {
            assert_eq!(rcode(target, name, 7).await, expected, "{name}");
        }
        // `||trail.guard.example...^` names a different (never matching) rule:
        // the direct matcher must not block these names.
        for name in ["trail.guard.example.", "a.trail.guard.example."] {
            assert_ne!(rcode(target, name, 7).await, 1, "{name}");
        }
        shutdown.cancel();
        serving.await.unwrap().expect("clean shutdown");
    });
}

fn plain(extra: &str) -> String {
    format!(
        "log: {{level: error}}\nplugins:\n  - tag: fwd\n    type: forward\n    args: {{upstreams: [{{addr: \"udp://127.0.0.1:15453\"}}]}}\n{extra}  - tag: entry\n    type: sequence\n    args: [{{exec: reject 3}}]\n  - tag: l\n    type: udp_server\n    args: {{entry: entry, listen: \"127.0.0.1:15353\"}}\n"
    )
}

#[test]
fn mapper_configuration_errors_follow_the_reference() {
    let provider =
        "  - tag: p\n    type: domain_set_light\n    args: {exps: [\"domain:a.example\"]}\n";
    let ok = |mapper: &str| compile_yaml(&plain(&format!("{provider}{mapper}")));
    let mapper = |args: &str| format!("  - tag: m\n    type: domain_mapper\n    args: {args}\n");

    assert!(ok(&mapper("{rules: [{tag: p, mark: 63}], default_mark: 63}")).is_ok());
    for (bad, why) in [
        ("{default_mark: 64}", "default_mark above 63"),
        ("{rules: [{tag: p, mark: 64}]}", "rule mark above 63"),
        ("{rules: [{tag: absent}]}", "unknown provider"),
        ("{rules: [{tag: fwd}]}", "tag is not a rule provider"),
        ("{rules: [{tag: p, extra: 1}]}", "unknown rule field"),
        ("{surprise: 1}", "unknown argument"),
    ] {
        assert!(ok(&mapper(bad)).is_err(), "{why} must be rejected");
    }
    let error = ok(&mapper("{rules: [{tag: absent}]}")).err().unwrap();
    assert!(error.reason.contains("not found"), "{error}");
    let error = ok(&mapper("{rules: [{tag: fwd}]}")).err().unwrap();
    assert!(error.reason.contains("rule export"), "{error}");

    // Provider arguments.
    assert!(compile_yaml(&plain("  - tag: s\n    type: sd_set_light\n    args: {}\n")).is_err());
    assert!(
        compile_yaml(&plain(
            "  - tag: s\n    type: si_set\n    args: {local_config: \"\"}\n"
        ))
        .is_err()
    );
    assert!(compile_yaml(&plain("  - tag: a\n    type: adguard_rule\n    args: {}\n")).is_err());
    assert!(
        compile_yaml(&plain(
            "  - tag: p\n    type: domain_set_light\n    args: {bogus: 1}\n"
        ))
        .is_err()
    );
}

#[test]
fn mark_is_a_context_mark_matcher_and_executor() {
    let config = compile_yaml(&plain(""));
    assert!(config.is_ok());
    for bad in [
        "mark nope",
        "mark -1",
        "mark +7",
        "mark 4294967296",
        "mark 1_0",
    ] {
        let yaml = plain("").replace(
            "args: [{exec: reject 3}]",
            &format!("args: [{{matches: \"{bad}\", exec: reject 3}}]"),
        );
        assert!(compile_yaml(&yaml).is_err(), "{bad}");
    }
    let yaml = plain("").replace(
        "args: [{exec: reject 3}]",
        "args: [{exec: mark 5 6}, {matches: mark 6, exec: reject 3}, {matches: \"!mark 7\", exec: reject 2}]",
    );
    assert!(compile_yaml(&yaml).is_ok());
}

#[test]
fn check_lists_no_gap_for_the_provider_tree_and_is_read_only() {
    let tree = TempTree::new("check", 15353);
    let before = snapshot(&tree.0);
    let report = check_config(&tree.config()).expect("check runs");
    assert!(report.findings.is_empty(), "{report}");
    assert_eq!(snapshot(&tree.0), before, "check must not write");
    // Starting is read-only for the provider state too (no adguard download,
    // no directory creation, no manifest rewrite).
    let host = HostAssembly::from_config_file(&tree.config()).expect("assembles");
    drop(host);
    assert_eq!(snapshot(&tree.0), before, "start must not write");
}

fn snapshot(root: &Path) -> Vec<(PathBuf, Vec<u8>, std::time::SystemTime)> {
    let mut out = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = std::fs::symlink_metadata(&path).unwrap();
        if metadata.is_dir() {
            for entry in std::fs::read_dir(&path).unwrap() {
                pending.push(entry.unwrap().path());
            }
        } else {
            out.push((
                path.clone(),
                std::fs::read(&path).unwrap(),
                metadata.modified().unwrap(),
            ));
        }
    }
    out.sort();
    out
}

#[test]
fn a_mapper_naming_an_unsupported_provider_is_a_cascade_in_check() {
    let tree = TempTree::new("cascade", 15353);
    let yaml = plain(
        "  - tag: p\n    type: sd_set\n    args: {}\n  - tag: m\n    type: domain_mapper\n    args: {rules: [{tag: p, mark: 1}]}\n",
    );
    std::fs::write(tree.config(), yaml).unwrap();
    let report = check_config(&tree.config()).expect("check runs");
    let kinds: Vec<_> = report
        .findings
        .iter()
        .map(|finding| (finding.tag.as_deref(), finding.kind.clone()))
        .collect();
    assert_eq!(
        kinds,
        [
            (Some("p"), CheckFindingKind::Gap),
            (
                Some("m"),
                CheckFindingKind::Cascade {
                    removed: "p".into()
                }
            ),
        ],
        "{report}"
    );
}

/// A regexp rule that is valid Go but outside what this host runs with Go
/// semantics must refuse the configuration (never be dropped silently), on
/// both the `domain_mapper` and the direct `qname $adguard` path.
#[test]
fn an_unsupported_but_valid_go_regexp_refuses_the_configuration() {
    let mapper = TempTree::new("refuse-mapper", 15353);
    let blocklist = mapper.0.join("rule/blocklist.txt");
    let mut text = std::fs::read_to_string(&blocklist).unwrap();
    text.push_str("regexp:\\p{L}+\\.letters\\.example\n");
    std::fs::write(&blocklist, text).unwrap();
    let error = load_and_compile(&mapper.config()).err().expect("refused");
    assert!(
        error.reason.contains("valid Go") && error.reason.contains("letters"),
        "{error}"
    );

    let direct = TempTree::new("refuse-direct", 15353);
    let rules = direct.0.join("adguard/aaaa.rules");
    let mut text = std::fs::read_to_string(&rules).unwrap();
    text.push_str("/\\pL+\\.letters\\.guard\\.example/\n");
    std::fs::write(&rules, text).unwrap();
    // The mapper path also sees the deny list; take adguard out of the mapper
    // so only `qname $adguard` can refuse.
    let config = direct.config();
    let yaml = std::fs::read_to_string(&config).unwrap();
    let yaml = yaml.replace(
        "        - {tag: adguard, mark: 5, output_tag: \"ads\"}\n",
        "",
    );
    std::fs::write(&config, yaml).unwrap();
    let error = load_and_compile(&config).err().expect("refused");
    assert!(
        error.reason.contains("valid Go") && error.reason.contains("letters"),
        "{error}"
    );
}
