use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use mosdns_native_host::{compile_yaml_with_base, load_and_compile};
use mosdns_sequence_core::{ExecutionControl, ExecutionState, MachineStep};

/// Compiles a configuration rooted at a per-test directory, so relative
/// state-file paths and known-artifact collisions are real filesystem facts.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("switch-decl-{}-{}", std::process::id(), name));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("test root");
        Self { root }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn compile(
        &self,
        yaml: &str,
    ) -> Result<mosdns_native_host::CompiledConfig, mosdns_native_host::ConfigError> {
        compile_yaml_with_base(yaml, &self.root)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// `expect_err` requires a `Debug` success type; assert the rejection without it.
fn expect_config_error(
    result: Result<mosdns_native_host::CompiledConfig, mosdns_native_host::ConfigError>,
    context: &str,
) -> mosdns_native_host::ConfigError {
    match result {
        Ok(_) => panic!("{context}"),
        Err(error) => error,
    }
}

/// The minimal accepted graph plus optional extra plugin declarations and an
/// explicit entry-rule body. Each entry rule is one YAML list item line.
fn config_with(extra_plugins: &str, entry_rules: &[&str]) -> String {
    let mut yaml = String::from(
        "log:\n  level: error\n\nplugins:\n  - tag: fwd\n    type: forward\n    args:\n      \
         upstreams:\n        - addr: \"udp://127.0.0.1:15453\"\n",
    );
    yaml.push_str(extra_plugins);
    yaml.push_str("  - tag: entry\n    type: sequence\n    args:\n");
    // Each rule is a complete list-item body beginning with `- `, followed
    // by any deeper-indented continuation lines it carries.
    for rule in entry_rules {
        yaml.push_str("      ");
        yaml.push_str(rule);
        yaml.push('\n');
    }
    yaml.push_str(
        "  - tag: srv\n    type: udp_server\n    args:\n      entry: entry\n      listen: \
         \"127.0.0.1:15353\"\n      enable_audit: false\n",
    );
    yaml
}

fn switch_plugin(type_number: u8, tag: &str, state_file: &str) -> String {
    format!(
        "  - tag: {tag}\n    type: switch{type_number}\n    args:\n      initial_value: \
         \"{state_file}\"\n"
    )
}

fn query_state(facts: &[(u32, &str)]) -> ExecutionState {
    let mut state = ExecutionState::new(
        mosdns_dns_core::QueryHeader {
            id: 1,
            qr: false,
            opcode: 0,
            qdcount: 1,
            ancount: 0,
            nscount: 0,
            arcount: 0,
        },
        mosdns_dns_core::QuestionInfo {
            qname_wire: vec![0],
            qtype: 1,
            qclass: 1,
        },
    );
    let facts: BTreeMap<u32, Arc<str>> = facts
        .iter()
        .map(|(key, value)| (*key, Arc::from(*value)))
        .collect();
    state.set_admission_facts(Arc::new(facts));
    state
}

/// The observable terminal a machine reaches before any upstream work.
enum Terminal {
    /// The matcher chain fell through to the forward dispatch.
    Dispatch,
    /// A `reject` rule executed; the response is synthesized.
    Rejected,
    /// The machine completed without dispatch or reject.
    Completed,
}

fn run_to_terminal(
    config: &mosdns_native_host::CompiledConfig,
    state: ExecutionState,
    fuel: u64,
) -> (Terminal, u64) {
    let mut machine = config
        .new_machine(state, ExecutionControl::with_fuel(fuel))
        .expect("machine");
    loop {
        match machine.step().expect("machine step") {
            MachineStep::Dispatch(_) => {
                return (Terminal::Dispatch, machine.control().remaining_budget());
            }
            MachineStep::Complete(_) => {
                let rejected = machine.state().response.synthesized_rcode() == Some(5);
                let terminal = if rejected {
                    Terminal::Rejected
                } else {
                    Terminal::Completed
                };
                return (terminal, machine.control().remaining_budget());
            }
            MachineStep::ScopeComplete(_) | MachineStep::ScopeAborted(_) => {}
        }
    }
}

/// One matcher expression under test: `matches: <expr>` then reject, otherwise
/// fall through to the forward. The declaration under test is `switch2`.
/// Double-quotes one matcher expression so quoted/empty expectations stay
/// one YAML scalar.
fn yaml_quoted(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn matcher_probe(matcher: &str, facts: &[(u32, &str)], fast_flags: u64) -> Terminal {
    let fixture = Fixture::new("probe");
    let yaml = config_with(
        &switch_plugin(2, "sw2", "sw2.txt"),
        &[
            &format!("- matches: {}\n        exec: reject", yaml_quoted(matcher)),
            "- exec: $fwd",
        ],
    );
    let mut state = query_state(facts);
    state.fast_flags = fast_flags;
    let config = fixture
        .compile(&yaml)
        .expect("probe configuration compiles");
    run_to_terminal(&config, state, 64).0
}

#[test]
fn all_seventeen_switch_types_compile_with_unique_tags_and_state_files() {
    let fixture = Fixture::new("all17");
    let mut extra = String::new();
    for type_number in 1..=17u8 {
        extra.push_str(&switch_plugin(
            type_number,
            &format!("sw{type_number}"),
            &format!("state{type_number}.txt"),
        ));
    }
    let yaml = config_with(&extra, &["- exec: $fwd"]);
    let config = fixture.compile(&yaml).expect("seventeen switches compile");
    assert_eq!(config.switches.len(), 17);
    for (index, declaration) in config.switches.iter().enumerate() {
        assert_eq!(
            declaration.type_number,
            u8::try_from(index).expect("type index") + 1
        );
        assert_eq!(declaration.tag, format!("sw{}", index + 1));
        assert_eq!(
            declaration.state_file,
            fixture.root.join(format!("state{}.txt", index + 1))
        );
    }
}

#[test]
fn relative_state_paths_resolve_against_the_including_files_directory() {
    let fixture = Fixture::new("include");
    std::fs::create_dir_all(fixture.path("sub")).expect("subdir");
    let included = fixture.path("sub/included.yaml");
    std::fs::write(
        &included,
        "plugins:\n  - tag: sw9\n    type: switch9\n    args:\n      initial_value: \"sw9.txt\"\n",
    )
    .expect("included yaml");
    let main = fixture.path("main.yaml");
    std::fs::write(
        &main,
        config_with("", &["- exec: $fwd"]).replace(
            "plugins:\n",
            &format!("include:\n  - \"{}\"\nplugins:\n", included.display()),
        ),
    )
    .expect("main yaml");
    let config = load_and_compile(&main).expect("included switch compiles");
    assert_eq!(config.switches.len(), 1);
    assert_eq!(config.switches[0].tag, "sw9");
    assert_eq!(config.switches[0].state_file, fixture.path("sub/sw9.txt"));
}

#[test]
fn duplicate_switch_type_is_rejected() {
    let fixture = Fixture::new("dup-type");
    let extra = format!(
        "{}{}",
        switch_plugin(3, "sw3", "a.txt"),
        switch_plugin(3, "sw3b", "b.txt")
    );
    let yaml = config_with(&extra, &["- exec: $fwd"]);
    let error = fixture
        .compile(&yaml)
        .map(|_| ())
        .expect_err("duplicate type rejected");
    assert!(
        error.reason.contains("switch3"),
        "unexpected reason: {error}"
    );
}

#[test]
fn duplicate_switch_tag_is_rejected() {
    let fixture = Fixture::new("dup-tag");
    let extra = format!(
        "{}{}",
        switch_plugin(1, "sw1", "a.txt"),
        switch_plugin(2, "sw1", "b.txt")
    );
    let yaml = config_with(&extra, &["- exec: $fwd"]);
    let error = fixture
        .compile(&yaml)
        .map(|_| ())
        .expect_err("duplicate tag rejected");
    assert!(error.reason.contains("sw1"), "unexpected reason: {error}");
}

#[test]
fn empty_missing_or_malformed_initial_value_is_rejected() {
    let fixture = Fixture::new("bad-initial");
    for (name, args) in [
        ("missing", "  - tag: sw1\n    type: switch1\n    args: {}\n"),
        (
            "empty",
            "  - tag: sw1\n    type: switch1\n    args:\n      initial_value: \"\"\n",
        ),
        (
            "non-string",
            "  - tag: sw1\n    type: switch1\n    args:\n      initial_value: 42\n",
        ),
    ] {
        let yaml = config_with(args, &["- exec: $fwd"]);
        let error = expect_config_error(
            fixture.compile(&yaml),
            &format!("{name} initial_value must be rejected"),
        );
        assert!(
            error.path.contains("args"),
            "unexpected path for {name}: {error}"
        );
    }
}

#[test]
fn duplicate_state_file_between_switches_is_rejected() {
    let fixture = Fixture::new("dup-path");
    let extra = format!(
        "{}{}",
        switch_plugin(1, "sw1", "shared.txt"),
        switch_plugin(16, "sw16", "shared.txt")
    );
    let yaml = config_with(&extra, &["- exec: $fwd"]);
    let error = fixture
        .compile(&yaml)
        .map(|_| ())
        .expect_err("duplicate path rejected");
    assert!(
        error.reason.contains("shared.txt"),
        "unexpected reason: {error}"
    );
}

#[test]
fn switch_state_file_colliding_with_known_artifacts_is_rejected() {
    // domain_set rule file
    let fixture = Fixture::new("collide-ds");
    std::fs::write(fixture.path("rules.txt"), "example.com\n").expect("rule file");
    let extra = format!(
        "  - tag: ds\n    type: domain_set\n    args:\n      files:\n        - \"{}\"\n{}",
        fixture.path("rules.txt").display(),
        switch_plugin(1, "sw1", "rules.txt")
    );
    let yaml = config_with(&extra, &["- exec: $fwd"]);
    let error = fixture
        .compile(&yaml)
        .map(|_| ())
        .expect_err("domain_set collision rejected");
    assert!(error.reason.contains("rules.txt"), "unexpected: {error}");

    // cache dump target
    let fixture = Fixture::new("collide-cache");
    let extra = format!(
        "  - tag: c1\n    type: cache\n    args:\n      dump_file: \"dump.bin\"\n{}",
        switch_plugin(1, "sw1", "dump.bin")
    );
    let yaml = config_with(&extra, &["- exec: $fwd"]);
    let error = fixture
        .compile(&yaml)
        .map(|_| ())
        .expect_err("cache dump collision rejected");
    assert!(error.reason.contains("dump.bin"), "unexpected: {error}");

    // included file
    let fixture = Fixture::new("collide-include");
    let included = fixture.path("inc.yaml");
    std::fs::write(
        &included,
        "plugins:\n  - tag: ds2\n    type: domain_set\n    args:\n      exps:\n        - \
         \"example.org\"\n",
    )
    .expect("included");
    let yaml = config_with(&switch_plugin(1, "sw1", "inc.yaml"), &["- exec: $fwd"]).replace(
        "plugins:\n",
        &format!("include:\n  - \"{}\"\nplugins:\n", included.display()),
    );
    let error = fixture
        .compile(&yaml)
        .map(|_| ())
        .expect_err("include collision rejected");
    assert!(error.reason.contains("inc.yaml"), "unexpected: {error}");

    // the main configuration file itself
    let fixture = Fixture::new("collide-main");
    let main = fixture.path("main.yaml");
    std::fs::write(
        &main,
        config_with(&switch_plugin(1, "sw1", "main.yaml"), &["- exec: $fwd"]),
    )
    .expect("main yaml");
    let error = load_and_compile(&main)
        .map(|_| ())
        .expect_err("main config collision rejected");
    assert!(error.reason.contains("main.yaml"), "unexpected: {error}");
}

#[cfg(unix)]
#[test]
fn hardlinked_state_files_are_rejected_as_inode_aliases() {
    let fixture = Fixture::new("hardlink");
    std::fs::write(fixture.path("a.txt"), "A\n").expect("a");
    std::fs::hard_link(fixture.path("a.txt"), fixture.path("b.txt")).expect("hard link");
    let extra = format!(
        "{}{}",
        switch_plugin(1, "sw1", "a.txt"),
        switch_plugin(2, "sw2", "b.txt")
    );
    let yaml = config_with(&extra, &["- exec: $fwd"]);
    let error = fixture
        .compile(&yaml)
        .map(|_| ())
        .expect_err("inode alias rejected");
    assert!(error.reason.contains("b.txt"), "unexpected: {error}");
}

#[test]
fn compilation_creates_no_state_files_or_directories() {
    let fixture = Fixture::new("no-files");
    let extra = switch_plugin(1, "sw1", "missing_dir/state1.txt");
    let yaml = config_with(&extra, &["- exec: $fwd"]);
    let config = fixture
        .compile(&yaml)
        .expect("compiling must not require the state file");
    assert_eq!(config.switches.len(), 1);
    assert!(!fixture.path("missing_dir").exists());
    assert!(!fixture.path("missing_dir/state1.txt").exists());
    assert!(!fixture.path("state1.txt").exists());
}

#[test]
fn named_switch_executable_is_a_no_op_that_consumes_one_dispatch_and_continues() {
    let fixture = Fixture::new("noop-exec");
    let with_switch = config_with(
        &switch_plugin(1, "sw1", "sw1.txt"),
        &["- exec: $sw1", "- exec: $fwd"],
    );
    let without_switch = config_with("", &["- exec: $fwd"]);
    let with_config = fixture.compile(&with_switch).expect("compile with switch");
    let without_config = fixture.compile(&without_switch).expect("compile without");

    let (terminal, with_budget) = run_to_terminal(&with_config, query_state(&[]), 64);
    assert!(matches!(terminal, Terminal::Dispatch));
    let (_, without_budget) = run_to_terminal(&without_config, query_state(&[]), 64);
    assert_eq!(with_budget + 1, without_budget);

    // The no-op leaves the query state untouched.
    let mut machine = with_config
        .new_machine(query_state(&[]), ExecutionControl::with_fuel(64))
        .expect("machine");
    let mut saw_dispatch = false;
    loop {
        match machine.step().expect("step") {
            MachineStep::Dispatch(_) => {
                saw_dispatch = true;
                break;
            }
            MachineStep::Complete(_) => break,
            MachineStep::ScopeComplete(_) | MachineStep::ScopeAborted(_) => {}
        }
    }
    assert!(saw_dispatch);
    let state = machine.state();
    assert_eq!(state.fast_flags, 0);
    assert!(state.marks.is_empty());
}

#[test]
fn bit_backed_a_matchers_read_the_live_query_fast_flag() {
    let fixture = Fixture::new("bit-a");
    for (type_number, bit) in [(1u8, 32u64), (14, 45), (16, 47), (17, 49)] {
        let yaml = config_with(
            &switch_plugin(type_number, &format!("sw{type_number}"), "state.txt"),
            &[
                &format!("- exec: fast_mark {bit}"),
                &format!("- matches: switch{type_number} A\n        exec: reject"),
                "- exec: $fwd",
            ],
        );
        let config = fixture.compile(&yaml).expect("compile");
        let (terminal, _) = run_to_terminal(&config, query_state(&[]), 64);
        assert!(
            matches!(terminal, Terminal::Rejected),
            "switch{type_number} A must match after fast_mark {bit}"
        );
    }

    // Wrong bits never satisfy a bit-backed A matcher.
    for (type_number, bit) in [(1u8, 46u64), (16, 46), (17, 48), (15, 49)] {
        let yaml = config_with(
            &switch_plugin(type_number, &format!("sw{type_number}"), "state.txt"),
            &[
                &format!("- exec: fast_mark {bit}"),
                &format!("- matches: switch{type_number} A\n        exec: reject"),
                "- exec: $fwd",
            ],
        );
        let config = fixture.compile(&yaml).expect("compile");
        let (terminal, _) = run_to_terminal(&config, query_state(&[]), 64);
        assert!(
            matches!(terminal, Terminal::Dispatch),
            "fast_mark {bit} must not satisfy switch{type_number} A"
        );
    }
}

#[test]
fn every_switch_type_uses_its_reserved_admission_bit() {
    let fixture = Fixture::new("bit-table");
    for type_number in 1..=14u8 {
        let bit = 31 + u64::from(type_number);
        let yaml = config_with(
            &switch_plugin(type_number, &format!("sw{type_number}"), "state.txt"),
            &[
                &format!("- exec: fast_mark {bit}"),
                &format!("- matches: switch{type_number} A\n        exec: reject"),
                "- exec: $fwd",
            ],
        );
        let config = fixture.compile(&yaml).expect("compile");
        let (terminal, _) = run_to_terminal(&config, query_state(&[]), 64);
        assert!(
            matches!(terminal, Terminal::Rejected),
            "switch{type_number} A must read bit {bit}"
        );
    }
}

#[test]
fn non_a_expectations_compare_the_admitted_value() {
    // switch2 is type 2, fact key 1.
    assert!(matches!(
        matcher_probe("switch2 B", &[(1, "B")], 0),
        Terminal::Rejected
    ));
    assert!(matches!(
        matcher_probe("switch2 B", &[(1, "C")], 0),
        Terminal::Dispatch
    ));
    assert!(matches!(
        matcher_probe("switch2 B", &[], 0),
        Terminal::Dispatch
    ));
    // Quoted and empty expectations survive the outer-quote trim.
    assert!(matches!(
        matcher_probe("switch2 'custom value'", &[(1, "custom value")], 0),
        Terminal::Rejected
    ));
    assert!(matches!(
        matcher_probe("switch2 \"\"", &[(1, "")], 0),
        Terminal::Rejected
    ));
    // Reversal composes with the ordinary engine reversal.
    assert!(matches!(
        matcher_probe("!switch2 B", &[(1, "B")], 0),
        Terminal::Dispatch
    ));
    assert!(matches!(
        matcher_probe("!switch2 B", &[(1, "C")], 0),
        Terminal::Rejected
    ));
}

#[test]
fn switch15_is_bitless_and_absent_owners_match_false() {
    // A declared switch15 compares values; no fast flag satisfies it.
    let fixture = Fixture::new("bitless");
    let yaml = config_with(
        &switch_plugin(15, "sw15", "state15.txt"),
        &[
            "- exec: fast_mark 46",
            "- exec: fast_mark 48",
            "- matches: switch15 A\n        exec: reject",
            "- exec: $fwd",
        ],
    );
    let config = fixture.compile(&yaml).expect("compile");
    let (terminal, _) = run_to_terminal(&config, query_state(&[]), 64);
    assert!(matches!(terminal, Terminal::Dispatch));

    let (terminal, _) = run_to_terminal(&config, query_state(&[(14, "A")]), 64);
    assert!(matches!(terminal, Terminal::Rejected));

    // Absent owners: non-A never matches, bit-backed A still reads the bit.
    assert!(matches!(
        matcher_probe("switch5 X", &[], 0),
        Terminal::Dispatch
    ));
    assert!(matches!(
        matcher_probe("!switch5 X", &[], 0),
        Terminal::Rejected
    ));
    assert!(matches!(
        matcher_probe("switch5 A", &[], 1 << 36),
        Terminal::Rejected
    ));
    // switch15 has no bit even when undeclared.
    assert!(matches!(
        matcher_probe("switch15 A", &[], 1 << 46),
        Terminal::Dispatch
    ));
}

#[test]
fn switch_matcher_and_executable_reference_errors() {
    let fixture = Fixture::new("ref-errors");

    // `$tag` matching against a declared switch is rejected.
    let yaml = config_with(
        &switch_plugin(1, "sw1", "sw1.txt"),
        &["- matches: $sw1\n        exec: reject", "- exec: $fwd"],
    );
    let error = fixture
        .compile(&yaml)
        .map(|_| ())
        .expect_err("named matcher rejected");
    assert!(error.reason.contains("sw1"), "unexpected reason: {error}");

    // A quick switch executable is rejected.
    let yaml = config_with(
        &switch_plugin(1, "sw1", "sw1.txt"),
        &["- exec: switch1", "- exec: $fwd"],
    );
    let error = fixture
        .compile(&yaml)
        .map(|_| ())
        .expect_err("quick exec rejected");
    assert!(
        error.reason.contains("switch1"),
        "unexpected reason: {error}"
    );

    // Types outside switch1..switch17 are unsupported matchers.
    let yaml = config_with(
        &switch_plugin(1, "sw1", "sw1.txt"),
        &[
            "- matches: switch18 A\n        exec: reject",
            "- exec: $fwd",
        ],
    );
    let error = fixture
        .compile(&yaml)
        .map(|_| ())
        .expect_err("switch18 rejected");
    assert!(
        error.reason.contains("switch18"),
        "unexpected reason: {error}"
    );
}

#[test]
fn noncanonical_switch_type_spellings_are_unsupported() {
    let fixture = Fixture::new("noncanonical");

    // Declarations: switch01/switch001 are not aliases of switch1.
    for kind in ["switch01", "switch001"] {
        let extra = format!(
            "  - tag: sw1\n    type: {kind}\n    args:\n      initial_value: \
             \"sw1.txt\"\n"
        );
        let yaml = config_with(&extra, &["- exec: $fwd"]);
        let error = expect_config_error(
            fixture.compile(&yaml),
            "noncanonical declaration must be rejected",
        );
        assert!(
            error.reason.contains("unsupported plugin type"),
            "{kind}: {error}"
        );
    }

    // Quick matchers: the same canonical rule applies.
    for matcher in ["switch01 A", "switch001 A"] {
        let yaml = config_with(
            &switch_plugin(1, "sw1", "sw1.txt"),
            &[
                &format!("- matches: {matcher}\n        exec: reject"),
                "- exec: $fwd",
            ],
        );
        let error = expect_config_error(
            fixture.compile(&yaml),
            "noncanonical matcher must be rejected",
        );
        assert!(
            error.reason.contains("unsupported matcher"),
            "{matcher}: {error}"
        );
    }

    // Executables reject them too.
    let yaml = config_with(
        &switch_plugin(1, "sw1", "sw1.txt"),
        &["- exec: switch01", "- exec: $fwd"],
    );
    let error = expect_config_error(
        fixture.compile(&yaml),
        "noncanonical executable must be rejected",
    );
    assert!(
        error.reason.contains("unsupported executable"),
        "switch01 exec: {error}"
    );
}

#[test]
fn admission_facts_survive_branch_and_sequence_calls() {
    let fixture = Fixture::new("branch");
    let yaml = config_with(
        &switch_plugin(2, "sw2", "sw2.txt"),
        &[
            "- exec: jump $child",
            "- matches: switch2 B\n        exec: reject",
            "- exec: $fwd",
        ],
    )
    .replace(
        "  - tag: srv\n",
        "  - tag: child\n    type: sequence\n    args:\n      - matches: switch2 B\n        \
         exec: accept\n  - tag: srv\n",
    );
    let config = fixture.compile(&yaml).expect("compile");
    // `accept` in the child completes the caller scope; the machine completes
    // without the entry reject firing, proving both scopes read the same
    // admitted value.
    let (terminal, _) = run_to_terminal(&config, query_state(&[(1, "B")]), 64);
    assert!(matches!(terminal, Terminal::Completed));
}

#[test]
fn mismatched_admission_values_fall_through_to_forwarding() {
    let fixture = Fixture::new("fallthrough");
    let yaml = config_with(
        &switch_plugin(2, "sw2", "sw2.txt"),
        &["- matches: switch2 B\n        exec: reject", "- exec: $fwd"],
    );
    let config = fixture.compile(&yaml).expect("compile");
    let (terminal, _) = run_to_terminal(&config, query_state(&[(1, "A")]), 64);
    assert!(matches!(terminal, Terminal::Dispatch));
}
