//! Actual basename CLI startup must retain the configuration directory for UI.
#![cfg(target_os = "linux")]
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::process::{Child, Command, Stdio};
use std::time::Duration;
struct Process(Option<Child>);
impl Drop for Process {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
#[test]
fn basename_config_serves_its_external_mount_in_actual_process() {
    let root = std::env::temp_dir().join(format!("native-hosting-basename-{}", std::process::id()));
    std::fs::create_dir_all(root.join("ui/proof")).unwrap();
    std::fs::write(root.join("ui/proof/index.html"), "basename-ui-proof").unwrap();
    let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
    let api = tcp.local_addr().unwrap().port();
    let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
    let dns = udp.local_addr().unwrap().port();
    std::fs::write(root.join("config.yaml"), format!("log: {{level: error}}\napi: {{http: '127.0.0.1:{api}'}}\nplugins:\n  - tag: forward\n    type: forward\n    args: {{upstreams: [{{addr: 'udp://127.0.0.1:9'}}]}}\n  - tag: entry\n    type: sequence\n    args: [{{exec: $forward}}]\n  - tag: main\n    type: udp_server\n    args: {{entry: entry, listen: '127.0.0.1:{dns}', enable_audit: false}}\n")).unwrap();
    drop(tcp);
    drop(udp);
    let mut process = Process(Some(
        Command::new(env!("CARGO_BIN_EXE_mosdns"))
            .args(["start", "-c", "config.yaml"])
            .current_dir(&root)
            .env("PATH", "/nonexistent-native-runtime-tools")
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .unwrap(),
    ));
    let mut response = String::new();
    for _ in 0..100 {
        if let Ok(mut stream) = TcpStream::connect(("127.0.0.1", api)) {
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            stream
                .write_all(b"GET /proof/ HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
                .unwrap();
            stream.read_to_string(&mut response).unwrap();
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.ends_with("basename-ui-proof"), "{response}");
    assert!(
        Command::new("/bin/kill")
            .args(["-INT", &process.0.as_ref().unwrap().id().to_string()])
            .status()
            .unwrap()
            .success()
    );
    assert!(process.0.as_mut().unwrap().wait().unwrap().success());
    process.0 = None;
    std::fs::remove_dir_all(root).unwrap();
}
