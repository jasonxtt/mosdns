//! `ui-manifest.json` of an external mosdns UI package (a directory `<config root>/ui/<name>/`
//! that carries one): what the package is and whether this host can serve it. Pure parsing and
//! the compatibility rule; reading the file from the pinned directory is `external_ui.rs`'s job.
//! A directory without a manifest is not a mosdns UI package and is served as the generic static
//! mount it always was.

use serde::Deserialize;

/// The interface level this host serves (the `/api/...`, `/plugins/...`, `/metrics` and
/// capability surface the maintained UI talks to). Backward-compatible additions do not change
/// it (the UI degrades per `/api/v1/capabilities`); a breaking change raises it.
pub(crate) const HOST_API_LEVEL: u64 = 1;
/// The oldest UI interface level this host still serves.
pub(crate) const MIN_UI_API_LEVEL: u64 = 1;
/// The largest manifest read.
pub(crate) const MAX_MANIFEST_BYTES: u64 = 64 * 1024;

/// What a compatible manifest tells the operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UiInfo {
    pub(crate) ui_version: String,
    pub(crate) built_at: Option<String>,
}

#[derive(Deserialize)]
struct Manifest {
    manifest_schema: Option<u64>,
    ui_version: Option<String>,
    built_at: Option<String>,
    api_level: Option<u64>,
    requires_api_level: Option<u64>,
}

fn displayable(text: &str, limit: usize) -> bool {
    !text.trim().is_empty() && text.chars().count() <= limit && !text.chars().any(char::is_control)
}

/// Checks a manifest against this host. The error is a short fixed-form reason (shown to the
/// operator in the WebUI) that never repeats file contents beyond the numbers it compares.
pub(crate) fn evaluate(bytes: &[u8]) -> Result<UiInfo, String> {
    let manifest: Manifest = serde_json::from_slice(bytes)
        .map_err(|_| "ui-manifest.json 不是合法的 JSON，或字段类型不对".to_owned())?;
    if manifest.manifest_schema != Some(1) {
        return Err("ui-manifest.json：manifest_schema 必须为 1".to_owned());
    }
    let ui_version = manifest
        .ui_version
        .filter(|version| displayable(version, 64))
        .ok_or("ui-manifest.json：ui_version 缺失或无效")?;
    let built_at = match manifest.built_at {
        Some(text) if displayable(&text, 64) => Some(text),
        Some(_) => return Err("ui-manifest.json：built_at 无效".to_owned()),
        None => None,
    };
    let api_level = manifest
        .api_level
        .ok_or("ui-manifest.json：缺少 api_level")?;
    let requires = manifest.requires_api_level.unwrap_or(api_level);
    if requires > HOST_API_LEVEL {
        return Err(format!(
            "此 UI 需要接口级别 {requires}，当前程序只提供 {HOST_API_LEVEL}，请升级程序"
        ));
    }
    if api_level < MIN_UI_API_LEVEL {
        return Err(format!(
            "此 UI 面向接口级别 {api_level}，当前程序至少需要 {MIN_UI_API_LEVEL}，请换用新的 UI 包"
        ));
    }
    Ok(UiInfo {
        ui_version,
        built_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest(json: &str) -> Result<UiInfo, String> {
        evaluate(json.as_bytes())
    }

    #[test]
    fn a_compatible_manifest_reports_its_version() {
        let info = manifest(
            r#"{"manifest_schema":1,"ui_version":"v1.2.3","built_at":"2026-10-17T10:00:00Z","api_level":1,"requires_api_level":1,"extra":true}"#,
        )
        .unwrap();
        assert_eq!(info.ui_version, "v1.2.3");
        assert_eq!(info.built_at.as_deref(), Some("2026-10-17T10:00:00Z"));
        // `requires_api_level` defaults to `api_level`; `built_at` is optional.
        let info = manifest(r#"{"manifest_schema":1,"ui_version":"x","api_level":1}"#).unwrap();
        assert_eq!(info.built_at, None);
    }

    #[test]
    fn the_compatibility_rule_is_about_interface_levels() {
        let level = |api, requires| {
            manifest(&format!(
                r#"{{"manifest_schema":1,"ui_version":"x","api_level":{api},"requires_api_level":{requires}}}"#
            ))
        };
        assert!(level(HOST_API_LEVEL, HOST_API_LEVEL).is_ok());
        assert!(
            level(HOST_API_LEVEL + 3, HOST_API_LEVEL).is_ok(),
            "a newer UI that only needs today's level works"
        );
        let too_new = level(HOST_API_LEVEL + 1, HOST_API_LEVEL + 1).unwrap_err();
        assert!(too_new.contains("需要接口级别 2"), "{too_new}");
        let too_old = level(MIN_UI_API_LEVEL - 1, 0).unwrap_err();
        assert!(too_old.contains("面向接口级别 0"), "{too_old}");
    }

    #[test]
    fn broken_manifests_are_refused_with_fixed_reasons() {
        for (text, reason) in [
            ("not json", "不是合法的 JSON"),
            ("[]", "不是合法的 JSON"),
            (
                r#"{"manifest_schema":2,"ui_version":"x","api_level":1}"#,
                "manifest_schema 必须为 1",
            ),
            (
                r#"{"ui_version":"x","api_level":1}"#,
                "manifest_schema 必须为 1",
            ),
            (r#"{"manifest_schema":1,"api_level":1}"#, "ui_version 缺失"),
            (
                r#"{"manifest_schema":1,"ui_version":"  ","api_level":1}"#,
                "ui_version 缺失",
            ),
            (
                r#"{"manifest_schema":1,"ui_version":"a\nb","api_level":1}"#,
                "ui_version 缺失",
            ),
            (
                r#"{"manifest_schema":1,"ui_version":"x","api_level":1,"built_at":"\u0007"}"#,
                "built_at 无效",
            ),
            (
                r#"{"manifest_schema":1,"ui_version":"x"}"#,
                "缺少 api_level",
            ),
            (
                r#"{"manifest_schema":1,"ui_version":"x","api_level":"1"}"#,
                "不是合法的 JSON",
            ),
        ] {
            let error = manifest(text).unwrap_err();
            assert!(error.contains(reason), "{text}: {error}");
        }
        let long = format!(
            r#"{{"manifest_schema":1,"ui_version":"{}","api_level":1}}"#,
            "v".repeat(65)
        );
        assert!(manifest(&long).unwrap_err().contains("ui_version"));
    }
}
