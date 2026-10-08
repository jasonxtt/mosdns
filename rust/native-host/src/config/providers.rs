//! Read-only rule providers (`domain_set_light`, `sd_set_light`,
//! `adguard_rule`, `si_set`) and the `domain_mapper` that merges them.
//!
//! Each provider is split into "arguments → spec" (compile time only) and
//! "spec → rules" (compile time and hot rebuilds on the blocking pool, see
//! `crate::rule_sources`). A missing or unusable source degrades the same way
//! the reference providers do (skipped with a diagnostic), because that is how
//! a production configuration with a stale manifest keeps starting. Nothing in
//! this module writes.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;

use mosdns_matcher_core::{
    IpPrefixList, RegexBuildError, ValuedBuildError, ValuedDomainMatcher, ValuedRule,
    decode_domain_srs, decode_ip_srs,
};
use mosdns_sequence_core::{
    ExecutionState, Executor, ExecutorError, ExecutorOutcome, RoutingField, StateMutation,
};

use super::{
    ConfigError, RawPlugin, RawValue, expect_integer, expect_map, expect_sequence, expect_string,
    policy_strings, resolve_relative,
};
use crate::special_groups::CandidateInputSet;

/// The largest provider file read into memory.
const MAX_FILE_BYTES: u64 = 128 * 1024 * 1024;
/// The reference line scanner rejects a line this long or longer.
const MAX_LINE_BYTES: usize = 64 * 1024;

/// One rule string and the source name that contributed it.
pub(crate) struct RuleEntry {
    pub(crate) rule: String,
    /// `None` means "the provider tag" (providers without per-source detail).
    pub(crate) source: Option<Arc<str>>,
}

pub(crate) type ProviderRules = Rc<Vec<RuleEntry>>;

type Inputs<'a> = Option<&'a mut CandidateInputSet>;

/// Reads a whole file. `Ok(None)` is a missing file (recorded as such for a
/// managed candidate); any other I/O failure is returned for the caller to
/// treat as fatal or as a skipped source.
fn read_file(path: &Path, inputs: &mut Inputs<'_>) -> std::io::Result<Option<Vec<u8>>> {
    let target = path.to_path_buf();
    let result = crate::transaction::blocking_io(move || {
        let file = std::fs::File::open(&target)?;
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "file exceeds the provider size limit",
            ));
        }
        Ok(bytes)
    })?;
    match result {
        Ok(bytes) => {
            if let Some(inputs) = inputs.as_deref_mut() {
                inputs
                    .record_bytes(path, &bytes)
                    .map_err(std::io::Error::other)?;
            }
            Ok(Some(bytes))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Some(inputs) = inputs.as_deref_mut() {
                inputs.record_missing(path).map_err(std::io::Error::other)?;
            }
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn args_map<'a>(
    plugin: &'a RawPlugin,
    allowed: &[&str],
) -> Result<(String, &'a super::RawMap), ConfigError> {
    let path = format!("{}.args", plugin.source_path);
    let args = expect_map(&plugin.args, &path, "provider args must be a mapping")?;
    args.reject_unknown(allowed, &path)?;
    Ok((path, args))
}

/// The arguments of one `domain_set_light`, with paths already resolved.
#[derive(Clone, Debug)]
pub(crate) struct LightSpec {
    pub(crate) tag: String,
    /// `$.plugins[i].args`, for error locations.
    path: String,
    exps: Vec<String>,
    /// `(index in files, resolved path)`; empty entries are left out.
    pub(crate) files: Vec<(usize, PathBuf)>,
}

impl LightSpec {
    /// The file `/save` and `/post` write: the first configured file, when it
    /// is a `.txt` file (the reference only posts to `.txt`).
    pub(crate) fn writable_file(&self) -> Option<&Path> {
        let (index, file) = self.files.first()?;
        (*index == 0
            && file
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| ext.eq_ignore_ascii_case("txt")))
        .then_some(file.as_path())
    }
}

/// `domain_set_light` arguments: `exps`, `files`, `sets` (accepted, ignored:
/// this provider never matches itself).
pub(super) fn light_spec(plugin: &RawPlugin) -> Result<LightSpec, ConfigError> {
    let (path, args) = args_map(plugin, &["exps", "sets", "files"])?;
    let exps = policy_strings(args.get("exps"), &format!("{path}.exps"))?;
    policy_strings(args.get("sets"), &format!("{path}.sets"))?;
    let files = policy_strings(args.get("files"), &format!("{path}.files"))?
        .iter()
        .enumerate()
        .filter(|(_, file)| !file.is_empty())
        .map(|(index, file)| (index, resolve_relative(file, &plugin.root_dir)))
        .collect();
    Ok(LightSpec {
        tag: plugin.tag.clone(),
        path,
        exps,
        files,
    })
}

/// `domain_set_light`: `exps` first, then each file (SRS or text). Rules are
/// kept as text; matching belongs to the `domain_mapper` that consumes them.
pub(crate) fn load_light(
    spec: &LightSpec,
    mut inputs: Inputs<'_>,
) -> Result<Vec<RuleEntry>, ConfigError> {
    let path = &spec.path;
    let mut out: Vec<RuleEntry> = spec
        .exps
        .iter()
        .map(|rule| RuleEntry {
            rule: rule.clone(),
            source: None,
        })
        .collect();
    for (index, resolved) in &spec.files {
        let bytes = match read_file(resolved, &mut inputs) {
            Ok(Some(bytes)) => bytes,
            // A missing file is an empty source, not an error.
            Ok(None) => continue,
            Err(error) => {
                return Err(ConfigError::new(
                    format!("{path}.files[{index}]"),
                    format!(
                        "failed to load file {index} {}: {error}",
                        resolved.display()
                    ),
                ));
            }
        };
        if let Ok(srs) = decode_domain_srs(&bytes, true) {
            out.extend(
                srs.rules
                    .into_iter()
                    .map(|rule| RuleEntry { rule, source: None }),
            );
            continue;
        }
        for line in bytes.split(|byte| *byte == b'\n') {
            if line.len() >= MAX_LINE_BYTES {
                return Err(ConfigError::new(
                    format!("{path}.files[{index}]"),
                    format!(
                        "failed to load file {index} {}: line too long",
                        resolved.display()
                    ),
                ));
            }
            let line = String::from_utf8_lossy(line);
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            out.push(RuleEntry {
                rule: line.to_owned(),
                source: None,
            });
        }
    }
    Ok(out)
}

/// One entry of a `local_config` / `config.json` source manifest, with every
/// field of the reference struct (management writes them back).
#[derive(Clone, Default, Debug, Eq, PartialEq)]
pub(crate) struct SourceEntry {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) kind: String,
    pub(crate) files: String,
    pub(crate) url: String,
    pub(crate) enabled: bool,
    pub(crate) enable_regexp: bool,
    pub(crate) auto_update: bool,
    pub(crate) update_interval_hours: i64,
    pub(crate) rule_count: i64,
    /// The RFC 3339 text exactly as read (or written); `None` is the zero time.
    pub(crate) last_updated: Option<String>,
}

/// Which reference struct decodes the manifest; each knows different fields,
/// and Go only type-checks the fields its own struct has.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Schema {
    /// `sd_set_light`: name,type,files,url,enabled,enable_regexp,…
    Sd,
    /// `si_set`: as `Sd` without `enable_regexp`.
    Si,
    /// `adguard_rule`: id,name,url,enabled,… (no `type`, `files`, `enable_regexp`).
    Adguard,
}

impl Schema {
    fn has(self, field: &str) -> bool {
        match field {
            "name"
            | "url"
            | "enabled"
            | "auto_update"
            | "update_interval_hours"
            | "rule_count"
            | "last_updated" => true,
            "id" => self == Self::Adguard,
            "type" | "files" => self != Self::Adguard,
            "enable_regexp" => self == Self::Sd,
            _ => false,
        }
    }
}

/// `time.Time` JSON: an RFC 3339 string with a valid calendar date and time.
fn is_rfc3339(text: &str) -> bool {
    // Go's fallback parser takes a one-digit hour (`T3:56:02Z`): pad it.
    let mut padded = text.as_bytes().to_vec();
    if padded.len() > 12 && padded[11].is_ascii_digit() && padded[12] == b':' {
        padded.insert(11, b'0');
    }
    let b = padded.as_slice();
    let digits = |from: usize, n: usize| -> Option<u32> {
        let part = b.get(from..from + n)?;
        part.iter()
            .all(u8::is_ascii_digit)
            .then(|| part.iter().fold(0, |acc, d| acc * 10 + u32::from(d - b'0')))
    };
    let (Some(year), Some(month), Some(day), Some(hour), Some(minute), Some(second)) = (
        digits(0, 4),
        digits(5, 2),
        digits(8, 2),
        digits(11, 2),
        digits(14, 2),
        digits(17, 2),
    ) else {
        return false;
    };
    if b.get(4) != Some(&b'-')
        || b.get(7) != Some(&b'-')
        || b.get(10) != Some(&b'T')
        || b.get(13) != Some(&b':')
        || b.get(16) != Some(&b':')
        || !(1..=12).contains(&month)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return false;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day < 1 || day > days {
        return false;
    }
    let mut i = 19;
    // Go accepts `.` and `,` before the fraction, and an offset of up to
    // 24:60 (checked against go1.26 over the whole 00..99 x 00..99 range).
    if matches!(b.get(i), Some(b'.' | b',')) {
        let start = i + 1;
        i = start;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    match b.get(i) {
        Some(b'Z') => i + 1 == b.len(),
        Some(b'+' | b'-') => {
            i + 6 == b.len()
                && digits(i + 1, 2).is_some_and(|h| h <= 24)
                && b.get(i + 3) == Some(&b':')
                && digits(i + 4, 2).is_some_and(|m| m <= 60)
        }
        _ => false,
    }
}

/// Decodes the manifest array the way Go's `encoding/json` fills the
/// reference struct: keys match case-insensitively and are applied in document
/// order (a later spelling overwrites an earlier one), `null` leaves a field
/// at its zero value, a value of the wrong type for a field *the struct has*
/// fails the whole array, and unknown keys are ignored.
impl<'de> serde::de::DeserializeSeed<'de> for Schema {
    type Value = Vec<SourceEntry>;

    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        struct Entries(Schema);
        impl<'de> serde::de::Visitor<'de> for Entries {
            type Value = Vec<SourceEntry>;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("an array of source objects")
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(
                self,
                mut seq: A,
            ) -> Result<Self::Value, A::Error> {
                let mut out = Vec::new();
                while let Some(entry) = seq.next_element_seed(EntrySeed(self.0))? {
                    out.push(entry);
                }
                Ok(out)
            }
        }
        deserializer.deserialize_seq(Entries(self))
    }
}

struct EntrySeed(Schema);

impl<'de> serde::de::DeserializeSeed<'de> for EntrySeed {
    type Value = SourceEntry;

    fn deserialize<D: serde::Deserializer<'de>>(
        self,
        deserializer: D,
    ) -> Result<Self::Value, D::Error> {
        struct Object(Schema);
        impl<'de> serde::de::Visitor<'de> for Object {
            type Value = SourceEntry;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a source object")
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                mut map: A,
            ) -> Result<Self::Value, A::Error> {
                use serde::de::Error;
                use serde_json::Value;
                let mut entry = SourceEntry::default();
                while let Some(key) = map.next_key::<String>()? {
                    // Go folds keys with Unicode simple folding; the only
                    // non-ASCII characters folding onto the ASCII letters are
                    // U+017F (long s) and U+212A (Kelvin sign).
                    let field: String = key
                        .chars()
                        .map(|c| match c {
                            '\u{017f}' => 's',
                            '\u{212a}' => 'k',
                            c => c.to_ascii_lowercase(),
                        })
                        .collect();
                    if !self.0.has(&field) {
                        // Go does not convert the value of a field its struct lacks.
                        map.next_value::<serde::de::IgnoredAny>()?;
                        continue;
                    }
                    if matches!(field.as_str(), "update_interval_hours" | "rule_count") {
                        // Go parses the raw literal as int64 (`-0` is fine, `1.0` is not).
                        let raw = map.next_value::<Box<serde_json::value::RawValue>>()?;
                        let token = raw.get();
                        if token == "null" {
                            continue;
                        }
                        let value = token.parse::<i64>().map_err(|_| {
                            A::Error::custom(format!("field {field} is not an int64"))
                        })?;
                        if field == "rule_count" {
                            entry.rule_count = value;
                        } else {
                            entry.update_interval_hours = value;
                        }
                        continue;
                    }
                    if field == "last_updated" {
                        // Go hands the raw token to `time.Time`, which does not
                        // unescape: any backslash makes the whole file fail.
                        let raw = map.next_value::<Box<serde_json::value::RawValue>>()?;
                        let token = raw.get();
                        let valid = token == "null"
                            || token
                                .strip_prefix('"')
                                .and_then(|rest| rest.strip_suffix('"'))
                                .is_some_and(|text| !text.contains('\\') && is_rfc3339(text));
                        if !valid {
                            return Err(A::Error::custom("field last_updated is not a time"));
                        }
                        if token != "null" {
                            entry.last_updated = Some(token[1..token.len() - 1].to_owned());
                        }
                        continue;
                    }
                    let value = map.next_value::<Value>()?;
                    let wrong = || A::Error::custom(format!("field {field} has the wrong type"));
                    match (field.as_str(), value) {
                        (_, Value::Null) => {
                            // Go leaves the field untouched.
                        }
                        ("id", Value::String(text)) => entry.id = text,
                        ("name", Value::String(text)) => entry.name = text,
                        ("files", Value::String(text)) => entry.files = text,
                        ("type", Value::String(text)) => entry.kind = text,
                        ("url", Value::String(text)) => entry.url = text,
                        ("enabled", Value::Bool(flag)) => entry.enabled = flag,
                        ("enable_regexp", Value::Bool(flag)) => entry.enable_regexp = flag,
                        ("auto_update", Value::Bool(flag)) => entry.auto_update = flag,
                        _ => return Err(wrong()),
                    }
                }
                Ok(entry)
            }
        }
        deserializer.deserialize_map(Object(self.0))
    }
}

fn parse_manifest(bytes: &[u8], schema: Schema) -> Result<Vec<SourceEntry>, String> {
    use serde::de::DeserializeSeed;
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let entries = schema
        .deserialize(&mut deserializer)
        .map_err(|e| e.to_string())?;
    deserializer.end().map_err(|e| e.to_string())?;
    Ok(entries)
}

/// Whether the JSON text holds a string Go repairs instead of rejecting: a
/// lone UTF-16 surrogate escape inside a string, or invalid UTF-8 anywhere.
fn has_repairable_string(bytes: &[u8]) -> bool {
    if std::str::from_utf8(bytes).is_err() {
        return true;
    }
    let hex4 = |at: usize| -> Option<u32> {
        let digits = bytes.get(at..at + 4)?;
        u32::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok()
    };
    let (mut i, mut in_string) = (0, false);
    while i < bytes.len() {
        match bytes[i] {
            b'"' => in_string = !in_string,
            b'\\' if in_string => {
                if bytes.get(i + 1) == Some(&b'u')
                    && let Some(unit) = hex4(i + 2)
                {
                    i += 6;
                    match unit {
                        0xd800..=0xdbff => {
                            let paired = bytes.get(i..i + 2) == Some(b"\\u")
                                && hex4(i + 2).is_some_and(|low| (0xdc00..=0xdfff).contains(&low));
                            if !paired {
                                return true;
                            }
                            i += 6;
                        }
                        0xdc00..=0xdfff => return true,
                        _ => {}
                    }
                    continue;
                }
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    false
}

/// Decodes one source object the way the reference API handlers decode a
/// request body (`json.NewDecoder(body).Decode(&source)`): the first JSON value
/// only, the same per-field rules as the manifest. `null` is the zero source.
pub(crate) fn parse_source_object(bytes: &[u8], schema: Schema) -> Result<SourceEntry, String> {
    use serde::de::DeserializeSeed;
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    if text.trim_start().starts_with("null") {
        return Ok(SourceEntry::default());
    }
    EntrySeed(schema)
        .deserialize(&mut deserializer)
        .map_err(|e| e.to_string())
}

/// Where one manifest-backed provider keeps its state.
#[derive(Clone, Debug)]
pub(crate) struct ManifestSpec {
    pub(crate) tag: String,
    pub(crate) schema: Schema,
    /// `local_config`, or `<dir>/config.json` for `adguard_rule`.
    pub(crate) manifest: PathBuf,
    /// The `adguard_rule` list directory (unused by the other kinds).
    pub(crate) dir: PathBuf,
    /// The configuration root that relative `files` resolve against.
    pub(crate) root_dir: PathBuf,
    /// `socks5` proxy address for downloads; empty means direct.
    pub(crate) socks5: String,
}

impl ManifestSpec {
    /// The local rule file of one source.
    pub(crate) fn source_file(&self, entry: &SourceEntry) -> PathBuf {
        match self.schema {
            Schema::Adguard => self.dir.join(format!("{}.rules", entry.id)),
            Schema::Sd | Schema::Si => resolve_relative(&entry.files, &self.root_dir),
        }
    }
}

/// A manifest as loaded at compile time.
pub(crate) struct Manifest {
    pub(crate) entries: Vec<SourceEntry>,
    /// The file exists but could not be parsed: the reference starts empty and
    /// would overwrite it on the next change; this host refuses changes.
    pub(crate) unreadable: bool,
}

/// Reads a JSON array manifest. A missing, empty or unparsable manifest is an
/// empty source list (the reference host logs it and starts empty). Entries
/// are deduplicated like the reference map: by `name` (sd/si, nameless entries
/// dropped) or by `id` (adguard); a later duplicate replaces the earlier one.
pub(crate) fn read_manifest(
    spec: &ManifestSpec,
    mut inputs: Inputs<'_>,
) -> Result<Manifest, ConfigError> {
    let (path, plugin_tag, schema) = (&spec.manifest, spec.tag.as_str(), spec.schema);
    let bytes = match read_file(path, &mut inputs) {
        Ok(Some(bytes)) => bytes,
        Ok(None) => {
            return Ok(Manifest {
                entries: Vec::new(),
                unreadable: false,
            });
        }
        Err(error) => {
            crate::diag::warn_line!(
                "{plugin_tag}: cannot read source manifest {}: {error}; starting empty",
                path.display()
            );
            return Ok(Manifest {
                entries: Vec::new(),
                unreadable: true,
            });
        }
    };
    if bytes.is_empty() {
        return Ok(Manifest {
            entries: Vec::new(),
            unreadable: false,
        });
    }
    let entries = match parse_manifest(&bytes, schema) {
        Ok(entries) => entries,
        // Go replaces lone surrogates and invalid UTF-8 inside strings with
        // U+FFFD and carries on; this host does not, so it refuses instead of
        // silently starting empty.
        Err(error) if has_repairable_string(&bytes) => {
            return Err(ConfigError::new(
                path.display().to_string(),
                format!(
                    "{plugin_tag}: source manifest has a string the reference repairs \
                     (lone surrogate or invalid UTF-8) which this host does not: {error}"
                ),
            ));
        }
        Err(error) => {
            crate::diag::warn_line!(
                "{plugin_tag}: cannot parse source manifest {}: {error}; starting empty",
                path.display()
            );
            return Ok(Manifest {
                entries: Vec::new(),
                unreadable: true,
            });
        }
    };
    let mut out: Vec<SourceEntry> = Vec::new();
    for entry in entries {
        if schema != Schema::Adguard && entry.name.is_empty() {
            crate::diag::warn_line!("{plugin_tag}: skipping a rule source with an empty name");
            continue;
        }
        let key = |e: &SourceEntry| {
            if schema == Schema::Adguard {
                e.id.clone()
            } else {
                e.name.clone()
            }
        };
        match out.iter().position(|existing| key(existing) == key(&entry)) {
            Some(index) => out[index] = entry,
            None => out.push(entry),
        }
    }
    Ok(Manifest {
        entries: out,
        unreadable: false,
    })
}

/// `sd_set_light`/`si_set` (`local_config`) and `adguard_rule` (`dir`)
/// arguments.
pub(super) fn manifest_spec(
    plugin: &RawPlugin,
    schema: Schema,
) -> Result<ManifestSpec, ConfigError> {
    let kind = match schema {
        Schema::Sd => "sd_set_light",
        Schema::Si => "si_set",
        Schema::Adguard => "adguard_rule",
    };
    let allowed: &[&str] = if schema == Schema::Adguard {
        &["dir", "socks5"]
    } else {
        &["socks5", "local_config"]
    };
    let (path, args) = args_map(plugin, allowed)?;
    let socks5 = match args.get("socks5") {
        Some(value) if !matches!(value, RawValue::Null) => {
            expect_string(value, &format!("{path}.socks5"))?
        }
        _ => String::new(),
    };
    let key = if schema == Schema::Adguard {
        "dir"
    } else {
        "local_config"
    };
    let value = match args.get(key) {
        Some(value) => expect_string(value, &format!("{path}.{key}"))?,
        None => String::new(),
    };
    if value.is_empty() {
        return Err(ConfigError::new(
            format!("{path}.{key}"),
            format!("{kind}: '{key}' must be specified"),
        ));
    }
    let resolved = resolve_relative(&value, &plugin.root_dir);
    let (manifest, dir) = if schema == Schema::Adguard {
        (resolved.join("config.json"), resolved)
    } else {
        (resolved, PathBuf::new())
    };
    Ok(ManifestSpec {
        tag: plugin.tag.clone(),
        schema,
        manifest,
        dir,
        root_dir: plugin.root_dir.clone(),
        socks5,
    })
}

/// Rule counts observed while loading, keyed like the manifest (`name`, or
/// `id` for adguard). Only sources that were actually read are listed.
pub(crate) type Counts = Vec<(String, i64)>;

/// `sd_set_light`: enabled manifest sources whose `.srs` file decodes. The
/// count of a source that cannot be opened or decoded is left unchanged, as
/// in the reference reload.
pub(crate) fn load_sd(
    spec: &ManifestSpec,
    sources: &[SourceEntry],
    mut inputs: Inputs<'_>,
) -> (Vec<RuleEntry>, Counts) {
    let tag = &spec.tag;
    let mut out = Vec::new();
    let mut counts = Vec::new();
    for source in sources {
        if !source.enabled || source.files.is_empty() {
            continue;
        }
        let file = spec.source_file(source);
        let bytes = match read_file(&file, &mut inputs) {
            Ok(Some(bytes)) => bytes,
            Ok(None) | Err(_) => {
                crate::diag::warn_line!(
                    "{tag}: cannot open source file {} for '{}'",
                    file.display(),
                    source.name
                );
                continue;
            }
        };
        let name: Arc<str> = Arc::from(source.name.as_str());
        match decode_domain_srs(&bytes, source.enable_regexp) {
            Ok(srs) => {
                if !srs.complete {
                    crate::diag::warn_line!(
                        "{tag}: source '{}' ended early; using the rules read so far",
                        source.name
                    );
                }
                counts.push((source.name.clone(), srs.rules.len() as i64));
                out.extend(srs.rules.into_iter().map(|rule| RuleEntry {
                    rule,
                    source: Some(Arc::clone(&name)),
                }));
            }
            Err(error) => {
                crate::diag::warn_line!("{tag}: source '{}': {error}", source.name);
            }
        }
    }
    (out, counts)
}

/// `si_set`: the CIDRs of every enabled source's `.srs` file. A missing or
/// unusable file counts as zero, as in the reference reload.
pub(crate) fn load_si(
    spec: &ManifestSpec,
    sources: &[SourceEntry],
    mut inputs: Inputs<'_>,
) -> (IpPrefixList, Counts) {
    let tag = &spec.tag;
    let mut list = IpPrefixList::new();
    let mut counts = Vec::new();
    for source in sources {
        if !source.enabled {
            continue;
        }
        if source.files.is_empty() {
            crate::diag::warn_line!(
                "{tag}: skipping enabled source '{}', local file path is empty",
                source.name
            );
            continue;
        }
        let file = spec.source_file(source);
        let mut count = 0_i64;
        match read_file(&file, &mut inputs) {
            // A missing file is "not there yet", not an error.
            Ok(None) => {}
            Ok(Some(bytes)) => match decode_ip_srs(&bytes) {
                Ok(srs) => {
                    // Prefixes read before a decode problem stay, as in the
                    // reference loader.
                    count = srs.prefixes.len() as i64;
                    for (address, bits) in srs.prefixes {
                        list.append(address, bits);
                    }
                }
                Err(error) => {
                    crate::diag::error_line!(
                        "{tag}: failed to load source '{}' from {}: {error}",
                        source.name,
                        file.display()
                    );
                }
            },
            Err(error) => {
                crate::diag::error_line!(
                    "{tag}: failed to load source '{}' from {}: {error}",
                    source.name,
                    file.display()
                );
            }
        }
        counts.push((source.name.clone(), count));
    }
    list.rebuild();
    (list, counts)
}

/// The rules of one `adguard_rule`: the block rules `domain_mapper` consumes
/// (shared), and the `@@` exception rules a direct `qname $tag` reference
/// needs in addition.
pub(crate) struct AdguardRules {
    pub(crate) allow: Vec<String>,
    pub(crate) deny: ProviderRules,
}

/// The direct `qname $tag` matcher of an `adguard_rule`: a name matches when
/// no exception rule matches and a block rule does.
pub(crate) struct Filtered {
    pub(crate) allow: mosdns_matcher_core::MixMatcher<()>,
    pub(crate) deny: mosdns_matcher_core::MixMatcher<()>,
}

impl AdguardRules {
    /// The reference provider's direct matcher.
    ///
    /// # Errors
    ///
    /// A `regexp:` rule that is valid Go but outside what this host can run
    /// with Go semantics is refused instead of being left out.
    pub(crate) fn filtered(&self, tag: &str) -> Result<Filtered, String> {
        let build = |rules: &mut dyn Iterator<Item = &str>| -> Result<_, String> {
            let mut set = mosdns_matcher_core::MixMatcher::new();
            set.set_default("domain");
            for rule in rules {
                if let Some(pattern) = rule.strip_prefix("regexp:") {
                    match mosdns_matcher_core::check_go_regexp(pattern) {
                        Ok(()) => {}
                        Err(error) if error.is_invalid_in_go() => {
                            crate::diag::warn_line!("{tag}: skip invalid regexp rule {rule:?}");
                            continue;
                        }
                        Err(error) => return Err(unsupported_regexp_message(tag, rule, &error)),
                    }
                }
                if let Some(reason) = unicode_lowercase_refusal(tag, rule) {
                    return Err(reason);
                }
                // Any other rule the matcher rejects is not part of the set.
                let _ = set.add(rule, ());
            }
            Ok(set)
        };
        Ok(Filtered {
            allow: build(&mut self.allow.iter().map(String::as_str))?,
            deny: build(&mut self.deny.iter().map(|entry| entry.rule.as_str()))?,
        })
    }
}

/// Parses one filter list into `(allow, deny)` rules and the reference rule
/// count (lines that produce a rule, minus regexps Go rejects). The reference
/// scanner stops at a line over its limit and keeps what it read before.
pub(crate) fn parse_adguard_list(
    tag: &str,
    name: &str,
    bytes: &[u8],
    allow: &mut Vec<String>,
    deny: &mut Vec<RuleEntry>,
) -> i64 {
    let mut count = 0;
    for line in bytes.split(|byte| *byte == b'\n') {
        if line.len() >= MAX_LINE_BYTES {
            crate::diag::warn_line!(
                "{tag}: rule list '{name}' has a line over the scanner limit; the rest is ignored"
            );
            break;
        }
        let line = String::from_utf8_lossy(line);
        let Some((is_allow, rule)) = adguard_line(&line) else {
            continue;
        };
        let invalid = rule.strip_prefix("regexp:").is_some_and(|pattern| {
            mosdns_matcher_core::check_go_regexp(pattern).is_err_and(|e| e.is_invalid_in_go())
        });
        if !invalid {
            count += 1;
        }
        if is_allow {
            allow.push(rule);
        } else {
            deny.push(RuleEntry { rule, source: None });
        }
    }
    count
}

/// `adguard_rule`: rules (`||domain^`, wildcards, `/regex/`, plain
/// hostnames) of every enabled list, read from `<dir>/<id>.rules`. Missing
/// list files are skipped (counted as zero). With `count_all`, disabled lists
/// are read for their count too (the reference reload counts every list).
pub(crate) fn load_adguard(
    spec: &ManifestSpec,
    sources: &[SourceEntry],
    count_all: bool,
    mut inputs: Inputs<'_>,
) -> (AdguardRules, Counts) {
    let tag = spec.tag.as_str();
    let mut allow = Vec::new();
    let mut deny = Vec::new();
    let mut counts = Vec::new();
    for source in sources {
        if !source.enabled && !count_all {
            continue;
        }
        let file = spec.source_file(source);
        let bytes = match read_file(&file, &mut inputs) {
            Ok(Some(bytes)) => bytes,
            Ok(None) | Err(_) => {
                if source.enabled {
                    crate::diag::warn_line!(
                        "{tag}: skipping enabled rule '{}', cannot open {}",
                        source.name,
                        file.display()
                    );
                }
                counts.push((source.id.clone(), 0));
                continue;
            }
        };
        if source.enabled {
            let count = parse_adguard_list(tag, &source.name, &bytes, &mut allow, &mut deny);
            counts.push((source.id.clone(), count));
        } else {
            let count =
                parse_adguard_list(tag, &source.name, &bytes, &mut Vec::new(), &mut Vec::new());
            counts.push((source.id.clone(), count));
        }
    }
    (
        AdguardRules {
            allow,
            deny: Rc::new(deny),
        },
        counts,
    )
}

fn is_word(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

/// `[\w.\-*]+`
fn is_block_body(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| is_word(b) || matches!(b, b'.' | b'-' | b'*'))
}

/// `[\w.\-]+`
fn is_host_body(text: &str) -> bool {
    !text.is_empty() && text.bytes().all(|b| is_word(b) || matches!(b, b'.' | b'-'))
}

/// One filter-list line to the rule it contributes, if any: `(true, rule)` for
/// an `@@` exception, `(false, rule)` for a block rule. Mirrors the reference
/// line filter and its block/regex/plain-host forms.
fn adguard_line(raw: &str) -> Option<(bool, String)> {
    let line = raw.trim();
    if line.is_empty() || line.starts_with('!') || line.starts_with('#') {
        return None;
    }
    // IP-mapping lines (hosts-file style) are ignored.
    if line.bytes().any(|b| b.is_ascii_digit())
        && (line.contains("127.0.0.1") || line.contains("0.0.0.0") || line.contains("::"))
        && line.split_whitespace().count() > 1
    {
        return None;
    }
    // Cosmetic/element-hiding rules.
    if line.contains("#?#") || line.contains("##") || line.contains("$$") {
        return None;
    }
    let wildcard = |body: &str| {
        let domain = body.strip_prefix("*.").unwrap_or(body);
        let domain = domain.strip_prefix('.').unwrap_or(domain);
        if domain.contains('*') {
            let pattern = domain.replace('.', "\\.").replace('*', ".*");
            format!("regexp:{pattern}")
        } else {
            format!("domain:{domain}")
        }
    };
    if let Some(body) = line.strip_prefix("@@||").and_then(|r| r.strip_suffix('^')) {
        if is_block_body(body) {
            return Some((true, wildcard(body)));
        }
    }
    if let Some(body) = line.strip_prefix("||").and_then(|r| r.strip_suffix('^')) {
        if is_block_body(body) {
            return Some((false, wildcard(body)));
        }
    }
    if line.len() >= 2 && line.starts_with('/') && line.ends_with('/') {
        let pattern = &line[1..line.len() - 1];
        return Some((false, format!("regexp:{pattern}")));
    }
    if is_host_body(line) && line.contains('.') {
        return Some((false, format!("full:{line}")));
    }
    None
}

/// The compiled `domain_mapper`. Its matcher is the current published
/// generation; a hot rebuild (`crate::rule_sources`) replaces it whole.
#[derive(Clone)]
pub struct DomainMapperConfig {
    pub tag: String,
    pub(crate) matcher: Rc<crate::rule_sources::Live<ValuedDomainMatcher>>,
    /// Names injected by `domain_output` (`target_mapper`). It is not part of
    /// a matcher generation: a rebuild neither clears nor reads it.
    pub(crate) hot: Rc<HotOverlay>,
    default_mark: u8,
    default_ctx_mark: u32,
    default_tag: String,
}

/// Exact-name additions to a `domain_mapper`, visible to the next query
/// without rebuilding the matcher (the reference `QuickAdd`). Each
/// `domain_output` owns one layer (its mark and tag are fixed), so it can drop
/// what it added; a name found in several layers gets all their marks and
/// tags, merged with whatever the matcher says about it, hot values first.
#[derive(Default)]
pub(crate) struct HotOverlay {
    layers: std::cell::RefCell<Vec<Rc<HotLayer>>>,
}

/// The names one `domain_output` injected with its fixed mark and tag.
pub(crate) struct HotLayer {
    /// Fast-flag bit `mark` (none for mark 0), as in [`MapperOutcome::fast_flags`].
    flags: u64,
    tag: String,
    names: std::cell::RefCell<std::collections::HashSet<Box<str>>>,
}

impl HotOverlay {
    /// A new layer that adds fast mark `mark` (0 adds none) and `tag`.
    pub(crate) fn layer(&self, mark: u8, tag: &str) -> Rc<HotLayer> {
        let layer = Rc::new(HotLayer {
            flags: if (1..64).contains(&mark) {
                1 << mark
            } else {
                0
            },
            tag: tag.to_owned(),
            names: std::cell::RefCell::default(),
        });
        self.layers.borrow_mut().push(Rc::clone(&layer));
        layer
    }

    /// The flags and tags of a presentation-form name (compared in ASCII lower case).
    pub(crate) fn lookup(&self, name: &str) -> Option<(u64, String)> {
        let layers = self.layers.borrow();
        if layers.iter().all(|layer| layer.names.borrow().is_empty()) {
            return None;
        }
        let lower = name.to_ascii_lowercase();
        let (mut flags, mut tags, mut hit) = (0, String::new(), false);
        for layer in layers.iter() {
            if layer.names.borrow().contains(lower.as_str()) {
                hit = true;
                flags |= layer.flags;
                tags = append_joined(&tags, &layer.tag);
            }
        }
        hit.then_some((flags, tags))
    }
}

impl HotLayer {
    /// Adds `name`, a lower-case presentation name with or without its
    /// trailing dot.
    pub(crate) fn add(&self, name: &str) {
        let key = name.strip_suffix('.').unwrap_or(name);
        let mut full = String::with_capacity(key.len() + 1);
        full.push_str(key);
        full.push('.');
        self.names.borrow_mut().insert(full.into_boxed_str());
    }

    /// Forgets every name (and returns the memory).
    pub(crate) fn clear(&self) {
        *self.names.borrow_mut() = std::collections::HashSet::new();
    }
}

/// The reference `appendJoinedValue`: the `|`-separated parts of `value` that
/// `joined` does not have yet, in order.
pub(crate) fn append_joined(joined: &str, value: &str) -> String {
    let mut out = joined.to_owned();
    for part in value.split('|') {
        let part = part.trim();
        if part.is_empty() || out.split('|').any(|have| have.trim() == part) {
            continue;
        }
        if !out.is_empty() {
            out.push('|');
        }
        out.push_str(part);
    }
    out
}

/// The query name in DNS presentation form, which is what the reference host
/// matches on: labels joined by dots with a trailing dot, the bytes `.`, space,
/// `'`, `@`, `;`, `(`, `)`, `"` and `\` backslash-escaped, and bytes outside
/// space..`~` written as `\DDD`. `None` for a malformed wire name.
pub(crate) fn presentation_name(wire: &[u8]) -> Option<String> {
    let mut out = String::new();
    let mut position = 0;
    loop {
        let length = usize::from(*wire.get(position)?);
        position += 1;
        if length == 0 {
            if position != wire.len() {
                return None;
            }
            if out.is_empty() {
                out.push('.');
            }
            return Some(out);
        }
        if length > 63 {
            return None;
        }
        for byte in wire.get(position..position + length)? {
            match *byte {
                b'.' | b' ' | b'\'' | b'@' | b';' | b'(' | b')' | b'"' | b'\\' => {
                    out.push('\\');
                    out.push(char::from(*byte));
                }
                0..b' ' | 0x7f.. => out.push_str(&format!("\\{byte:03}")),
                other => out.push(char::from(other)),
            }
        }
        out.push('.');
        position += length;
    }
}

/// What a mapper decided for one query name.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MapperOutcome {
    /// Fast-flag bits to set (bit `n` is fast mark `n`).
    pub fast_flags: u64,
    pub ctx_marks: Vec<u32>,
    /// Joined output tags (`a|b`), when any.
    pub domain_set: Option<String>,
    /// Joined rule sources, when any.
    pub matched_rule_source: Option<String>,
    pub matched: bool,
}

impl DomainMapperConfig {
    /// The number of unique rules in the current generation.
    #[must_use]
    pub fn rule_count(&self) -> usize {
        self.matcher.get().len()
    }

    /// Evaluates a query name given in DNS wire form, exactly as the executor
    /// does. Any well-formed name is evaluated (underscore, leading-hyphen and
    /// other non-hostname labels included); a malformed one is a miss.
    #[must_use]
    pub fn evaluate_wire(&self, qname_wire: &[u8]) -> MapperOutcome {
        let name = presentation_name(qname_wire);
        self.evaluate(name.as_deref())
    }

    /// Evaluates a presentation-form query name (no trailing dot required).
    #[must_use]
    pub fn evaluate(&self, name: Option<&str>) -> MapperOutcome {
        let matcher = self.matcher.get();
        let hot = name.and_then(|name| self.hot.lookup(name));
        if let Some(result) = name.and_then(|name| matcher.r#match(name)) {
            // Rule mark `m` is stored as bit `m - 1` of the result mask and
            // set as fast flag `m`.
            let mut outcome = MapperOutcome {
                fast_flags: result.fast_marks << 1,
                ctx_marks: result.ctx_marks,
                domain_set: (!result.joined_tags.is_empty()).then_some(result.joined_tags),
                matched_rule_source: (!result.joined_sources.is_empty())
                    .then_some(result.joined_sources),
                matched: true,
            };
            if let Some((flags, tags)) = hot {
                outcome.fast_flags |= flags;
                let merged = append_joined(&tags, outcome.domain_set.as_deref().unwrap_or(""));
                outcome.domain_set = (!merged.is_empty()).then_some(merged);
            }
            return outcome;
        }
        if let Some((flags, tags)) = hot {
            return MapperOutcome {
                fast_flags: flags,
                ctx_marks: Vec::new(),
                domain_set: (!tags.is_empty()).then_some(tags),
                matched_rule_source: None,
                matched: true,
            };
        }
        MapperOutcome {
            fast_flags: if self.default_mark == 0 {
                0
            } else {
                1 << self.default_mark
            },
            ctx_marks: if self.default_ctx_mark == 0 {
                Vec::new()
            } else {
                vec![self.default_ctx_mark]
            },
            domain_set: (!self.default_tag.is_empty()).then(|| self.default_tag.clone()),
            matched_rule_source: None,
            matched: false,
        }
    }

    pub(super) fn executor(&self) -> Box<dyn Executor> {
        Box::new(MapperExecutor {
            config: self.clone(),
        })
    }
}

struct MapperExecutor {
    config: DomainMapperConfig,
}

impl Executor for MapperExecutor {
    fn execute(&self, state: &mut ExecutionState) -> Result<ExecutorOutcome, ExecutorError> {
        let outcome = self.config.evaluate_wire(&state.query.question.qname_wire);
        if outcome.fast_flags != 0 {
            state.apply_mutation(StateMutation::SetFastFlags(
                state.fast_flags | outcome.fast_flags,
            ));
        }
        for mark in outcome.ctx_marks {
            state.apply_mutation(StateMutation::AddMark(mark));
        }
        if let Some(tags) = outcome.domain_set {
            state.apply_mutation(StateMutation::SetRouting {
                field: RoutingField::DomainSet,
                value: Some(tags),
            });
        }
        if let Some(sources) = outcome.matched_rule_source {
            state.apply_mutation(StateMutation::SetRouting {
                field: RoutingField::MatchedRuleSource,
                value: Some(sources),
            });
        }
        Ok(ExecutorOutcome::Continue)
    }
}

/// One `domain_mapper` rule: which provider, and what a match applies.
#[derive(Clone, Debug)]
pub(crate) struct MapperRule {
    pub(crate) tag: String,
    mark: u8,
    ctx_mark: u32,
    output_tag: String,
}

/// The validated arguments of one `domain_mapper`.
#[derive(Clone, Debug)]
pub(crate) struct MapperSpec {
    pub(crate) tag: String,
    /// `$.plugins[i].args.rules`, for error locations.
    path: String,
    pub(crate) rules: Vec<MapperRule>,
    default_mark: u8,
    default_ctx_mark: u32,
    default_tag: String,
}

/// Validates a `domain_mapper` definition; every rule must name a loaded
/// provider.
pub(super) fn mapper_spec(
    plugin: &RawPlugin,
    kind_of: &dyn Fn(&str) -> Option<String>,
    is_provider: &dyn Fn(&str) -> bool,
) -> Result<MapperSpec, ConfigError> {
    let (path, args) = args_map(
        plugin,
        &["rules", "default_mark", "default_ctx_mark", "default_tag"],
    )?;
    let mark = |value: Option<&RawValue>, field: &str| -> Result<u8, ConfigError> {
        let Some(value) = value else { return Ok(0) };
        let number = expect_integer(value, &format!("{path}.{field}"))?;
        u8::try_from(number)
            .ok()
            .filter(|mark| *mark <= 63)
            .ok_or_else(|| {
                ConfigError::new(
                    format!("{path}.{field}"),
                    format!("{field} must be between 0 and 63, got {number}"),
                )
            })
    };
    let ctx = |value: Option<&RawValue>, field: &str| -> Result<u32, ConfigError> {
        let Some(value) = value else { return Ok(0) };
        let number = expect_integer(value, &format!("{path}.{field}"))?;
        u32::try_from(number).map_err(|_| {
            ConfigError::new(
                format!("{path}.{field}"),
                format!("{field} must be an unsigned 32-bit integer, got {number}"),
            )
        })
    };
    let default_mark = mark(args.get("default_mark"), "default_mark")?;
    let default_ctx_mark = ctx(args.get("default_ctx_mark"), "default_ctx_mark")?;
    let default_tag = match args.get("default_tag") {
        Some(value) => expect_string(value, &format!("{path}.default_tag"))?,
        None => String::new(),
    };

    let mut rules = Vec::new();
    if let Some(list) = args.get("rules") {
        if !matches!(list, RawValue::Null) {
            for (index, rule) in expect_sequence(list, &format!("{path}.rules"))?
                .iter()
                .enumerate()
            {
                let rule_path = format!("{path}.rules[{index}]");
                let map = expect_map(rule, &rule_path, "rule must be a mapping")?;
                map.reject_unknown(&["tag", "mark", "ctx_mark", "output_tag"], &rule_path)?;
                let tag = expect_string(
                    map.required("tag", &rule_path)?,
                    &format!("{rule_path}.tag"),
                )?;
                let rule_mark = mark(map.get("mark"), "mark")?;
                let ctx_mark = ctx(map.get("ctx_mark"), "ctx_mark")?;
                let output_tag = match map.get("output_tag") {
                    Some(value) => expect_string(value, &format!("{rule_path}.output_tag"))?,
                    None => String::new(),
                };
                if !is_provider(&tag) {
                    let at = format!("{rule_path}.tag");
                    return Err(match kind_of(&tag) {
                        None => ConfigError::unknown_reference(
                            at,
                            format!("plugin {tag} not found"),
                            &tag,
                        ),
                        Some(_) => ConfigError::new(
                            at,
                            format!("plugin {tag} does not support rule export"),
                        ),
                    });
                }
                rules.push(MapperRule {
                    tag,
                    mark: rule_mark,
                    ctx_mark,
                    output_tag,
                });
            }
        }
    }
    Ok(MapperSpec {
        tag: plugin.tag.clone(),
        path: format!("{path}.rules"),
        rules,
        default_mark,
        default_ctx_mark,
        default_tag,
    })
}

/// Builds a mapper matcher from the provider rules (looked up by tag). Used
/// at compile time and by hot rebuilds on the blocking pool.
pub(crate) fn build_mapper_matcher<'a>(
    spec: &MapperSpec,
    rules_of: &dyn Fn(&str) -> Option<&'a [RuleEntry]>,
) -> Result<ValuedDomainMatcher, ConfigError> {
    let mut valued: Vec<ValuedRule> = Vec::new();
    for rule in &spec.rules {
        let tag = rule.tag.as_str();
        let target_tag = if rule.output_tag.is_empty() {
            tag
        } else {
            rule.output_tag.as_str()
        };
        let fast_mask = if rule.mark > 0 {
            1u64 << (rule.mark - 1)
        } else {
            0
        };
        for entry in rules_of(tag).unwrap_or_default() {
            let source = entry
                .source
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or(tag);
            valued.push(ValuedRule::new(
                entry.rule.clone(),
                fast_mask,
                (rule.ctx_mark > 0).then_some(rule.ctx_mark),
                target_tag,
                source,
            ));
        }
    }
    build_matcher(valued, &spec.tag, &spec.path)
}

impl MapperSpec {
    /// The compiled mapper around an already built matcher.
    pub(super) fn config(&self, matcher: ValuedDomainMatcher) -> DomainMapperConfig {
        DomainMapperConfig {
            tag: self.tag.clone(),
            matcher: Rc::new(crate::rule_sources::Live::new(matcher)),
            hot: Rc::new(HotOverlay::default()),
            default_mark: self.default_mark,
            default_ctx_mark: self.default_ctx_mark,
            default_tag: self.default_tag.clone(),
        }
    }
}

/// Whether Go's `strings.ToLower` (a per-rune simple mapping) turns the text
/// into ASCII. Rust's full mapping differs only for U+0130, which Go maps to `i`.
fn go_lowercase_is_ascii(text: &str) -> bool {
    text.chars()
        .all(|c| c == '\u{130}' || c.to_lowercase().all(|lower| lower.is_ascii()))
}

/// The reference lowercases `domain:`/`keyword:` patterns with Unicode rules
/// (the Kelvin sign becomes `k`), so such a rule can match ASCII names; this
/// host does not reproduce that, so it refuses instead of dropping the rule.
fn unicode_lowercase_refusal(tag: &str, rule: &str) -> Option<String> {
    let (kind @ ("domain" | "keyword"), pattern) = rule.split_once(':')? else {
        return None;
    };
    (!rule.is_ascii() && go_lowercase_is_ascii(pattern)).then(|| {
        format!(
            "{tag}: {kind} rule {rule:?} is non-ASCII but lowercases to ASCII in the \
             reference; rewrite or remove the rule"
        )
    })
}

/// Why this host would refuse a configuration holding `rule` (a valid-Go
/// regexp outside the gate, or a Unicode-lowercase rule), if it would. Used
/// to keep downloads and posted lists from making the next start fail.
pub(crate) fn rule_refusal(tag: &str, rule: &str) -> Option<String> {
    if let Some(pattern) = rule.strip_prefix("regexp:") {
        return match mosdns_matcher_core::check_go_regexp(pattern) {
            Err(error) if !error.is_invalid_in_go() => {
                Some(unsupported_regexp_message(tag, rule, &error))
            }
            _ => None,
        };
    }
    unicode_lowercase_refusal(tag, rule)
}

fn unsupported_regexp_message(tag: &str, rule: &str, error: &RegexBuildError) -> String {
    format!(
        "{tag}: regexp rule {rule:?} is valid Go but uses syntax this host cannot run \
         with Go semantics ({error}); rewrite or remove the rule"
    )
}

fn known_rule_kind(rule: &str) -> bool {
    matches!(
        rule.split_once(':'),
        Some(("full" | "domain" | "keyword" | "regexp", _))
    )
}

/// Builds the matcher, skipping a rule the reference mapper would skip (an
/// unprefixed or unknown-type rule, an invalid regexp, a non-ASCII pattern)
/// with a diagnostic instead of failing the whole mapper.
fn build_matcher(
    mut rules: Vec<ValuedRule>,
    tag: &str,
    path: &str,
) -> Result<ValuedDomainMatcher, ConfigError> {
    rules.retain(|rule| {
        let keep = known_rule_kind(&rule.rule);
        if !keep {
            crate::diag::warn_line!("{tag}: skip invalid domain rule {:?}", rule.rule);
        }
        keep
    });
    // Validate the rules that can fail to compile one by one, so a list with
    // many unusable regexps costs one cheap check each instead of one full
    // rebuild each. A regexp Go rejects is skipped (as the reference mapper
    // does); a regexp that is valid Go but outside what this host runs with
    // Go semantics refuses the configuration rather than silently losing the
    // rule. A non-regexp rule with a non-ASCII pattern cannot match any
    // query name (names are ASCII in presentation form), so dropping it is
    // equivalent to keeping it.
    let mut refused: Option<String> = None;
    rules.retain(|rule| {
        if let Some(pattern) = rule.rule.strip_prefix("regexp:") {
            return match mosdns_matcher_core::check_go_regexp(pattern) {
                Ok(()) => true,
                Err(error) if error.is_invalid_in_go() => {
                    crate::diag::warn_line!("{tag}: skip invalid regexp rule {:?}", rule.rule);
                    false
                }
                Err(error) => {
                    refused
                        .get_or_insert_with(|| unsupported_regexp_message(tag, &rule.rule, &error));
                    false
                }
            };
        }
        let keep = rule.rule.is_ascii();
        if !keep && let Some(reason) = unicode_lowercase_refusal(tag, &rule.rule) {
            refused.get_or_insert(reason);
            return false;
        }
        if !keep {
            crate::diag::warn_line!("{tag}: skip non-ASCII rule {:?}", rule.rule);
        }
        keep
    });
    if let Some(reason) = refused {
        return Err(ConfigError::new(path, reason));
    }
    for _ in 0..100_000 {
        match ValuedDomainMatcher::build(&rules) {
            Ok(matcher) => return Ok(matcher),
            Err(
                ValuedBuildError::InvalidRule(rule)
                | ValuedBuildError::InvalidRegex { rule, .. }
                | ValuedBuildError::UnsupportedRule { rule, .. },
            ) => {
                crate::diag::warn_line!("{tag}: skip invalid rule {rule:?}");
                rules.retain(|candidate| candidate.rule != rule);
            }
        }
    }
    Err(ConfigError::new(
        "$",
        format!("{tag}: too many invalid rules to build a matcher"),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adguard_forms_map_to_mapper_rules() {
        let cases = [
            ("||ads.example.com^", Some("domain:ads.example.com")),
            ("||*.tracker.net^", Some("domain:tracker.net")),
            ("||.dot.example^", Some("domain:dot.example")),
            ("||a*.example.com^", Some("regexp:a.*\\.example\\.com")),
            ("/^ad[0-9]+\\./", Some("regexp:^ad[0-9]+\\.")),
            ("plain.example.org", Some("full:plain.example.org")),
            ("nodots", None),
            ("@@||allowed.example^", None),
            ("! comment", None),
            ("# comment", None),
            ("127.0.0.1 localhost.example", None),
            ("example.com##.banner", None),
            ("||bad^$third-party", None),
            ("", None),
        ];
        for (line, expected) in cases {
            let got = adguard_line(line).and_then(|(allow, rule)| (!allow).then_some(rule));
            assert_eq!(got.as_deref(), expected, "{line}");
        }
    }

    fn ids(bytes: &str, schema: Schema) -> Result<Vec<(String, String, bool)>, String> {
        parse_manifest(bytes.as_bytes(), schema).map(|entries| {
            entries
                .into_iter()
                .map(|e| (e.id, e.name, e.enabled))
                .collect()
        })
    }

    #[test]
    fn non_ascii_rules_that_lowercase_to_ascii_are_refused() {
        // Kelvin sign: Go lowercases it to `k`, so the rule matches ASCII names.
        assert!(unicode_lowercase_refusal("t", "domain:\u{212a}.example").is_some());
        assert!(unicode_lowercase_refusal("t", "keyword:\u{212a}").is_some());
        // U+0130 is `i` in Go's simple mapping (Rust yields `i` + a combining dot).
        assert!(unicode_lowercase_refusal("t", "domain:\u{130}.example").is_some());
        assert!(unicode_lowercase_refusal("t", "keyword:\u{130}").is_some());
        // Genuinely non-ASCII, ASCII, full: and regexp: rules are not refused.
        assert!(unicode_lowercase_refusal("t", "domain:例.example").is_none());
        assert!(unicode_lowercase_refusal("t", "domain:a.example").is_none());
        assert!(unicode_lowercase_refusal("t", "full:\u{212a}.example").is_none());
        assert!(unicode_lowercase_refusal("t", "regexp:\u{212a}").is_none());
    }

    #[test]
    fn manifest_decoding_follows_go_json() {
        // Keys match case-insensitively; the later spelling in the document wins.
        let both = r#"[{"name":"s","enabled":true,"Enabled":false}]"#;
        assert_eq!(
            ids(both, Schema::Sd).unwrap(),
            [("".into(), "s".into(), false)]
        );
        let both = r#"[{"name":"s","ENABLED":false,"enabled":true}]"#;
        assert_eq!(
            ids(both, Schema::Sd).unwrap(),
            [("".into(), "s".into(), true)]
        );
        // `null` leaves a field at its zero value, anywhere.
        let nulls =
            r#"[{"name":"a","files":null,"enabled":null,"rule_count":null,"last_updated":null}]"#;
        assert!(ids(nulls, Schema::Sd).is_ok());
        // A repeated key is type-checked at every occurrence.
        assert!(ids(r#"[{"enabled":"yes","enabled":true}]"#, Schema::Sd).is_err());
        // Only the fields the plugin's own struct has are type-checked.
        let extra = r#"[{"name":"a","enable_regexp":"unused","files":5}]"#;
        assert!(
            ids(extra, Schema::Si).is_err(),
            "`files` is a string in si_set"
        );
        let extra = r#"[{"name":"a","enable_regexp":"unused"}]"#;
        assert!(
            ids(extra, Schema::Si).is_ok(),
            "si_set has no enable_regexp"
        );
        assert!(ids(extra, Schema::Sd).is_err());
        let guard = r#"[{"id":"x","files":5,"type":7,"enable_regexp":1}]"#;
        assert!(
            ids(guard, Schema::Adguard).is_ok(),
            "adguard has no files/type/enable_regexp"
        );
        assert!(ids(r#"[{"id":5}]"#, Schema::Adguard).is_err());
        assert!(
            ids(r#"[{"id":5}]"#, Schema::Sd).is_ok(),
            "sd_set_light has no id"
        );
        // Integers must be int64 literals; times must be RFC 3339.
        assert!(ids(r#"[{"rule_count":-0}]"#, Schema::Sd).is_ok());
        assert!(ids(r#"[{"rule_count":1.0}]"#, Schema::Sd).is_err());
        assert!(ids(r#"[{"rule_count":1e2}]"#, Schema::Sd).is_err());
        assert!(ids(r#"[{"rule_count":1.5}]"#, Schema::Sd).is_err());
        assert!(ids(r#"[{"rule_count":18446744073709551615}]"#, Schema::Sd).is_err());
        assert!(ids(r#"[{"update_interval_hours":-3}]"#, Schema::Sd).is_ok());
        for good in [
            "2026-05-13T23:56:02,5Z",
            "2026-05-13T23:56:02+24:00",
            "2026-05-13T23:56:02-00:00",
            "2026-05-13T23:56:02.956038327+08:00",
            "2026-05-13T23:56:02Z",
            "0001-01-01T00:00:00Z",
            "2026-05-13T3:56:02Z",
            "2026-05-13T3:56:02.5Z",
            "2026-05-13T23:56:02+23:60",
            "2026-05-13T23:56:02-24:00",
        ] {
            let json = format!(r#"[{{"last_updated":"{good}"}}]"#);
            assert!(ids(&json, Schema::Sd).is_ok(), "{good}");
        }
        // Go validates the raw token: an escape is never a valid time.
        assert!(
            ids(
                r#"[{"last_updated":"2026-05-13\u005423:56:02Z"}]"#,
                Schema::Sd
            )
            .is_err()
        );
        assert!(
            ids(
                r#"[{"last_updated":"2026-05-13T23:56:02\u005a"}]"#,
                Schema::Sd
            )
            .is_err()
        );
        assert!(ids(r#"[{"last_updated":null}]"#, Schema::Sd).is_ok());
        assert!(ids(r#"[{"last_updated":5}]"#, Schema::Sd).is_err());
        for bad in [
            "not-a-time",
            "2026-13-01T00:00:00Z",
            "2026-02-30T00:00:00Z",
            "2026-05-13 23:56:02Z",
            "2026-05-13T23:56:02",
            "2026-05-13T24:00:00Z",
            "2026-05-13t23:56:02Z",
            "2026-05-13T23:56:02z",
            "2026-05-13T23:56:02.Z",
            "2026-05-13T23:56:60Z",
            "2026-05-13T23:56:02+0800",
            "2026-05-13T23:56:02+25:00",
            "2026-05-13T3:56:02+25:00",
            "2026-05-13T23:56:02+23:61",
            "2026-05-13T3:56:2Z",
            "2026-5-13T3:56:02Z",
            "",
        ] {
            let json = format!(r#"[{{"last_updated":"{bad}"}}]"#);
            assert!(ids(&json, Schema::Sd).is_err(), "{bad:?}");
        }
        // Keys fold with Unicode simple folding (long s, Kelvin sign).
        let folded =
            parse_manifest("[{\"name\":\"a\",\"fileſ\":\"f\"}]".as_bytes(), Schema::Sd).unwrap();
        assert_eq!(folded[0].files, "f");
        // Values of fields the plugin's struct lacks are not even converted.
        assert!(ids(r#"[{"id":1e1000,"name":"a"}]"#, Schema::Sd).is_ok());
        // Strings Go repairs (U+FFFD) are reported so the caller can refuse.
        for text in [
            r#"[{"url":"\ud800"}]"#,
            r#"[{"url":"\udc00"}]"#,
            r#"[{"url":"\ud800\u0041"}]"#,
            r#"[{"url":"a\\\udc00"}]"#,
        ] {
            assert!(has_repairable_string(text.as_bytes()), "{text}");
        }
        assert!(has_repairable_string(b"[{\"url\":\"\xff\"}]"));
        for text in [
            r#"[{"url":"\ud83d\ude00"}]"#,
            r#"[{"url":"\\ud800"}]"#,
            r#"[{"url":"\u12"}]"#,
        ] {
            assert!(!has_repairable_string(text.as_bytes()), "{text}");
        }
        // Not an array / not objects / trailing data.
        assert!(ids("{}", Schema::Sd).is_err());
        assert!(ids("[1]", Schema::Sd).is_err());
        assert!(ids("[] []", Schema::Sd).is_err());
    }

    #[test]
    fn exception_lines_become_allow_rules() {
        assert_eq!(
            adguard_line("@@||ok.example^"),
            Some((true, "domain:ok.example".to_owned()))
        );
        assert_eq!(
            adguard_line("@@||*.cdn.example^"),
            Some((true, "domain:cdn.example".to_owned()))
        );
        assert_eq!(adguard_line("@@||bad^$important"), None);
    }

    #[test]
    fn a_lone_hostname_with_ip_text_is_not_an_ip_mapping() {
        // One token only: the IP-mapping filter needs a second field.
        assert_eq!(
            adguard_line("0.0.0.0"),
            Some((false, "full:0.0.0.0".to_owned()))
        );
    }
}
