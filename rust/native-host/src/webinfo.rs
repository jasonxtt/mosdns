//! The `webinfo` plugin: one JSON document kept in `file`, read with `GET`
//! and replaced with `PUT` at the plugin's own `/plugins/<tag>` path (the
//! WebUI stores client names this way). The file is replaced atomically, so a
//! failed save keeps the previous document on disk and in memory.
//!
//! JSON is read and written the way the reference host does: object keys come
//! out sorted, strings are HTML-escaped, and the saved file is indented with
//! two spaces.

use std::cell::RefCell;
use std::path::PathBuf;

use serde_json::Value;

use crate::rule_sources::Reply;

const JSON_UTF8: &str = "application/json; charset=utf-8";

/// The runtime owner of one `webinfo` plugin.
pub struct WebInfo {
    pub tag: String,
    file: PathBuf,
    data: RefCell<Value>,
    write_lock: tokio::sync::Mutex<()>,
}

impl WebInfo {
    /// Reads `file`: a missing or empty file is an empty object, an invalid
    /// document is an error.
    pub(crate) fn load(tag: &str, file: PathBuf) -> Result<Self, String> {
        let data = match std::fs::read(&file) {
            Ok(bytes) if bytes.is_empty() => Value::Object(serde_json::Map::new()),
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
                format!("failed to parse json from file {}: {error}", file.display())
            })?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Value::Object(serde_json::Map::new())
            }
            Err(error) => return Err(format!("failed to read {}: {error}", file.display())),
        };
        Ok(Self {
            tag: tag.to_owned(),
            file,
            data: RefCell::new(data),
            write_lock: tokio::sync::Mutex::new(()),
        })
    }

    /// Serves `GET` and `PUT` on the plugin root.
    pub(crate) async fn handle(&self, method: &str, body: &[u8]) -> Reply {
        match method {
            "GET" => Reply {
                status: 200,
                content_type: Some(JSON_UTF8),
                body: encode_line(&self.data.borrow()),
            },
            "PUT" => self.put(body).await,
            _ => Reply {
                status: 405,
                content_type: None,
                body: Vec::new(),
            },
        }
    }

    async fn put(&self, body: &[u8]) -> Reply {
        let Ok(data) = serde_json::from_slice::<Value>(body) else {
            return json_error(400, "Invalid JSON format in request body");
        };
        let _serialized = self.write_lock.lock().await;
        let bytes = escape_html(
            serde_json::to_string_pretty(&data)
                .unwrap_or_default()
                .as_str(),
        )
        .into_bytes();
        let file = self.file.clone();
        let saved = crate::transaction::blocking_io(move || {
            crate::rule_sources::write_atomically(&file, &bytes)
        });
        if !matches!(saved, Ok(Ok(()))) {
            return json_error(500, "Failed to save data to file");
        }
        let reply = Reply {
            status: 200,
            content_type: Some(JSON_UTF8),
            body: encode_line(&data),
        };
        *self.data.borrow_mut() = data;
        reply
    }
}

fn json_error(status: u16, message: &str) -> Reply {
    Reply {
        status,
        content_type: Some(JSON_UTF8),
        body: encode_line(&serde_json::json!({ "error": message })),
    }
}

/// Compact JSON plus a newline, HTML-escaped like Go's `json.Encoder`.
fn encode_line(value: &Value) -> Vec<u8> {
    let mut text = escape_html(&serde_json::to_string(value).unwrap_or_default());
    text.push('\n');
    text.into_bytes()
}

/// Go's JSON encoder escapes these characters inside strings; none of them
/// can occur outside a string in valid JSON text.
fn escape_html(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for character in json.chars() {
        match character {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_matches_the_reference_encoder() {
        let value: Value = serde_json::from_str(r#"{"b":"<x&y>","a":[1,{"z":null}]}"#).unwrap();
        assert_eq!(
            String::from_utf8(encode_line(&value)).unwrap(),
            "{\"a\":[1,{\"z\":null}],\"b\":\"\\u003cx\\u0026y\\u003e\"}\n"
        );
    }
}
