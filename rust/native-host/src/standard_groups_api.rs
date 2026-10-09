//! Standard-mode API; the older native_management handlers remain frozen.
use crate::config::{
    CompiledConfig,
    settings::UpstreamEntry,
    standard_groups::{self, Group, Source},
};
use crate::rule_sources::Reply;
use crate::runtime_snapshot::{ApplyFailure, RuntimeControl};
use serde::Deserialize;

fn json(status: u16, value: serde_json::Value) -> Reply {
    let mut body = serde_json::to_vec(&value).unwrap_or_default();
    body.push(b'\n');
    Reply {
        status,
        content_type: Some("application/json"),
        body,
        headers: Vec::new(),
    }
}
fn error(status: u16, message: impl ToString) -> Reply {
    json(status, serde_json::json!({"error":message.to_string()}))
}
fn failure(error_value: ApplyFailure) -> Reply {
    let status = match &error_value {
        ApplyFailure::Invalid(_) => 400,
        ApplyFailure::Conflict(_) | ApplyFailure::Busy(_) | ApplyFailure::ResourceConflict(_) => {
            409
        }
        ApplyFailure::Shutdown | ApplyFailure::RecoveryRequired(_) => 503,
        ApplyFailure::Failed(_) | ApplyFailure::CommittedCleanup { .. } => 500,
    };
    error(status, error_value)
}

pub(crate) async fn handle(
    control: Option<&RuntimeControl>,
    config: &CompiledConfig,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Reply> {
    let profile = config.standard_groups.as_ref()?;
    let path = path.trim_end_matches('/');
    if path == "/api/v1/upstream/sources" {
        return Some(if method != "GET" {
            error(405, "method not allowed")
        } else {
            json(200, serde_json::Value::Array(config.effective_upstreams.iter().filter(|(tag, _)| !standard_groups::reserved(tag)).map(|(tag, entries)| {
                serde_json::json!({"plugin_tag":tag,"upstreams": entries.iter().map(|entry| serde_json::json!({"tag":entry.tag,"protocol":entry.protocol_name(),"address":entry.addr,"enabled":entry.enabled})).collect::<Vec<_>>()})
            }).collect()))
        });
    }
    let slot_text = if path == "/api/v1/special-groups" {
        None
    } else if let Some(slot) = path.strip_prefix("/api/v1/special-groups/") {
        Some(slot)
    } else {
        return None;
    };
    if method == "GET" && slot_text.is_none() {
        return Some(json(
            200,
            serde_json::Value::Array(profile.groups.iter().map(|g| profile.view(g)).collect()),
        ));
    }
    let Some(control) = control else {
        return Some(error(503, "runtime control is unavailable"));
    };
    let reply = match (method, slot_text) {
        ("POST", None) => post(control, body).await,
        ("DELETE", Some(text)) => {
            let Ok(slot) = text.parse::<u32>() else {
                return Some(error(400, "invalid slot"));
            };
            if slot < 50 {
                return Some(error(400, "slot must be >= 50"));
            }
            let current = control.view();
            let mut groups = current.config.standard_groups.as_ref()?.groups.clone();
            let before = groups.len();
            groups.retain(|g| g.slot != slot);
            if groups.len() == before {
                return Some(error(404, "special group not found"));
            }
            drop(current);
            match control.apply_groups(groups).await {
                Ok(_) => Reply {
                    status: 204,
                    content_type: None,
                    body: Vec::new(),
                    headers: Vec::new(),
                },
                Err(e) => failure(e),
            }
        }
        _ => error(405, "method not allowed"),
    };
    Some(reply)
}

async fn post(control: &RuntimeControl, body: &[u8]) -> Reply {
    #[derive(Deserialize)]
    struct Payload {
        #[serde(default)]
        slot: u32,
        name: String,
        #[serde(default)]
        listen_port: u16,
        #[serde(default)]
        custom_port_only: bool,
        upstream_sources: Option<Vec<Source>>,
        upstreams: Option<Vec<UpstreamEntry>>,
    }
    let payload: Payload = match serde_json::from_slice(body) {
        Ok(value) => value,
        Err(e) => return error(400, format!("invalid request body: {e}")),
    };
    let current = control.view();
    let mut groups = current
        .config
        .standard_groups
        .as_ref()
        .expect("standard groups")
        .groups
        .clone();
    let mut slot = payload.slot;
    if slot == 0 {
        slot = 50;
        while groups.iter().any(|g| g.slot == slot) {
            let Some(next) = slot.checked_add(1) else {
                return error(400, "slot space exhausted");
            };
            slot = next;
        }
    }
    let mut group = groups
        .iter()
        .find(|g| g.slot == slot)
        .cloned()
        .unwrap_or_else(|| Group {
            slot,
            ..Group::default()
        });
    group.name = payload.name;
    group.listen_port = payload.listen_port;
    group.custom_port_only = payload.custom_port_only;
    if let Some(sources) = payload.upstream_sources {
        group.upstream_sources = sources;
    }
    if let Some(entries) = payload.upstreams {
        group.owned_upstreams = entries;
    }
    groups.retain(|g| g.slot != slot);
    groups.push(group);
    if let Err(e) = standard_groups::normalize(&mut groups) {
        return error(
            if e.to_string().contains("already") || e.to_string().contains("其他") {
                409
            } else {
                400
            },
            e,
        );
    }
    drop(current);
    match control.apply_groups(groups).await {
        Err(e) => failure(e),
        Ok(_) => {
            let view = control.view();
            let profile = view
                .config
                .standard_groups
                .as_ref()
                .expect("committed groups");
            json(
                200,
                profile.view(
                    profile
                        .groups
                        .iter()
                        .find(|g| g.slot == slot)
                        .expect("committed slot"),
                ),
            )
        }
    }
}
