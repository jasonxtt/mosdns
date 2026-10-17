use super::*;

/// A state file exactly as the reference host wrote it in production.
const GO_BACKGROUND: &str = "{\n  \"mode\": \"upload\",\n  \"light_color\": \"#f8fafc\",\n  \"dark_color\": \"#0f172a\",\n  \"upload_id\": \"18a929bc99f3d3e8\",\n  \"opacity\": 0.44,\n  \"blur\": 4\n}";
const GO_TEXT: &str = "{\n  \"light\": {\n    \"mode\": \"default\",\n    \"color\": \"#1e252b\"\n  },\n  \"dark\": {\n    \"mode\": \"default\",\n    \"color\": \"#f8fafc\"\n  }\n}";
const GO_BUTTON: &str = "{\n  \"light\": {\n    \"mode\": \"custom\",\n    \"color\": \"#057a48\"\n  },\n  \"dark\": {\n    \"mode\": \"custom\",\n    \"color\": \"#53b54c\"\n  }\n}";

const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR-test-image";

struct Tree(PathBuf);

impl Tree {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!("appearance-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("webinfo")).unwrap();
        Self(root)
    }

    fn file(&self, relative: &str) -> PathBuf {
        self.0.join("webinfo").join(relative)
    }
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn run<T>(future: impl std::future::Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(future)
}

fn call(tree: &Tree, method: &str, path: &str, body: &[u8]) -> Reply {
    run(handle(
        &tree.0,
        method,
        &format!("{PREFIX}{path}"),
        None,
        body,
    ))
    .expect("an appearance route")
}

fn multipart(parts: &[(&str, Option<&str>, &[u8])]) -> (String, Vec<u8>) {
    let boundary = "----testboundary7MA4YWxk";
    let mut body = Vec::new();
    for (name, filename, content) in parts {
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"").as_bytes(),
        );
        if let Some(filename) = filename {
            body.extend_from_slice(
                format!("; filename=\"{filename}\"\r\nContent-Type: application/octet-stream")
                    .as_bytes(),
            );
        }
        body.extend_from_slice(b"\r\n\r\n");
        body.extend_from_slice(content);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

fn upload_png(tree: &Tree, data: &[u8]) -> Reply {
    let (content_type, body) = multipart(&[("file", Some("a.png"), data)]);
    run(handle(
        &tree.0,
        "POST",
        &format!("{PREFIX}panel-background/upload"),
        Some(&content_type),
        &body,
    ))
    .unwrap()
}

fn value(reply: &Reply) -> serde_json::Value {
    serde_json::from_slice(&reply.body)
        .unwrap_or_else(|_| panic!("JSON body: {:?}", String::from_utf8_lossy(&reply.body)))
}

#[test]
fn files_written_by_the_reference_host_are_read_and_answered_as_it_would() {
    let tree = Tree::new("go-files");
    fs::write(tree.file(SETTINGS_FILE), GO_BACKGROUND).unwrap();
    fs::write(tree.file(TEXT_FILE), GO_TEXT).unwrap();
    fs::write(tree.file(BUTTON_FILE), GO_BUTTON).unwrap();
    let background = call(&tree, "GET", "panel-background", b"");
    assert_eq!(
        (background.status, background.content_type),
        (200, Some("application/json"))
    );
    assert!(background.body.ends_with(b"\n"));
    // `color` mirrors the light colour; no history file exists, so there is no image URL.
    assert_eq!(
        String::from_utf8(background.body).unwrap(),
        "{\"mode\":\"upload\",\"color\":\"#f8fafc\",\"light_color\":\"#f8fafc\",\"dark_color\":\"#0f172a\",\"upload_id\":\"18a929bc99f3d3e8\",\"opacity\":0.44,\"blur\":4}\n"
    );
    assert_eq!(
        value(&call(&tree, "GET", "text-color", b"")),
        serde_json::json!({"light": {"mode": "default", "color": "#1e252b"}, "dark": {"mode": "default", "color": "#f8fafc"}})
    );
    assert_eq!(
        value(&call(&tree, "GET", "button-color", b"")),
        serde_json::json!({"light": {"mode": "custom", "color": "#057a48"}, "dark": {"mode": "custom", "color": "#53b54c"}})
    );
}

#[test]
fn what_is_written_has_the_reference_layout_and_survives_a_second_read() {
    let tree = Tree::new("layout");
    let reply = call(&tree, "POST", "button-color", br##"{"light":{"mode":"CUSTOM","color":"#057A48"},"dark":{"mode":"other","color":"#123456"}}"##);
    assert_eq!(reply.status, 200);
    // `MarshalIndent`: two spaces, no trailing newline, defaults for anything but `custom`.
    assert_eq!(
        fs::read_to_string(tree.file(BUTTON_FILE)).unwrap(),
        "{\n  \"light\": {\n    \"mode\": \"custom\",\n    \"color\": \"#057a48\"\n  },\n  \"dark\": {\n    \"mode\": \"default\",\n    \"color\": \"#40d889\"\n  }\n}"
    );
    let reply = call(
        &tree,
        "POST",
        "panel-background",
        br##"{"mode":"color","color":"#ABCDEF","opacity":0.5,"blur":7}"##,
    );
    assert_eq!(reply.status, 200);
    assert_eq!(
        fs::read_to_string(tree.file(SETTINGS_FILE)).unwrap(),
        "{\n  \"mode\": \"color\",\n  \"color\": \"#abcdef\",\n  \"light_color\": \"#abcdef\",\n  \"dark_color\": \"#abcdef\",\n  \"opacity\": 0.5,\n  \"blur\": 7\n}"
    );
    assert_eq!(
        value(&call(&tree, "GET", "panel-background", b"")),
        value(&reply)
    );
}

#[test]
fn whole_numbers_and_field_order_are_written_like_go() {
    let tree = Tree::new("go-format");
    let reply = call(
        &tree,
        "POST",
        "panel-background",
        br#"{"mode":"none","opacity":1,"blur":0}"#,
    );
    assert!(String::from_utf8_lossy(&reply.body).contains("\"opacity\":1,\"blur\":0}"));
    assert!(
        fs::read_to_string(tree.file(SETTINGS_FILE))
            .unwrap()
            .contains("\"opacity\": 1,\n")
    );
    let reply = call(
        &tree,
        "POST",
        "panel-background",
        br#"{"mode":"none","opacity":0}"#,
    );
    assert!(String::from_utf8_lossy(&reply.body).contains("\"opacity\":0,"));
    upload_png(&tree, PNG);
    let history =
        String::from_utf8(call(&tree, "GET", "panel-background/history", b"").body).unwrap();
    let item = history.split("\"items\":[{").nth(1).unwrap();
    let keys: Vec<_> = ["\"id\"", "\"image_url\"", "\"size\"", "\"modified_at\""]
        .iter()
        .map(|key| item.find(key).unwrap())
        .collect();
    assert!(keys.windows(2).all(|pair| pair[0] < pair[1]), "{item}");
}

#[test]
fn normalization_follows_the_reference() {
    let normalized = |json: &str| normalize_background(serde_json::from_str(json).unwrap());
    let unknown = normalized(r#"{"mode":" Upload "}"#);
    assert_eq!(
        (
            unknown.mode.as_str(),
            unknown.light_color.as_str(),
            unknown.dark_color.as_str()
        ),
        ("upload", "#f8fafc", "#0f172a")
    );
    assert_eq!(normalized(r#"{"mode":"gradient"}"#).mode, "none");
    let clamped = normalized(r#"{"opacity":3,"blur":-5}"#);
    assert_eq!((clamped.opacity, clamped.blur), (1.0, 0));
    assert_eq!(normalized(r#"{"opacity":-1,"blur":99}"#).blur, 40);
    // Absent fields are zero in the reference's decoder (a valid opacity and blur).
    assert_eq!(normalized("{}").opacity, 0.0);
    // An invalid colour falls back; a legacy `color` fills the light and dark colours.
    assert_eq!(
        normalized(r##"{"light_color":"#12","color":"#00ff00"}"##).light_color,
        "#00ff00"
    );
    assert_eq!(
        normalized(r#"{"mode":"color","dark_color":"112233"}"#).color,
        "#112233"
    );
    assert_eq!(normalized(r#"{"upload_id":"abc"}"#).upload_id, "");
    assert_eq!(
        normalized(r#"{"upload_id":" abcdef "}"#).upload_id,
        "abcdef"
    );
    assert_eq!(normalize_hex("  #ABCDEF ", "x"), "#abcdef");
    assert_eq!(normalize_hex("#abcde", "#FFFFFF"), "#ffffff");
}

#[test]
fn applying_a_background_checks_its_inputs_like_the_reference() {
    let tree = Tree::new("apply");
    let post = |body: &str| call(&tree, "POST", "panel-background", body.as_bytes());
    for (body, status, message) in [
        ("not json", 400, None),
        (r#"{"mode":"url","url":"  "}"#, 400, Some("url 不能为空")),
        (
            r#"{"mode":"upload"}"#,
            400,
            Some("请先上传本地图片，再应用本地背景"),
        ),
        (
            r#"{"mode":"upload","upload_id":"missing01"}"#,
            400,
            Some("所选历史背景图片不存在，请重新选择"),
        ),
        (
            r#"{"mode":"url","url":" https://example.test/a.png ","upload_id":"leftover1","opacity":0.2}"#,
            200,
            None,
        ),
    ] {
        let reply = post(body);
        assert_eq!(reply.status, status, "{body}");
        if let Some(message) = message {
            assert_eq!(value(&reply)["error"], message);
        }
    }
    let url = value(&call(&tree, "GET", "panel-background", b""));
    assert_eq!(
        (
            url["url"].as_str(),
            url["image_url"].as_str(),
            url.get("upload_id")
        ),
        (
            Some("https://example.test/a.png"),
            Some("https://example.test/a.png"),
            None
        )
    );
}

#[test]
fn an_upload_is_kept_in_the_history_and_becomes_the_current_image() {
    let tree = Tree::new("upload");
    assert_eq!(
        call(&tree, "GET", "panel-background/history", b"").body,
        b"{\"items\":[]}\n"
    );
    assert_eq!(
        call(&tree, "GET", "panel-background/image", b"").status,
        404
    );
    let reply = upload_png(&tree, PNG);
    assert_eq!(
        reply.status,
        200,
        "{}",
        String::from_utf8_lossy(&reply.body)
    );
    let reply = value(&reply);
    let id = reply["upload_id"].as_str().unwrap().to_owned();
    assert_eq!(
        (reply["message"].as_str(), reply["size"].as_u64()),
        (Some("上传成功"), Some(PNG.len() as u64))
    );
    assert!(reply["image_url"].as_str().unwrap().starts_with(&format!(
        "/api/v1/appearance/panel-background/history/{id}?v="
    )));
    assert_eq!(fs::read(tree.file(IMAGE_FILE)).unwrap(), PNG);
    assert_eq!(
        fs::read(tree.file(&format!("{HISTORY_DIR}/{id}.png"))).unwrap(),
        PNG
    );
    // Serving: sniffed type, cache header, last-modified.
    let image = call(&tree, "GET", "panel-background/image", b"");
    assert_eq!(
        (image.status, image.content_type, image.body.as_slice()),
        (200, Some("image/png"), PNG)
    );
    assert!(
        image
            .headers
            .iter()
            .any(|(k, v)| *k == "Cache-Control" && v == "public, max-age=300")
    );
    assert!(
        image
            .headers
            .iter()
            .any(|(k, v)| *k == "Last-Modified" && v.ends_with(" GMT"))
    );
    let history = value(&call(&tree, "GET", "panel-background/history", b""));
    assert_eq!(history["items"].as_array().unwrap().len(), 1);
    assert_eq!(
        (
            history["items"][0]["id"].as_str(),
            history["items"][0]["size"].as_u64()
        ),
        (Some(id.as_str()), Some(PNG.len() as u64))
    );
    assert!(
        history["items"][0]["modified_at"]
            .as_str()
            .unwrap()
            .ends_with('Z')
    );
    let one = call(&tree, "GET", &format!("panel-background/history/{id}"), b"");
    assert_eq!(
        (one.status, one.content_type, one.body.as_slice()),
        (200, Some("image/png"), PNG)
    );
    // Applying it by id, and by "the current image", both work.
    let applied = value(&call(
        &tree,
        "POST",
        "panel-background",
        format!(r#"{{"mode":"upload","upload_id":"{id}","opacity":1,"blur":3}}"#).as_bytes(),
    ));
    assert!(
        applied["image_url"]
            .as_str()
            .unwrap()
            .contains(&format!("history/{id}?v="))
    );
    let current = value(&call(
        &tree,
        "POST",
        "panel-background",
        br#"{"mode":"upload","opacity":1,"blur":3}"#,
    ));
    assert!(
        current["image_url"]
            .as_str()
            .unwrap()
            .starts_with("/api/v1/appearance/panel-background/image?v=")
    );
    // Deleting the active history entry resets the setting to "none".
    call(
        &tree,
        "POST",
        "panel-background",
        format!(r#"{{"mode":"upload","upload_id":"{id}"}}"#).as_bytes(),
    );
    assert_eq!(
        value(&call(
            &tree,
            "DELETE",
            &format!("panel-background/history/{id}"),
            b""
        ))["message"],
        "历史背景已删除"
    );
    assert_eq!(
        value(&call(&tree, "GET", "panel-background", b""))["mode"],
        "none"
    );
    assert_eq!(
        call(
            &tree,
            "DELETE",
            &format!("panel-background/history/{id}"),
            b""
        )
        .status,
        404
    );
    assert_eq!(
        call(&tree, "GET", &format!("panel-background/history/{id}"), b"").status,
        404
    );
}

#[test]
fn clearing_the_history_counts_files_and_leaves_subdirectories() {
    let tree = Tree::new("clear");
    assert_eq!(
        value(&call(&tree, "DELETE", "panel-background/history", b""))["count"],
        0
    );
    let first = value(&upload_png(&tree, PNG))["upload_id"]
        .as_str()
        .unwrap()
        .to_owned();
    std::thread::sleep(std::time::Duration::from_millis(2));
    let second = value(&upload_png(&tree, b"GIF89a-more-bytes"))["upload_id"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_ne!(first, second);
    assert!(tree.file(&format!("{HISTORY_DIR}/{second}.gif")).exists());
    fs::create_dir(tree.file(&format!("{HISTORY_DIR}/keep-me"))).unwrap();
    call(
        &tree,
        "POST",
        "panel-background",
        format!(r#"{{"mode":"upload","upload_id":"{first}"}}"#).as_bytes(),
    );
    let cleared = value(&call(&tree, "DELETE", "panel-background/history", b""));
    assert_eq!(
        (cleared["message"].as_str(), cleared["count"].as_u64()),
        (Some("历史背景已清空"), Some(2))
    );
    assert!(tree.file(&format!("{HISTORY_DIR}/keep-me")).is_dir());
    assert_eq!(
        value(&call(&tree, "GET", "panel-background", b""))["mode"],
        "none"
    );
    // The uploaded image itself is not part of the history.
    assert!(tree.file(IMAGE_FILE).exists());
}

#[test]
fn uploads_are_bounded_and_must_be_images() {
    let tree = Tree::new("limits");
    let error_of = |reply: &Reply| {
        (
            reply.status,
            value(reply)["error"].as_str().unwrap().to_owned(),
        )
    };
    assert_eq!(
        error_of(&upload_png(&tree, b"")),
        (400, "上传文件为空".into())
    );
    let text = error_of(&upload_png(&tree, b"just some text"));
    assert_eq!(text.0, 400);
    assert!(
        text.1.starts_with("仅支持图片文件，当前类型: text/plain"),
        "{}",
        text.1
    );
    // Exactly the limit is accepted; one byte more is refused.
    let mut limit = PNG.to_vec();
    limit.resize(MAX_UPLOAD, 7);
    assert_eq!(upload_png(&tree, &limit).status, 200);
    limit.push(7);
    assert_eq!(
        error_of(&upload_png(&tree, &limit)),
        (400, "图片大小不能超过 20MB".into())
    );
    // A body beyond the framing allowance is refused before it is parsed.
    let oversized = vec![0_u8; MAX_UPLOAD_BODY + 1];
    let reply = run(handle(
        &tree.0,
        "POST",
        &format!("{PREFIX}panel-background/upload"),
        Some("multipart/form-data; boundary=x"),
        &oversized,
    ))
    .unwrap();
    assert_eq!(reply.status, 400);
    // Not multipart, no boundary, no file part, a field that is not a file.
    let post = |content_type: Option<&str>, body: &[u8]| {
        run(handle(
            &tree.0,
            "POST",
            &format!("{PREFIX}panel-background/upload"),
            content_type,
            body,
        ))
        .unwrap()
    };
    assert_eq!(post(Some("application/json"), b"{}").status, 400);
    assert_eq!(post(Some("multipart/form-data"), b"x").status, 400);
    let (content_type, body) = multipart(&[("other", None, b"value")]);
    let reply = post(Some(&content_type), &body);
    assert!(
        value(&reply)["error"]
            .as_str()
            .unwrap()
            .starts_with("缺少上传文件")
    );
    let (content_type, body) = multipart(&[("file", None, PNG)]);
    assert_eq!(
        post(Some(&content_type), &body).status,
        400,
        "a `file` field without a file name is a value"
    );
}

#[test]
fn multipart_parsing_finds_the_file_among_other_parts_and_keeps_binary_bytes() {
    let tree = Tree::new("multipart");
    let mut data = PNG.to_vec();
    data.extend_from_slice(b"\r\n--tricky\r\n\x00\xff");
    let (content_type, body) = multipart(&[
        ("note", None, b"hello"),
        ("file", Some("x.png"), &data),
        ("tail", None, b"z"),
    ]);
    assert_eq!(
        multipart_file(Some(&content_type), &body).unwrap(),
        data.as_slice()
    );
    let quoted = content_type.replace("boundary=", "boundary=\"") + "\"";
    assert_eq!(
        multipart_file(Some(&quoted), &body).unwrap(),
        data.as_slice()
    );
    assert_eq!(upload_png(&tree, &data).status, 200);
    assert!(multipart_file(None, b"").is_err());
}

#[test]
fn a_failed_write_leaves_the_previous_state_and_no_temporary_files() {
    let tree = Tree::new("failure");
    assert_eq!(
        call(
            &tree,
            "POST",
            "text-color",
            br##"{"light":{"mode":"custom","color":"#111111"}}"##
        )
        .status,
        200
    );
    let before = fs::read(tree.file(TEXT_FILE)).unwrap();
    // The history directory name is taken by a file: the upload cannot be stored.
    fs::write(tree.file(HISTORY_DIR), "x").unwrap();
    let reply = upload_png(&tree, PNG);
    assert_eq!(reply.status, 500);
    assert!(
        value(&reply)["error"]
            .as_str()
            .unwrap()
            .starts_with("保存背景历史图片失败")
    );
    assert!(!tree.file(IMAGE_FILE).exists(), "no half-applied upload");
    // The colour file's name is taken by a directory: the save fails, nothing else changes.
    fs::remove_file(tree.file(TEXT_FILE)).unwrap();
    fs::create_dir(tree.file(TEXT_FILE)).unwrap();
    let reply = call(
        &tree,
        "POST",
        "text-color",
        br#"{"light":{"mode":"default"}}"#,
    );
    assert_eq!(
        (
            reply.status,
            value(&reply)["error"]
                .as_str()
                .unwrap()
                .starts_with("save text color settings failed")
        ),
        (500, true)
    );
    assert!(fs::read_dir(tree.file("")).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tmp-")
    }));
    assert!(!before.is_empty());
}

#[test]
fn a_corrupt_state_file_answers_with_defaults_and_the_other_files_are_untouched() {
    let tree = Tree::new("corrupt");
    fs::write(tree.file(SETTINGS_FILE), "{not json").unwrap();
    fs::write(tree.file(TEXT_FILE), "[]").unwrap();
    let background = value(&call(&tree, "GET", "panel-background", b""));
    assert_eq!(
        (
            background["mode"].as_str(),
            background["opacity"].as_f64(),
            background["blur"].as_i64()
        ),
        (Some("none"), Some(0.9), Some(10))
    );
    assert_eq!(
        value(&call(&tree, "GET", "text-color", b""))["light"]["color"],
        "#1e252b"
    );
    assert_eq!(
        fs::read_to_string(tree.file(SETTINGS_FILE)).unwrap(),
        "{not json",
        "a read never rewrites"
    );
}

#[test]
fn history_ids_are_validated_and_unknown_routes_and_methods_are_distinguished() {
    let tree = Tree::new("routes");
    let status = |method: &str, path: &str| {
        run(handle(
            &tree.0,
            method,
            &format!("{PREFIX}{path}"),
            None,
            b"",
        ))
        .map(|reply| reply.status)
    };
    assert_eq!(
        status("GET", "panel-background/history/abc"),
        Some(400),
        "too short an id"
    );
    assert_eq!(
        status("DELETE", "panel-background/history/..%2Fx"),
        Some(400)
    );
    assert_eq!(
        status("GET", "panel-background/history/../../etc"),
        None,
        "no path traversal route"
    );
    assert_eq!(status("PUT", "panel-background"), Some(405));
    assert_eq!(status("GET", "panel-background/upload"), Some(405));
    assert_eq!(
        status("PATCH", "panel-background/history/abcdef"),
        Some(405)
    );
    assert_eq!(status("GET", "unknown"), None);
    assert!(run(handle(&tree.0, "GET", "/api/v1/other", None, b"")).is_none());
}

#[test]
fn the_image_type_signatures_match_the_reference_detector() {
    for (data, expected) in [
        (&b"\x00\x00\x01\x00rest"[..], "image/x-icon"),
        (b"BMxx", "image/bmp"),
        (b"GIF87a", "image/gif"),
        (b"GIF89a", "image/gif"),
        (b"RIFF\x01\x02\x03\x04WEBPVP8 ", "image/webp"),
        (b"RIFF\x01\x02\x03\x04WAVEfmt ", "application/octet-stream"),
        (b"\x89PNG\r\n\x1a\n", "image/png"),
        (b"\xff\xd8\xff\xe0", "image/jpeg"),
        (
            b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
            "text/plain; charset=utf-8",
        ),
        (b"\x00\x01\x02\x03", "application/octet-stream"),
    ] {
        assert_eq!(sniff(data), expected, "{data:?}");
    }
    assert_eq!(extension("image/jpeg"), "jpg");
    assert_eq!(extension("image/x-icon"), "bin");
}

#[test]
fn http_dates_use_the_gmt_form() {
    assert_eq!(
        http_date(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000)).as_deref(),
        Some("Tue, 14 Nov 2023 22:13:20 GMT")
    );
}
