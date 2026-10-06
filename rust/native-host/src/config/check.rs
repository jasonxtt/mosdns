//! Side-effect-free `mosdns check`: runs the same compiler as `start`
//! repeatedly, removing each rejected definition (or neutralizing one
//! rejected sequence item) until the remainder compiles, so one run lists
//! every gap. No support list is kept here; the compiler is the only judge.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

use super::{
    CompileSources, ConfigError, ConfigErrorKind, LogLevel, ROOT_FIELDS, RawMap, RawPlugin,
    RawValue, apply_managed_definitions, compile_api, compile_log, decode_plugin, expect_map,
    expect_sequence, expect_string, finish_compile, included_plugins, load_yaml, managed_opt_in,
    parse_yaml, read_included, reject_generated_include, resolve_relative,
};

const SKIPPED: &str =
    "SKIPPED managed-store open/recovery and pending-transaction gate (start-only; writes state)";
const NOTE_FIRST_ERROR: &str =
    "NOTE first-error-per-definition; rerun after fixing to see further gaps";
const NOTE_SEQUENCE: &str = "NOTE sequences are checked per matcher/exec/rule item; the failing item is neutralized and checking continues";

/// How one finding relates to the definitions the check removed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CheckFindingKind {
    /// The definition or item itself is not supported.
    Gap,
    /// Rejected only because it references a definition removed earlier.
    Cascade { removed: String },
    /// Not attributable to one definition; checking stops here.
    Global,
}

/// One compiler rejection with its location.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckFinding {
    pub kind: CheckFindingKind,
    pub tag: Option<String>,
    pub plugin_type: Option<String>,
    /// The declaring file as written in the configuration.
    pub source: String,
    /// Field path inside `source`.
    pub field: String,
    /// The compiler's original rejection reason.
    pub message: String,
}

/// The complete result of one check run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CheckReport {
    pub findings: Vec<CheckFinding>,
    /// Declared plugin definitions (including undecodable and managed
    /// generated ones).
    pub definitions: usize,
}

impl CheckReport {
    fn count(&self, matches: impl Fn(&CheckFindingKind) -> bool) -> usize {
        self.findings
            .iter()
            .filter(|finding| matches(&finding.kind))
            .count()
    }

    /// `0` when the configuration compiles unchanged, otherwise `1`.
    #[must_use]
    pub fn exit_code(&self) -> i32 {
        i32::from(!self.findings.is_empty())
    }
}

impl fmt::Display for CheckReport {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for finding in &self.findings {
            let tag = finding.tag.as_deref().unwrap_or("-");
            let kind = finding.plugin_type.as_deref().unwrap_or("-");
            let location = format!("{}:{}", finding.source, finding.field);
            match &finding.kind {
                CheckFindingKind::Gap => {
                    writeln!(
                        formatter,
                        "GAP {tag} {kind} {location}  {}",
                        finding.message
                    )?;
                }
                CheckFindingKind::Cascade { removed } => writeln!(
                    formatter,
                    "CASCADE {tag} {kind} {location}  references removed `{removed}`; {}",
                    finding.message
                )?,
                CheckFindingKind::Global => {
                    writeln!(formatter, "GLOBAL {location}  {}", finding.message)?;
                }
            }
        }
        writeln!(formatter, "{SKIPPED}")?;
        writeln!(formatter, "{NOTE_FIRST_ERROR}")?;
        writeln!(formatter, "{NOTE_SEQUENCE}")?;
        writeln!(
            formatter,
            "SUMMARY gaps={} cascades={} globals={} definitions={}",
            self.count(|kind| *kind == CheckFindingKind::Gap),
            self.count(|kind| matches!(kind, CheckFindingKind::Cascade { .. })),
            self.count(|kind| *kind == CheckFindingKind::Global),
            self.definitions
        )
    }
}

/// Where a decoded definition came from, for report locations.
struct Origin {
    source: String,
    /// `plugins[i]` within `source`.
    local: String,
}

struct Checker {
    label: String,
    findings: Vec<CheckFinding>,
}

impl Checker {
    fn global(&mut self, error: &ConfigError) {
        let label = self.label.clone();
        self.global_in(&label, "$", error);
    }

    /// Records a GLOBAL whose path is rooted at `root` inside `source`.
    fn global_in(&mut self, source: &str, root: &str, error: &ConfigError) {
        let field = match error.path.strip_prefix(root) {
            Some(rest) => rest.strip_prefix('.').unwrap_or(rest).to_owned(),
            None => error.path.clone(),
        };
        self.findings.push(CheckFinding {
            kind: CheckFindingKind::Global,
            tag: None,
            plugin_type: None,
            source: source.to_owned(),
            field,
            message: error.reason.clone(),
        });
    }

    /// Runs `compile` on a top-level mapping, recording and dropping each
    /// unsupported field until it passes or fails for another reason.
    fn tolerate<T>(
        &mut self,
        mut value: RawValue,
        prefix: &str,
        compile: impl Fn(&RawValue) -> Result<T, ConfigError>,
    ) -> Option<T> {
        loop {
            let error = match compile(&value) {
                Ok(compiled) => return Some(compiled),
                Err(error) => error,
            };
            self.global(&error);
            let key = error
                .path
                .strip_prefix(prefix)
                .and_then(|rest| rest.strip_prefix('.'));
            match (&error.kind, key, &mut value) {
                (ConfigErrorKind::UnsupportedField, Some(key), RawValue::Map(map))
                    if map.get(key).is_some() =>
                {
                    map.entries.retain(|(entry, _)| entry != key);
                }
                _ => return None,
            }
        }
    }
}

/// Checks one configuration file without side effects. `Err` means the
/// configuration (or an include) cannot be read or is not valid YAML; every
/// rejection of parsed content, structural ones included, is a finding.
pub fn check_config(path: &Path) -> Result<CheckReport, ConfigError> {
    let yaml = load_yaml(path)?;
    let raw = parse_yaml(&yaml)?;
    // Same base as `start`: the parent, or `.` for a bare file name.
    let base_dir = path
        .parent()
        .filter(|base| !base.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut checker = Checker {
        label: path.display().to_string(),
        findings: Vec::new(),
    };
    let root_value = match expect_map(&raw, "$", "top level must be a mapping") {
        Ok(root) => RawValue::Map(root.clone()),
        Err(error) => {
            checker.global(&error);
            return Ok(report(checker, 0));
        }
    };
    let root = match checker.tolerate(root_value.clone(), "$", |value| {
        let root = expect_map(value, "$", "top level must be a mapping")?;
        root.reject_unknown(ROOT_FIELDS, "$")?;
        Ok(root.clone())
    }) {
        Some(root) => root,
        None => return Ok(report(checker, 0)),
    };

    let log = match root.required("log", "$") {
        Ok(value) => checker.tolerate(value.clone(), "$.log", compile_log),
        Err(error) => {
            checker.global(&error);
            None
        }
    }
    .unwrap_or(LogLevel::Error);
    let api = root
        .get("api")
        .and_then(|value| checker.tolerate(value.clone(), "$.api", compile_api));
    let managed = match managed_opt_in(&root_value) {
        Ok(true) => match crate::special_groups::ManagedProfile::load(base_dir) {
            Ok(profile) => Some(profile),
            Err(error) => {
                checker.global(&error);
                return Ok(report(checker, 0));
            }
        },
        Ok(false) => None,
        Err(error) => {
            checker.global(&error);
            None
        }
    };

    let mut decoded = Decoded::default();
    let mut include_paths: Vec<PathBuf> = Vec::new();
    if let Some(includes) = root.get("include") {
        let includes = expect_sequence(includes, "$.include").unwrap_or_else(|error| {
            checker.global(&error);
            &[]
        });
        for (index, include) in includes.iter().enumerate() {
            let expression = match expect_string(include, &format!("$.include[{index}]")) {
                Ok(expression) => expression,
                Err(error) => {
                    checker.global(&error);
                    continue;
                }
            };
            include_paths.push(resolve_relative(&expression, base_dir));
            if let Some(profile) = &managed {
                if let Err(error) = reject_generated_include(profile, &expression) {
                    checker.global(&error);
                    continue;
                }
            }
            let (path, raw) = read_included(&expression, base_dir, None)?;
            let included = match included_plugins(&path, &raw) {
                Ok(included) => included,
                Err(error) => {
                    checker.global_in(&expression, &path.display().to_string(), &error);
                    continue;
                }
            };
            decoded.decode(
                &mut checker,
                &included.plugins,
                &included.display,
                &expression,
                &included.base_dir,
            );
        }
    }
    let main_plugins = root
        .required("plugins", "$")
        .and_then(|plugins| expect_sequence(plugins, "$.plugins"))
        .unwrap_or_else(|error| {
            checker.global(&error);
            &[]
        });
    let label = checker.label.clone();
    decoded.decode(&mut checker, main_plugins, "$", &label, base_dir);
    let Decoded {
        mut plugins,
        mut origins,
        mut removed,
        undecoded,
    } = decoded;
    if let Some(profile) = &managed {
        let before = plugins.len();
        if let Err(error) = apply_managed_definitions(profile, &mut plugins) {
            checker.global(&error);
            return Ok(report(checker, plugins.len() + undecoded));
        }
        for plugin in &plugins[before..] {
            let (source, local) = plugin
                .source_path
                .split_once('.')
                .unwrap_or(("@managed-generated", &plugin.source_path));
            origins.push(Origin {
                source: source.to_owned(),
                local: local.to_owned(),
            });
        }
    }

    let total = plugins.len();
    let declared = total + undecoded;
    let sources = CompileSources {
        config_origin: None,
        include_paths: &include_paths,
    };
    let mut active = vec![true; total];
    let mut seen: BTreeSet<(usize, String)> = BTreeSet::new();
    // Original positions of the remaining items of each edited exec list.
    let mut exec_lists: BTreeMap<(usize, usize), Vec<usize>> = BTreeMap::new();
    // Every round removes a definition or neutralizes one sequence item.
    let budget = total + plugins.iter().map(item_budget).sum::<usize>() + 1;
    for _ in 0..budget {
        let current = plugins
            .iter()
            .zip(&active)
            .filter(|(_, active)| **active)
            .map(|(plugin, _)| plugin.clone())
            .collect();
        let error = match finish_compile(
            log,
            api.clone(),
            current,
            managed.clone(),
            None,
            None,
            &sources,
        ) {
            Ok(_) => return Ok(report(checker, declared)),
            Err(error) => error,
        };
        let Some(index) = locate(&plugins, &active, &error.path) else {
            checker.global(&error);
            return Ok(report(checker, declared));
        };
        let plugin = &plugins[index];
        let rest = error.path[plugin.source_path.len()..].to_owned();
        let reported = match exec_item(&rest) {
            Some((rule, item)) => match exec_lists.get(&(index, rule)) {
                Some(positions) => format!(".args[{rule}].exec[{}]", positions[item]),
                None => rest.clone(),
            },
            None => rest.clone(),
        };
        if seen.insert((index, reported.clone())) {
            checker.findings.push(CheckFinding {
                kind: match &error.kind {
                    ConfigErrorKind::UnknownReference { tag } if removed.contains(tag) => {
                        CheckFindingKind::Cascade {
                            removed: tag.clone(),
                        }
                    }
                    _ => CheckFindingKind::Gap,
                },
                tag: Some(plugin.tag.clone()),
                plugin_type: Some(plugin.kind.clone()),
                source: origins[index].source.clone(),
                field: format!("{}{reported}", origins[index].local),
                message: error.reason.clone(),
            });
            if neutralize(&mut plugins[index], index, &rest, &mut exec_lists) {
                continue;
            }
        }
        active[index] = false;
        removed.insert(plugins[index].tag.clone());
    }
    checker.global(&ConfigError::new("$", "check iteration budget exhausted"));
    Ok(report(checker, declared))
}

fn report(checker: Checker, definitions: usize) -> CheckReport {
    CheckReport {
        findings: checker.findings,
        definitions,
    }
}

/// Decoded definitions with their origins, plus what failed to decode.
#[derive(Default)]
struct Decoded {
    plugins: Vec<RawPlugin>,
    origins: Vec<Origin>,
    /// Tags left out of compilation, for cascade classification.
    removed: BTreeSet<String>,
    undecoded: usize,
}

impl Decoded {
    /// Decodes one plugin list; a definition that does not decode is a gap
    /// and is left out of compilation.
    fn decode(
        &mut self,
        checker: &mut Checker,
        values: &[RawValue],
        display: &str,
        source: &str,
        base_dir: &Path,
    ) {
        for (index, value) in values.iter().enumerate() {
            let source_path = format!("{display}.plugins[{index}]");
            let local = format!("plugins[{index}]");
            match decode_plugin(value, &source_path, base_dir) {
                Ok(plugin) => {
                    self.plugins.push(plugin);
                    self.origins.push(Origin {
                        source: source.to_owned(),
                        local,
                    });
                }
                Err(error) => {
                    let field = |key: &str| match value {
                        RawValue::Map(map) => match map.get(key) {
                            Some(RawValue::String(text)) => Some(text.clone()),
                            _ => None,
                        },
                        _ => None,
                    };
                    let rest = error.path.strip_prefix(&source_path).unwrap_or("");
                    // An undecodable definition still counts as removed so its
                    // referrers are cascades; an untagged one gets a unique key.
                    self.removed
                        .insert(field("tag").unwrap_or_else(|| format!("\0{source_path}")));
                    self.undecoded += 1;
                    checker.findings.push(CheckFinding {
                        kind: CheckFindingKind::Gap,
                        tag: field("tag"),
                        plugin_type: field("type"),
                        source: source.to_owned(),
                        field: format!("{local}{rest}"),
                        message: error.reason,
                    });
                }
            }
        }
    }
}

/// The active definition an error path belongs to.
fn locate(plugins: &[RawPlugin], active: &[bool], path: &str) -> Option<usize> {
    plugins.iter().zip(active).position(|(plugin, active)| {
        *active
            && path
                .strip_prefix(plugin.source_path.as_str())
                .is_some_and(|rest| {
                    rest.is_empty() || rest.starts_with('.') || rest.starts_with('[')
                })
    })
}

/// Upper bound of neutralization rounds one definition can take.
fn item_budget(plugin: &RawPlugin) -> usize {
    match (&plugin.kind[..], &plugin.args) {
        ("sequence", RawValue::Sequence(rules)) => rules
            .iter()
            .map(|rule| {
                let RawValue::Map(map) = rule else {
                    return 1;
                };
                let count = |key| match map.get(key) {
                    Some(RawValue::Sequence(items)) => items.len(),
                    _ => 1,
                };
                count("matches") + count("exec") + 1
            })
            .sum(),
        _ => 0,
    }
}

/// `.args[rule].exec[item]` exactly.
fn exec_item(rest: &str) -> Option<(usize, usize)> {
    let (rule, item) = rest.strip_prefix(".args[")?.split_once("].exec[")?;
    Some((rule.parse().ok()?, item.strip_suffix(']')?.parse().ok()?))
}

/// Neutralizes the rejected sequence item so the rest of the sequence keeps
/// being checked: a matcher becomes `_true`, an exec list item is dropped,
/// a single exec is dropped, anything else replaces the rule with
/// `matches: _true`. Returns `false` when the error is not inside one rule,
/// so the whole definition must be removed.
fn neutralize(
    plugin: &mut RawPlugin,
    plugin_index: usize,
    rest: &str,
    exec_lists: &mut BTreeMap<(usize, usize), Vec<usize>>,
) -> bool {
    if plugin.kind != "sequence" {
        return false;
    }
    let Some((rule_index, tail)) = rest
        .strip_prefix(".args[")
        .and_then(|rest| rest.split_once(']'))
    else {
        return false;
    };
    let (Ok(rule_index), RawValue::Sequence(rules)) =
        (rule_index.parse::<usize>(), &mut plugin.args)
    else {
        return false;
    };
    let Some(rule) = rules.get_mut(rule_index) else {
        return false;
    };
    let always = || RawValue::String("_true".into());
    if let RawValue::Map(map) = rule {
        let matcher = tail
            .strip_prefix(".matches[")
            .and_then(|tail| tail.strip_suffix(']'))
            .and_then(|index| index.parse::<usize>().ok());
        if let Some(matcher) = matcher {
            for (key, value) in &mut map.entries {
                if key != "matches" {
                    continue;
                }
                match value {
                    RawValue::String(_) if matcher == 0 => *value = always(),
                    RawValue::Sequence(items) if matcher < items.len() => {
                        items[matcher] = always();
                    }
                    _ => break,
                }
                return true;
            }
        }
        if let Some((rule, item)) = exec_item(rest) {
            for (key, value) in &mut map.entries {
                if let (true, RawValue::Sequence(items)) = (key == "exec", value) {
                    if item < items.len() {
                        let positions = exec_lists
                            .entry((plugin_index, rule))
                            .or_insert_with(|| (0..items.len()).collect());
                        items.remove(item);
                        positions.remove(item);
                    }
                }
            }
            if matches!(map.get("exec"), Some(RawValue::Sequence(items)) if !items.is_empty()) {
                return true;
            }
        }
        if tail == ".exec" || tail.starts_with(".exec[") {
            map.entries.retain(|(key, _)| key != "exec");
            if map.get("matches").is_none() {
                map.entries.push(("matches".into(), always()));
            }
            return true;
        }
    }
    *rule = RawValue::Map(RawMap {
        entries: vec![("matches".into(), always())],
    });
    true
}
