//! The configuration package (`config_rust_up.zip`): validation, backup, application,
//! restoration and the package state file.
//!
//! A package is a zip with a root `manifest.json` and the managed files. The rules are
//! the reference's (`coremain/config_update.go`): only `config_custom.yaml` and
//! `sub_config/*.yaml` are managed, every file is hashed in the manifest, nothing outside
//! the manifest may be in the package, and paths cannot leave the configuration root.
//! The state file is this host's own (`webinfo/rust_config_update_state.json`): the
//! reference's `config_update_state.json` belongs to the Go host and must not be shared.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::archive::{ZipLimits, read_zip};
use super::fsio::durable_write;
use super::{CHANNEL, MAX_CONFIG_PACKAGE_BYTES, sha256_hex, valid_sha256};

pub(crate) const STATE_FILE: &str = "webinfo/rust_config_update_state.json";
const MANIFEST_NAME: &str = "manifest.json";
const MAX_ENTRIES: usize = 1024;
const MAX_FILE_BYTES: u64 = 64 << 20;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Manifest {
    pub(crate) format: u32,
    pub(crate) channel: String,
    pub(crate) package_id: String,
    pub(crate) config_schema: u32,
    pub(crate) managed_files: Vec<String>,
    #[serde(default)]
    pub(crate) create_if_missing: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) delete_files: Vec<String>,
    pub(crate) sha256: BTreeMap<String, String>,
}

/// A parsed and fully validated package.
#[derive(Debug)]
pub(crate) struct Package {
    pub(crate) manifest: Manifest,
    pub(crate) files: BTreeMap<String, Vec<u8>>,
    /// SHA-256 of the whole zip.
    pub(crate) sha256: String,
}

/// Written by the host from the WebUI settings; a package never touches it.
const HOST_GENERATED: &str = "sub_config/special_groups.yaml";

fn allowed_managed(path: &str) -> bool {
    path == "config_custom.yaml"
        || (path.starts_with("sub_config/")
            && path.ends_with(".yaml")
            && path.len() > 16
            && path != HOST_GENERATED)
}

fn switch_state_path(path: &str) -> bool {
    let Some(rest) = path.strip_prefix("rule/switch") else {
        return false;
    };
    let Some(digits) = rest.strip_suffix(".txt") else {
        return false;
    };
    !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
}

/// A relative path without `.`/`..` components, absolute prefixes or backslashes.
pub(crate) fn clean_relative(path: &str) -> Result<String, String> {
    let normalized = path.trim().replace('\\', "/");
    if normalized.starts_with('/') {
        return Err(format!("invalid config package path {path:?}"));
    }
    let mut parts = Vec::new();
    for part in normalized.split('/') {
        match part {
            "" | "." => {}
            ".." => return Err(format!("invalid config package path {path:?}")),
            other => parts.push(other),
        }
    }
    if parts.is_empty() {
        return Err(format!("invalid config package path {path:?}"));
    }
    Ok(parts.join("/"))
}

/// The package's single root directory prefix (`name/`) when every file is inside one.
fn single_root_prefix(names: &[String]) -> String {
    let mut root: Option<&str> = None;
    for name in names {
        let Some((first, _rest)) = name.split_once('/') else {
            return String::new();
        };
        match root {
            None => root = Some(first),
            Some(existing) if existing == first => {}
            Some(_) => return String::new(),
        }
    }
    root.map(|root| format!("{root}/")).unwrap_or_default()
}

/// Parses `data` as a configuration package for `required_schema`/`required_id`.
pub(crate) fn parse(
    data: &[u8],
    required_schema: u32,
    required_id: &str,
) -> Result<Package, String> {
    if data.is_empty() {
        return Err("empty config package".to_owned());
    }
    let zip = read_zip(
        data,
        ZipLimits {
            max_entries: MAX_ENTRIES,
            max_entry_bytes: MAX_FILE_BYTES,
            max_total_bytes: MAX_CONFIG_PACKAGE_BYTES as u64,
        },
    )?;
    let normalized: Vec<String> = zip
        .iter()
        .map(|file| file.name.replace('\\', "/"))
        .collect();
    let prefix = single_root_prefix(&normalized);
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (file, name) in zip.into_iter().zip(normalized) {
        let name = clean_relative(name.strip_prefix(&prefix).unwrap_or(&name))?;
        if files.insert(name.clone(), file.data).is_some() {
            return Err(format!("duplicate zip entry {name:?}"));
        }
    }
    let manifest_bytes = files
        .get(MANIFEST_NAME)
        .ok_or("config package has no manifest.json")?;
    let manifest: Manifest = serde_json::from_slice(manifest_bytes)
        .map_err(|error| format!("invalid config manifest: {error}"))?;
    validate(&manifest, &files, required_schema, required_id)?;
    Ok(Package {
        manifest,
        files,
        sha256: sha256_hex(data),
    })
}

fn validate(
    manifest: &Manifest,
    files: &BTreeMap<String, Vec<u8>>,
    required_schema: u32,
    required_id: &str,
) -> Result<(), String> {
    if manifest.format != 1 {
        return Err(format!(
            "unsupported config manifest format {}",
            manifest.format
        ));
    }
    if manifest.channel != CHANNEL {
        return Err(format!(
            "config package channel {:?} does not match {CHANNEL:?}",
            manifest.channel
        ));
    }
    if manifest.config_schema != required_schema {
        return Err(format!(
            "config package schema {} does not match required schema {required_schema}",
            manifest.config_schema
        ));
    }
    if manifest.package_id != required_id {
        return Err(format!(
            "config package id {:?} does not match required id {required_id:?}",
            manifest.package_id
        ));
    }
    if manifest.managed_files.is_empty() {
        return Err("config manifest has no managed files".to_owned());
    }
    let mut managed = BTreeSet::new();
    for raw in &manifest.managed_files {
        let path = clean_relative(raw)?;
        if !allowed_managed(&path) {
            return Err(format!(
                "managed file {path:?} is outside the managed config set"
            ));
        }
        let content = files
            .get(&path)
            .ok_or_else(|| format!("managed file {path:?} is missing from package"))?;
        let want = manifest
            .sha256
            .get(&path)
            .map(|value| value.trim().to_ascii_lowercase())
            .unwrap_or_default();
        if !valid_sha256(&want) {
            return Err(format!("managed file {path:?} has no valid sha256"));
        }
        if sha256_hex(content) != want {
            return Err(format!("sha256 mismatch for {path:?}"));
        }
        if !managed.insert(path.clone()) {
            return Err(format!("duplicate managed file {path:?}"));
        }
    }
    for (raw, value) in &manifest.create_if_missing {
        let path = clean_relative(raw)?;
        if !switch_state_path(&path) {
            return Err(format!(
                "create-if-missing file {path:?} is not an allowed switch state file"
            ));
        }
        if managed.contains(&path) {
            return Err(format!(
                "file {path:?} cannot be both managed and create-if-missing"
            ));
        }
        if value.trim() != "A" && value.trim() != "B" {
            return Err(format!(
                "create-if-missing switch {path:?} must default to A or B"
            ));
        }
    }
    let mut deleted = BTreeSet::new();
    for raw in &manifest.delete_files {
        let path = clean_relative(raw)?;
        if !path.starts_with("sub_config/") || !path.ends_with(".yaml") || path == HOST_GENERATED {
            return Err(format!(
                "delete file {path:?} is outside the managed config set"
            ));
        }
        if managed.contains(&path) {
            return Err(format!("file {path:?} cannot be both managed and deleted"));
        }
        if !deleted.insert(path.clone()) {
            return Err(format!("duplicate delete file {path:?}"));
        }
    }
    for name in files.keys() {
        if name != MANIFEST_NAME && !managed.contains(name) {
            return Err(format!("package contains unlisted payload file {name:?}"));
        }
    }
    Ok(())
}

impl Package {
    /// Every relative path this package may create, replace or delete.
    pub(crate) fn touched_paths(&self) -> Vec<String> {
        let mut paths = BTreeSet::new();
        for path in &self.manifest.managed_files {
            paths.insert(clean_relative(path).unwrap_or_default());
        }
        for path in self.manifest.create_if_missing.keys() {
            paths.insert(clean_relative(path).unwrap_or_default());
        }
        for path in &self.manifest.delete_files {
            paths.insert(clean_relative(path).unwrap_or_default());
        }
        paths.into_iter().collect()
    }
}

/// `<root>/<relative>`, refusing a path whose existing components include a symlink (a
/// managed file is never reached through one, and never replaces one's target).
pub(crate) fn target_path(root: &Path, relative: &str) -> Result<PathBuf, String> {
    let relative = clean_relative(relative)?;
    let mut path = root.to_path_buf();
    for part in relative.split('/') {
        path.push(part);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(format!("{relative:?} is reached through a symbolic link"));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("cannot inspect {relative:?}: {error}")),
        }
    }
    Ok(path)
}

/// The package state file (`applied_schema` 0 = no package applied yet).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ConfigState {
    #[serde(default)]
    pub(crate) format: u32,
    #[serde(default)]
    pub(crate) applied_schema: u32,
    #[serde(default)]
    pub(crate) required_schema: u32,
    #[serde(default)]
    pub(crate) status: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) package_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) package_sha256: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) binary_version: String,
    #[serde(default)]
    pub(crate) files_updated: u32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) backup_dir: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) message: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) last_error: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) updated_at: String,
}

pub(crate) fn load_state(root: &Path) -> ConfigState {
    std::fs::read(root.join(STATE_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ConfigState>(&bytes).ok())
        .map(|mut state| {
            state.format = 1;
            state
        })
        .unwrap_or(ConfigState {
            format: 1,
            ..ConfigState::default()
        })
}

pub(crate) fn write_state(root: &Path, state: &ConfigState) -> std::io::Result<()> {
    let mut state = state.clone();
    state.format = 1;
    let bytes = serde_json::to_vec_pretty(&state).map_err(std::io::Error::other)?;
    durable_write(&root.join(STATE_FILE), &bytes, 0o600)
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupEntry {
    path: String,
    existed: bool,
    #[serde(default)]
    mode: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct BackupManifest {
    /// `None`: there was no state file.
    previous_state: Option<ConfigState>,
    entries: Vec<BackupEntry>,
}

/// Saves every file `package` may touch (and the state file) under `backup_dir`, so
/// [`restore`] can put the configuration back exactly. Nothing is changed in `root`.
pub(crate) fn backup(root: &Path, backup_dir: &Path, package: &Package) -> Result<(), String> {
    let mut entries = Vec::new();
    for relative in package.touched_paths() {
        let target = target_path(root, &relative)?;
        match std::fs::symlink_metadata(&target) {
            Ok(metadata) if metadata.is_file() => {
                let data = std::fs::read(&target)
                    .map_err(|error| format!("backup {relative:?}: {error}"))?;
                let mode = std::os::unix::fs::PermissionsExt::mode(&metadata.permissions()) & 0o777;
                durable_write(&backup_dir.join("files").join(&relative), &data, 0o600)
                    .map_err(|error| format!("backup {relative:?}: {error}"))?;
                entries.push(BackupEntry {
                    path: relative,
                    existed: true,
                    mode,
                });
            }
            Ok(_) => {
                return Err(format!(
                    "config update target {relative:?} is not a regular file"
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                entries.push(BackupEntry {
                    path: relative,
                    existed: false,
                    mode: 0,
                })
            }
            Err(error) => return Err(format!("inspect {relative:?}: {error}")),
        }
    }
    let state_path = root.join(STATE_FILE);
    let previous_state = std::fs::read(&state_path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<ConfigState>(&bytes).ok());
    let manifest = BackupManifest {
        previous_state,
        entries,
    };
    let bytes = serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?;
    durable_write(&backup_dir.join("backup_manifest.json"), &bytes, 0o600)
        .map_err(|error| format!("write backup manifest: {error}"))
}

/// Removes `target` and makes the removal durable (the directory is flushed even when the
/// file was already gone, so an earlier unflushed removal cannot come back after a power
/// loss). Returns whether a file was removed.
fn remove_durable(target: &Path) -> std::io::Result<bool> {
    let removed = match std::fs::remove_file(target) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(error),
    };
    match target.parent().map(super::fsio::sync_dir) {
        Some(Err(error)) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(removed),
    }
}

/// Puts the files and the state file back as [`backup`] saw them. Idempotent: running it
/// again (after an interruption, say) gives the same result.
pub(crate) fn restore(root: &Path, backup_dir: &Path) -> Result<(), String> {
    let bytes = std::fs::read(backup_dir.join("backup_manifest.json"))
        .map_err(|error| format!("read backup manifest: {error}"))?;
    let manifest: BackupManifest =
        serde_json::from_slice(&bytes).map_err(|error| format!("backup manifest: {error}"))?;
    for entry in &manifest.entries {
        let target = target_path(root, &entry.path)?;
        if entry.existed {
            let data = std::fs::read(backup_dir.join("files").join(&entry.path))
                .map_err(|error| format!("read backup of {:?}: {error}", entry.path))?;
            let mode = if entry.mode == 0 { 0o644 } else { entry.mode };
            durable_write(&target, &data, mode)
                .map_err(|error| format!("restore {:?}: {error}", entry.path))?;
        } else {
            remove_durable(&target).map_err(|error| format!("remove {:?}: {error}", entry.path))?;
        }
    }
    let state_path = root.join(STATE_FILE);
    match manifest.previous_state {
        Some(state) => {
            write_state(root, &state).map_err(|error| format!("restore state: {error}"))?
        }
        None => match std::fs::remove_file(&state_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("remove state: {error}")),
        },
    }
    if let Some(parent) = state_path.parent() {
        let _ = super::fsio::sync_dir(parent);
    }
    Ok(())
}

/// Writes the package into `root` (managed files, missing switch files, deletions) and
/// returns how many files changed. Every file is replaced atomically; a failure part-way
/// is undone by [`restore`] from the backup the caller made first.
pub(crate) fn apply(root: &Path, package: &Package) -> Result<u32, String> {
    let mut updated = 0;
    for raw in &package.manifest.managed_files {
        let relative = clean_relative(raw)?;
        let target = target_path(root, &relative)?;
        durable_write(&target, &package.files[&relative], 0o644)
            .map_err(|error| format!("write managed file {relative:?}: {error}"))?;
        updated += 1;
    }
    for (raw, value) in &package.manifest.create_if_missing {
        let relative = clean_relative(raw)?;
        let target = target_path(root, &relative)?;
        if target.exists() {
            continue;
        }
        durable_write(&target, value.trim().as_bytes(), 0o644)
            .map_err(|error| format!("create missing file {relative:?}: {error}"))?;
        updated += 1;
    }
    for raw in &package.manifest.delete_files {
        let relative = clean_relative(raw)?;
        let target = target_path(root, &relative)?;
        if remove_durable(&target)
            .map_err(|error| format!("delete obsolete file {relative:?}: {error}"))?
        {
            updated += 1;
        }
    }
    Ok(updated)
}

/// A scratch configuration root that looks like `root` with `package` applied, built from
/// symbolic links and the package's own files, so a candidate program can `check` the new
/// configuration without anything in the live root changing.
pub(crate) fn build_preview(root: &Path, package: &Package, preview: &Path) -> Result<(), String> {
    let io = |what: &str, error: std::io::Error| format!("{what}: {error}");
    // Links must be absolute: they live deep inside the preview, so a relative root (`-d .`)
    // would resolve them against the wrong directory.
    let root = &root
        .canonicalize()
        .map_err(|error| io("resolve configuration root", error))?;
    std::fs::create_dir_all(preview).map_err(|error| io("create preview", error))?;
    let managed: BTreeSet<String> = package
        .manifest
        .managed_files
        .iter()
        .filter_map(|path| clean_relative(path).ok())
        .collect();
    let deleted: BTreeSet<String> = package
        .manifest
        .delete_files
        .iter()
        .filter_map(|path| clean_relative(path).ok())
        .collect();
    let created: BTreeSet<String> = package
        .manifest
        .create_if_missing
        .keys()
        .filter_map(|path| clean_relative(path).ok())
        .collect();
    fn link_tree(
        real: &Path,
        view: &Path,
        relative: &str,
        replaced: &BTreeSet<String>,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 8 {
            return Err("configuration tree is too deep".to_owned());
        }
        for entry in
            std::fs::read_dir(real).map_err(|error| format!("read {relative:?}: {error}"))?
        {
            let entry = entry.map_err(|error| error.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let child = if relative.is_empty() {
                name.clone()
            } else {
                format!("{relative}/{name}")
            };
            if replaced.contains(&child) {
                continue;
            }
            let kind = entry.file_type().map_err(|error| error.to_string())?;
            if kind.is_dir() {
                std::fs::create_dir_all(view.join(&name)).map_err(|error| error.to_string())?;
                link_tree(
                    &entry.path(),
                    &view.join(&name),
                    &child,
                    replaced,
                    depth + 1,
                )?;
            } else {
                std::os::unix::fs::symlink(entry.path(), view.join(&name))
                    .map_err(|error| format!("link {child:?}: {error}"))?;
            }
        }
        Ok(())
    }
    // Top level: everything except the update machinery is reachable through links, so
    // relative references from the configuration resolve exactly as in the live root.
    let mut replaced: BTreeSet<String> = managed.union(&deleted).cloned().collect();
    replaced.extend(
        created
            .iter()
            .filter(|path| root.join(path).exists())
            .cloned(),
    );
    for entry in std::fs::read_dir(root).map_err(|error| io("read root", error))? {
        let entry = entry.map_err(|error| io("read root", error))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "update" || name == "backup" {
            continue;
        }
        let kind = entry
            .file_type()
            .map_err(|error| io("inspect root", error))?;
        if kind.is_dir() && name == "sub_config" {
            std::fs::create_dir_all(preview.join(&name))
                .map_err(|error| io("sub_config", error))?;
            link_tree(
                &entry.path(),
                &preview.join(&name),
                "sub_config",
                &replaced,
                1,
            )?;
        } else if replaced.contains(&name) {
            continue;
        } else {
            std::os::unix::fs::symlink(entry.path(), preview.join(&name))
                .map_err(|error| format!("link {name:?}: {error}"))?;
        }
    }
    for relative in &package.manifest.managed_files {
        let relative = clean_relative(relative)?;
        durable_write(&preview.join(&relative), &package.files[&relative], 0o644)
            .map_err(|error| format!("preview {relative:?}: {error}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use crate::update::archive::ZipWriter;

    fn temp(name: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("update-package-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// A package zip: `files` are the payload, `manifest` JSON is built from the pieces.
    fn package_zip(
        managed: &[(&str, &str)],
        create: &[(&str, &str)],
        delete: &[&str],
        extra: &[(&str, &str)],
        edit: impl Fn(&mut serde_json::Value),
    ) -> Vec<u8> {
        let mut sha = serde_json::Map::new();
        for (path, content) in managed {
            sha.insert((*path).to_owned(), sha256_hex(content.as_bytes()).into());
        }
        let mut manifest = serde_json::json!({
            "format": 1,
            "channel": "rust",
            "package_id": "rust-config-schema-1",
            "config_schema": 1,
            "managed_files": managed.iter().map(|(path, _)| path).collect::<Vec<_>>(),
            "create_if_missing": create.iter().map(|(p, v)| ((*p).to_owned(), (*v).into())).collect::<serde_json::Map<String, serde_json::Value>>(),
            "delete_files": delete,
            "sha256": sha,
        });
        edit(&mut manifest);
        let mut zip = ZipWriter::default();
        zip.add("manifest.json", manifest.to_string().as_bytes())
            .unwrap();
        for (path, content) in managed.iter().chain(extra) {
            zip.add(path, content.as_bytes()).unwrap();
        }
        zip.finish().unwrap()
    }

    fn good() -> Vec<u8> {
        package_zip(
            &[
                ("config_custom.yaml", "log: {}\n"),
                ("sub_config/a.yaml", "plugins: []\n"),
            ],
            &[("rule/switch3.txt", "B")],
            &["sub_config/old.yaml"],
            &[],
            |_| {},
        )
    }

    #[test]
    fn a_valid_package_parses_and_lists_what_it_touches() {
        let package = parse(&good(), 1, "rust-config-schema-1").unwrap();
        assert_eq!(package.manifest.managed_files.len(), 2);
        assert_eq!(
            package.touched_paths(),
            [
                "config_custom.yaml",
                "rule/switch3.txt",
                "sub_config/a.yaml",
                "sub_config/old.yaml"
            ]
        );
        assert_eq!(package.sha256, sha256_hex(&good()));
    }

    #[test]
    fn a_single_root_directory_in_the_zip_is_stripped() {
        let mut zip = ZipWriter::default();
        let content = "log: {}\n";
        let manifest = serde_json::json!({
            "format": 1, "channel": "rust", "package_id": "rust-config-schema-1", "config_schema": 1,
            "managed_files": ["config_custom.yaml"],
            "sha256": {"config_custom.yaml": sha256_hex(content.as_bytes())},
        });
        zip.add(
            "config_rust_up/manifest.json",
            manifest.to_string().as_bytes(),
        )
        .unwrap();
        zip.add("config_rust_up/config_custom.yaml", content.as_bytes())
            .unwrap();
        let package = parse(&zip.finish().unwrap(), 1, "rust-config-schema-1").unwrap();
        assert!(package.files.contains_key("config_custom.yaml"));
    }

    #[test]
    fn schema_id_channel_format_and_hashes_must_match_or_the_package_is_refused() {
        let base = |edit: fn(&mut serde_json::Value)| {
            package_zip(&[("config_custom.yaml", "x")], &[], &[], &[], edit)
        };
        let cases: [(&str, Vec<u8>); 6] = [
            ("schema", base(|m| m["config_schema"] = 2.into())),
            (
                "package id",
                base(|m| m["package_id"] = "main-config-schema-4".into()),
            ),
            ("channel", base(|m| m["channel"] = "main".into())),
            ("format", base(|m| m["format"] = 2.into())),
            (
                "hash",
                base(|m| m["sha256"]["config_custom.yaml"] = "0".repeat(64).into()),
            ),
            ("unknown field", base(|m| m["surprise"] = 1.into())),
        ];
        for (what, zip) in cases {
            assert!(parse(&zip, 1, "rust-config-schema-1").is_err(), "{what}");
        }
        assert!(
            parse(&good(), 2, "rust-config-schema-1")
                .unwrap_err()
                .contains("schema")
        );
        assert!(
            parse(&good(), 1, "rust-config-schema-2")
                .unwrap_err()
                .contains("id")
        );
    }

    #[test]
    fn paths_outside_the_managed_set_and_hostile_names_are_refused() {
        let refuse = |zip: Vec<u8>, what: &str| {
            assert!(parse(&zip, 1, "rust-config-schema-1").is_err(), "{what}");
        };
        refuse(
            package_zip(&[("webinfo/x.yaml", "x")], &[], &[], &[], |_| {}),
            "webinfo",
        );
        refuse(
            package_zip(&[("../config_custom.yaml", "x")], &[], &[], &[], |_| {}),
            "traversal",
        );
        refuse(
            package_zip(&[("/etc/passwd", "x")], &[], &[], &[], |_| {}),
            "absolute",
        );
        refuse(
            package_zip(&[("sub_config/x.txt", "x")], &[], &[], &[], |_| {}),
            "extension",
        );
        refuse(
            package_zip(
                &[("config_custom.yaml", "x")],
                &[("rule/other.txt", "A")],
                &[],
                &[],
                |_| {},
            ),
            "create path",
        );
        refuse(
            package_zip(
                &[("config_custom.yaml", "x")],
                &[("rule/switch1.txt", "C")],
                &[],
                &[],
                |_| {},
            ),
            "create value",
        );
        refuse(
            package_zip(
                &[("config_custom.yaml", "x")],
                &[],
                &["config_custom.yaml"],
                &[],
                |_| {},
            ),
            "delete path",
        );
        refuse(
            package_zip(
                &[("config_custom.yaml", "x")],
                &[],
                &["sub_config/a.yaml", "sub_config/a.yaml"],
                &[],
                |_| {},
            ),
            "duplicate delete",
        );
        refuse(
            package_zip(
                &[("config_custom.yaml", "x")],
                &[],
                &[],
                &[("rule/extra.txt", "y")],
                |_| {},
            ),
            "unlisted payload",
        );
        refuse(
            package_zip(
                &[("config_custom.yaml", "x"), ("config_custom.yaml", "x")],
                &[],
                &[],
                &[],
                |_| {},
            ),
            "duplicate entry",
        );
        refuse(
            package_zip(
                &[("config_custom.yaml", "x")],
                &[("config_custom.yaml", "A")],
                &[],
                &[],
                |_| {},
            ),
            "both managed and created",
        );
        refuse(Vec::new(), "empty");
        refuse(b"PK\x03\x04junk".to_vec(), "not a zip");
        let mut no_manifest = ZipWriter::default();
        no_manifest.add("config_custom.yaml", b"x").unwrap();
        refuse(no_manifest.finish().unwrap(), "no manifest");
    }

    #[test]
    fn clean_relative_resolves_dots_and_rejects_escapes() {
        assert_eq!(
            clean_relative("./sub_config//a.yaml").unwrap(),
            "sub_config/a.yaml"
        );
        assert_eq!(
            clean_relative(r"sub_config\a.yaml").unwrap(),
            "sub_config/a.yaml"
        );
        for bad in ["", ".", "..", "a/../b", "/abs", "a/.."] {
            assert!(clean_relative(bad).is_err(), "{bad}");
        }
    }

    fn root_with_config(name: &str) -> PathBuf {
        let root = temp(name);
        std::fs::write(root.join("config_custom.yaml"), "old custom\n").unwrap();
        std::fs::create_dir_all(root.join("sub_config")).unwrap();
        std::fs::write(root.join("sub_config/a.yaml"), "old a\n").unwrap();
        std::fs::write(root.join("sub_config/old.yaml"), "obsolete\n").unwrap();
        std::fs::write(root.join("sub_config/settings.yaml"), "user settings\n").unwrap();
        std::fs::create_dir_all(root.join("webinfo")).unwrap();
        std::fs::write(root.join("webinfo/settings.json"), "{}").unwrap();
        root
    }

    #[test]
    fn apply_changes_only_the_listed_files_and_restore_puts_everything_back() {
        let root = root_with_config("apply");
        let package = parse(&good(), 1, "rust-config-schema-1").unwrap();
        let backup_dir = root.join("update/txn/config-backup");
        backup(&root, &backup_dir, &package).unwrap();
        write_state(
            &root,
            &ConfigState {
                applied_schema: 0,
                status: "success".into(),
                ..ConfigState::default()
            },
        )
        .unwrap();
        assert_eq!(apply(&root, &package).unwrap(), 4);
        assert_eq!(
            std::fs::read_to_string(root.join("config_custom.yaml")).unwrap(),
            "log: {}\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("sub_config/a.yaml")).unwrap(),
            "plugins: []\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("rule/switch3.txt")).unwrap(),
            "B"
        );
        assert!(
            !root.join("sub_config/old.yaml").exists(),
            "obsolete file deleted"
        );
        // Not listed: untouched.
        assert_eq!(
            std::fs::read_to_string(root.join("sub_config/settings.yaml")).unwrap(),
            "user settings\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("webinfo/settings.json")).unwrap(),
            "{}"
        );
        // An existing switch file is never overwritten.
        std::fs::write(root.join("rule/switch3.txt"), "A").unwrap();
        // Both managed files are rewritten; the switch file exists and the obsolete one is gone.
        assert_eq!(apply(&root, &package).unwrap(), 2);
        assert_eq!(
            std::fs::read_to_string(root.join("rule/switch3.txt")).unwrap(),
            "A"
        );

        // Restore: old contents back, the created file gone, the deleted file back, state back.
        write_state(
            &root,
            &ConfigState {
                applied_schema: 1,
                status: "success".into(),
                ..ConfigState::default()
            },
        )
        .unwrap();
        restore(&root, &backup_dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("config_custom.yaml")).unwrap(),
            "old custom\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("sub_config/a.yaml")).unwrap(),
            "old a\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("sub_config/old.yaml")).unwrap(),
            "obsolete\n"
        );
        assert!(!root.join("rule/switch3.txt").exists());
        assert_eq!(load_state(&root).applied_schema, 0);
        // Idempotent: a second restore changes nothing.
        restore(&root, &backup_dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("config_custom.yaml")).unwrap(),
            "old custom\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_missing_state_file_is_restored_as_missing() {
        let root = root_with_config("state");
        let package = parse(&good(), 1, "rust-config-schema-1").unwrap();
        let backup_dir = root.join("update/t/config-backup");
        backup(&root, &backup_dir, &package).unwrap();
        write_state(
            &root,
            &ConfigState {
                applied_schema: 1,
                ..ConfigState::default()
            },
        )
        .unwrap();
        restore(&root, &backup_dir).unwrap();
        assert!(!root.join(STATE_FILE).exists());
        assert_eq!(load_state(&root).applied_schema, 0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_target_reached_through_a_symlink_is_refused() {
        let root = root_with_config("symlink");
        let outside = temp("symlink-outside");
        std::fs::write(outside.join("a.yaml"), "outside\n").unwrap();
        std::fs::remove_file(root.join("sub_config/a.yaml")).unwrap();
        std::os::unix::fs::symlink(outside.join("a.yaml"), root.join("sub_config/a.yaml")).unwrap();
        let package = parse(&good(), 1, "rust-config-schema-1").unwrap();
        assert!(
            target_path(&root, "sub_config/a.yaml")
                .unwrap_err()
                .contains("symbolic link")
        );
        assert!(backup(&root, &root.join("update/x/config-backup"), &package).is_err());
        assert!(apply(&root, &package).is_err());
        assert_eq!(
            std::fs::read_to_string(outside.join("a.yaml")).unwrap(),
            "outside\n"
        );
        // A symlinked directory component is refused too.
        std::fs::remove_file(root.join("sub_config/a.yaml")).unwrap();
        std::fs::remove_dir_all(root.join("sub_config")).unwrap();
        std::os::unix::fs::symlink(&outside, root.join("sub_config")).unwrap();
        assert!(target_path(&root, "sub_config/b.yaml").is_err());
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(outside).unwrap();
    }

    #[test]
    fn a_write_failure_part_way_is_undone_by_the_backup() {
        let root = root_with_config("partial");
        let package = parse(&good(), 1, "rust-config-schema-1").unwrap();
        let backup_dir = root.join("update/p/config-backup");
        backup(&root, &backup_dir, &package).unwrap();
        // The second managed file cannot be written: its directory is a regular file.
        std::fs::remove_dir_all(root.join("sub_config")).unwrap();
        std::fs::write(root.join("sub_config"), "not a directory").unwrap();
        assert!(apply(&root, &package).is_err());
        // The first file was already replaced when the second failed.
        assert_eq!(
            std::fs::read_to_string(root.join("config_custom.yaml")).unwrap(),
            "log: {}\n"
        );
        std::fs::remove_file(root.join("sub_config")).unwrap();
        std::fs::create_dir_all(root.join("sub_config")).unwrap();
        restore(&root, &backup_dir).unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("config_custom.yaml")).unwrap(),
            "old custom\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("sub_config/a.yaml")).unwrap(),
            "old a\n"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_host_generated_special_groups_file_can_be_neither_written_nor_deleted() {
        let target = "sub_config/special_groups.yaml";
        assert!(
            parse(
                &package_zip(
                    &[("config_custom.yaml", "x"), (target, "x")],
                    &[],
                    &[],
                    &[],
                    |_| {}
                ),
                1,
                "rust-config-schema-1"
            )
            .is_err(),
            "managed"
        );
        assert!(
            parse(
                &package_zip(&[("config_custom.yaml", "x")], &[], &[target], &[], |_| {}),
                1,
                "rust-config-schema-1"
            )
            .is_err(),
            "deleted"
        );
    }

    #[test]
    fn a_removal_is_reported_and_repeating_it_is_harmless() {
        let root = temp("remove");
        let file = root.join("gone.yaml");
        std::fs::write(&file, "x").unwrap();
        assert!(remove_durable(&file).unwrap());
        assert!(!remove_durable(&file).unwrap(), "already gone");
        assert!(!remove_durable(&root.join("no-dir/x.yaml")).unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_preview_links_are_absolute_so_a_relative_root_still_works() {
        let root = root_with_config("preview-relative");
        std::fs::write(root.join("rule.txt"), "rules").unwrap();
        // The same root, named relative to the current directory.
        let up = std::env::current_dir().unwrap().components().count() - 1;
        let relative = PathBuf::from("../".repeat(up)).join(root.strip_prefix("/").unwrap());
        let package = parse(&good(), 1, "rust-config-schema-1").unwrap();
        let preview = temp("preview-relative-view");
        build_preview(&relative, &package, &preview).unwrap();
        assert!(
            std::fs::read_link(preview.join("rule.txt"))
                .unwrap()
                .is_absolute()
        );
        assert_eq!(
            std::fs::read_to_string(preview.join("rule.txt")).unwrap(),
            "rules"
        );
        assert_eq!(
            std::fs::read_to_string(preview.join("sub_config/settings.yaml")).unwrap(),
            "user settings\n"
        );
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(preview).unwrap();
    }

    #[test]
    fn the_preview_root_shows_the_new_files_and_links_everything_else_without_touching_the_root() {
        let root = root_with_config("preview");
        std::fs::create_dir_all(root.join("update/x")).unwrap();
        std::fs::write(root.join("rule.txt"), "rules").unwrap();
        let package = parse(&good(), 1, "rust-config-schema-1").unwrap();
        let preview = temp("preview-view");
        build_preview(&root, &package, &preview).unwrap();
        assert_eq!(
            std::fs::read_to_string(preview.join("config_custom.yaml")).unwrap(),
            "log: {}\n"
        );
        assert_eq!(
            std::fs::read_to_string(preview.join("sub_config/a.yaml")).unwrap(),
            "plugins: []\n"
        );
        assert!(
            !preview.join("sub_config/old.yaml").exists(),
            "deleted files are absent"
        );
        assert_eq!(
            std::fs::read_to_string(preview.join("sub_config/settings.yaml")).unwrap(),
            "user settings\n"
        );
        assert_eq!(
            std::fs::read_to_string(preview.join("rule.txt")).unwrap(),
            "rules"
        );
        assert_eq!(
            std::fs::read_to_string(preview.join("webinfo/settings.json")).unwrap(),
            "{}"
        );
        assert!(
            !preview.join("update").exists(),
            "the update machinery is not part of the view"
        );
        // The live root is unchanged.
        assert_eq!(
            std::fs::read_to_string(root.join("config_custom.yaml")).unwrap(),
            "old custom\n"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("sub_config/old.yaml")).unwrap(),
            "obsolete\n"
        );
        std::fs::remove_dir_all(root).unwrap();
        std::fs::remove_dir_all(preview).unwrap();
        let _ = std::io::stdout().flush();
    }
}
