//! Online update of the native program and its configuration package.
//!
//! Releases come from GitHub Releases of `jasonxtt/mosdns`, tags `rust-vX.Y.Z` only. An
//! update downloads the program and, when the release needs a newer configuration
//! package, that package; verifies every sha256; pre-checks the candidate; and then
//! hands over to a guard process (`update-guard`, see `guard.rs`) that replaces the
//! files, runs the candidate as its child and rolls everything back when the candidate
//! is not healthy in time. The design and its failure analysis are in
//! `docs/rust/contracts/native-update.md`.

pub(crate) mod api;
pub(crate) mod archive;
pub(crate) mod fsio;
pub(crate) mod guard;
pub(crate) mod package;
pub(crate) mod transaction;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use mosdns_upstream_core::TransportCancellation;
use serde::{Deserialize, Serialize};

/// The configuration package schema this program requires; bump together with
/// `REQUIRED_CONFIG_PACKAGE_ID` (and the package build script) when the structure of
/// the managed configuration files changes. Binary-only releases leave both alone.
pub(crate) const REQUIRED_CONFIG_SCHEMA: u32 = 1;
pub(crate) const REQUIRED_CONFIG_PACKAGE_ID: &str = "rust-config-schema-1";
/// The `channel` of the configuration package and of the update manifest.
pub(crate) const CHANNEL: &str = "rust";
/// The release asset holding the executable (a `tar.gz` with a file named `mosdns`).
pub(crate) const RELEASE_ASSET: &str = "mosdns-rust-linux-amd64.tar.gz";
/// The release asset naming every artifact's sha256 and the configuration package.
pub(crate) const MANIFEST_ASSET: &str = "mosdns-rust-update-manifest.json";
const DEFAULT_API_URL: &str = "https://api.github.com/repos/jasonxtt/mosdns/releases?per_page=100";
const RELEASE_PAGE: &str = "https://github.com/jasonxtt/mosdns/releases/tag/";
const TAG_PREFIX: &str = "rust-v";
const CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);

pub(crate) const MAX_LISTING_BYTES: usize = 8 << 20;
pub(crate) const MAX_MANIFEST_BYTES: usize = 1 << 20;
pub(crate) const MAX_ARCHIVE_BYTES: usize = 64 << 20;
pub(crate) const MAX_BINARY_BYTES: u64 = 128 << 20;
pub(crate) const MAX_CONFIG_PACKAGE_BYTES: usize = 64 << 20;
pub(crate) const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(300);

/// The release listing endpoint; `MOSDNS_UPDATE_API_URL` replaces it (tests, mirrors).
/// Nothing downloaded from anywhere skips the sha256 checks.
pub(crate) fn api_url() -> String {
    std::env::var("MOSDNS_UPDATE_API_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_API_URL.to_owned())
}

/// The architecture the updater supports.
pub(crate) fn architecture() -> &'static str {
    "linux/amd64"
}

/// Whether this build can install an update at all.
pub(crate) fn platform_supported() -> bool {
    cfg!(all(target_os = "linux", target_arch = "x86_64"))
}

/// `X.Y.Z` of a `rust-vX.Y.Z` tag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Version(u64, u64, u64);

/// Parses `rust-vX.Y.Z` (and nothing else: Go's `v…`/`lite-v…` tags are not ours).
pub(crate) fn parse_tag(tag: &str) -> Option<Version> {
    let lowered = tag.trim().to_ascii_lowercase();
    let rest = lowered.strip_prefix(TAG_PREFIX)?;
    let mut parts = rest.split('.');
    let mut next = || {
        let part = parts.next()?;
        if part.is_empty() || part.len() > 9 || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        part.parse::<u64>().ok()
    };
    let version = Version(next()?, next()?, next()?);
    parts.next().is_none().then_some(version)
}

/// Lower-case hex SHA-256.
pub(crate) fn sha256_hex(data: &[u8]) -> String {
    use std::fmt::Write;
    let digest = ring::digest::digest(&ring::digest::SHA256, data);
    digest.as_ref().iter().fold(String::new(), |mut out, byte| {
        let _ = write!(out, "{byte:02x}");
        out
    })
}

/// SHA-256 of a file, read in pieces.
pub(crate) fn file_sha256(path: &Path) -> std::io::Result<String> {
    use std::fmt::Write;
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut context = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buffer = vec![0_u8; 1 << 16];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        context.update(&buffer[..read]);
    }
    Ok(context
        .finish()
        .as_ref()
        .iter()
        .fold(String::new(), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        }))
}

/// 64 hexadecimal characters.
pub(crate) fn valid_sha256(value: &str) -> bool {
    let value = value.trim();
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// A release as GitHub's list API reports it (only what is used).
#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Release {
    #[serde(default)]
    pub(crate) tag_name: String,
    #[serde(default)]
    pub(crate) published_at: Option<String>,
    #[serde(default)]
    pub(crate) draft: bool,
    #[serde(default)]
    pub(crate) prerelease: bool,
    #[serde(default)]
    pub(crate) assets: Vec<Asset>,
}

#[derive(Clone, Debug, Default, Deserialize)]
pub(crate) struct Asset {
    #[serde(default)]
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) browser_download_url: String,
    #[serde(default)]
    pub(crate) updated_at: Option<String>,
    /// `sha256:<hex>` when GitHub reports it.
    #[serde(default)]
    pub(crate) digest: Option<String>,
}

impl Release {
    pub(crate) fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|asset| asset.name == name)
    }
}

/// The newest non-draft, non-prerelease `rust-v*` release.
pub(crate) fn latest_release(releases: Vec<Release>) -> Option<(Version, Release)> {
    releases
        .into_iter()
        .filter(|release| !release.draft && !release.prerelease)
        .filter_map(|release| parse_tag(&release.tag_name).map(|version| (version, release)))
        .max_by_key(|(version, _)| *version)
}

/// `mosdns-rust-update-manifest.json`.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateManifest {
    pub(crate) format: u32,
    pub(crate) channel: String,
    pub(crate) version: String,
    pub(crate) required_config_schema: u32,
    #[serde(default)]
    pub(crate) config_package_id: String,
    pub(crate) artifacts: BTreeMap<String, Artifact>,
    #[serde(default)]
    pub(crate) config: Option<ConfigArtifact>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Artifact {
    pub(crate) sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConfigArtifact {
    pub(crate) url: String,
    pub(crate) sha256: String,
}

/// Parses and checks the manifest of the release `tag`.
pub(crate) fn parse_manifest(data: &[u8], tag: &str) -> Result<UpdateManifest, String> {
    let manifest: UpdateManifest =
        serde_json::from_slice(data).map_err(|error| format!("更新清单无法解析: {error}"))?;
    if manifest.format != 1 {
        return Err(format!("不支持的更新清单格式 {}", manifest.format));
    }
    if manifest.channel != CHANNEL {
        return Err(format!(
            "更新清单通道 {:?} 不是 {CHANNEL:?}",
            manifest.channel
        ));
    }
    if manifest.version != tag {
        return Err(format!(
            "更新清单版本 {:?} 与目标版本 {tag:?} 不一致",
            manifest.version
        ));
    }
    if manifest.required_config_schema > 0 && manifest.config_package_id.trim().is_empty() {
        return Err("更新清单缺少配置包 ID".to_owned());
    }
    let artifact = manifest
        .artifacts
        .get(RELEASE_ASSET)
        .ok_or_else(|| format!("更新清单未包含二进制资产 {RELEASE_ASSET:?}"))?;
    if !valid_sha256(&artifact.sha256) {
        return Err(format!("二进制资产 {RELEASE_ASSET:?} 的 SHA-256 无效"));
    }
    if let Some(config) = &manifest.config {
        if config.url.trim().is_empty() || !valid_sha256(&config.sha256) {
            return Err("更新清单中的配置资产无效".to_owned());
        }
    }
    Ok(manifest)
}

/// The `update` object of the health document: `idle`, `pending` (a transaction is being
/// carried out) or `rolled_back` (the last update was undone, with the reason).
pub(crate) fn health_summary(root: &Path) -> serde_json::Value {
    if let Some(tx) = transaction::pending(root) {
        return serde_json::json!({"state": "pending", "target_version": tx.target_version});
    }
    match transaction::latest_rollback(root) {
        Some(reason) => serde_json::json!({"state": "rolled_back", "last_error": reason}),
        None => serde_json::json!({"state": "idle"}),
    }
}

/// What `GET /api/v1/update/status` returns (Go's `UpdateStatus`; empty optional fields
/// are left out, as there). `amd64_v3_capable`/`current_is_v3` do not exist here: the
/// native build has no GOAMD64 levels.
#[derive(Clone, Debug, Default, Serialize)]
pub(crate) struct UpdateStatus {
    pub(crate) current_version: String,
    pub(crate) latest_version: String,
    pub(crate) release_url: String,
    pub(crate) architecture: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) asset_name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) download_url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) asset_signature: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) current_signature: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) published_at: Option<String>,
    pub(crate) checked_at: String,
    pub(crate) cache_expires_at: String,
    pub(crate) update_available: bool,
    pub(crate) cached: bool,
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) config_auto_updated: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) config_schema_required: u32,
    #[serde(skip_serializing_if = "is_zero")]
    pub(crate) config_schema_applied: u32,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) config_update_status: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) config_update_message: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) config_update_error: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) config_update_backup: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) config_package_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) message: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) pending_restart: bool,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub(crate) rollback_performed: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub(crate) rollback_message: String,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

pub(crate) fn now_text() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

fn after_text(delay: Duration) -> String {
    (time::OffsetDateTime::now_utc() + delay)
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default()
}

/// The signature GitHub lets us build for an asset: its digest, else its update time.
fn asset_signature(asset: &Asset) -> String {
    if let Some(digest) = asset
        .digest
        .as_deref()
        .and_then(|d| d.strip_prefix("sha256:"))
    {
        if valid_sha256(digest) {
            return format!("{}:{}", asset.name, digest.to_ascii_lowercase());
        }
    }
    match &asset.updated_at {
        Some(updated) => format!("{}:{updated}", asset.name),
        None => asset.browser_download_url.clone(),
    }
}

/// Where a check goes and how it connects.
pub(crate) struct CheckEnv<'a> {
    /// `host:port` of the configured SOCKS5 proxy, empty for direct.
    pub(crate) socks5: &'a str,
    /// The configuration root (for `installed.json`), when there is one.
    pub(crate) root: Option<&'a Path>,
    pub(crate) cancel: &'a TransportCancellation,
}

struct Cached {
    at: Instant,
    status: UpdateStatus,
    release: Release,
}

/// The process-wide update state: the 15-minute check cache and the single-apply guard.
pub(crate) struct Manager {
    cache: Mutex<Option<Cached>>,
    busy: std::sync::atomic::AtomicBool,
}

pub(crate) fn manager() -> &'static Manager {
    static MANAGER: std::sync::OnceLock<Manager> = std::sync::OnceLock::new();
    MANAGER.get_or_init(|| Manager {
        cache: Mutex::new(None),
        busy: std::sync::atomic::AtomicBool::new(false),
    })
}

/// Releases the single-apply guard when dropped.
pub(crate) struct ApplyGuard(&'static Manager);

impl Drop for ApplyGuard {
    fn drop(&mut self) {
        self.0
            .busy
            .store(false, std::sync::atomic::Ordering::SeqCst);
    }
}

impl Manager {
    /// At most one apply at a time in this process.
    pub(crate) fn begin_apply(&'static self) -> Option<ApplyGuard> {
        // `then`, not `then_some`: a refused caller must not build (and drop) a guard, which
        // would clear the flag the running apply still holds.
        (!self.busy.swap(true, std::sync::atomic::Ordering::SeqCst)).then(|| ApplyGuard(self))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Cached>> {
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The newest `rust-v*` release, with the 15-minute cache unless `force`. Returns the
    /// status and the release it describes.
    pub(crate) async fn check(
        &self,
        env: &CheckEnv<'_>,
        force: bool,
    ) -> Result<(UpdateStatus, Release), String> {
        if !force {
            if let Some(cached) = self.lock().as_ref().filter(|c| c.at.elapsed() < CACHE_TTL) {
                let mut status = cached.status.clone();
                status.cached = true;
                status.checked_at = now_text();
                status.cache_expires_at = after_text(CACHE_TTL.saturating_sub(cached.at.elapsed()));
                return Ok((status, cached.release.clone()));
            }
        }
        let releases = fetch_releases(env).await?;
        let (version, release) =
            latest_release(releases).ok_or_else(|| "未找到 rust-v* 发布版本".to_owned())?;
        let current_tag = crate::build_identity::VERSION;
        let current = parse_tag(current_tag);
        let mut status = UpdateStatus {
            current_version: current_tag.to_owned(),
            latest_version: release.tag_name.clone(),
            release_url: format!("{RELEASE_PAGE}{}", release.tag_name),
            architecture: architecture().to_owned(),
            published_at: release.published_at.clone(),
            checked_at: now_text(),
            cache_expires_at: after_text(CACHE_TTL),
            current_signature: env
                .root
                .map(transaction::installed_signature)
                .unwrap_or_default(),
            ..UpdateStatus::default()
        };
        match release.asset(RELEASE_ASSET) {
            Some(asset) => {
                status.asset_name = asset.name.clone();
                status.download_url = asset.browser_download_url.clone();
                status.asset_signature = asset_signature(asset);
            }
            None => {
                status.message = format!("未找到适用于 {} 的安装包", architecture());
            }
        }
        status.update_available = match current {
            Some(current) => version > current,
            None => true,
        };
        if !platform_supported() {
            status.update_available = false;
            status.message = format!("此平台不支持在线更新（仅 {}）", architecture());
        }
        *self.lock() = Some(Cached {
            at: Instant::now(),
            status: status.clone(),
            release: release.clone(),
        });
        Ok((status, release))
    }
}

async fn fetch_releases(env: &CheckEnv<'_>) -> Result<Vec<Release>, String> {
    let body = download(env.socks5, &api_url(), MAX_LISTING_BYTES, env.cancel).await?;
    serde_json::from_slice::<Vec<Release>>(&body)
        .map_err(|error| format!("发布列表无法解析: {error}"))
}

/// One bounded download through the configured proxy, then (when the proxy fails) once
/// directly, as the reference does.
pub(crate) async fn download(
    socks5: &str,
    url: &str,
    max_bytes: usize,
    cancel: &TransportCancellation,
) -> Result<Vec<u8>, String> {
    match attempt(socks5, url, max_bytes, cancel).await {
        Ok(body) => Ok(body),
        Err(error) if !socks5.is_empty() => {
            crate::diag::warn_line!("update: download via proxy failed ({error}); trying directly");
            attempt("", url, max_bytes, cancel).await
        }
        Err(error) => Err(error),
    }
}

async fn attempt(
    socks5: &str,
    url: &str,
    max_bytes: usize,
    cancel: &TransportCancellation,
) -> Result<Vec<u8>, String> {
    crate::rule_download::fetch(
        crate::rule_download::Fetch {
            url,
            socks5,
            timeout: if max_bytes > MAX_LISTING_BYTES {
                DOWNLOAD_TIMEOUT
            } else {
                FETCH_TIMEOUT
            },
            max_bytes,
            roots: None,
        },
        cancel,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_second_apply_does_not_free_the_first() {
        static LOCAL: Manager = Manager {
            cache: Mutex::new(None),
            busy: std::sync::atomic::AtomicBool::new(false),
        };
        let first = LOCAL.begin_apply().expect("free");
        assert!(LOCAL.begin_apply().is_none(), "second is refused");
        assert!(LOCAL.begin_apply().is_none(), "and so is a third");
        drop(first);
        assert!(
            LOCAL.begin_apply().is_some(),
            "free again once the first is done"
        );
    }

    #[test]
    fn only_rust_tags_parse_and_compare_numerically() {
        assert_eq!(parse_tag("rust-v0.1.0"), Some(Version(0, 1, 0)));
        assert_eq!(parse_tag("  RUST-V10.2.30 "), Some(Version(10, 2, 30)));
        assert!(parse_tag("rust-v0.10.0") > parse_tag("rust-v0.9.9"));
        for bad in [
            "v0.7.1",
            "lite-v1.0.0",
            "rust-v1.0",
            "rust-v1.0.0.1",
            "rust-v1.0.x",
            "rust-v-1.0.0",
            "rust-v1.0.0-rc1",
            "rust-v",
            "rust-1.0.0",
            "",
            "rust-v1..0",
            "rust-v1.0.+1",
        ] {
            assert_eq!(parse_tag(bad), None, "{bad}");
        }
    }

    fn release(tag: &str, draft: bool, prerelease: bool) -> Release {
        Release {
            tag_name: tag.to_owned(),
            draft,
            prerelease,
            ..Release::default()
        }
    }

    #[test]
    fn the_newest_published_rust_release_wins_and_go_releases_are_ignored() {
        let picked = latest_release(vec![
            release("v9.9.9", false, false),
            release("rust-v0.1.0", false, false),
            release("rust-v0.3.0", true, false),
            release("rust-v0.2.0", false, true),
            release("rust-v0.1.5", false, false),
            release("lite-v5.0.0", false, false),
        ])
        .unwrap();
        assert_eq!(picked.1.tag_name, "rust-v0.1.5");
        assert!(latest_release(vec![release("v1.0.0", false, false)]).is_none());
        assert!(latest_release(Vec::new()).is_none());
    }

    fn manifest_json(version: &str, channel: &str) -> String {
        format!(
            r#"{{"format":1,"channel":"{channel}","version":"{version}","required_config_schema":1,
            "config_package_id":"rust-config-schema-1",
            "artifacts":{{"{RELEASE_ASSET}":{{"sha256":"{}"}}}},
            "config":{{"url":"http://x/config.zip","sha256":"{}"}}}}"#,
            "a".repeat(64),
            "b".repeat(64)
        )
    }

    #[test]
    fn the_manifest_must_match_the_release_and_carry_valid_digests() {
        let ok = parse_manifest(
            manifest_json("rust-v1.2.3", "rust").as_bytes(),
            "rust-v1.2.3",
        )
        .unwrap();
        assert_eq!(ok.required_config_schema, 1);
        assert!(
            parse_manifest(
                manifest_json("rust-v1.2.4", "rust").as_bytes(),
                "rust-v1.2.3"
            )
            .unwrap_err()
            .contains("不一致")
        );
        assert!(
            parse_manifest(
                manifest_json("rust-v1.2.3", "main").as_bytes(),
                "rust-v1.2.3"
            )
            .unwrap_err()
            .contains("通道")
        );
        let bad_digest = manifest_json("rust-v1.2.3", "rust").replace(&"a".repeat(64), "zz");
        assert!(parse_manifest(bad_digest.as_bytes(), "rust-v1.2.3").is_err());
        let unknown = manifest_json("rust-v1.2.3", "rust").replacen('{', r#"{"surprise":1,"#, 1);
        assert!(parse_manifest(unknown.as_bytes(), "rust-v1.2.3").is_err());
        let no_asset = manifest_json("rust-v1.2.3", "rust").replace(RELEASE_ASSET, "other.tar.gz");
        assert!(
            parse_manifest(no_asset.as_bytes(), "rust-v1.2.3")
                .unwrap_err()
                .contains("未包含")
        );
        let format2 = manifest_json("rust-v1.2.3", "rust").replace("\"format\":1", "\"format\":2");
        assert!(parse_manifest(format2.as_bytes(), "rust-v1.2.3").is_err());
        assert!(parse_manifest(b"not json", "rust-v1.2.3").is_err());
    }

    #[test]
    fn sha256_helpers_agree_with_the_known_digest() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(valid_sha256(&"A".repeat(64)));
        assert!(!valid_sha256(&"a".repeat(63)));
        assert!(!valid_sha256(&"g".repeat(64)));
        let path = std::env::temp_dir().join(format!("update-sha-{}", std::process::id()));
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(file_sha256(&path).unwrap(), sha256_hex(b"abc"));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn a_status_leaves_out_empty_optional_fields_like_the_reference() {
        let status = UpdateStatus {
            current_version: "rust-v0.1.0".to_owned(),
            architecture: "linux/amd64".to_owned(),
            ..UpdateStatus::default()
        };
        let value = serde_json::to_value(&status).unwrap();
        let keys: Vec<_> = value.as_object().unwrap().keys().cloned().collect();
        for key in [
            "current_version",
            "latest_version",
            "release_url",
            "architecture",
            "checked_at",
            "cache_expires_at",
            "update_available",
            "cached",
        ] {
            assert!(keys.contains(&key.to_owned()), "{key}");
        }
        for key in [
            "asset_name",
            "pending_restart",
            "rollback_performed",
            "message",
            "published_at",
        ] {
            assert!(!keys.contains(&key.to_owned()), "{key}");
        }
    }
}
