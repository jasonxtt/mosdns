//! Immutable embedded UI. Static bodies share binary-owned bytes and never
//! allocate a bundle copy in the request owner.
use mosdns_upstream_core::TransportCancellation;
use std::io;
use std::sync::Arc;
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::sync::Semaphore;

struct EmbeddedAsset {
    path: &'static str,
    bytes: &'static [u8],
    etag: &'static str,
}
include!(concat!(env!("OUT_DIR"), "/embedded_ui.rs"));

pub(crate) fn decoded_path(target: &str) -> Option<String> {
    let raw = target.split('?').next()?;
    if !raw.starts_with('/') {
        return None;
    }
    let mut result = String::new();
    for (index, component) in raw.split('/').enumerate() {
        if index > 0 {
            result.push('/');
        }
        let mut bytes = Vec::new();
        let mut input = component.bytes();
        while let Some(byte) = input.next() {
            let byte = if byte == b'%' {
                let high = char::from(input.next()?).to_digit(16)?;
                let low = char::from(input.next()?).to_digit(16)?;
                u8::try_from(high * 16 + low).ok()?
            } else {
                byte
            };
            if byte == 0 || byte == b'/' || byte == b'\\' || byte.is_ascii_control() {
                return None;
            }
            bytes.push(byte);
        }
        let part = std::str::from_utf8(&bytes).ok()?;
        if part == "." || part == ".." {
            return None;
        }
        result.push_str(part);
    }
    Some(result)
}

pub(crate) fn mime(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

/// Preserve only a syntactically valid query whose decoded bytes contain no
/// control characters. Redirect headers never reflect unsafe input.
pub(crate) fn redirect_target(path: &str, target: &str) -> Option<String> {
    let Some((_, query)) = target.split_once('?') else {
        return Some(path.to_owned());
    };
    let mut bytes = Vec::new();
    let mut input = query.bytes();
    while let Some(byte) = input.next() {
        let byte = if byte == b'%' {
            let high = char::from(input.next()?).to_digit(16)?;
            let low = char::from(input.next()?).to_digit(16)?;
            u8::try_from(high * 16 + low).ok()?
        } else {
            byte
        };
        if byte.is_ascii_control() {
            return None;
        }
        bytes.push(byte);
    }
    std::str::from_utf8(&bytes).ok()?;
    Some(format!("{path}?{query}"))
}

fn matches_etag(header: Option<&str>, etag: &str) -> bool {
    header.is_some_and(|value| {
        value.split(',').any(|item| {
            let item = item.trim();
            item == "*" || item.strip_prefix("W/").unwrap_or(item) == etag
        })
    })
}

/// Returns false only when the target belongs to the ordinary API dispatcher.
pub(crate) async fn serve_embedded<W: AsyncWrite + Unpin>(
    writer: &mut W,
    target: &str,
    method: &str,
    conditional: Option<&str>,
    slots: Arc<Semaphore>,
    shutdown: &TransportCancellation,
) -> io::Result<bool> {
    let raw = target.split('?').next().unwrap_or(target);
    let decoded = decoded_path(target);
    let route = decoded.as_deref().unwrap_or(raw);
    if !(matches!(route, "/" | "/log" | "/log/")
        || route == "/assets"
        || route.starts_with("/assets/"))
    {
        return Ok(false);
    }
    let asset = decoded
        .as_ref()
        .and_then(|path| {
            EMBEDDED
                .binary_search_by_key(&path.as_str(), |asset| asset.path)
                .ok()
        })
        .map(|index| &EMBEDDED[index]);
    let representation = matches!(method, "GET" | "HEAD") && asset.is_some();
    let permit = if representation {
        slots.try_acquire_owned().ok()
    } else {
        None
    };
    let response = async {
        let (status, reason, body, extra) = if !matches!(method, "GET" | "HEAD") {
            (
                405,
                "Method Not Allowed",
                &b""[..],
                "Allow: GET, HEAD\r\n".to_owned(),
            )
        } else if route == "/log/" && redirect_target("/log", target).is_none() {
            (404, "Not Found", &b""[..], String::new())
        } else if route == "/log/" {
            (
                301,
                "Moved Permanently",
                &b""[..],
                format!(
                    "Location: {}\r\n",
                    redirect_target("/log", target).unwrap_or_default()
                ),
            )
        } else if representation && permit.is_none() {
            (503, "Service Unavailable", &b""[..], String::new())
        } else if let Some(asset) = asset {
            let status = if matches_etag(conditional, asset.etag) {
                304
            } else {
                200
            };
            let kind = if matches!(asset.path, "/" | "/log") {
                "text/html; charset=utf-8"
            } else {
                mime(asset.path)
            };
            (
                status,
                if status == 304 { "Not Modified" } else { "OK" },
                asset.bytes,
                format!(
                    "Content-Type: {kind}\r\nCache-Control: no-cache\r\nETag: {}\r\nX-Content-Type-Options: nosniff\r\n",
                    asset.etag
                ),
            )
        } else {
            (404, "Not Found", &b""[..], String::new())
        };
        let length = body.len();
        writer.write_all(format!("HTTP/1.1 {status} {reason}\r\n{extra}Content-Length: {length}\r\nConnection: close\r\n\r\n").as_bytes()).await?;
        if method != "HEAD" && status != 304 {
            for chunk in body.chunks(64 * 1024) {
                writer.write_all(chunk).await?;
            }
        }
        writer.flush().await
    };
    tokio::select! {
        biased;
        () = shutdown.cancelled() => {},
        result = tokio::time::timeout(std::time::Duration::from_secs(10), response) => {
            result.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "static response deadline"))??;
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncReadExt;

    #[test]
    fn slow_body_slots_are_bounded_and_cancelled_work_releases_them() {
        crate::HostRuntime::new().unwrap().block_on(async {
            let slots = Arc::new(Semaphore::new(4));
            let shutdown = TransportCancellation::new();
            let mut tasks = Vec::new();
            let mut readers = Vec::new();
            for _ in 0..4 {
                let (mut writer, reader) = tokio::io::duplex(32);
                readers.push(reader);
                let slots = slots.clone();
                let shutdown = shutdown.clone();
                tasks.push(tokio::task::spawn_local(async move {
                    serve_embedded(
                        &mut writer,
                        "/assets/vue-log/app.js",
                        "GET",
                        None,
                        slots,
                        &shutdown,
                    )
                    .await
                }));
            }
            tokio::task::yield_now().await;
            assert_eq!(slots.available_permits(), 0);
            let (mut writer, mut reader) = tokio::io::duplex(1024);
            assert!(
                serve_embedded(&mut writer, "/", "GET", None, slots.clone(), &shutdown)
                    .await
                    .unwrap()
            );
            drop(writer);
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).await.unwrap();
            assert!(bytes.starts_with(b"HTTP/1.1 503"));
            for (method, target, status) in [
                ("POST", "/", "405"),
                ("GET", "/log/?x=1", "301"),
                ("GET", "/assets/missing", "404"),
            ] {
                let (mut writer, mut reader) = tokio::io::duplex(1024);
                assert!(
                    serve_embedded(&mut writer, target, method, None, slots.clone(), &shutdown)
                        .await
                        .unwrap()
                );
                drop(writer);
                let mut bytes = Vec::new();
                reader.read_to_end(&mut bytes).await.unwrap();
                assert!(
                    String::from_utf8(bytes)
                        .unwrap()
                        .starts_with(&format!("HTTP/1.1 {status}")),
                    "{method} {target}"
                );
                assert_eq!(slots.available_permits(), 0);
            }
            shutdown.cancel();
            for task in tasks {
                assert!(task.await.unwrap().unwrap());
            }
            assert_eq!(slots.available_permits(), 4);
            drop(readers);
        });
    }

    #[test]
    fn decoding_never_allows_component_separators_or_traversal() {
        for target in [
            "/assets/%2e%2e/x",
            "/assets/a%2fb",
            "/assets/%00",
            "/assets/%ff",
            "/assets/%",
            "/assets/a\\b",
        ] {
            assert!(decoded_path(target).is_none(), "{target}");
        }
        assert_eq!(
            decoded_path("/assets/a%20b.js?q=x"),
            Some("/assets/a b.js".into())
        );
        assert!(matches_etag(Some("W/\"hash\", \"other\""), "\"hash\""));
    }
}
