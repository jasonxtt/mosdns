//! Slice 0 of the local-rule editing workflow: the Go-visible text-file rule
//! policy, managed-profile eligibility and the scoped management listen
//! address.
//!
//! These are public configuration-surface tests. They use real YAML and real
//! rule files on disk; no matcher, parser or loader is mocked.

use std::fs;
use std::path::PathBuf;

use mosdns_native_host::{CompiledConfig, ConfigError, compile_yaml_with_base, load_and_compile};

/// Unwraps a configuration rejection, because `CompiledConfig` carries no
/// `Debug` impl and therefore cannot be inspected by `expect_err`.
fn expect_config_error(result: Result<CompiledConfig, ConfigError>, message: &str) -> ConfigError {
    match result {
        Ok(_) => panic!("{message}"),
        Err(error) => error,
    }
}

fn expect_config(result: Result<CompiledConfig, ConfigError>, message: &str) -> CompiledConfig {
    match result {
        Ok(config) => config,
        Err(error) => panic!("{message}: {error}"),
    }
}

/// One owned fixture directory holding a config and its real rule files.
struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("mosdns-managed-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).expect("fixture directory");
        Self { root }
    }

    fn write(&self, relative: &str, contents: &str) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("fixture parent");
        }
        fs::write(&path, contents).expect("fixture file");
        path
    }

    fn write_bytes(&self, relative: &str, contents: &[u8]) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("fixture parent");
        }
        fs::write(&path, contents).expect("fixture file");
        path
    }

    fn config(&self) -> PathBuf {
        self.root.join("config.yaml")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Builds a minimal valid native configuration with the given extra plugin
/// blocks (already indented to the `plugins:` list level) and an optional
/// top-level `api:` block.
fn config_yaml(extra_plugins: &str, api: &str) -> String {
    let template = r#"log:
  level: error
__API__plugins:
  - tag: sequence_main
    type: sequence
    args:
      - exec: $forward_main
__EXTRA__  - tag: forward_main
    type: forward
    args:
      upstreams:
        - addr: "udp://127.0.0.1:25453"
  - tag: listener
    type: udp_server
    args:
      entry: sequence_main
      listen: "127.0.0.1:25353"
      enable_audit: false
"#;
    template
        .replace("__API__", api)
        .replace("__EXTRA__", extra_plugins)
}

/// One `domain_set` plugin block with a pre-indented `args:` body.
fn domain_set_plugin(tag: &str, args_body: &str) -> String {
    format!("  - tag: {tag}\n    type: domain_set\n    args:\n{args_body}")
}

fn compile(fixture: &Fixture, yaml: &str) -> Result<CompiledConfig, ConfigError> {
    compile_yaml_with_base(yaml, fixture.root.as_path())
}

#[test]
fn text_file_rules_follow_go_acceptance_and_skip_invalid_lines() {
    let fixture = Fixture::new("text-acceptance");
    fixture.write(
        "rules/managed.txt",
        "# whole-line comment\n   # indented comment\n\t\nvalid-one.example\nregexp:[\n  spaced.example  \nfoo.example#inline\nvalid-two.example\n",
    );
    let yaml = config_yaml(
        &domain_set_plugin("managed", "      files:\n        - rules/managed.txt\n"),
        "",
    );
    let config = expect_config(compile(&fixture, &yaml), "managed profile must load");
    let set = config
        .domain_set("managed")
        .expect("managed domain_set is registered");
    let managed = set
        .managed
        .as_ref()
        .expect("one .txt file with no exps is a managed profile");

    // Go trims outer whitespace, skips blank and whole-line `#` lines, keeps an
    // inline `#` as part of the candidate rule, and skips an invalid individual
    // rule without failing the load.
    assert_eq!(
        managed.rules(),
        vec![
            "valid-one.example".to_owned(),
            "spaced.example".to_owned(),
            "foo.example#inline".to_owned(),
            "valid-two.example".to_owned(),
        ],
        "accepted rules must match Go's text-file policy"
    );
    assert!(managed.file().ends_with("rules/managed.txt"), "{managed:?}");
    assert!(set.matches("valid-one.example"));
    assert!(set.matches("spaced.example"));
    assert!(
        !set.matches("foo.example"),
        "an inline `#` must not be treated as a comment that truncates the rule"
    );
    assert!(set.matches("valid-two.example"));
}

#[test]
fn a_rule_file_that_is_not_utf8_fails_with_its_path() {
    let fixture = Fixture::new("non-utf8");
    fixture.write_bytes(
        "rules/managed.txt",
        &[0x66, 0x6f, 0x6f, 0x2e, 0xff, 0xfe, 0x0a],
    );
    let yaml = config_yaml(
        &domain_set_plugin("managed", "      files:\n        - rules/managed.txt\n"),
        "",
    );
    let error = expect_config_error(
        compile(&fixture, &yaml),
        "a non-UTF8 rule file must not load silently",
    );
    assert!(error.reason.contains("managed.txt"), "{error}");
    assert!(error.path.contains("files[0]"), "{error}");
}

#[test]
fn missing_rule_file_fails_with_its_source_path() {
    let fixture = Fixture::new("missing-managed");
    let yaml = config_yaml(
        &domain_set_plugin("managed", "      files:\n        - rules/absent.txt\n"),
        "",
    );
    let error = expect_config_error(
        compile(&fixture, &yaml),
        "a configured but missing rule file must fail before bind",
    );
    assert!(error.reason.contains("absent.txt"), "{error}");
    assert!(error.path.contains("files[0]"), "{error}");
}

#[test]
fn invalid_exps_remain_a_strict_load_error() {
    let fixture = Fixture::new("strict-exps");
    let yaml = config_yaml(
        &domain_set_plugin("strict", "      exps:\n        - \"regexp:[\"\n"),
        "",
    );
    let error = expect_config_error(
        compile(&fixture, &yaml),
        "an invalid exps entry must stay fatal, unlike a file rule",
    );
    assert!(error.path.contains("exps[0]"), "{error}");
    assert!(
        error.reason.contains("regexp:["),
        "the offending expression must be reported: {error}"
    );
}

#[test]
fn domain_set_sets_remains_an_unsupported_load_error() {
    let fixture = Fixture::new("sets-unsupported");
    let yaml = config_yaml(
        &domain_set_plugin("legacy", "      sets:\n        - other_tag\n"),
        "",
    );
    let error = expect_config_error(
        compile(&fixture, &yaml),
        "`sets` must stay unsupported and is outside this task",
    );
    assert!(error.path.contains(".args.sets"), "{error}");
    assert!(error.reason.contains("unsupported"), "{error}");
}

#[test]
fn managed_profile_requires_exactly_one_txt_file() {
    let fixture = Fixture::new("eligibility");
    fixture.write("rules/one.txt", "one.example\n");
    fixture.write("rules/two.txt", "two.example\n");
    fixture.write("rules/plain.rules", "plain.example\n");

    // Exactly one `.txt` file and no `exps` is the only managed shape.
    let eligible = config_yaml(
        &domain_set_plugin("eligible", "      files:\n        - rules/one.txt\n"),
        "",
    );
    let config = expect_config(compile(&fixture, &eligible), "eligible profile");
    assert!(
        config
            .domain_set("eligible")
            .expect("tag")
            .managed
            .is_some(),
        "a single .txt file is a managed profile"
    );

    // A case-insensitive `.txt` extension is still Go's POST precondition.
    let upper = config_yaml(
        &domain_set_plugin("upper", "      files:\n        - rules/one.txt\n"),
        "",
    );
    fs::rename(
        fixture.root.join("rules/one.txt"),
        fixture.root.join("rules/one.TXT"),
    )
    .expect("rename");
    let upper = upper.replace("rules/one.txt", "rules/one.TXT");
    let config = expect_config(compile(&fixture, &upper), "uppercase .txt profile");
    assert!(
        config.domain_set("upper").expect("tag").managed.is_some(),
        "`.TXT` must be accepted like Go's EqualFold extension check"
    );

    // Query-only shapes stay loadable but are not management targets.
    for (tag, files) in [
        (
            "two_files",
            "      files:\n        - rules/one.TXT\n        - rules/two.txt\n",
        ),
        (
            "exps_and_file",
            "      exps:\n        - inline.example\n      files:\n        - rules/one.TXT\n",
        ),
        ("non_txt", "      files:\n        - rules/plain.rules\n"),
    ] {
        let yaml = config_yaml(&domain_set_plugin(tag, files), "");
        let config = expect_config(compile(&fixture, &yaml), "query-only shape must still load");
        assert!(
            config.domain_set(tag).expect("tag").managed.is_none(),
            "`{tag}` is not a single-.txt managed profile"
        );
    }
}

#[test]
fn two_managed_tags_may_not_share_one_rule_file() {
    let fixture = Fixture::new("shared-file");
    fixture.write("rules/shared.txt", "shared.example\n");
    let yaml = config_yaml(
        &format!(
            "{}{}",
            domain_set_plugin("first", "      files:\n        - rules/shared.txt\n"),
            domain_set_plugin("second", "      files:\n        - rules/shared.txt\n"),
        ),
        "",
    );
    let error = expect_config_error(
        compile(&fixture, &yaml),
        "two managed tags must not silently overwrite one file",
    );
    assert!(error.reason.contains("shared.txt"), "{error}");
}

#[test]
fn duplicate_domain_set_tags_are_rejected() {
    let fixture = Fixture::new("duplicate-tag");
    fixture.write("rules/one.txt", "one.example\n");
    let yaml = config_yaml(
        &format!(
            "{}{}",
            domain_set_plugin("same", "      files:\n        - rules/one.txt\n"),
            domain_set_plugin("same", "      files:\n        - rules/one.txt\n"),
        ),
        "",
    );
    let error = expect_config_error(compile(&fixture, &yaml), "duplicate tags must fail");
    assert!(error.reason.contains("duplicate"), "{error}");
}

#[test]
fn two_managed_tags_keep_independent_rules() {
    let fixture = Fixture::new("two-tags");
    fixture.write("rules/alpha.txt", "alpha.example\n");
    fixture.write("rules/beta.txt", "beta.example\n");
    let yaml = config_yaml(
        &format!(
            "{}{}",
            domain_set_plugin("alpha", "      files:\n        - rules/alpha.txt\n"),
            domain_set_plugin("beta", "      files:\n        - rules/beta.txt\n"),
        ),
        "",
    );
    let config = expect_config(compile(&fixture, &yaml), "two managed tags");
    let alpha = config.domain_set("alpha").expect("alpha");
    let beta = config.domain_set("beta").expect("beta");
    assert!(alpha.matches("alpha.example"));
    assert!(!alpha.matches("beta.example"));
    assert!(beta.matches("beta.example"));
    assert!(!beta.matches("alpha.example"));
    assert_eq!(
        alpha.managed.as_ref().expect("managed alpha").rules(),
        vec!["alpha.example".to_owned()]
    );
    assert_eq!(
        beta.managed.as_ref().expect("managed beta").rules(),
        vec!["beta.example".to_owned()]
    );
}

#[test]
fn a_managed_file_inside_an_include_resolves_from_the_included_directory() {
    let fixture = Fixture::new("included-managed");
    fixture.write("sub/rules/managed.txt", "included.example\n");
    fixture.write(
        "sub/routes.yaml",
        "plugins:\n  - tag: included_set\n    type: domain_set\n    args:\n      files:\n        - rules/managed.txt\n",
    );
    let yaml =
        config_yaml("", "").replace("plugins:\n", "include:\n  - sub/routes.yaml\nplugins:\n");
    let config = expect_config(
        load_and_compile(&write_config(&fixture, &yaml)),
        "an included managed rule file resolves from the included YAML directory",
    );
    let set = config.domain_set("included_set").expect("included tag");
    let managed = set.managed.as_ref().expect("managed included profile");
    assert_eq!(managed.rules(), vec!["included.example".to_owned()]);
    assert!(
        managed.file().ends_with("sub/rules/managed.txt"),
        "the managed file must resolve beside the included YAML: {managed:?}"
    );
}

fn write_config(fixture: &Fixture, yaml: &str) -> PathBuf {
    let path = fixture.config();
    fs::write(&path, yaml).expect("fixture config");
    path
}

#[test]
fn api_listen_address_is_optional_and_strictly_validated() {
    let fixture = Fixture::new("api-config");

    // A DNS-only configuration stays valid and exposes no management listener.
    let config = expect_config(
        compile(&fixture, &config_yaml("", "")),
        "a DNS-only configuration must keep compiling",
    );
    assert!(config.api.is_none(), "no `api` block means no HTTP owner");

    let with_api = config_yaml("", "api:\n  http: \"127.0.0.1:25354\"\n");
    let config = expect_config(compile(&fixture, &with_api), "scoped api block");
    let api = config.api.as_ref().expect("api listener");
    assert_eq!(api.http.to_string(), "127.0.0.1:25354");

    for (name, api_block) in [
        ("missing-http", "api:\n  listen: \"127.0.0.1:25354\"\n"),
        (
            "unknown-field",
            "api:\n  http: \"127.0.0.1:25354\"\n  tls: true\n",
        ),
        ("not-a-map", "api: \"127.0.0.1:25354\"\n"),
        ("bad-address", "api:\n  http: \"not-an-address\"\n"),
        ("empty-address", "api:\n  http: \"\"\n"),
        ("port-only", "api:\n  http: \"25354\"\n"),
    ] {
        let yaml = config_yaml("", api_block);
        let error = expect_config_error(
            compile(&fixture, &yaml),
            &format!("unsupported api shape `{name}` must fail"),
        );
        assert!(
            error.path.starts_with("$.api"),
            "`{name}` error must name the api path: {error}"
        );
    }
}
