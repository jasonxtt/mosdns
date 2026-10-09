//! Schema-4 standard-mode groups. JSON is authoritative; YAML is derived.
use super::{ConfigError, EffectiveItem, RawMap, RawPlugin, RawValue, settings};
use serde::{Deserialize, Serialize};
use settings::UpstreamEntry;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

pub(crate) const STATE: &str = "webinfo/special_upstream_groups.json";
pub(crate) const GENERATED: &str = "sub_config/special_groups.yaml";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct Source {
    pub(crate) kind: String,
    pub(crate) plugin_tag: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) upstream_tag: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub(crate) struct Group {
    pub(crate) slot: u32,
    pub(crate) name: String,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub(crate) listen_port: u16,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub(crate) custom_port_only: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) upstream_sources: Vec<Source>,
    #[serde(default)]
    pub(crate) owned_upstreams: Vec<UpstreamEntry>,
}
fn is_zero(port: &u16) -> bool {
    *port == 0
}

#[derive(Clone, Debug)]
pub(crate) struct Resolution {
    pub(crate) entries: Vec<UpstreamEntry>,
    pub(crate) warnings: Vec<String>,
    raw: Vec<RawValue>,
}

#[derive(Clone, Debug)]
pub(crate) struct Profile {
    pub(crate) base: PathBuf,
    pub(crate) groups: Vec<Group>,
    pub(crate) resolutions: BTreeMap<u32, Resolution>,
    pub(crate) generated: Vec<u8>,
    pub(crate) expected_state: Option<Vec<u8>>,
}

pub(crate) fn read_state(base: &Path) -> Result<Option<Vec<u8>>, ConfigError> {
    use std::io::Read as _;
    let read = || -> std::io::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        std::fs::File::open(base.join(STATE))?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut bytes)?;
        Ok(bytes)
    };
    match read() {
        Ok(bytes) if bytes.len() <= 1024 * 1024 => Ok(Some(bytes)),
        Ok(_) => Err(error("group state exceeds 1 MiB")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(error(format!("cannot read group state: {e}"))),
    }
}
fn error(reason: impl Into<String>) -> ConfigError {
    ConfigError::new(STATE, reason)
}

pub(crate) fn normalize(groups: &mut [Group]) -> Result<(), ConfigError> {
    let mut slots = BTreeSet::new();
    let mut names = BTreeSet::new();
    let mut ports = BTreeSet::new();
    for group in groups.iter_mut() {
        group.name = group.name.trim().to_owned();
        if group.slot < 50 {
            return Err(error("slot must be >= 50"));
        }
        if group.name.is_empty() {
            return Err(error("name is required"));
        }
        if !slots.insert(group.slot) {
            return Err(error("duplicate slot"));
        }
        if !names.insert(group.name.to_lowercase()) {
            return Err(error("name already exists"));
        }
        if group.listen_port == 53 {
            return Err(error("监听端口不能使用 53"));
        }
        if group.listen_port == 0 {
            group.custom_port_only = false;
        } else if !ports.insert(group.listen_port) {
            return Err(error("监听端口已被其他专属分流组使用"));
        }
        let mut seen = BTreeSet::new();
        let mut sources = Vec::new();
        for mut source in std::mem::take(&mut group.upstream_sources) {
            source.kind = source.kind.trim().to_lowercase();
            source.plugin_tag = source.plugin_tag.trim().to_owned();
            source.upstream_tag = source.upstream_tag.trim().to_owned();
            if !matches!(source.kind.as_str(), "group" | "upstream")
                || source.plugin_tag.is_empty()
                || (source.kind == "upstream" && source.upstream_tag.is_empty())
            {
                return Err(error("invalid special upstream source"));
            }
            if reserved(&source.plugin_tag) {
                return Err(error("专属分流组不能引用其他专属分流组"));
            }
            if seen.insert((
                source.kind.clone(),
                source.plugin_tag.clone(),
                source.upstream_tag.clone(),
            )) {
                sources.push(source);
            }
        }
        group.upstream_sources = sources;
        let mut tags = BTreeSet::new();
        for (index, entry) in group.owned_upstreams.iter_mut().enumerate() {
            entry.tag = entry.tag.trim().to_owned();
            entry.protocol = entry.protocol_name();
            if entry.tag.is_empty() || !tags.insert(entry.tag.clone()) {
                return Err(error("专属自有上游标识为空或重复"));
            }
            if entry.enabled {
                entry.validate(index).map_err(error)?;
            }
        }
    }
    groups.sort_by_key(|group| group.slot);
    Ok(())
}
pub(crate) fn reserved(tag: &str) -> bool {
    tag == "special_upstream_matcher"
        || tag == "sequence_special"
        || [
            "special_upstream_",
            "special_route_",
            "special_manual_",
            "cache_special_",
            "sequence_special_",
            "special_udp_server_",
            "special_tcp_server_",
        ]
        .iter()
        .any(|prefix| tag.starts_with(prefix))
}

/// Keep Go's persisted proxy switch name; settings' use_proxy is an input alias.
pub(crate) fn entry_json(entry: &UpstreamEntry) -> serde_json::Value {
    let mut value = serde_json::to_value(entry).unwrap_or_default();
    if let Some(map) = value.as_object_mut() {
        map.remove("use_proxy");
        map.insert("use_socks_proxy".into(), entry.proxy_switch().into());
    }
    value
}
pub(crate) fn state_bytes(groups: &[Group]) -> Vec<u8> {
    let values: Vec<_> = groups
        .iter()
        .map(|group| {
            let mut value = serde_json::to_value(group).unwrap_or_default();
            value["owned_upstreams"] =
                serde_json::Value::Array(group.owned_upstreams.iter().map(entry_json).collect());
            value
        })
        .collect();
    serde_json::to_vec_pretty(&values).unwrap_or_default()
}
impl Profile {
    pub(crate) fn recover(&self) -> Result<(), ConfigError> {
        if read_state(&self.base)? != self.expected_state {
            return Err(error("group state changed before startup recovery"));
        }
        crate::runtime_snapshot::write_durably(&self.base.join(GENERATED), &self.generated)
            .map_err(|e| error(format!("generated config recovery: {e}")))?;
        let active: BTreeSet<_> = self.groups.iter().map(|g| g.slot).collect();
        for (directory, prefix, suffix) in [
            ("cache", "cache_special_", ".dump"),
            ("srs", "special_", ".json"),
            ("rule", "special_", ".txt"),
        ] {
            let paths = match std::fs::read_dir(self.base.join(directory)) {
                Ok(paths) => paths,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => return Err(error(e.to_string())),
            };
            for path in paths {
                let path = path.map_err(|e| error(e.to_string()))?;
                let name = path.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                let Some(slot) = name
                    .strip_prefix(prefix)
                    .and_then(|n| n.strip_suffix(suffix))
                    .and_then(|n| n.parse::<u32>().ok())
                else {
                    continue;
                };
                if slot >= 50 && !active.contains(&slot) {
                    std::fs::remove_file(path.path()).map_err(|e| error(e.to_string()))?;
                }
            }
        }
        Ok(())
    }
    pub(crate) fn view(&self, group: &Group) -> serde_json::Value {
        let resolution = &self.resolutions[&group.slot];
        let slot = group.slot;
        let mut value = serde_json::to_value(group).unwrap_or_default();
        value["owned_upstreams"] =
            serde_json::Value::Array(group.owned_upstreams.iter().map(entry_json).collect());
        for (key, text) in [
            ("key", format!("special_{slot}")),
            ("upstream_plugin_tag", format!("special_upstream_{slot}")),
            ("diversion_plugin_tag", format!("special_route_{slot}")),
            ("manual_plugin_tag", format!("special_manual_{slot}")),
            ("local_config", format!("srs/special_{slot}.json")),
            ("manual_rule_path", format!("rule/special_{slot}.txt")),
        ] {
            value[key] = text.into();
        }
        value["effective_upstream_count"] = resolution.entries.len().into();
        value["upstream_active"] = (!resolution.entries.is_empty()).into();
        if !resolution.warnings.is_empty() {
            value["upstream_warnings"] = serde_json::json!(resolution.warnings);
        }
        value
    }
}

fn text(map: &RawMap, key: &str) -> String {
    match map.get(key) {
        Some(RawValue::String(s)) => s.clone(),
        _ => String::new(),
    }
}
fn set(map: &mut RawMap, key: &str, value: RawValue) {
    map.entries.retain(|(k, _)| k != key);
    map.entries.push((key.to_owned(), value));
}

fn raw_json(raw: &RawValue) -> serde_json::Value {
    match raw {
        RawValue::Null => serde_json::Value::Null,
        RawValue::Bool(value) => (*value).into(),
        RawValue::Number(super::RawNumber::Signed(value)) => (*value).into(),
        RawValue::Number(super::RawNumber::Unsigned(value)) => (*value).into(),
        RawValue::Number(super::RawNumber::Float) => serde_json::Value::Null,
        RawValue::String(value) => value.clone().into(),
        RawValue::Sequence(values) => {
            serde_json::Value::Array(values.iter().map(raw_json).collect())
        }
        RawValue::Map(map) => serde_json::Value::Object(
            map.entries
                .iter()
                .map(|(key, value)| (key.clone(), raw_json(value)))
                .collect(),
        ),
    }
}

pub(super) fn apply(
    base: &Path,
    candidate: Option<&[Group]>,
    definitions: &mut Vec<RawPlugin>,
    applied: &mut settings::Applied,
) -> Result<Profile, ConfigError> {
    let expected_state = read_state(base)?;
    let mut groups = match candidate {
        Some(groups) => groups.to_vec(),
        None => match expected_state.as_deref() {
            None | Some([]) => Vec::new(),
            Some(bytes) => serde_json::from_slice(bytes)
                .map_err(|e| error(format!("invalid group state: {e}")))?,
        },
    };
    normalize(&mut groups)?;
    if definitions.iter().any(|p| reserved(&p.tag)) {
        return Err(error("reserved generated plugin tag collision"));
    }
    // Ordinary definitions have already passed settings::apply. Their entries
    // now hold the resolved literal proxy, including entry-specific references.
    let catalog: BTreeMap<String, Vec<RawValue>> = definitions
        .iter()
        .filter(|p| p.kind == "aliapi")
        .map(|p| {
            let items = match &p.args {
                RawValue::Map(args) => match args.get("upstreams") {
                    Some(RawValue::Sequence(items)) => items.clone(),
                    _ => Vec::new(),
                },
                _ => Vec::new(),
            };
            (p.tag.clone(), items)
        })
        .collect();
    let proxies: Vec<String> = definitions
        .iter()
        .filter(|p| p.kind == "socks5_proxy")
        .map(|p| match &p.args {
            RawValue::Map(map) => text(map, "addr"),
            _ => String::new(),
        })
        .collect();
    let mut resolutions = BTreeMap::new();
    let mut runtime_tags: BTreeSet<String> = definitions
        .iter()
        .filter(|p| matches!(p.kind.as_str(), "aliapi" | "forward"))
        .flat_map(|p| match &p.args {
            RawValue::Map(args) => match args.get("upstreams") {
                Some(RawValue::Sequence(items)) => items
                    .iter()
                    .filter_map(|item| match item {
                        RawValue::Map(map) => Some(text(map, "tag")),
                        _ => None,
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            },
            _ => Vec::new(),
        })
        .collect();
    for group in &groups {
        if group
            .owned_upstreams
            .iter()
            .any(UpstreamEntry::proxy_switch)
            && proxies.len() != 1
        {
            return Err(error(if proxies.is_empty() {
                "no socks5_proxy object for owned upstream use_proxy=true"
            } else {
                "ambiguous: several socks5_proxy objects for owned upstream use_proxy=true"
            }));
        }
        let mut resolution = Resolution {
            entries: Vec::new(),
            warnings: Vec::new(),
            raw: Vec::new(),
        };
        for entry in group.owned_upstreams.iter().filter(|entry| entry.enabled) {
            let mut raw = settings::upstream_to_raw(entry);
            if let RawValue::Map(map) = &mut raw {
                map.entries.retain(|(key, _)| key != "use_proxy");
                if entry.proxy_switch() {
                    let proxy = match proxies.as_slice() {
                        [] => {
                            return Err(error(
                                "no socks5_proxy object for owned upstream use_proxy=true",
                            ));
                        }
                        [proxy] => proxy,
                        _ => {
                            return Err(error(
                                "ambiguous: several socks5_proxy objects for owned upstream use_proxy=true",
                            ));
                        }
                    };
                    set(map, "socks5", RawValue::String(proxy.clone()));
                }
            }
            resolution.entries.push(entry.clone());
            resolution.raw.push(raw);
        }
        let whole: BTreeSet<_> = group
            .upstream_sources
            .iter()
            .filter(|s| s.kind == "group")
            .map(|s| s.plugin_tag.as_str())
            .collect();
        let mut seen = BTreeSet::new();
        for source in &group.upstream_sources {
            let Some(entries) = applied.upstreams.get(&source.plugin_tag) else {
                resolution
                    .warnings
                    .push(format!("源上游组不存在：{}", source.plugin_tag));
                continue;
            };
            if source.kind == "upstream" && whole.contains(source.plugin_tag.as_str()) {
                continue;
            }
            let mut found = false;
            let mut active = 0;
            for (index, entry) in entries.iter().enumerate() {
                if source.kind == "upstream" && entry.tag != source.upstream_tag {
                    continue;
                }
                found = true;
                if !entry.enabled {
                    if source.kind == "upstream" {
                        resolution.warnings.push(format!(
                            "源上游已禁用：{} / {}",
                            source.plugin_tag, source.upstream_tag
                        ));
                    }
                    continue;
                }
                let key = (
                    source.plugin_tag.clone(),
                    if entry.tag.is_empty() {
                        format!("#{index}")
                    } else {
                        entry.tag.clone()
                    },
                );
                if !seen.insert(key) {
                    continue;
                }
                let raw = catalog[&source.plugin_tag]
                    .iter()
                    .find(|raw| matches!(raw, RawValue::Map(map) if text(map, "tag") == entry.tag))
                    .cloned()
                    .ok_or_else(|| error("resolved upstream entry missing"))?;
                let mut raw = raw;
                // Literal plugin-wide socks5 is inherited too.
                if let RawValue::Map(map) = &mut raw {
                    if let Some(RawPlugin {
                        args: RawValue::Map(args),
                        ..
                    }) = definitions.iter().find(|p| p.tag == source.plugin_tag)
                    {
                        for key in ["bootstrap", "bootstrap_version"] {
                            let inherits = map.get(key).is_none()
                                || (key == "bootstrap" && text(map, key).is_empty())
                                || (key == "bootstrap_version"
                                    && map
                                        .get(key)
                                        .is_some_and(|value| raw_json(value).as_i64() == Some(0)));
                            if inherits {
                                if let Some(value) = args.get(key) {
                                    set(map, key, value.clone());
                                }
                            }
                        }
                    }
                    if text(map, "socks5").is_empty() {
                        if let Some(RawPlugin {
                            args: RawValue::Map(args),
                            ..
                        }) = definitions.iter().find(|p| p.tag == source.plugin_tag)
                        {
                            let proxy = text(args, "socks5");
                            if !proxy.is_empty() {
                                set(map, "socks5", RawValue::String(proxy));
                            }
                        }
                    }
                }
                resolution.entries.push(entry.clone());
                resolution.raw.push(raw);
                active += 1;
            }
            if source.kind == "upstream" && !found {
                resolution.warnings.push(format!(
                    "源上游不存在：{} / {}",
                    source.plugin_tag, source.upstream_tag
                ));
            }
            if source.kind == "group" && active == 0 {
                resolution
                    .warnings
                    .push(format!("源上游组没有启用中的上游：{}", source.plugin_tag));
            }
        }
        if resolution.entries.is_empty() {
            resolution
                .warnings
                .push("没有可用的有效上游，本专属分流组当前不参与分流".to_owned());
        }
        for (index, raw) in resolution.raw.iter_mut().enumerate() {
            if let RawValue::Map(map) = raw {
                // This identity exists only in derived runtime YAML, never in state.
                let base = format!("special_{}_{}", group.slot, index);
                let mut tag = base.clone();
                let mut suffix = 2;
                while !runtime_tags.insert(tag.clone()) {
                    tag = format!("{base}_{suffix}");
                    suffix += 1;
                }
                set(map, "tag", RawValue::String(tag));
            }
        }
        let value = resolution
            .raw
            .iter()
            .map(|raw| match raw {
                RawValue::Map(map) => format!(
                    "{}={} socks5={}",
                    text(map, "tag"),
                    text(map, "addr"),
                    text(map, "socks5")
                ),
                _ => String::new(),
            })
            .collect::<Vec<_>>()
            .join(", ");
        applied.items.push(EffectiveItem {
            object: format!("special_upstream_{}", group.slot),
            kind: "aliapi",
            field: "upstreams",
            value,
            source: STATE,
        });
        applied.items.push(EffectiveItem {
            object: format!("special_upstream_{}", group.slot),
            kind: "special_groups",
            field: "sources",
            value: serde_json::to_string(&group.upstream_sources).unwrap_or_default(),
            source: STATE,
        });
        for warning in &resolution.warnings {
            eprintln!("special_{}: {warning}", group.slot);
        }
        applied.upstreams.insert(
            format!("special_upstream_{}", group.slot),
            resolution.entries.clone(),
        );
        resolutions.insert(group.slot, resolution);
    }
    let generated = render(&groups, &resolutions);
    let raw = super::parse_yaml(&generated)?;
    let map = super::expect_map(&raw, GENERATED, "generated groups")?;
    let plugins = super::expect_sequence(map.required("plugins", GENERATED)?, GENERATED)?;
    for (index, plugin) in plugins.iter().enumerate() {
        definitions.push(super::decode_plugin(
            plugin,
            &format!("{GENERATED}.plugins[{index}]"),
            base,
        )?);
    }
    Ok(Profile {
        base: base.to_path_buf(),
        groups,
        resolutions,
        generated: generated.into_bytes(),
        expected_state,
    })
}

fn render(groups: &[Group], resolutions: &BTreeMap<u32, Resolution>) -> String {
    let mut b = String::from("plugins:\n# 由 special_groups API 自动生成。不要手动编辑。\n");
    for g in groups {
        let n = g.slot;
        let _ = write!(
            b,
            "  - tag: special_route_{n}\n    type: sd_set_light\n    args:\n      socks5: \"\"\n      local_config: \"srs/special_{n}.json\"\n\n  - tag: special_manual_{n}\n    type: domain_set_light\n    args:\n      files:\n        - \"rule/special_{n}.txt\"\n\n  - tag: cache_special_{n}\n    type: cache\n    args:\n      size: 20000000\n      lazy_cache_ttl: 259200000\n      dump_file: cache/cache_special_{n}.dump\n      dump_interval: 36000\n\n  - tag: special_upstream_{n}\n    type: aliapi\n    args:\n      concurrent: 2\n      upstreams:\n"
        );
        if resolutions[&n].raw.is_empty() {
            b.push_str("        - addr: \"223.5.5.5\"\n\n");
        } else {
            // JSON flow mappings are YAML too. Only this block differs from Go.
            for raw in &resolutions[&n].raw {
                let _ = writeln!(b, "        - {}", raw_json(raw));
            }
            b.push('\n');
        }
        let _ = write!(
            b,
            "  - tag: sequence_special_{n}\n    type: sequence\n    args:\n      - matches: switch4 'A'\n        exec: $cache_special_{n}\n      - exec: flow_setter group=special_{n} sequence=sequence_special_{n} upstream=special_upstream_{n}\n      - exec: $special_upstream_{n}\n      - exec: cname_remover\n\n"
        );
        if !resolutions[&n].entries.is_empty() && g.listen_port != 0 {
            for protocol in ["udp", "tcp"] {
                let _ = write!(
                    b,
                    "  - tag: special_{protocol}_server_{n}\n    type: {protocol}_server\n    args:\n      entry: sequence_special_{n}\n      enable_audit: true\n      listen: \":{}\"\n\n",
                    g.listen_port
                );
            }
        }
    }
    let main: Vec<_> = groups
        .iter()
        .filter(|g| !g.custom_port_only && !resolutions[&g.slot].entries.is_empty())
        .collect();
    b.push_str("  - tag: special_upstream_matcher\n    type: domain_mapper\n    args:\n      default_mark: 0\n      default_tag: \"\"\n");
    if main.is_empty() {
        b.push_str("      rules: []\n\n");
    } else {
        b.push_str("      rules:\n");
        for g in &main {
            for kind in ["route", "manual"] {
                let n = g.slot;
                let _ = write!(
                    b,
                    "        - tag: special_{kind}_{n}\n          ctx_mark: {n}\n          output_tag: \"特殊上游{n}\"\n"
                );
            }
        }
        b.push('\n');
    }
    b.push_str("  - tag: sequence_special\n    type: sequence\n");
    if main.is_empty() {
        b.push_str("    args: []\n\n");
    } else {
        b.push_str("    args:\n");
        for g in main {
            let n = g.slot;
            let _ = write!(
                b,
                "      - matches: mark {n}\n        exec: $sequence_special_{n}\n      - matches: mark {n}\n        exec: exit\n"
            );
        }
        b.push('\n');
    }
    b
}
