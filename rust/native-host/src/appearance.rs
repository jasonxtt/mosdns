//! Server-side appearance: the panel background (an uploaded image with a
//! history, a URL or a colour) and the text and button colours of the WebUI.
//!
//! Routes, status codes, response bodies and the files under `webinfo/` match
//! the reference host (`coremain/api_appearance.go`), so either host can read
//! what the other wrote. See `docs/rust/contracts/native-webui-hosting.md`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::rule_sources::{Reply, write_atomically};

/// The largest accepted image (also the reference's limit).
pub(crate) const MAX_UPLOAD: usize = 20 * 1024 * 1024;
/// Multipart framing allowed on top of the image (the reference's `+1024`).
pub(crate) const MAX_UPLOAD_BODY: usize = MAX_UPLOAD + 1024;

const SETTINGS_FILE: &str = "appearance_settings.json";
const TEXT_FILE: &str = "appearance_text_settings.json";
const BUTTON_FILE: &str = "appearance_button_settings.json";
const IMAGE_FILE: &str = "panel_background.bin";
const HISTORY_DIR: &str = "panel_background_history";
const PREFIX: &str = "/api/v1/appearance/";

/// Appearance changes are rare; one lock keeps the settings file, the image and
/// the history consistent with each other.
static LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// One image upload at a time: a body can be 20 MiB, so the listener claims this slot
/// before it reads one (a second concurrent upload is answered 503 at once).
pub(crate) static UPLOAD_SLOT: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);

// ---------------------------------------------------------------- replies

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

// ---------------------------------------------------------------- settings

#[derive(Serialize, Clone, Debug, PartialEq)]
struct Background {
    mode: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    color: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    light_color: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    dark_color: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    upload_id: String,
    #[serde(serialize_with = "go_float")]
    opacity: f64,
    blur: i64,
}

/// A number the way Go's `encoding/json` writes it: `1` and `0`, not `1.0` and `0.0`.
fn go_float<S: serde::Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
    if value.fract() == 0.0 && value.abs() < 1e15 {
        serializer.serialize_i64(*value as i64)
    } else {
        serializer.serialize_f64(*value)
    }
}

/// What a request or a file may carry: every field optional, as in Go's decoder
/// (a missing or `null` field is its zero value; a wrong type is an error).
#[derive(Deserialize, Default)]
struct BackgroundInput {
    mode: Option<String>,
    url: Option<String>,
    color: Option<String>,
    light_color: Option<String>,
    dark_color: Option<String>,
    upload_id: Option<String>,
    opacity: Option<f64>,
    blur: Option<i64>,
}

#[derive(Serialize)]
struct BackgroundResponse {
    mode: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    url: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    color: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    light_color: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    dark_color: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    upload_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    image_url: String,
    #[serde(serialize_with = "go_float")]
    opacity: f64,
    blur: i64,
}

/// One history entry, fields in the reference's declaration order.
#[derive(Serialize)]
struct HistoryItem {
    id: String,
    image_url: String,
    size: u64,
    modified_at: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
struct ColorSetting {
    #[serde(default)]
    mode: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    color: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
struct ColorSettings {
    #[serde(default)]
    light: ColorSetting,
    #[serde(default)]
    dark: ColorSetting,
}

fn default_background() -> Background {
    Background {
        mode: "none".into(),
        url: String::new(),
        color: String::new(),
        light_color: "#f8fafc".into(),
        dark_color: "#0f172a".into(),
        upload_id: String::new(),
        opacity: 0.9,
        blur: 10,
    }
}

/// `^#?[0-9a-fA-F]{6}$` after trimming, lower-cased with a leading `#`; otherwise the lower-cased fallback.
fn normalize_hex(raw: &str, fallback: &str) -> String {
    let value = raw.trim();
    let digits = value.strip_prefix('#').unwrap_or(value);
    if digits.len() == 6 && digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        format!("#{}", digits.to_ascii_lowercase())
    } else {
        fallback.to_ascii_lowercase()
    }
}

/// `^[a-zA-Z0-9_-]{6,64}$` after trimming, else empty.
fn normalize_upload_id(raw: &str) -> String {
    let value = raw.trim();
    let valid = (6..=64).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
    if valid {
        value.to_owned()
    } else {
        String::new()
    }
}

fn normalize_background(raw: BackgroundInput) -> Background {
    let mut out = default_background();
    out.mode = match raw.mode.unwrap_or_default().trim().to_lowercase().as_str() {
        "url" => "url",
        "upload" => "upload",
        "color" => "color",
        _ => "none",
    }
    .into();
    out.url = raw.url.unwrap_or_default().trim().to_owned();
    out.color = normalize_hex(&raw.color.unwrap_or_default(), "");
    out.light_color = normalize_hex(&raw.light_color.unwrap_or_default(), "");
    out.dark_color = normalize_hex(&raw.dark_color.unwrap_or_default(), "");
    if out.light_color.is_empty() {
        out.light_color = normalize_hex(&out.color, "#f8fafc");
    }
    if out.dark_color.is_empty() {
        out.dark_color = normalize_hex(&out.color, "#0f172a");
    }
    if out.mode == "color" && out.color.is_empty() {
        out.color = out.dark_color.clone();
    }
    out.upload_id = normalize_upload_id(&raw.upload_id.unwrap_or_default());
    // An absent field is zero in Go's decoder, which is a valid opacity and blur.
    out.opacity = raw.opacity.unwrap_or(0.0).clamp(0.0, 1.0);
    out.blur = raw.blur.unwrap_or(0).clamp(0, 40);
    out
}

fn default_colors(button: bool) -> ColorSettings {
    let (light, dark) = if button {
        ("#40d889", "#40d889")
    } else {
        ("#1e252b", "#f8fafc")
    };
    ColorSettings {
        light: ColorSetting {
            mode: "default".into(),
            color: light.into(),
        },
        dark: ColorSetting {
            mode: "default".into(),
            color: dark.into(),
        },
    }
}

fn normalize_colors(raw: ColorSettings, button: bool) -> ColorSettings {
    let defaults = default_colors(button);
    let one = |raw: ColorSetting, base: &ColorSetting| {
        if raw.mode.trim().eq_ignore_ascii_case("custom") {
            ColorSetting {
                mode: "custom".into(),
                color: normalize_hex(&raw.color, &base.color),
            }
        } else {
            base.clone()
        }
    };
    ColorSettings {
        light: one(raw.light, &defaults.light),
        dark: one(raw.dark, &defaults.dark),
    }
}

// ---------------------------------------------------------------- files

struct Paths {
    settings: PathBuf,
    text: PathBuf,
    button: PathBuf,
    image: PathBuf,
    history: PathBuf,
}

fn paths(root: &Path) -> Paths {
    let webinfo = root.join("webinfo");
    Paths {
        settings: webinfo.join(SETTINGS_FILE),
        text: webinfo.join(TEXT_FILE),
        button: webinfo.join(BUTTON_FILE),
        image: webinfo.join(IMAGE_FILE),
        history: webinfo.join(HISTORY_DIR),
    }
}

/// Reads a JSON state file: `Ok(None)` when it does not exist.
fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<Option<T>, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| error.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

/// Go's `MarshalIndent`: two-space indentation and no trailing newline.
fn write_json(path: &Path, value: &impl Serialize) -> std::io::Result<()> {
    let bytes = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
    write_atomically(path, &bytes)
}

fn load_background(paths: &Paths) -> Result<Background, String> {
    Ok(read_json::<BackgroundInput>(&paths.settings)?
        .map_or_else(default_background, normalize_background))
}

fn load_colors(path: &Path, button: bool) -> Result<ColorSettings, String> {
    Ok(read_json::<ColorSettings>(path)?.map_or_else(
        || default_colors(button),
        |raw| normalize_colors(raw, button),
    ))
}

fn warn_fallback(what: &str, reason: &str) {
    crate::diag::warn_line!("load {what} settings failed, fallback to defaults: {reason}");
}

// ---------------------------------------------------------------- images

/// The image types Go's `http.DetectContentType` recognises; anything else is not an image.
fn sniff(data: &[u8]) -> &'static str {
    let head = &data[..data.len().min(512)];
    if head.starts_with(b"\x00\x00\x01\x00") || head.starts_with(b"\x00\x00\x02\x00") {
        "image/x-icon"
    } else if head.starts_with(b"BM") {
        "image/bmp"
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        "image/gif"
    } else if head.len() >= 14 && head.starts_with(b"RIFF") && &head[8..14] == b"WEBPVP" {
        "image/webp"
    } else if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if head.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if head
        .iter()
        .all(|byte| matches!(byte, b'\t' | b'\n' | b'\r' | 0x0c | 0x20..=0x7e | 0x80..=0xff))
    {
        "text/plain; charset=utf-8"
    } else {
        "application/octet-stream"
    }
}

fn extension(content_type: &str) -> &'static str {
    match content_type {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "image/bmp" => "bmp",
        "image/svg+xml" => "svg",
        _ => "bin",
    }
}

fn history_url(id: &str, modified: Option<&fs::Metadata>) -> String {
    let version = modified.map_or(0, mtime_nanos);
    if version > 0 {
        format!("{PREFIX}panel-background/history/{id}?v={version}")
    } else {
        format!("{PREFIX}panel-background/history/{id}")
    }
}

fn uploaded_url(paths: &Paths) -> String {
    fs::metadata(&paths.image).map_or_else(
        |_| String::new(),
        |meta| format!("{PREFIX}panel-background/image?v={}", mtime_nanos(&meta)),
    )
}

fn mtime_nanos(meta: &fs::Metadata) -> i128 {
    meta.modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos() as i128)
}

/// The history file of `id` (the first `<id>.*` in name order) and its metadata.
fn find_history(paths: &Paths, id: &str) -> Result<(PathBuf, fs::Metadata), std::io::Error> {
    let id = normalize_upload_id(id);
    if id.is_empty() {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    let mut names: Vec<String> = fs::read_dir(&paths.history)?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| !kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| name.starts_with(&format!("{id}.")))
        .collect();
    names.sort();
    let name = names
        .into_iter()
        .next()
        .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
    let path = paths.history.join(name);
    let meta = fs::metadata(&path)?;
    Ok((path, meta))
}

fn response_for(paths: &Paths, settings: &Background) -> BackgroundResponse {
    let mut image_url = String::new();
    match settings.mode.as_str() {
        "url" => image_url = settings.url.clone(),
        "upload" => {
            if !settings.upload_id.is_empty() {
                if let Ok((_, meta)) = find_history(paths, &settings.upload_id) {
                    image_url = history_url(&settings.upload_id, Some(&meta));
                }
            }
            if image_url.is_empty() {
                image_url = uploaded_url(paths);
            }
        }
        _ => {}
    }
    BackgroundResponse {
        mode: settings.mode.clone(),
        url: settings.url.clone(),
        color: normalize_hex(&settings.light_color, &settings.color),
        light_color: normalize_hex(&settings.light_color, "#f8fafc"),
        dark_color: normalize_hex(&settings.dark_color, "#0f172a"),
        upload_id: settings.upload_id.clone(),
        image_url,
        opacity: settings.opacity,
        blur: settings.blur,
    }
}

/// `Mon, 02 Jan 2006 15:04:05 GMT`.
fn http_date(time: std::time::SystemTime) -> Option<String> {
    let seconds = time.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    let format = time::format_description::parse(
        "[weekday repr:short], [day] [month repr:short] [year] [hour]:[minute]:[second] GMT",
    )
    .ok()?;
    time::OffsetDateTime::from_unix_timestamp(i64::try_from(seconds).ok()?)
        .ok()?
        .format(&format)
        .ok()
}

fn image_reply(data: Vec<u8>, meta: &fs::Metadata) -> Reply {
    let sniffed = sniff(&data);
    let content_type = if sniffed.starts_with("image/") {
        sniffed
    } else {
        "application/octet-stream"
    };
    let mut headers = vec![("Cache-Control", "public, max-age=300".to_owned())];
    if let Some(date) = meta.modified().ok().and_then(http_date) {
        headers.push(("Last-Modified", date));
    }
    Reply {
        status: 200,
        content_type: Some(content_type),
        body: data,
        headers,
    }
}

// ---------------------------------------------------------------- multipart

/// The bytes of the `file` part of a `multipart/form-data` body (a part that
/// carries a file name), or why there is none.
fn multipart_file<'a>(content_type: Option<&str>, body: &'a [u8]) -> Result<&'a [u8], String> {
    let content_type = content_type.unwrap_or_default();
    let mut parameters = content_type.split(';');
    if !parameters
        .next()
        .is_some_and(|media| media.trim().eq_ignore_ascii_case("multipart/form-data"))
    {
        return Err("request Content-Type isn't multipart/form-data".into());
    }
    let boundary = parameters
        .filter_map(|parameter| parameter.split_once('='))
        .find(|(key, _)| key.trim().eq_ignore_ascii_case("boundary"))
        .map(|(_, value)| value.trim().trim_matches('"'))
        .filter(|value| !value.is_empty() && value.len() <= 200)
        .ok_or("no multipart boundary param in Content-Type")?;
    let delimiter = format!("--{boundary}");
    let delimiter = delimiter.as_bytes();
    let mut position = find(body, delimiter, 0).ok_or("multipart: NextPart: EOF")?;
    loop {
        position += delimiter.len();
        let rest = &body[position..];
        if rest.starts_with(b"--") {
            return Err("http: no such file".into());
        }
        let rest_start = if rest.starts_with(b"\r\n") {
            position + 2
        } else {
            return Err("multipart: NextPart: malformed delimiter".into());
        };
        let headers_end = find(body, b"\r\n\r\n", rest_start).ok_or("multipart: NextPart: EOF")?;
        let headers = std::str::from_utf8(&body[rest_start..headers_end])
            .map_err(|_| "multipart: malformed part headers".to_owned())?;
        let content_start = headers_end + 4;
        let next = format!("\r\n--{boundary}");
        let content_end =
            find(body, next.as_bytes(), content_start).ok_or("multipart: NextPart: EOF")?;
        if is_file_part(headers) {
            return Ok(&body[content_start..content_end]);
        }
        position = content_end + 2;
    }
}

fn is_file_part(headers: &str) -> bool {
    headers.split("\r\n").any(|line| {
        let Some((name, value)) = line.split_once(':') else {
            return false;
        };
        if !name.trim().eq_ignore_ascii_case("content-disposition") {
            return false;
        }
        let parameter = |wanted: &str| {
            value.split(';').skip(1).find_map(|item| {
                let (key, value) = item.split_once('=')?;
                key.trim()
                    .eq_ignore_ascii_case(wanted)
                    .then(|| value.trim().trim_matches('"').to_owned())
            })
        };
        value
            .split(';')
            .next()
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case("form-data"))
            && parameter("name").as_deref() == Some("file")
            && parameter("filename").is_some()
    })
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    haystack
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle)
        .map(|offset| from + offset)
}

// ---------------------------------------------------------------- handlers

/// The appearance routes, or `None` when `path` is not one of them.
pub(crate) async fn handle(
    root: &Path,
    method: &str,
    path: &str,
    content_type: Option<&str>,
    body: &[u8],
) -> Option<Reply> {
    let rest = path.strip_prefix(PREFIX)?;
    let paths = paths(root);
    let history_id = rest.strip_prefix("panel-background/history/");
    Some(match (rest, method) {
        ("panel-background", "GET") => get_background(&paths),
        ("panel-background", "POST") => set_background(&paths, body).await,
        ("panel-background/upload", "POST") => upload(&paths, content_type, body).await,
        ("panel-background/image", "GET") => get_image(paths).await,
        ("panel-background/history", "GET") => list_history(&paths),
        ("panel-background/history", "DELETE") => clear_history(&paths).await,
        (_, "GET") if history_id.is_some_and(|id| !id.contains('/')) => {
            get_history_image(paths, history_id.unwrap_or_default().to_owned()).await
        }
        (_, "DELETE") if history_id.is_some_and(|id| !id.contains('/')) => {
            delete_history(&paths, history_id.unwrap_or_default()).await
        }
        ("text-color", "GET") => get_colors(&paths.text, false),
        ("text-color", "POST") => set_colors(&paths.text, body, false).await,
        ("button-color", "GET") => get_colors(&paths.button, true),
        ("button-color", "POST") => set_colors(&paths.button, body, true).await,
        (
            "panel-background"
            | "panel-background/upload"
            | "panel-background/image"
            | "panel-background/history"
            | "text-color"
            | "button-color",
            _,
        ) => not_allowed(),
        (_, _) if history_id.is_some_and(|id| !id.contains('/')) => not_allowed(),
        _ => return None,
    })
}

fn get_background(paths: &Paths) -> Reply {
    let settings = load_background(paths).unwrap_or_else(|reason| {
        warn_fallback("panel background", &reason);
        default_background()
    });
    json(200, &response_for(paths, &settings))
}

async fn set_background(paths: &Paths, body: &[u8]) -> Reply {
    let input: BackgroundInput = match serde_json::from_slice(body) {
        Ok(input) => input,
        Err(reason) => return error(400, format!("invalid request body: {reason}")),
    };
    let mut settings = normalize_background(input);
    if settings.mode == "url" && settings.url.trim().is_empty() {
        return error(400, "url 不能为空");
    }
    if settings.mode == "color" && settings.color.trim().is_empty() {
        return error(400, "纯色背景不能为空");
    }
    if settings.mode == "upload" {
        if settings.upload_id.is_empty() {
            if fs::metadata(&paths.image).is_err() {
                return error(400, "请先上传本地图片，再应用本地背景");
            }
        } else if find_history(paths, &settings.upload_id).is_err() {
            return error(400, "所选历史背景图片不存在，请重新选择");
        }
        settings.url.clear();
    } else {
        settings.upload_id.clear();
    }
    if settings.mode != "url" {
        settings.url.clear();
    }
    let _guard = LOCK.lock().await;
    if let Err(reason) = write_json(&paths.settings, &settings) {
        return error(
            500,
            format!("save panel background settings failed: {reason}"),
        );
    }
    json(200, &response_for(paths, &settings))
}

async fn upload(paths: &Paths, content_type: Option<&str>, body: &[u8]) -> Reply {
    if body.len() > MAX_UPLOAD_BODY {
        return error(400, "解析上传内容失败: http: request body too large");
    }
    let data = match multipart_file(content_type, body) {
        Ok(data) => data,
        Err(reason) if reason == "http: no such file" => {
            return error(400, format!("缺少上传文件: {reason}"));
        }
        Err(reason) => return error(400, format!("解析上传内容失败: {reason}")),
    };
    if data.is_empty() {
        return error(400, "上传文件为空");
    }
    if data.len() > MAX_UPLOAD {
        return error(400, "图片大小不能超过 20MB");
    }
    let content_type = sniff(data);
    if !content_type.starts_with("image/") {
        return error(400, format!("仅支持图片文件，当前类型: {content_type}"));
    }
    let id = format!(
        "{:x}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos())
    );
    let history = paths
        .history
        .join(format!("{id}.{}", extension(content_type)));
    let image = paths.image.clone();
    let bytes = data.to_vec();
    let size = bytes.len();
    let _guard = LOCK.lock().await;
    let written = tokio::task::spawn_blocking(move || {
        write_atomically(&history, &bytes)
            .map_err(|reason| format!("保存背景历史图片失败: {reason}"))?;
        if let Err(reason) = write_atomically(&image, &bytes) {
            let _ = fs::remove_file(&history);
            return Err(format!("保存上传图片失败: {reason}"));
        }
        let meta = fs::metadata(&history).ok();
        Ok::<_, String>(meta)
    })
    .await;
    match written {
        Ok(Ok(meta)) => json(
            200,
            &serde_json::json!({
                "message": "上传成功",
                "upload_id": id,
                "image_url": history_url(&id, meta.as_ref()),
                "size": size,
            }),
        ),
        Ok(Err(reason)) => error(500, reason),
        Err(reason) => error(500, format!("保存上传图片失败: {reason}")),
    }
}

async fn read_image(
    path: PathBuf,
    missing: &'static str,
) -> Result<(Vec<u8>, fs::Metadata), Reply> {
    let outcome = tokio::task::spawn_blocking(move || {
        let data = fs::read(&path)?;
        let meta = fs::metadata(&path)?;
        Ok::<_, std::io::Error>((data, meta))
    })
    .await;
    match outcome {
        Ok(Ok((data, _))) if data.is_empty() => Err(error(404, "背景图片为空")),
        Ok(Ok(found)) => Ok(found),
        Ok(Err(reason)) if reason.kind() == std::io::ErrorKind::NotFound => {
            Err(error(404, missing))
        }
        Ok(Err(reason)) => Err(error(500, format!("读取背景图片失败: {reason}"))),
        Err(reason) => Err(error(500, format!("读取背景图片失败: {reason}"))),
    }
}

async fn get_image(paths: Paths) -> Reply {
    match read_image(paths.image, "未找到已上传的背景图片").await {
        Ok((data, meta)) => image_reply(data, &meta),
        Err(reply) => reply,
    }
}

fn list_history(paths: &Paths) -> Reply {
    let entries = match fs::read_dir(&paths.history) {
        Ok(entries) => entries,
        Err(reason) if reason.kind() == std::io::ErrorKind::NotFound => {
            return json(200, &serde_json::json!({ "items": [] }));
        }
        Err(reason) => return error(500, format!("读取背景历史失败: {reason}")),
    };
    let mut items = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Some((stem, _)) = name.split_once('.') else {
            continue;
        };
        let id = normalize_upload_id(stem);
        if id.is_empty() {
            continue;
        }
        let Ok(meta) = fs::metadata(entry.path()) else {
            continue;
        };
        let modified = meta
            .modified()
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|duration| {
                time::OffsetDateTime::from_unix_timestamp(duration.as_secs() as i64).ok()
            })
            .and_then(|time| {
                time.format(&time::format_description::well_known::Rfc3339)
                    .ok()
            })
            .unwrap_or_default();
        items.push((
            modified.clone(),
            name.clone(),
            HistoryItem {
                image_url: history_url(&id, Some(&meta)),
                id,
                size: meta.len(),
                modified_at: modified,
            },
        ));
    }
    // Newest first (the reference sorts by the second-resolution timestamp); the name keeps ties stable.
    items.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    #[derive(Serialize)]
    struct History {
        items: Vec<HistoryItem>,
    }
    json(
        200,
        &History {
            items: items.into_iter().map(|item| item.2).collect(),
        },
    )
}

async fn get_history_image(paths: Paths, id: String) -> Reply {
    let id = normalize_upload_id(&id);
    if id.is_empty() {
        return error(400, "无效的历史背景 ID");
    }
    let Ok((path, _)) = find_history(&paths, &id) else {
        return error(404, "未找到历史背景图片");
    };
    match read_image(path, "未找到历史背景图片").await {
        Ok((data, meta)) => image_reply(data, &meta),
        Err(reply) => reply,
    }
}

/// Forgets an uploaded background that no longer exists (what the reference does after a delete).
fn drop_active_upload(paths: &Paths, only_id: Option<&str>) {
    if let Ok(mut settings) = load_background(paths) {
        let matches = settings.mode == "upload"
            && match only_id {
                Some(id) => settings.upload_id == id,
                None => !settings.upload_id.is_empty(),
            };
        if matches {
            settings.mode = "none".into();
            settings.upload_id.clear();
            settings.url.clear();
            let _ = write_json(&paths.settings, &settings);
        }
    }
}

async fn delete_history(paths: &Paths, raw_id: &str) -> Reply {
    let id = normalize_upload_id(raw_id);
    if id.is_empty() {
        return error(400, "无效的历史背景 ID");
    }
    let _guard = LOCK.lock().await;
    let Ok((path, _)) = find_history(paths, &id) else {
        return error(404, "未找到历史背景图片");
    };
    if let Err(reason) = fs::remove_file(&path) {
        return error(500, format!("删除背景历史失败: {reason}"));
    }
    drop_active_upload(paths, Some(&id));
    json(200, &serde_json::json!({ "message": "历史背景已删除" }))
}

async fn clear_history(paths: &Paths) -> Reply {
    let _guard = LOCK.lock().await;
    let entries = match fs::read_dir(&paths.history) {
        Ok(entries) => entries,
        Err(reason) if reason.kind() == std::io::ErrorKind::NotFound => {
            return json(
                200,
                &serde_json::json!({ "message": "历史背景为空", "count": 0 }),
            );
        }
        Err(reason) => return error(500, format!("读取背景历史目录失败: {reason}")),
    };
    let mut removed = 0;
    for entry in entries.filter_map(Result::ok) {
        if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        if fs::remove_file(entry.path()).is_ok() {
            removed += 1;
        }
    }
    drop_active_upload(paths, None);
    json(
        200,
        &serde_json::json!({ "message": "历史背景已清空", "count": removed }),
    )
}

fn get_colors(path: &Path, button: bool) -> Reply {
    let settings = load_colors(path, button).unwrap_or_else(|reason| {
        warn_fallback(if button { "button color" } else { "text color" }, &reason);
        default_colors(button)
    });
    json(200, &settings)
}

async fn set_colors(path: &Path, body: &[u8], button: bool) -> Reply {
    let input: ColorSettings = match serde_json::from_slice(body) {
        Ok(input) => input,
        Err(reason) => return error(400, format!("invalid request body: {reason}")),
    };
    let settings = normalize_colors(input, button);
    let _guard = LOCK.lock().await;
    if let Err(reason) = write_json(path, &settings) {
        let what = if button { "button" } else { "text" };
        return error(500, format!("save {what} color settings failed: {reason}"));
    }
    json(200, &settings)
}

#[cfg(test)]
mod tests;
