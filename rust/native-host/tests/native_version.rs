//! The version branch must run without config or runtime construction.
#[test]
fn version_is_a_bounded_standalone_command() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_mosdns"))
        .arg("version")
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!("{}\n", option_env!("MOSDNS_BUILD_VERSION").unwrap_or("dev"))
    );
    let extra = std::process::Command::new(env!("CARGO_BIN_EXE_mosdns"))
        .args(["version", "extra"])
        .output()
        .unwrap();
    assert!(!extra.status.success());
    assert!(extra.stdout.is_empty());
}
