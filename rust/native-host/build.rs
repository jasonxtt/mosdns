use std::collections::BTreeMap;
use std::fmt::Write;
use std::path::{Path, PathBuf};

fn regular(path: &Path) {
    let metadata = std::fs::symlink_metadata(path).expect("required embedded file missing");
    assert!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "unsafe embedded file: {}",
        path.display()
    );
    let name = path.file_name().unwrap().to_string_lossy();
    assert!(
        !name.starts_with('.')
            && !name.ends_with(".map")
            && !name.ends_with(".pem")
            && !name.ends_with(".key"),
        "unexpected embedded file: {name}"
    );
}

fn collect(directory: &Path, root: &Path, assets: &mut BTreeMap<String, PathBuf>) {
    let metadata = std::fs::symlink_metadata(directory).expect("embedded directory missing");
    assert!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "unsafe embedded directory"
    );
    println!("cargo:rerun-if-changed={}", directory.display());
    for entry in std::fs::read_dir(directory).expect("embedded assets directory missing") {
        let entry = entry.expect("embedded asset directory entry");
        let path = entry.path();
        let kind = entry.file_type().expect("embedded asset type");
        assert!(!kind.is_symlink(), "embedded symlink: {}", path.display());
        if kind.is_dir() {
            collect(&path, root, assets);
        } else {
            regular(&path);
            let key = format!(
                "/{}",
                path.strip_prefix(root)
                    .unwrap()
                    .to_str()
                    .expect("UTF-8 asset path")
                    .replace('\\', "/")
            );
            assets.insert(key, path);
        }
    }
}

fn main() {
    println!("cargo:rerun-if-env-changed=MOSDNS_BUILD_VERSION");
    if let Ok(version) = std::env::var("MOSDNS_BUILD_VERSION") {
        assert!(
            !version.trim().is_empty() && !version.contains(['\n', '\r']),
            "invalid product version"
        );
        println!("cargo:rustc-env=MOSDNS_BUILD_VERSION={version}");
    }
    let root =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("../../coremain/www");
    let mut assets = BTreeMap::new();
    collect(&root.join("assets"), &root, &mut assets);
    for (route, file) in [("/", "log.html"), ("/log", "log1.html")] {
        let path = root.join(file);
        regular(&path);
        let html = std::fs::read_to_string(&path).expect("embedded HTML");
        for fragment in html.split(['"', '\'']) {
            if fragment.starts_with("/assets/") {
                let local = fragment.split('?').next().unwrap();
                assert!(
                    assets.contains_key(local),
                    "embedded root references missing asset: {local}"
                );
            }
        }
        assets.insert(route.to_owned(), path);
    }
    let mut generated = String::from("static EMBEDDED: &[EmbeddedAsset] = &[\n");
    for (route, path) in assets {
        println!("cargo:rerun-if-changed={}", path.display());
        let bytes = std::fs::read(&path).expect("embedded bytes");
        let digest = ring::digest::digest(&ring::digest::SHA256, &bytes);
        let mut hex = String::new();
        for byte in digest.as_ref() {
            write!(hex, "{byte:02x}").unwrap();
        }
        writeln!(
            generated,
            "EmbeddedAsset {{ path: {route:?}, bytes: include_bytes!({:?}), etag: {:?} }},",
            path.canonicalize().unwrap().to_str().unwrap(),
            format!("\"{hex}\"")
        )
        .unwrap();
    }
    generated.push_str("];\n");
    std::fs::write(
        PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("embedded_ui.rs"),
        generated,
    )
    .unwrap();
}
