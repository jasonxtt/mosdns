//! `/api/v1/update/*` and `/api/v1/config/{export,update_from_url}`.
//!
//! Routes, status codes and response keys follow the reference (`coremain/api_update.go`,
//! `config_manager.go`); the differences are the safety rules of `native-update.md`:
//! only the host's own configuration directory is exported or replaced, a configuration
//! update must be a valid package, and everything goes through the guarded transaction.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::Duration;

use mosdns_upstream_core::TransportCancellation;
use serde::Deserialize;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

use super::archive::{ZipWriter, extract_tar_gz_file};
use super::package::{self, Package};
use super::transaction::{self, StageInput, Transaction};
use super::{
    CheckEnv, MANIFEST_ASSET, MAX_ARCHIVE_BYTES, MAX_BINARY_BYTES, MAX_CONFIG_PACKAGE_BYTES,
    MAX_MANIFEST_BYTES, RELEASE_ASSET, REQUIRED_CONFIG_PACKAGE_ID, REQUIRED_CONFIG_SCHEMA,
    UpdateStatus, manager,
};
use crate::config::CompiledConfig;
use crate::rule_sources::Reply;
use crate::runtime_snapshot::RuntimeControl;

const STATUS_LIMIT: Duration = Duration::from_secs(3);
const CHECK_LIMIT: Duration = Duration::from_secs(8);
const APPLY_CHECK_LIMIT: Duration = Duration::from_secs(25);
/// The guard starts this long after the answer, so the answer is sent first.
const GUARD_DELAY: Duration = Duration::from_millis(750);
const HEALTH_TIMEOUT_SECS: u64 = 60;
const MAX_EXPORT_BYTES: u64 = 128 << 20;
const MAX_EXPORT_FILES: usize = 100_000;
const MAX_EXPORT_DEPTH: usize = 16;

/// One export at a time: each builds the whole zip in memory (up to `MAX_EXPORT_BYTES`) and
/// the buffer stays until a slow client has read it, which many requests together could
/// turn into an out-of-memory kill of the DNS service.
static EXPORT_SLOT: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(1)));

tokio::task_local! {
    /// Permits that must outlive the handler: the management server scopes this around a
    /// connection (one request each), so a permit pushed here is released only after the
    /// response has been written.
    pub(crate) static HOLD: RefCell<Vec<OwnedSemaphorePermit>>;
}

/// Keeps `permit` until the connection ends (released at once outside a connection).
fn hold_until_connection_ends(permit: OwnedSemaphorePermit) {
    let _ = HOLD.try_with(|held| held.borrow_mut().push(permit));
}

/// How long the guard waits for the candidate's health; `MOSDNS_UPDATE_HEALTH_TIMEOUT_SECS`
/// (1–600) shortens it for tests.
fn health_timeout() -> u64 {
    std::env::var("MOSDNS_UPDATE_HEALTH_TIMEOUT_SECS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| (1..=600).contains(value))
        .unwrap_or(HEALTH_TIMEOUT_SECS)
}

/// What the handlers need from the running host.
pub(crate) struct Context<'a> {
    pub(crate) control: Option<&'a RuntimeControl>,
    pub(crate) config: &'a CompiledConfig,
    /// The configuration root (the directory of `webinfo/`).
    pub(crate) root: Option<&'a Path>,
    pub(crate) shutdown: &'a TransportCancellation,
}

fn json(status: u16, value: &impl serde::Serialize) -> Reply {
    let mut body = serde_json::to_vec(value).unwrap_or_default();
    body.push(b'\n');
    Reply {
        status,
        content_type: Some("application/json"),
        body,
        headers: Vec::new(),
    }
}

fn error(status: u16, message: impl std::fmt::Display) -> Reply {
    json(status, &serde_json::json!({ "error": message.to_string() }))
}

/// `http.Error`: plain text with a final newline.
fn text_error(status: u16, message: impl std::fmt::Display) -> Reply {
    Reply {
        status,
        content_type: Some("text/plain; charset=utf-8"),
        body: format!("{message}\n").into_bytes(),
        headers: Vec::new(),
    }
}

fn not_allowed() -> Reply {
    text_error(405, "method not allowed")
}

/// Why the updater cannot run here (shown as the capability's reason), if it cannot.
pub(crate) fn update_unsupported(root_known: bool) -> Option<&'static str> {
    if !super::platform_supported() {
        return Some("此平台不支持在线更新（仅 linux/amd64）");
    }
    config_management_unsupported(root_known)
}

/// Why configuration export / update from a URL cannot run here, if they cannot.
pub(crate) fn config_management_unsupported(root_known: bool) -> Option<&'static str> {
    if !root_known {
        return Some("此主机没有配置目录，无法管理配置");
    }
    if crate::restart::target().is_none() {
        return Some("此进程没有记录可执行文件路径，无法在线更新或应用配置");
    }
    None
}

/// The update and configuration routes, or `None` when `path` is not one of them.
pub(crate) async fn handle(
    ctx: &Context<'_>,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Reply> {
    Some(match (path, method) {
        ("/api/v1/update/status", "GET") => status(ctx, false).await,
        ("/api/v1/update/check", "POST") => status(ctx, true).await,
        ("/api/v1/update/apply", "POST") => apply(ctx, body).await,
        ("/api/v1/config/export", "POST") => export(ctx, body).await,
        ("/api/v1/config/update_from_url", "POST") => update_from_url(ctx, body).await,
        (
            "/api/v1/update/status"
            | "/api/v1/update/check"
            | "/api/v1/update/apply"
            | "/api/v1/config/export"
            | "/api/v1/config/update_from_url",
            _,
        ) => not_allowed(),
        _ => return None,
    })
}

/// The configuration and transaction facts every status carries.
fn decorate(status: &mut UpdateStatus, root: Option<&Path>) {
    status.config_schema_required = REQUIRED_CONFIG_SCHEMA;
    status.config_package_id = REQUIRED_CONFIG_PACKAGE_ID.to_owned();
    let Some(root) = root else {
        return;
    };
    let state = package::load_state(root);
    status.config_schema_applied = state.applied_schema;
    status.config_update_status = state.status;
    status.config_update_message = state.message;
    status.config_update_error = state.last_error;
    status.config_update_backup = state.backup_dir;
    if !state.package_id.is_empty() {
        status.config_package_id = state.package_id;
    }
    status.pending_restart = transaction::pending(root).is_some();
    if let Some(message) = transaction::latest_rollback(root) {
        status.rollback_performed = true;
        status.rollback_message = message;
        if status.message.is_empty() {
            status.message = "上次更新启动失败，已自动恢复旧程序和旧配置".to_owned();
        }
    }
}

fn fallback(reason: &str) -> UpdateStatus {
    UpdateStatus {
        current_version: crate::build_identity::VERSION.to_owned(),
        architecture: super::architecture().to_owned(),
        checked_at: super::now_text(),
        cache_expires_at: super::now_text(),
        message: format!("检查更新失败：{reason}"),
        ..UpdateStatus::default()
    }
}

async fn status(ctx: &Context<'_>, force: bool) -> Reply {
    let socks5 = crate::settings_api::proxy_address(ctx.config);
    let env = CheckEnv {
        socks5: &socks5,
        root: ctx.root,
        cancel: ctx.shutdown,
    };
    let limit = if force { CHECK_LIMIT } else { STATUS_LIMIT };
    let mut status = match tokio::time::timeout(limit, manager().check(&env, force)).await {
        Ok(Ok((status, _release))) => status,
        Ok(Err(reason)) => fallback(&reason),
        Err(_) => fallback(&format!("超过 {} 秒未响应", limit.as_secs())),
    };
    decorate(&mut status, ctx.root);
    json(200, &status)
}

#[derive(Deserialize, Default)]
struct ApplyRequest {
    #[serde(default)]
    force: bool,
    /// Accepted and ignored: the native build has no GOAMD64 levels.
    #[serde(default)]
    #[allow(dead_code)]
    prefer_v3: bool,
}

fn action(status: &UpdateStatus, restart: bool, notes: &str) -> Reply {
    let mut value = serde_json::json!({
        "status": status,
        "installed": false,
        "restart_required": restart,
    });
    if !notes.is_empty() {
        value["notes"] = notes.into();
    }
    json(200, &value)
}

/// Hands over to the update guard in a moment: the old program's copy, `update-guard`.
fn schedule_guard(control: &RuntimeControl, transaction_path: &Path, tx: &Transaction) -> bool {
    crate::restart::set_pending(crate::restart::RestartTarget::guard(
        PathBuf::from(&tx.previous_binary_path),
        transaction_path.to_path_buf(),
    ));
    if control.schedule_restart(GUARD_DELAY) {
        true
    } else {
        crate::restart::clear_pending();
        false
    }
}

/// The relative name of the configuration file inside `root` (for the pre-flight `check`).
fn config_name(root: &Path, config_path: &Path) -> String {
    config_path
        .strip_prefix(root)
        .ok()
        .and_then(|relative| relative.to_str())
        .map_or_else(|| "config_custom.yaml".to_owned(), str::to_owned)
}

/// Stages (on a blocking thread) and, when that works, schedules the guard. The error is
/// `(status, message)`.
#[allow(clippy::too_many_arguments)]
async fn stage_and_schedule(
    ctx: &Context<'_>,
    root: &Path,
    kind: &'static str,
    target_version: String,
    asset_signature: String,
    candidate: Option<Vec<u8>>,
    package: Option<(Package, Vec<u8>)>,
) -> Result<(), (u16, String)> {
    let (Some(control), Some(target)) = (ctx.control, crate::restart::target()) else {
        return Err((501, "此进程不能在线更新".to_owned()));
    };
    let Some(args) = target.arguments() else {
        return Err((500, "启动参数不是 UTF-8，无法记录更新事务".to_owned()));
    };
    let Some(listen) = control.api_listen() else {
        return Err((503, "管理接口未启用，无法进行健康确认".to_owned()));
    };
    let executable = target.executable().to_path_buf();
    let config_path = ctx
        .config
        .reload
        .as_ref()
        .map(|reload| reload.config_path.clone())
        .unwrap_or_else(|| root.join("config_custom.yaml"));
    let root_owned = root.to_path_buf();
    let health_url = super::guard::health_url(listen);
    let check_name = config_name(root, &config_path);
    let expected = (kind == "binary").then(|| target_version.clone());
    let health_timeout_secs = health_timeout();
    let staged = tokio::task::spawn_blocking(move || {
        let package_ref = package
            .as_ref()
            .map(|(package, bytes)| (package, bytes.as_slice()));
        transaction::stage(
            &StageInput {
                root: &root_owned,
                config_path: &config_path,
                executable: &executable,
                args,
                health_url,
                health_timeout_secs,
                kind,
                previous_version: crate::build_identity::VERSION,
                target_version: &target_version,
                asset_signature: &asset_signature,
                candidate: candidate.as_deref(),
                package: package_ref,
            },
            &|program, preview| {
                super::guard::preflight(
                    program,
                    expected.as_deref(),
                    preview.map(|preview| (preview, check_name.as_str())),
                )
            },
        )
    })
    .await
    .map_err(|error| (500, format!("暂存更新失败: {error}")))?
    .map_err(|reason| (500, format!("暂存更新失败: {reason}")))?;
    let (path, tx) = staged;
    if !schedule_guard(control, &path, &tx) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
        return Err((409, "已有重启或更新正在进行".to_owned()));
    }
    Ok(())
}

async fn apply(ctx: &Context<'_>, body: &[u8]) -> Reply {
    let request: ApplyRequest = if body.iter().all(u8::is_ascii_whitespace) {
        ApplyRequest::default()
    } else {
        match serde_json::from_slice(body) {
            Ok(request) => request,
            Err(reason) => return error(400, reason),
        }
    };
    if let Some(reason) = update_unsupported(ctx.root.is_some()) {
        return error(501, reason);
    }
    let Some(root) = ctx.root else {
        return error(501, "此主机没有配置目录");
    };
    let Some(_busy) = manager().begin_apply() else {
        return error(409, "已有更新正在进行");
    };
    if transaction::pending(root).is_some() {
        return error(409, "上一次更新尚未完成");
    }
    let socks5 = crate::settings_api::proxy_address(ctx.config);
    let env = CheckEnv {
        socks5: &socks5,
        root: ctx.root,
        cancel: ctx.shutdown,
    };
    let (mut status, release) =
        match tokio::time::timeout(APPLY_CHECK_LIMIT, manager().check(&env, true)).await {
            Ok(Ok(found)) => found,
            Ok(Err(reason)) => return error(502, reason),
            Err(_) => return error(502, "检查更新超时"),
        };
    decorate(&mut status, ctx.root);
    if !status.update_available && !request.force {
        return action(&status, false, "");
    }
    let tag = release.tag_name.clone();
    let fail = |status: &mut UpdateStatus, reason: String| {
        status.message.clone_from(&reason);
        error(502, reason)
    };
    let Some(asset) = release.asset(RELEASE_ASSET) else {
        return fail(
            &mut status,
            format!("未找到适用于 {} 的安装包", super::architecture()),
        );
    };
    let Some(manifest_asset) = release.asset(MANIFEST_ASSET) else {
        return fail(
            &mut status,
            format!("版本 {tag} 缺少 {MANIFEST_ASSET}，拒绝执行非事务更新"),
        );
    };
    let manifest_bytes = match super::download(
        &socks5,
        &manifest_asset.browser_download_url,
        MAX_MANIFEST_BYTES,
        ctx.shutdown,
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(reason) => return fail(&mut status, format!("下载更新清单失败: {reason}")),
    };
    let manifest = match super::parse_manifest(&manifest_bytes, &tag) {
        Ok(manifest) => manifest,
        Err(reason) => return fail(&mut status, reason),
    };
    let archive = match super::download(
        &socks5,
        &asset.browser_download_url,
        MAX_ARCHIVE_BYTES,
        ctx.shutdown,
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(reason) => return fail(&mut status, format!("下载失败: {reason}")),
    };
    // Hashing and unpacking take a moment: not on the thread that answers DNS.
    let expected = manifest.artifacts[RELEASE_ASSET].sha256.clone();
    let extracted = tokio::task::spawn_blocking(move || {
        if !super::sha256_hex(&archive).eq_ignore_ascii_case(expected.trim()) {
            return Err("二进制校验失败: SHA-256 与更新清单不一致".to_owned());
        }
        extract_tar_gz_file(&archive, "mosdns", MAX_BINARY_BYTES, MAX_BINARY_BYTES * 2)
            .map_err(|reason| format!("解压失败: {reason}"))
    })
    .await
    .unwrap_or_else(|error| Err(format!("解压失败: {error}")));
    let candidate = match extracted {
        Ok(bytes) => bytes,
        Err(reason) => return fail(&mut status, reason),
    };
    // A configuration package only when the release needs a newer schema than applied.
    let applied = package::load_state(root).applied_schema;
    let package = if applied < manifest.required_config_schema {
        let Some(config) = &manifest.config else {
            return fail(
                &mut status,
                format!(
                    "目标版本需要配置 schema {}，但更新清单没有配置资产",
                    manifest.required_config_schema
                ),
            );
        };
        let bytes =
            match super::download(&socks5, &config.url, MAX_CONFIG_PACKAGE_BYTES, ctx.shutdown)
                .await
            {
                Ok(bytes) => bytes,
                Err(reason) => {
                    return fail(&mut status, format!("配置包下载失败，已取消更新: {reason}"));
                }
            };
        let (expected, schema, id) = (
            config.sha256.clone(),
            manifest.required_config_schema,
            manifest.config_package_id.clone(),
        );
        let parsed = tokio::task::spawn_blocking(move || {
            if !super::sha256_hex(&bytes).eq_ignore_ascii_case(expected.trim()) {
                return Err("配置包校验失败，已取消更新: SHA-256 不一致".to_owned());
            }
            package::parse(&bytes, schema, &id)
                .map(|package| (package, bytes))
                .map_err(|reason| format!("配置包不适用于目标版本，已取消更新: {reason}"))
        })
        .await
        .unwrap_or_else(|error| Err(format!("配置包处理失败: {error}")));
        match parsed {
            Ok(found) => Some(found),
            Err(reason) => return fail(&mut status, reason),
        }
    } else {
        None
    };
    if let Err((code, reason)) = stage_and_schedule(
        ctx,
        root,
        "binary",
        tag,
        status.asset_signature.clone(),
        Some(candidate),
        package,
    )
    .await
    {
        status.message.clone_from(&reason);
        return error(
            if code == 409 || code == 501 || code == 503 {
                code
            } else {
                502
            },
            reason,
        );
    }
    let notes = "二进制与所需配置已下载并校验，正在安全切换…";
    status.pending_restart = true;
    status.message = notes.to_owned();
    action(&status, true, notes)
}

#[derive(Deserialize, Default)]
struct ConfigRequest {
    #[serde(default)]
    url: String,
    #[serde(default)]
    dir: String,
}

fn decode_config_request(body: &[u8]) -> Result<ConfigRequest, Reply> {
    serde_json::from_slice(body).map_err(|_| text_error(400, "Invalid request body"))
}

/// The request's `dir` must be this host's configuration directory.
fn check_dir(ctx: &Context<'_>, dir: &str) -> Result<PathBuf, Reply> {
    let Some(root) = ctx.root else {
        return Err(text_error(501, "此主机没有配置目录"));
    };
    let same = std::fs::canonicalize(dir)
        .ok()
        .zip(std::fs::canonicalize(root).ok())
        .is_some_and(|(requested, actual)| requested == actual);
    if same {
        Ok(root.to_path_buf())
    } else {
        Err(text_error(
            400,
            "dir must be the configuration directory of this host",
        ))
    }
}

async fn export(ctx: &Context<'_>, body: &[u8]) -> Reply {
    let request = match decode_config_request(body) {
        Ok(request) => request,
        Err(reply) => return reply,
    };
    if request.dir.is_empty() {
        return text_error(400, "dir is required");
    }
    let root = match check_dir(ctx, &request.dir) {
        Ok(root) => root,
        Err(reply) => return reply,
    };
    let Ok(permit) = Arc::clone(&EXPORT_SLOT).try_acquire_owned() else {
        return text_error(503, "another export is in progress");
    };
    let built = tokio::task::spawn_blocking(move || export_zip(&root)).await;
    match built {
        Ok(Ok(body)) => {
            hold_until_connection_ends(permit);
            Reply {
                status: 200,
                content_type: Some("application/zip"),
                body,
                headers: vec![(
                    "Content-Disposition",
                    format!(
                        "attachment; filename=\"mosdns_backup_{}.zip\"",
                        transaction::now_secs()
                    ),
                )],
            }
        }
        Ok(Err((code, reason))) => text_error(code, reason),
        Err(reason) => text_error(500, format!("export failed: {reason}")),
    }
}

/// The configuration root as a stored zip: everything but `backup/` anywhere and the
/// top-level `update/` (program copies and transactions), symbolic links skipped.
fn export_zip(root: &Path) -> Result<Vec<u8>, (u16, String)> {
    fn walk(
        dir: &Path,
        relative: &str,
        depth: usize,
        files: &mut Vec<(String, PathBuf, u64)>,
        total: &mut u64,
    ) -> Result<(), (u16, String)> {
        if depth > MAX_EXPORT_DEPTH {
            return Err((500, "directory tree is too deep".to_owned()));
        }
        let entries = std::fs::read_dir(dir)
            .map_err(|error| (500, format!("read {}: {error}", dir.display())))?;
        for entry in entries {
            let entry = entry.map_err(|error| (500, error.to_string()))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let child = if relative.is_empty() {
                name.clone()
            } else {
                format!("{relative}/{name}")
            };
            let metadata = std::fs::symlink_metadata(entry.path())
                .map_err(|error| (500, format!("inspect {child}: {error}")))?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                if name == "backup" || (depth == 0 && name == "update") {
                    continue;
                }
                walk(&entry.path(), &child, depth + 1, files, total)?;
            } else if metadata.is_file() {
                *total = total.saturating_add(metadata.len());
                if *total > MAX_EXPORT_BYTES || files.len() >= MAX_EXPORT_FILES {
                    return Err((
                        413,
                        "configuration directory is too large to export".to_owned(),
                    ));
                }
                files.push((child, entry.path(), metadata.len()));
            }
        }
        Ok(())
    }
    let mut files = Vec::new();
    let mut total = 0;
    walk(root, "", 0, &mut files, &mut total)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut zip = ZipWriter::default();
    for (name, path, _size) in files {
        let data = std::fs::read(&path).map_err(|error| (500, format!("read {name}: {error}")))?;
        zip.add(&name, &data).map_err(|reason| (500, reason))?;
        if zip.len() as u64 > MAX_EXPORT_BYTES + (1 << 20) {
            return Err((
                413,
                "configuration directory is too large to export".to_owned(),
            ));
        }
    }
    zip.finish().map_err(|reason| (500, reason))
}

async fn update_from_url(ctx: &Context<'_>, body: &[u8]) -> Reply {
    let request = match decode_config_request(body) {
        Ok(request) => request,
        Err(reply) => return reply,
    };
    if request.url.is_empty() || request.dir.is_empty() {
        return text_error(400, "url and dir are required");
    }
    let root = match check_dir(ctx, &request.dir) {
        Ok(root) => root,
        Err(reply) => return reply,
    };
    if !(request.url.starts_with("http://") || request.url.starts_with("https://")) {
        return text_error(400, "url must be an http(s) address");
    }
    if let Some(reason) = config_management_unsupported(true) {
        return text_error(501, reason);
    }
    let Some(_busy) = manager().begin_apply() else {
        return text_error(409, "已有更新正在进行");
    };
    if transaction::pending(&root).is_some() {
        return text_error(409, "上一次更新尚未完成");
    }
    let socks5 = crate::settings_api::proxy_address(ctx.config);
    let bytes = match super::download(
        &socks5,
        &request.url,
        MAX_CONFIG_PACKAGE_BYTES,
        ctx.shutdown,
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(reason) => return text_error(500, format!("下载配置包失败: {reason}")),
    };
    let parsed = tokio::task::spawn_blocking(move || {
        package::parse(&bytes, REQUIRED_CONFIG_SCHEMA, REQUIRED_CONFIG_PACKAGE_ID)
            .map(|package| (package, bytes))
    })
    .await
    .unwrap_or_else(|error| Err(error.to_string()));
    let (package, bytes) = match parsed {
        Ok(found) => found,
        Err(reason) => return text_error(400, format!("配置包无效: {reason}")),
    };
    let files = package.manifest.managed_files.len() + package.manifest.delete_files.len();
    if let Err((code, reason)) = stage_and_schedule(
        ctx,
        &root,
        "config",
        crate::build_identity::VERSION.to_owned(),
        String::new(),
        None,
        Some((package, bytes)),
    )
    .await
    {
        return text_error(code, reason);
    }
    json(
        200,
        &serde_json::json!({
            "message": format!("Update successful. {files} files will be updated. Restarting..."),
            "status": "success",
        }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::archive::{ZipLimits, read_zip};

    fn temp(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("update-api-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn an_export_permit_lives_until_the_connection_ends_and_only_one_exists() {
        let slot = Arc::new(Semaphore::new(1));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(HOLD.scope(Default::default(), async {
            let permit = Arc::clone(&slot).try_acquire_owned().unwrap();
            hold_until_connection_ends(permit);
            assert_eq!(
                slot.available_permits(),
                0,
                "held after the handler returned"
            );
            assert!(
                Arc::clone(&slot).try_acquire_owned().is_err(),
                "a second export is refused meanwhile"
            );
        }));
        assert_eq!(slot.available_permits(), 1, "freed with the connection");
        // Outside a connection (no scope) nothing is held back.
        hold_until_connection_ends(Arc::clone(&slot).try_acquire_owned().unwrap());
        assert_eq!(slot.available_permits(), 1);
    }

    #[test]
    fn the_export_contains_the_configuration_but_not_backups_updates_or_links() {
        let root = temp("export");
        std::fs::create_dir_all(root.join("sub_config")).unwrap();
        std::fs::create_dir_all(root.join("backup/old")).unwrap();
        std::fs::create_dir_all(root.join("update/txn-1")).unwrap();
        std::fs::create_dir_all(root.join("sub_config/backup")).unwrap();
        std::fs::write(root.join("config_custom.yaml"), "log: {}\n").unwrap();
        std::fs::write(root.join("sub_config/a.yaml"), "x").unwrap();
        std::fs::write(root.join("sub_config/backup/b.yaml"), "hidden").unwrap();
        std::fs::write(root.join("backup/old/c.yaml"), "hidden").unwrap();
        std::fs::write(root.join("update/txn-1/mosdns.previous"), "binary").unwrap();
        std::os::unix::fs::symlink("/etc/passwd", root.join("link")).unwrap();
        let bytes = export_zip(&root).unwrap();
        let files = read_zip(
            &bytes,
            ZipLimits {
                max_entries: 100,
                max_entry_bytes: 1 << 20,
                max_total_bytes: 1 << 20,
            },
        )
        .unwrap();
        let names: Vec<_> = files.iter().map(|file| file.name.as_str()).collect();
        assert_eq!(names, ["config_custom.yaml", "sub_config/a.yaml"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn config_names_are_relative_to_the_root() {
        let root = Path::new("/etc/mosdns");
        assert_eq!(
            config_name(root, Path::new("/etc/mosdns/config_custom.yaml")),
            "config_custom.yaml"
        );
        assert_eq!(
            config_name(root, Path::new("/etc/mosdns/conf/main.yaml")),
            "conf/main.yaml"
        );
        assert_eq!(
            config_name(root, Path::new("/elsewhere/main.yaml")),
            "config_custom.yaml"
        );
    }

    #[test]
    fn a_status_carries_the_config_facts_the_rollback_and_the_pending_restart() {
        let root = temp("decorate");
        let mut status = UpdateStatus::default();
        decorate(&mut status, Some(&root));
        assert_eq!(status.config_schema_required, REQUIRED_CONFIG_SCHEMA);
        assert_eq!(status.config_schema_applied, 0);
        assert!(!status.pending_restart && !status.rollback_performed);
        package::write_state(
            &root,
            &package::ConfigState {
                applied_schema: 1,
                status: "success".into(),
                package_id: "rust-config-schema-1".into(),
                message: "配置已升级到 schema 1".into(),
                ..Default::default()
            },
        )
        .unwrap();
        let mut status = UpdateStatus::default();
        decorate(&mut status, Some(&root));
        assert_eq!(
            (
                status.config_schema_applied,
                status.config_update_status.as_str()
            ),
            (1, "success")
        );
        assert_eq!(status.config_update_message, "配置已升级到 schema 1");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_capability_reasons_name_what_is_missing() {
        assert!(
            config_management_unsupported(false)
                .unwrap()
                .contains("配置目录")
        );
        // In a test process nothing captured a restart target.
        assert!(
            config_management_unsupported(true)
                .unwrap()
                .contains("可执行文件")
        );
        assert!(update_unsupported(true).is_some());
    }
}
