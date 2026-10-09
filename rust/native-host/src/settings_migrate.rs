//! One-time migration of the reference host's override files to the native
//! `webinfo/settings.json` (M1-f2). The inputs are `config_overrides.json`,
//! `upstream_overrides.json` and `domain_generation_settings.json`; the
//! outputs are the settings (differences from the YAML only) and a report that
//! names everything that could not be carried over. The reference's inference
//! and fallback rules are reproduced from `coremain/api_upstream.go`.
//! See `docs/rust/contracts/native-settings.md` and the task's `design.md`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::config::settings::{
    Addr, NO_V4, NO_V6, PluginSwitch, REMEMBER_DIRECT, REMEMBER_PROXY, Settings, UpstreamEntry,
    YamlDefaults,
};

/// The reference gives these groups the global proxy when an entry uses a
/// proxy without an address of its own.
const FALLBACK_GROUPS: [&str; 2] = ["foreign", "foreignecs"];

/// The reference's `maxConcurrentQueries` (`plugin/executable/aliapi/aliapi.go`).
const MAX_CONCURRENT: usize = 20;

/// What to migrate and where the result goes.
pub struct Options {
    /// The reference configuration directory (`webinfo/` below it, or the
    /// three files directly in it).
    pub from: PathBuf,
    /// The native configuration file and `-d` root.
    pub config: PathBuf,
    pub dir: Option<PathBuf>,
    /// Receives `settings.json` and `migration-report.txt`.
    pub out: PathBuf,
}

/// The result of a migration run.
#[derive(Debug)]
pub struct Outcome {
    /// 0: everything mapped; 1: some item needs a person.
    pub exit_code: i32,
    pub report: String,
}

#[derive(Default, Deserialize)]
struct GoGlobal {
    #[serde(default)]
    socks5: String,
    #[serde(default)]
    ecs: String,
    #[serde(default)]
    replacements: Vec<GoReplacement>,
}

#[derive(Deserialize)]
struct GoReplacement {
    #[serde(default)]
    original: String,
    #[serde(default)]
    new: String,
}

#[derive(Deserialize)]
struct GoGeneration {
    #[serde(default = "yes")]
    enabled: bool,
    #[serde(default = "yes")]
    remember_direct: bool,
    #[serde(default = "yes")]
    remember_proxy: bool,
    #[serde(default = "yes")]
    no_v4: bool,
    #[serde(default = "yes")]
    no_v6: bool,
}

fn yes() -> bool {
    true
}

/// The reference state that is migrated.
#[derive(Default)]
struct GoState {
    global: Option<GoGlobal>,
    upstreams: Option<BTreeMap<String, Vec<UpstreamEntry>>>,
    generation: Option<GoGeneration>,
}

/// What the native configuration says without any settings.
struct Context {
    /// `socks5_proxy` objects and their YAML addresses.
    proxies: BTreeMap<String, String>,
    ecs: BTreeMap<String, String>,
    /// The `aliapi` groups in configuration order and their YAML lists.
    groups: Vec<String>,
    upstreams: BTreeMap<String, Vec<UpstreamEntry>>,
    concurrent: BTreeMap<String, usize>,
    generation: BTreeMap<String, bool>,
}

impl Context {
    fn new(
        defaults: &YamlDefaults,
        groups: Vec<String>,
        concurrent: BTreeMap<String, usize>,
    ) -> Self {
        Self {
            proxies: defaults.proxies.clone(),
            ecs: defaults.ecs.clone(),
            groups,
            upstreams: defaults.upstreams.clone(),
            concurrent,
            generation: defaults.generation.clone(),
        }
    }
}

#[derive(Default)]
struct Report {
    lines: Vec<String>,
    unmapped: usize,
}

impl Report {
    fn info(&mut self, text: impl AsRef<str>) {
        self.lines.push(format!("[信息] {}", text.as_ref()));
    }

    fn mapped(&mut self, text: impl AsRef<str>) {
        self.lines.push(format!("[映射] {}", text.as_ref()));
    }

    fn renamed(&mut self, text: impl AsRef<str>) {
        self.lines.push(format!("[改名] {}", text.as_ref()));
    }

    fn unmapped(&mut self, text: impl AsRef<str>) {
        self.unmapped += 1;
        self.lines.push(format!("[无法映射] {}", text.as_ref()));
    }
}

/// Runs the whole migration and writes its two outputs.
pub fn run(options: &Options) -> Result<Outcome, String> {
    let (config_path, root) =
        crate::cli::resolve_config_paths(&options.config, options.dir.as_deref())
            .map_err(|error| error.to_string())?;
    let yaml = crate::config::load_yaml(&config_path).map_err(|error| error.to_string())?;
    let base_dir = config_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf();
    let root_dir = root.unwrap_or_else(|| base_dir.clone());
    let compiled =
        crate::config::compile_file_with_roots(&yaml, &config_path, &base_dir, &root_dir)
            .map_err(|error| format!("the native configuration does not compile: {error}"))?;
    let groups: Vec<String> = compiled
        .forward_definitions
        .iter()
        .filter(|definition| {
            compiled
                .yaml_defaults
                .upstreams
                .contains_key(&definition.tag)
        })
        .map(|definition| definition.tag.clone())
        .collect();
    let concurrent = compiled
        .forward_definitions
        .iter()
        .map(|definition| (definition.tag.clone(), definition.concurrent))
        .collect();
    let context = Context::new(&compiled.yaml_defaults, groups, concurrent);

    let mut report = Report::default();
    if crate::config::settings::Settings::path(&root_dir).is_file() {
        report.info(
            "原生配置根目录里已经有 webinfo/settings.json（原生里做过的修改）；\
             本工具的输出不包含它，直接覆盖会丢失那些修改，请人工合并",
        );
    }
    let state = read_go_state(&options.from, &mut report)?;
    let settings = plan(&state, &context, &mut report);

    // The result must load and say what was planned.
    let reload = compiled
        .reload
        .clone()
        .ok_or_else(|| "the native configuration was not loaded from a file".to_owned())?;
    let applied = crate::config::recompile(&reload, &settings)
        .map_err(|error| format!("the migrated settings do not load: {error}"))?;
    summarize(&applied, &mut report);

    std::fs::create_dir_all(&options.out)
        .map_err(|error| format!("{}: {error}", options.out.display()))?;
    let settings_path = options.out.join("settings.json");
    if settings_path.exists() {
        return Err(format!(
            "{} exists; remove it first",
            settings_path.display()
        ));
    }
    let mut text = String::new();
    let _ = writeln!(
        text,
        "迁移报告：{} 项无法映射（退出码 {}）",
        report.unmapped,
        i32::from(report.unmapped > 0)
    );
    for line in &report.lines {
        text.push_str(line);
        text.push('\n');
    }
    // Both files are written under temporary names first; `settings.json`
    // appears last, so its presence means the whole result is there.
    let report_path = options.out.join("migration-report.txt");
    let temporary = [
        (options.out.join(".settings.json.tmp"), settings.to_bytes()),
        (
            options.out.join(".migration-report.txt.tmp"),
            text.clone().into_bytes(),
        ),
    ];
    let published = (|| {
        for (path, bytes) in &temporary {
            std::fs::write(path, bytes)?;
        }
        std::fs::rename(&temporary[1].0, &report_path)?;
        std::fs::rename(&temporary[0].0, &settings_path)
    })();
    if let Err(error) = published {
        for (path, _) in &temporary {
            let _ = std::fs::remove_file(path);
        }
        let _ = std::fs::remove_file(&report_path);
        return Err(format!("{}: {error}", options.out.display()));
    }
    Ok(Outcome {
        exit_code: i32::from(report.unmapped > 0),
        report: text,
    })
}

fn read_go_state(from: &Path, report: &mut Report) -> Result<GoState, String> {
    let mut state = GoState::default();
    let find = |name: &str| {
        // The reference moves older layouts (the root, `state/`) into `webinfo/`.
        [
            from.join("webinfo").join(name),
            from.join("state").join(name),
            from.join(name),
        ]
        .into_iter()
        .find(|path| path.is_file())
    };
    let read = |name: &str| -> Result<Option<Vec<u8>>, String> {
        find(name)
            .map(|path| {
                std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))
            })
            .transpose()
    };
    match read("config_overrides.json")? {
        Some(bytes) => {
            state.global = Some(
                serde_json::from_slice(&bytes)
                    .map_err(|error| format!("config_overrides.json: {error}"))?,
            );
        }
        None => report.info("没有 config_overrides.json：代理与 ECS 保持 YAML 默认值"),
    }
    match read("upstream_overrides.json")? {
        Some(bytes) => {
            state.upstreams = Some(
                serde_json::from_slice(&bytes)
                    .map_err(|error| format!("upstream_overrides.json: {error}"))?,
            );
        }
        None => report.info("没有 upstream_overrides.json：上游列表保持 YAML 默认值"),
    }
    match read("domain_generation_settings.json")? {
        Some(bytes) => {
            state.generation = Some(
                serde_json::from_slice(&bytes)
                    .map_err(|error| format!("domain_generation_settings.json: {error}"))?,
            );
        }
        None => report.info("没有 domain_generation_settings.json：生成开关保持默认（全开）"),
    }
    Ok(state)
}

/// The migration proper: pure over the reference state and the YAML defaults.
fn plan(state: &GoState, context: &Context, report: &mut Report) -> Settings {
    let mut settings = Settings::default();
    let global_socks5 = state
        .global
        .as_ref()
        .map(|global| global.socks5.trim().to_owned())
        .unwrap_or_default();
    if let Some(global) = &state.global {
        global_values(global, context, &mut settings, report);
    }
    if let Some(upstreams) = &state.upstreams {
        upstream_lists(upstreams, &global_socks5, context, &mut settings, report);
    }
    if let Some(generation) = &state.generation {
        generation_switches(generation, context, &mut settings, report);
    }
    report.info(
        "未迁移：special_upstream_groups.json 等其它状态文件（special_groups 托管由 M1-e 处理）；\
         原生不读取任何 Go 覆盖文件",
    );
    settings
}

/// The one object of a kind, or why there is not exactly one.
fn only<'a>(
    objects: &'a BTreeMap<String, String>,
    kind: &str,
) -> Result<(&'a String, &'a String), String> {
    let mut all = objects.iter();
    match (all.next(), all.next()) {
        (Some(one), None) => Ok(one),
        (None, _) => Err(format!("配置里没有 {kind} 对象")),
        _ => Err(format!(
            "配置里有多个 {kind} 对象（{}），无法判断对应哪一个",
            objects.keys().cloned().collect::<Vec<_>>().join("、")
        )),
    }
}

fn global_values(
    global: &GoGlobal,
    context: &Context,
    settings: &mut Settings,
    report: &mut Report,
) {
    for (value, objects, kind, label) in [
        (
            global.socks5.trim(),
            &context.proxies,
            "socks5_proxy",
            "全局 socks5",
        ),
        (global.ecs.trim(), &context.ecs, "ecs_address", "全局 ecs"),
    ] {
        if value.is_empty() {
            report.info(format!("{label} 为空：不覆盖 YAML 默认值"));
            continue;
        }
        let (tag, yaml) = match only(objects, kind) {
            Ok(found) => found,
            Err(reason) => {
                report.unmapped(format!("{label}：{reason}"));
                continue;
            }
        };
        if value == yaml {
            report.info(format!("{label} 与 {tag} 的 YAML 默认值相同：不写入"));
            continue;
        }
        let map = if kind == "socks5_proxy" {
            &mut settings.proxies
        } else {
            &mut settings.ecs
        };
        map.insert(
            tag.clone(),
            Addr {
                addr: value.to_owned(),
            },
        );
        report.mapped(format!("{label} → {kind} `{tag}`"));
        if kind == "ecs_address" && (value.split_whitespace().count() > 1 || value.contains('/')) {
            report.info("ecs 值带有掩码或附加字段；原生只使用其中的 IP（与原生 quick ecs 一致）");
        }
    }
    for rule in &global.replacements {
        if rule.original.trim().is_empty() {
            continue;
        }
        report.unmapped(format!(
            "replacements：`{}` → `{}` 不受支持（原生用 socks5_proxy/ecs_address 对象，不做任意字符串替换）；请改写 YAML",
            rule.original, rule.new
        ));
    }
}

/// Whether the reference host used a proxy for this entry: an explicit
/// `use_socks_proxy`, else TCP-class protocols with an own address or in a
/// group that falls back to the global address.
fn go_uses_proxy(group: &str, entry: &UpstreamEntry) -> bool {
    let fallback = FALLBACK_GROUPS.contains(&group.trim());
    let supports = matches!(
        entry.protocol_name().as_str(),
        "tcp" | "tls" | "https" | "quic"
    );
    entry
        .use_socks_proxy
        .or(entry.use_proxy)
        .unwrap_or(supports && (!entry.socks5.trim().is_empty() || fallback))
}

fn upstream_lists(
    files: &BTreeMap<String, Vec<UpstreamEntry>>,
    global_socks5: &str,
    context: &Context,
    settings: &mut Settings,
    report: &mut Report,
) {
    for tag in files.keys() {
        if !context.upstreams.contains_key(tag) {
            report.unmapped(format!(
                "upstream_overrides.{tag}：配置里没有名为 `{tag}` 的 aliapi（special group 的上游绑定属于 M1-e）"
            ));
        }
    }
    let mut mapped: BTreeMap<String, Vec<UpstreamEntry>> = BTreeMap::new();
    for group in &context.groups {
        let enabled: Vec<&UpstreamEntry> = files
            .get(group)
            .into_iter()
            .flatten()
            .filter(|entry| entry.enabled)
            .collect();
        if enabled.is_empty() {
            // Go ran the YAML list of its own configuration, which this tool
            // cannot see; the native default list may differ from it.
            report.unmapped(format!(
                "{group}：覆盖文件里没有这个组或没有启用的条目，Go 实际运行的是它自己 YAML 里的上游列表（本工具看不到）；\
                 原生使用配置包的默认列表，请核对两者是否一致，不一致就在上游页设置（停用的预置条目不迁移）"
            ));
            continue;
        }
        let entries = &files[group];
        let go_concurrent = enabled.len().clamp(1, MAX_CONCURRENT);
        if context
            .concurrent
            .get(group)
            .is_some_and(|yaml| *yaml != go_concurrent)
        {
            report.info(format!(
                "{group}：Go 按启用数把 concurrent 重算为 {go_concurrent}，原生保持 YAML 的 {}（如需一致请改 YAML）",
                context.concurrent[group]
            ));
        }
        let list = group_entries(group, entries, global_socks5, report);
        if list.iter().any(|entry| entry.enabled) {
            mapped.insert(group.clone(), list);
        } else {
            report.unmapped(format!(
                "{group}：没有任何启用条目可以迁移，该组的列表未写入（运行时将使用 YAML 默认列表）"
            ));
        }
    }

    // Upstream tags are unique across groups in the native host.
    let mut used: BTreeSet<String> = BTreeSet::new();
    for group in &context.groups {
        let Some(list) = mapped.get_mut(group) else {
            for entry in context.upstreams.get(group).into_iter().flatten() {
                used.insert(entry.tag.clone());
            }
            continue;
        };
        for entry in list.iter_mut().filter(|entry| entry.enabled) {
            let original = entry.tag.clone();
            let mut name = original.clone();
            while used.contains(&name) {
                name = if name == original {
                    format!("{original}-{group}")
                } else {
                    format!("{name}-")
                };
            }
            if name != original {
                report.renamed(format!(
                    "{group}：上游标签 `{original}` 与其它组重复，原生要求全局唯一，改名为 `{name}`"
                ));
                entry.tag = name.clone();
            }
            used.insert(name);
        }
    }

    for (group, list) in mapped {
        let canonical = |entries: &[UpstreamEntry]| {
            entries
                .iter()
                .map(UpstreamEntry::canonical)
                .collect::<Vec<_>>()
        };
        if context
            .upstreams
            .get(&group)
            .is_some_and(|yaml| canonical(yaml) == canonical(&list))
        {
            report.info(format!("{group}：迁移后的列表与 YAML 默认相同，不写入"));
            continue;
        }
        report.mapped(format!(
            "{group}：{} 个条目（启用 {}）→ settings.upstreams.{group}",
            list.len(),
            list.iter().filter(|entry| entry.enabled).count()
        ));
        settings.upstreams.insert(group, list);
    }
}

/// One group's entries in the native shape; what cannot be carried over is
/// reported, an enabled entry that is left out is an unmapped item.
fn group_entries(
    group: &str,
    entries: &[UpstreamEntry],
    global_socks5: &str,
    report: &mut Report,
) -> Vec<UpstreamEntry> {
    let mut out = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let name = if entry.tag.is_empty() {
            format!("#{}", index + 1)
        } else {
            entry.tag.clone()
        };
        let place = format!("{group}[{name}]");
        let used = go_uses_proxy(group, entry);
        let literal = entry.socks5.trim();
        let tcp_class = matches!(entry.protocol_name().as_str(), "tcp" | "tls" | "https");
        let use_proxy = if !used {
            false
        } else if !tcp_class {
            report.info(format!(
                "{place}：协议 {} 不使用代理（与 Go 一致，原生也忽略）",
                entry.protocol_name()
            ));
            false
        } else if literal.is_empty() {
            if FALLBACK_GROUPS.contains(&group) {
                true
            } else {
                report.info(format!(
                    "{place}：Go 里勾选了代理但条目没有代理地址，实际直连；原生记为不走代理"
                ));
                false
            }
        } else if literal == global_socks5 {
            true
        } else {
            let reason = "条目有自己的 socks5 地址且与全局不同，原生只有一个 main_proxy，无法表示"
                .to_owned();
            drop_entry(report, &place, entry, &reason);
            continue;
        };
        let mut mapped = entry.clone();
        mapped.socks5.clear();
        mapped.use_socks_proxy = None;
        mapped.use_proxy = Some(use_proxy);
        if let Err(reason) = mapped.validate(index) {
            drop_entry(report, &place, entry, &reason);
            continue;
        }
        out.push(mapped);
    }
    out
}

fn drop_entry(report: &mut Report, place: &str, entry: &UpstreamEntry, reason: &str) {
    if entry.enabled {
        report.unmapped(format!("{place}：{reason}；该条目未迁移"));
    } else {
        report.info(format!("{place}：停用的条目丢弃（{reason}）"));
    }
}

fn generation_switches(
    generation: &GoGeneration,
    context: &Context,
    settings: &mut Settings,
    report: &mut Report,
) {
    if !generation.enabled {
        settings.domain_generation.enabled = Some(false);
        report.mapped("域名生成主开关关闭 → domain_generation.enabled=false");
    }
    for (tag, value) in [
        (REMEMBER_DIRECT, generation.remember_direct),
        (REMEMBER_PROXY, generation.remember_proxy),
        (NO_V4, generation.no_v4),
        (NO_V6, generation.no_v6),
    ] {
        let Some(yaml) = context.generation.get(tag) else {
            if !value {
                report.unmapped(format!(
                    "域名生成开关 `{tag}` 关闭，但配置里没有名为 `{tag}` 的 domain_output"
                ));
            }
            continue;
        };
        if *yaml != value {
            settings
                .domain_generation
                .plugins
                .insert(tag.to_owned(), PluginSwitch { enabled: value });
            report.mapped(format!(
                "域名生成开关 `{tag}`={value} → domain_generation.plugins"
            ));
        }
    }
}

/// The effective values after the migration, one line each.
fn summarize(applied: &crate::config::CompiledConfig, report: &mut Report) {
    report
        .lines
        .push("生效值汇总（用迁移后的设置重新编译得到）：".to_owned());
    for item in applied.settings_report() {
        let value = if item.kind == "socks5_proxy" {
            if item.value.is_empty() {
                "（无代理）".to_owned()
            } else {
                "（已设置）".to_owned()
            }
        } else {
            item.value.clone()
        };
        report.lines.push(format!(
            "  {} {} {}={} 来源={}",
            item.kind, item.object, item.field, value, item.source
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> Context {
        let mut upstreams = BTreeMap::new();
        for (group, tag, addr) in [
            ("foreign", "google", "https://dns.google/dns-query"),
            (
                "foreignecs",
                "google-foreignecs",
                "https://dns.google/dns-query",
            ),
            ("domestic", "ali", "udp://223.5.5.5"),
        ] {
            upstreams.insert(
                group.to_owned(),
                vec![UpstreamEntry {
                    tag: tag.to_owned(),
                    enabled: true,
                    protocol: addr.split("://").next().unwrap().to_owned(),
                    addr: addr.to_owned(),
                    use_proxy: Some(group.starts_with("foreign")),
                    ..UpstreamEntry::default()
                }],
            );
        }
        Context {
            proxies: BTreeMap::from([("main_proxy".to_owned(), String::new())]),
            ecs: BTreeMap::from([("client_ecs".to_owned(), "2408:8888::8".to_owned())]),
            groups: vec!["domestic".into(), "foreign".into(), "foreignecs".into()],
            upstreams,
            concurrent: BTreeMap::from([
                ("domestic".into(), 2),
                ("foreign".into(), 2),
                ("foreignecs".into(), 1),
            ]),
            generation: BTreeMap::from([("my_realiplist".to_owned(), true)]),
        }
    }

    fn run_plan(json: &str) -> (Settings, Report) {
        let state = GoState {
            global: Some(serde_json::from_str(r#"{"socks5":"10.0.0.1:1080","ecs":""}"#).unwrap()),
            upstreams: Some(serde_json::from_str(json).unwrap()),
            generation: None,
        };
        let mut report = Report::default();
        let settings = plan(&state, &context(), &mut report);
        (settings, report)
    }

    #[test]
    fn global_values_land_on_their_objects_and_defaults_are_not_written() {
        let state = GoState {
            global: Some(
                serde_json::from_str(
                    r#"{"socks5":" 10.0.0.1:1080 ","ecs":"2408:8888::8","replacements":[{"original":"a","new":"b"},{"original":"","new":"x"}]}"#,
                )
                .unwrap(),
            ),
            ..GoState::default()
        };
        let mut report = Report::default();
        let settings = plan(&state, &context(), &mut report);
        assert_eq!(settings.proxies["main_proxy"].addr, "10.0.0.1:1080");
        assert!(settings.ecs.is_empty(), "equal to the YAML value");
        assert_eq!(report.unmapped, 1, "{:?}", report.lines);
        assert!(
            report
                .lines
                .iter()
                .any(|line| line.contains("replacements") && line.contains("`a`"))
        );
    }

    #[test]
    fn the_reference_proxy_inference_becomes_explicit_use_proxy() {
        let (settings, report) = run_plan(
            r#"{
              "foreign": [
                {"tag":"g","enabled":true,"protocol":"https","addr":"https://dns.google/dns-query","dial_addr":"8.8.8.8"},
                {"tag":"direct","enabled":true,"protocol":"https","addr":"https://1.1.1.1/dns-query","use_socks_proxy":false},
                {"tag":"u","enabled":true,"protocol":"udp","addr":"9.9.9.9","use_socks_proxy":true},
                {"tag":"own","enabled":true,"protocol":"tcp","addr":"1.0.0.1","socks5":"10.0.0.1:1080"}
              ],
              "domestic": [
                {"tag":"ali","enabled":true,"protocol":"https","addr":"https://dns.alidns.com/dns-query","use_socks_proxy":true},
                {"tag":"cn","enabled":true,"protocol":"tcp","addr":"114.114.114.114"}
              ],
              "foreignecs": [
                {"tag":"google","enabled":true,"protocol":"https","addr":"https://dns.google/dns-query"}
              ]}"#,
        );
        let proxy_of = |group: &str, tag: &str| {
            settings.upstreams[group]
                .iter()
                .find(|entry| entry.tag == tag)
                .unwrap()
                .use_proxy
        };
        assert_eq!(
            proxy_of("foreign", "g"),
            Some(true),
            "tag fallback of foreign"
        );
        assert_eq!(proxy_of("foreign", "direct"), Some(false));
        assert_eq!(
            proxy_of("foreign", "u"),
            Some(false),
            "UDP never uses a proxy"
        );
        assert_eq!(
            proxy_of("foreign", "own"),
            Some(true),
            "literal equal to the global address"
        );
        assert_eq!(
            proxy_of("domestic", "ali"),
            Some(false),
            "no address: Go connected directly"
        );
        assert_eq!(proxy_of("domestic", "cn"), Some(false));
        assert_eq!(report.unmapped, 0, "{:?}", report.lines);
    }

    #[test]
    fn what_cannot_be_carried_over_is_reported_not_dropped_silently() {
        let (settings, report) = run_plan(
            r#"{
              "foreign": [
                {"tag":"ok","enabled":true,"protocol":"https","addr":"https://dns.google/dns-query"},
                {"tag":"mark","enabled":true,"protocol":"udp","addr":"9.9.9.9","so_mark":5},
                {"tag":"signed","enabled":true,"protocol":"aliapi","server_addr":"223.5.5.5"},
                {"tag":"mine","enabled":true,"protocol":"tcp","addr":"1.0.0.1","socks5":"192.0.2.9:1"},
                {"tag":"old","enabled":false,"protocol":"aliapi","server_addr":"223.5.5.5"}
              ],
              "ghost": [{"tag":"x","enabled":true,"protocol":"udp","addr":"1.1.1.1"}]}"#,
        );
        assert_eq!(
            settings.upstreams["foreign"].len(),
            1,
            "only the mappable entry is written"
        );
        // Four entries/tags, plus `domestic` and `foreignecs`, which the file does not have.
        assert_eq!(report.unmapped, 6, "{:?}", report.lines);
        let joined = report.lines.join("\n");
        for needle in ["mark", "signed", "mine", "ghost", "domestic", "foreignecs"] {
            assert!(joined.contains(needle), "{needle} missing from\n{joined}");
        }
        assert!(
            joined.contains("[信息] foreign[old]"),
            "disabled entries are noted: {joined}"
        );
    }

    #[test]
    fn a_list_equal_to_the_yaml_is_not_written_and_a_duplicate_tag_is_renamed() {
        let (settings, report) = run_plan(
            r#"{
              "foreign": [{"tag":"google","enabled":true,"protocol":"https","addr":"https://dns.google/dns-query"}],
              "foreignecs": [{"tag":"google","enabled":true,"protocol":"https","addr":"https://dns.google/dns-query"}],
              "domestic": [{"tag":"ali","enabled":true,"protocol":"udp","addr":"udp://223.5.5.5"}]}"#,
        );
        // All lists equal the YAML defaults once `foreignecs` is renamed.
        assert!(settings.upstreams.is_empty(), "{:?}", settings.upstreams);
        assert!(
            report
                .lines
                .iter()
                .any(|line| line.starts_with("[改名]") && line.contains("google-foreignecs"))
        );
        assert_eq!(report.unmapped, 0);
    }

    #[test]
    fn a_group_whose_reference_list_cannot_be_seen_needs_a_person() {
        // `foreign` has only disabled entries (the reference ran its own YAML
        // list), `foreignecs` is missing from the file, `domestic` is fine.
        let (settings, report) = run_plan(
            r#"{
              "foreign": [{"tag":"google","enabled":false,"protocol":"https","addr":"https://dns.google/dns-query"}],
              "domestic": [{"tag":"ali","enabled":true,"protocol":"udp","addr":"udp://223.5.5.5"}]}"#,
        );
        assert!(settings.upstreams.is_empty(), "{:?}", settings.upstreams);
        assert_eq!(report.unmapped, 2, "{:?}", report.lines);
        let joined = report.lines.join("\n");
        assert!(
            joined.contains("[无法映射] foreign：") && joined.contains("[无法映射] foreignecs："),
            "{joined}"
        );
    }

    #[test]
    fn generation_switches_are_written_only_when_they_differ() {
        let state = GoState {
            generation: Some(
                serde_json::from_str(
                    r#"{"enabled":false,"remember_direct":true,"remember_proxy":false,"no_v4":true,"no_v6":true}"#,
                )
                .unwrap(),
            ),
            ..GoState::default()
        };
        let mut report = Report::default();
        let settings = plan(&state, &context(), &mut report);
        assert_eq!(settings.domain_generation.enabled, Some(false));
        assert!(settings.domain_generation.plugins.is_empty());
        // `my_fakeiplist` is off in Go but the configuration has no such list.
        assert_eq!(report.unmapped, 1, "{:?}", report.lines);
    }
}
