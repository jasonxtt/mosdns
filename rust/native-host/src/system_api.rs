//! `GET/POST /api/v1/system/webui-port` and `POST /api/v1/system/restart`.
//!
//! The port is stored in `webinfo/webui_port_settings.json` and replaces the
//! `api.http` port when the host starts, so a change takes effect on the next
//! start (the reference behaves the same; the WebUI follows it with a restart).
//! The restart is an in-place one, see `restart.rs`.

use std::net::SocketAddr;
use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::config::CompiledConfig;
use crate::rule_sources::{Reply, write_atomically};
use crate::runtime_snapshot::RuntimeControl;

const PORT_FILE: &str = "webinfo/webui_port_settings.json";
const DEFAULT_DELAY_MS: i64 = 300;
/// A restart is not scheduled further ahead than this (the reference has no bound).
const MAX_DELAY_MS: i64 = 10 * 60 * 1000;

fn json(status: u16, value: &impl Serialize) -> Reply {
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

fn not_allowed() -> Reply {
    Reply {
        status: 405,
        content_type: Some("text/plain; charset=utf-8"),
        body: b"method not allowed\n".to_vec(),
        headers: Vec::new(),
    }
}

/// The port saved in `webinfo/webui_port_settings.json`: `Ok(None)` when there is no
/// file or its port is 0 (no override), `Err` when the file cannot be used.
pub(crate) fn saved_port(root: &Path) -> Result<Option<u16>, String> {
    let bytes = match std::fs::read(root.join(PORT_FILE)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    #[derive(serde::Deserialize)]
    struct Saved {
        #[serde(default)]
        port: Option<i64>,
    }
    let saved: Saved = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    match saved.port.unwrap_or(0) {
        0 => Ok(None),
        port @ 1..=65535 => Ok(Some(port as u16)),
        _ => Err("端口必须在 1-65535 之间".to_owned()),
    }
}

/// The management address with the saved port applied (startup priority: the file wins over YAML).
pub(crate) fn with_saved_port(root: &Path, configured: SocketAddr) -> SocketAddr {
    match saved_port(root) {
        Ok(Some(port)) => SocketAddr::new(configured.ip(), port),
        Ok(None) => configured,
        Err(reason) => {
            crate::diag::warn_line!("failed to load webui port settings: {reason}");
            configured
        }
    }
}

fn save_port(root: &Path, port: u16) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(&serde_json::json!({ "port": port }))
        .map_err(std::io::Error::other)?;
    write_atomically(&root.join(PORT_FILE), &bytes)
}

/// The numeric `port` of a request body as the reference's decoder reads it: absent or
/// `null` is 0, anything that is not an integer is an error.
fn requested_port(body: &[u8]) -> Result<i64, String> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|error| error.to_string())?;
    let object = value
        .as_object()
        .ok_or("json: cannot unmarshal into a request object")?;
    match object.get("port") {
        None | Some(serde_json::Value::Null) => Ok(0),
        Some(number) => number
            .as_i64()
            .ok_or_else(|| "json: cannot unmarshal the port into an integer".to_owned()),
    }
}

/// The management address a running host listens on.
fn active_address(control: Option<&RuntimeControl>, config: &CompiledConfig) -> Option<SocketAddr> {
    control
        .and_then(RuntimeControl::api_listen)
        .or_else(|| config.api.as_ref().map(|api| api.http))
}

fn get_port(root: &Path, active: SocketAddr) -> Reply {
    let configured = match saved_port(root) {
        Ok(saved) => saved.unwrap_or(active.port()),
        Err(reason) => return error(500, format!("读取 WebUI 端口设置失败: {reason}")),
    };
    json(
        200,
        &serde_json::json!({
            "port": configured,
            "active_port": active.port(),
            "active_addr": active.to_string(),
            "pending_restart": configured != active.port(),
        }),
    )
}

fn set_port(root: &Path, active: SocketAddr, body: &[u8]) -> Reply {
    let requested = match requested_port(body) {
        Ok(port) => port,
        Err(reason) => return error(400, format!("无效请求: {reason}")),
    };
    let Ok(port) = u16::try_from(requested)
        .map_err(|_| ())
        .and_then(|port| if port == 0 { Err(()) } else { Ok(port) })
    else {
        return error(400, "端口必须在 1-65535 之间");
    };
    let target = SocketAddr::new(active.ip(), port);
    if target != active && std::net::TcpListener::bind(target).is_err() {
        return error(409, format!("端口 {port} 已被占用"));
    }
    if let Err(reason) = save_port(root, port) {
        return error(500, format!("保存 WebUI 端口设置失败: {reason}"));
    }
    json(
        200,
        &serde_json::json!({
            "port": port,
            "active_port": active.port(),
            "active_addr": active.to_string(),
            "target_addr": target.to_string(),
            "pending_restart": port != active.port(),
            "message": "WebUI 端口已保存，重启后生效",
        }),
    )
}

/// Schedules the in-place restart; the answer is sent before it happens.
fn restart(control: Option<&RuntimeControl>, body: &[u8]) -> Reply {
    // A body that does not decode is ignored, as in the reference.
    let requested = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("delay_ms").and_then(serde_json::Value::as_i64))
        .unwrap_or(0);
    let delay = if requested <= 0 {
        DEFAULT_DELAY_MS
    } else {
        requested.min(MAX_DELAY_MS)
    };
    let Some(control) = control else {
        return error(
            501,
            "self-restart is unavailable: the host is not supervised",
        );
    };
    let Some(target) = crate::restart::target() else {
        return error(
            501,
            "self-restart is unavailable: the executable path was not captured at start",
        );
    };
    if let Err(reason) = target.preflight() {
        return error(500, reason);
    }
    control.schedule_restart(Duration::from_millis(delay as u64));
    json(
        200,
        &serde_json::json!({ "status": "scheduled", "delay_ms": delay }),
    )
}

/// The system routes, or `None` when `path` is not one of them.
pub(crate) fn handle(
    control: Option<&RuntimeControl>,
    config: &CompiledConfig,
    root: Option<&Path>,
    method: &str,
    path: &str,
    body: &[u8],
) -> Option<Reply> {
    Some(match (path, method) {
        ("/api/v1/system/webui-port", "GET" | "POST") => {
            let Some(active) = active_address(control, config) else {
                return Some(error(503, "当前未启用 WebUI/API 服务"));
            };
            let Some(root) = root else {
                return Some(error(503, "此主机没有状态目录，无法保存 WebUI 端口"));
            };
            if method == "GET" {
                get_port(root, active)
            } else {
                set_port(root, active, body)
            }
        }
        ("/api/v1/system/restart", "POST") => restart(control, body),
        ("/api/v1/system/webui-port" | "/api/v1/system/restart", _) => not_allowed(),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("system-api-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(path.join("webinfo")).unwrap();
        path
    }

    fn value(reply: &Reply) -> serde_json::Value {
        serde_json::from_slice(&reply.body).unwrap()
    }

    #[test]
    fn a_saved_port_overrides_the_configured_one_only_when_valid() {
        let root = temp("override");
        let configured: SocketAddr = "[::]:9099".parse().unwrap();
        assert_eq!(with_saved_port(&root, configured), configured, "no file");
        // Exactly what the reference host writes.
        std::fs::write(root.join(PORT_FILE), "{\n  \"port\": 80\n}").unwrap();
        assert_eq!(
            with_saved_port(&root, configured),
            "[::]:80".parse().unwrap()
        );
        assert_eq!(saved_port(&root), Ok(Some(80)));
        for (text, expected) in [
            ("{\"port\":0}", Ok(None)),
            ("{}", Ok(None)),
            ("{\"port\":70000}", Err(())),
            ("{\"port\":-1}", Err(())),
            ("nope", Err(())),
        ] {
            std::fs::write(root.join(PORT_FILE), text).unwrap();
            assert_eq!(saved_port(&root).map_err(|_| ()), expected, "{text}");
            assert_eq!(with_saved_port(&root, configured), configured, "{text}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn get_reports_the_active_and_the_saved_port() {
        let root = temp("get");
        let active: SocketAddr = "0.0.0.0:9099".parse().unwrap();
        let reply = get_port(&root, active);
        assert_eq!(
            value(&reply),
            serde_json::json!({"port": 9099, "active_port": 9099, "active_addr": "0.0.0.0:9099", "pending_restart": false})
        );
        std::fs::write(root.join(PORT_FILE), "{\"port\": 8080}").unwrap();
        assert_eq!(value(&get_port(&root, active))["pending_restart"], true);
        // Keys come out sorted, like the reference's `map[string]any`.
        assert_eq!(
            String::from_utf8(get_port(&root, active).body).unwrap(),
            "{\"active_addr\":\"0.0.0.0:9099\",\"active_port\":9099,\"pending_restart\":true,\"port\":8080}\n"
        );
        std::fs::write(root.join(PORT_FILE), "broken").unwrap();
        assert_eq!(get_port(&root, active).status, 500);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn post_validates_checks_availability_and_saves_without_switching() {
        let root = temp("post");
        let blocker = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let taken = blocker.local_addr().unwrap().port();
        let free = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let active: SocketAddr = "127.0.0.1:9099".parse().unwrap();
        for (body, status) in [
            ("not json", 400),
            ("[]", 400),
            ("{\"port\": \"80\"}", 400),
            ("{\"port\": 80.5}", 400),
            ("{}", 400),
            ("{\"port\": 0}", 400),
            ("{\"port\": 65536}", 400),
            ("{\"port\": -3}", 400),
        ] {
            assert_eq!(
                set_port(&root, active, body.as_bytes()).status,
                status,
                "{body}"
            );
        }
        assert!(
            !root.join(PORT_FILE).exists(),
            "rejected requests save nothing"
        );
        let reply = set_port(&root, active, format!("{{\"port\": {taken}}}").as_bytes());
        assert_eq!(reply.status, 409);
        assert_eq!(value(&reply)["error"], format!("端口 {taken} 已被占用"));
        assert!(!root.join(PORT_FILE).exists());
        let reply = set_port(&root, active, format!("{{\"port\": {free}}}").as_bytes());
        assert_eq!(reply.status, 200);
        assert_eq!(
            value(&reply),
            serde_json::json!({"port": free, "active_port": 9099, "active_addr": "127.0.0.1:9099", "target_addr": format!("127.0.0.1:{free}"), "pending_restart": true, "message": "WebUI 端口已保存，重启后生效"})
        );
        assert_eq!(
            std::fs::read_to_string(root.join(PORT_FILE)).unwrap(),
            format!("{{\n  \"port\": {free}\n}}")
        );
        // Re-saving the active port needs no free socket (it is the one in use) and is not pending.
        let reply = set_port(&root, active, b"{\"port\": 9099}");
        assert_eq!(
            (reply.status, value(&reply)["pending_restart"].clone()),
            (200, serde_json::json!(false))
        );
        drop(blocker);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn the_restart_request_is_refused_where_it_cannot_be_carried_out() {
        // No supervisor (control) at all: 501 and nothing scheduled.
        let reply = restart(None, br#"{"delay_ms": 10}"#);
        assert_eq!(reply.status, 501);
        assert!(
            value(&reply)["error"]
                .as_str()
                .unwrap()
                .contains("unavailable")
        );
    }
}
