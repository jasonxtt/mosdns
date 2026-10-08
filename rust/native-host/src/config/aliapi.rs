//! The `aliapi` plugin: a forward-style upstream group whose entries are
//! standard DNS upstreams (UDP, TCP, DoT, DoH) with an optional per-upstream
//! or plugin-wide SOCKS5 proxy. The signed AliDNS API is not supported and is
//! rejected explicitly.
//!
//! [`AliApiArgs`] is the typed entry point: whatever supplies the values (the
//! YAML decoder here, or a settings layer later) builds an `AliApiArgs` and
//! calls [`AliApiArgs::build`], which merges the plugin-wide defaults into each
//! upstream and produces the same definition the `forward` plugin compiles to,
//! SOCKS5 routes included.

use std::net::SocketAddr;
use std::time::Duration;

use super::{
    ConfigError, ForwardDefinitionConfig, ForwardEntryConfig, ForwardTargetOptions, RawMap,
    RawNumber, RawPlugin, RawValue, build_forward_target, expect_bool, expect_map, expect_sequence,
    expect_string, hex_identity, parse_numeric_text,
};
use mosdns_upstream_core::Endpoint;

/// The most upstreams one `aliapi` query may race.
pub(crate) const MAX_CONCURRENT: usize = 20;

const SIGNED_API: &str = "the signed AliDNS API is not supported; use a standard DNS upstream";

/// Plugin-wide values an upstream inherits when it leaves its own empty.
#[derive(Clone, Debug, Default)]
pub(crate) struct AliApiDefaults {
    pub socks5: String,
    pub so_mark: i64,
    pub bind_to_device: String,
    pub bootstrap: String,
    pub bootstrap_version: i64,
}

/// One upstream as the reference host declares it; empty strings and zero
/// numbers mean "not set".
#[derive(Clone, Debug, Default)]
pub(crate) struct AliApiUpstream {
    pub tag: String,
    pub addr: String,
    pub dial_addr: String,
    pub idle_timeout: i64,
    /// Milliseconds; zero selects the five second default.
    pub upstream_query_timeout: i64,
    /// `dns` (default) or `aliapi` (signed API, rejected).
    pub kind: String,
    pub enable_pipeline: bool,
    pub enable_http3: bool,
    pub insecure_skip_verify: bool,
    pub socks5: String,
    pub so_mark: i64,
    pub bind_to_device: String,
    pub bootstrap: String,
    pub bootstrap_version: i64,
}

/// The parsed arguments of one `aliapi` plugin.
#[derive(Clone, Debug, Default)]
pub(crate) struct AliApiArgs {
    pub upstreams: Vec<AliApiUpstream>,
    /// Zero or negative selects one; above [`MAX_CONCURRENT`] is clamped.
    pub concurrent: i64,
    pub defaults: AliApiDefaults,
    /// The first signed-API field present, if any.
    pub signed_field: Option<&'static str>,
}

pub(super) fn compile(plugin: &RawPlugin) -> Result<ForwardDefinitionConfig, ConfigError> {
    let path = format!("{}.args", plugin.source_path);
    let args = expect_map(&plugin.args, &path, "aliapi args must be a mapping")?;
    AliApiArgs::from_yaml(args, &path)?.build(&plugin.tag, &path)
}

impl AliApiArgs {
    const PLUGIN_KEYS: &'static [&'static str] = &[
        "upstreams",
        "concurrent",
        "account_id",
        "access_key_id",
        "access_key_secret",
        "server_addr",
        "ecs_client_ip",
        "ecs_client_mask",
        "socks5",
        "so_mark",
        "bind_to_device",
        "bootstrap",
        "bootstrap_version",
    ];
    const UPSTREAM_KEYS: &'static [&'static str] = &[
        "tag",
        "addr",
        "dial_addr",
        "idle_timeout",
        "upstream_query_timeout",
        "type",
        "max_conns",
        "enable_pipeline",
        "enable_http3",
        "insecure_skip_verify",
        "socks5",
        "so_mark",
        "bind_to_device",
        "bootstrap",
        "bootstrap_version",
    ];

    /// Decodes the YAML `args` mapping into typed values.
    pub(super) fn from_yaml(args: &RawMap, path: &str) -> Result<Self, ConfigError> {
        args.reject_unknown(Self::PLUGIN_KEYS, path)?;
        // Any appearance of a signed-API field is a request for that API,
        // whatever its value.
        let signed_field = [
            "account_id",
            "access_key_id",
            "access_key_secret",
            "server_addr",
            "ecs_client_ip",
            "ecs_client_mask",
        ]
        .into_iter()
        .find(|key| args.get(key).is_some());
        let defaults = AliApiDefaults {
            socks5: string_field(args, "socks5", path)?,
            so_mark: int_field(args, "so_mark", path)?,
            bind_to_device: string_field(args, "bind_to_device", path)?,
            bootstrap: string_field(args, "bootstrap", path)?,
            bootstrap_version: int_field(args, "bootstrap_version", path)?,
        };
        let upstreams_path = format!("{path}.upstreams");
        let mut upstreams = Vec::new();
        if let Some(value) = args.get("upstreams") {
            for (index, item) in expect_sequence(value, &upstreams_path)?.iter().enumerate() {
                let item_path = format!("{upstreams_path}[{index}]");
                let item = expect_map(item, &item_path, "upstream must be a mapping")?;
                item.reject_unknown(Self::UPSTREAM_KEYS, &item_path)?;
                upstreams.push(AliApiUpstream {
                    tag: string_field(item, "tag", &item_path)?,
                    addr: string_field(item, "addr", &item_path)?,
                    dial_addr: string_field(item, "dial_addr", &item_path)?,
                    idle_timeout: int_field(item, "idle_timeout", &item_path)?,
                    upstream_query_timeout: int_field(item, "upstream_query_timeout", &item_path)?,
                    kind: string_field(item, "type", &item_path)?,
                    enable_pipeline: bool_field(item, "enable_pipeline", &item_path)?,
                    enable_http3: bool_field(item, "enable_http3", &item_path)?,
                    insecure_skip_verify: bool_field(item, "insecure_skip_verify", &item_path)?,
                    socks5: string_field(item, "socks5", &item_path)?,
                    so_mark: int_field(item, "so_mark", &item_path)?,
                    bind_to_device: string_field(item, "bind_to_device", &item_path)?,
                    bootstrap: string_field(item, "bootstrap", &item_path)?,
                    bootstrap_version: int_field(item, "bootstrap_version", &item_path)?,
                });
            }
        }
        Ok(Self {
            upstreams,
            concurrent: int_field(args, "concurrent", path)?,
            defaults,
            signed_field,
        })
    }

    /// Builds the upstream group: every upstream inherits the plugin-wide
    /// defaults it leaves empty, the SOCKS5 proxy applies to TCP-based
    /// schemes only, and options the native host cannot honor are rejected
    /// instead of ignored. `plugin_tag` names a single-upstream entry.
    pub(crate) fn build(
        &self,
        plugin_tag: &str,
        path: &str,
    ) -> Result<ForwardDefinitionConfig, ConfigError> {
        if let Some(field) = self.signed_field {
            return Err(ConfigError::new(format!("{path}.{field}"), SIGNED_API));
        }
        let upstreams_path = format!("{path}.upstreams");
        if self.upstreams.is_empty() {
            return Err(ConfigError::new(
                upstreams_path,
                "upstreams must not be empty",
            ));
        }
        let concurrent =
            usize::try_from(self.concurrent.clamp(1, MAX_CONCURRENT as i64)).unwrap_or(1);
        let mut entries: Vec<ForwardEntryConfig> = Vec::with_capacity(self.upstreams.len());
        for (index, upstream) in self.upstreams.iter().enumerate() {
            let item_path = format!("{upstreams_path}[{index}]");
            let entry = self.entry(upstream, index, plugin_tag, &item_path)?;
            if let Some(tag) = &entry.tag {
                if entries.iter().any(|other| other.tag.as_ref() == Some(tag)) {
                    return Err(ConfigError::new(
                        format!("{item_path}.tag"),
                        format!("duplicated upstream tag `{tag}`"),
                    ));
                }
            }
            entries.push(entry);
        }
        Ok(ForwardDefinitionConfig {
            tag: plugin_tag.to_owned(),
            entries,
            concurrent,
        })
    }

    fn entry(
        &self,
        upstream: &AliApiUpstream,
        index: usize,
        plugin_tag: &str,
        path: &str,
    ) -> Result<ForwardEntryConfig, ConfigError> {
        if upstream.kind == "aliapi" {
            return Err(ConfigError::new(format!("{path}.type"), SIGNED_API));
        }
        let defaults = &self.defaults;
        let socks5 = pick(&upstream.socks5, &defaults.socks5);
        let so_mark = if upstream.so_mark == 0 {
            defaults.so_mark
        } else {
            upstream.so_mark
        };
        let bind_to_device = pick(&upstream.bind_to_device, &defaults.bind_to_device);
        let bootstrap = pick(&upstream.bootstrap, &defaults.bootstrap);
        let bootstrap_version = if upstream.bootstrap_version == 0 {
            defaults.bootstrap_version
        } else {
            upstream.bootstrap_version
        };
        if upstream.addr.is_empty() {
            return Err(ConfigError::new(format!("{path}.addr"), "addr is required"));
        }
        for (key, set) in [
            ("idle_timeout", upstream.idle_timeout > 0),
            ("enable_pipeline", upstream.enable_pipeline),
            ("enable_http3", upstream.enable_http3),
            ("so_mark", so_mark != 0),
            ("bind_to_device", !bind_to_device.is_empty()),
        ] {
            if set {
                return Err(ConfigError::new(
                    format!("{path}.{key}"),
                    "option is unsupported",
                ));
            }
        }
        let query_timeout = match upstream.upstream_query_timeout {
            0 => Duration::from_secs(5),
            millis if millis > 0 => Duration::from_millis(millis.unsigned_abs()),
            _ => {
                return Err(ConfigError::new(
                    format!("{path}.upstream_query_timeout"),
                    "timeout must not be negative",
                ));
            }
        };
        let bootstrap = if bootstrap.is_empty() {
            None
        } else {
            Some(parse_numeric_text(
                bootstrap,
                53,
                &format!("{path}.bootstrap"),
            )?)
        };
        let bootstrap_version = match bootstrap_version {
            0 => None,
            4 | 6 => Some(u8::try_from(bootstrap_version).unwrap_or_default()),
            other => {
                return Err(ConfigError::new(
                    format!("{path}.bootstrap_version"),
                    format!("unsupported bootstrap_version {other}; expected 0, 4, or 6"),
                ));
            }
        };
        let target = build_forward_target(
            &upstream.addr,
            ForwardTargetOptions {
                dial_addr: (!upstream.dial_addr.is_empty()).then(|| upstream.dial_addr.clone()),
                bootstrap,
                bootstrap_version,
                query_timeout,
                insecure_skip_verify: upstream.insecure_skip_verify,
                socks5: (!socks5.is_empty()).then(|| socks5.to_owned()),
            },
            path,
        )?;
        let endpoint: Option<Endpoint> = target.dial_addr.map(|dial: SocketAddr| {
            Endpoint::new(dial, target.scheme.endpoint_transport()).expect("validated dial")
        });
        let tag = (!upstream.tag.is_empty()).then(|| upstream.tag.clone());
        let identity = tag.clone().unwrap_or_else(|| {
            if self.upstreams.len() == 1 {
                plugin_tag.to_owned()
            } else {
                format!("@native-forward:{}:{index}", hex_identity(plugin_tag))
            }
        });
        Ok(ForwardEntryConfig {
            tag,
            response_identity: std::sync::Arc::from(identity.as_str()),
            identity,
            endpoint,
            target,
        })
    }
}

fn pick<'a>(own: &'a str, inherited: &'a str) -> &'a str {
    if own.is_empty() { inherited } else { own }
}

/// A string field; absent and `null` read as empty.
fn string_field(map: &RawMap, key: &str, path: &str) -> Result<String, ConfigError> {
    match map.get(key) {
        None | Some(RawValue::Null) => Ok(String::new()),
        Some(value) => expect_string(value, &format!("{path}.{key}")),
    }
}

fn bool_field(map: &RawMap, key: &str, path: &str) -> Result<bool, ConfigError> {
    match map.get(key) {
        None | Some(RawValue::Null) => Ok(false),
        Some(value) => expect_bool(value, &format!("{path}.{key}")),
    }
}

/// An integer field; absent and `null` read as zero.
fn int_field(map: &RawMap, key: &str, path: &str) -> Result<i64, ConfigError> {
    match map.get(key) {
        None | Some(RawValue::Null) => Ok(0),
        Some(RawValue::Number(RawNumber::Signed(value))) => Ok(*value),
        Some(RawValue::Number(RawNumber::Unsigned(value))) => i64::try_from(*value)
            .map_err(|_| ConfigError::new(format!("{path}.{key}"), "integer is out of range")),
        Some(_) => Err(ConfigError::new(
            format!("{path}.{key}"),
            "expected an integer",
        )),
    }
}
