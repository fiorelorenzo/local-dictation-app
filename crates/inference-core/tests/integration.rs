use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

use tempfile::TempDir;

fn binary_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("..");
    p.push("..");
    p.push("target");
    p.push("debug");
    p.push("inference-core");
    p
}

struct TestServer {
    child: std::process::Child,
    socket: PathBuf,
    _tmp: TempDir,
}

impl TestServer {
    fn spawn() -> Self {
        Self::spawn_with_env(&[])
    }

    fn spawn_with_env(extra: &[(&str, &str)]) -> Self {
        let tmp = TempDir::new().expect("tmp dir");
        let socket = tmp.path().join("sidecar.sock");
        let mut cmd = Command::new(binary_path());
        cmd.env("SIDECAR_SOCKET_PATH", &socket)
            .env("SIDECAR_LOG_LEVEL", "info");
        for (k, v) in extra {
            cmd.env(k, v);
        }
        let child = cmd.spawn().expect("spawn sidecar");
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if socket.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(socket.exists(), "sidecar did not create socket within 3s");
        Self { child, socket, _tmp: tmp }
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Performs a raw HTTP GET over a UNIX socket and returns (status, body).
/// We use hyper directly because reqwest does not support unix:// URLs.
async fn unix_get(socket: &std::path::Path, path: &str) -> (hyper::StatusCode, String) {
    use hyper::body::Bytes;
    use hyper::Request;
    use hyperlocal::{UnixConnector, Uri};
    use http_body_util::{BodyExt, Empty};

    let connector = UnixConnector;
    let client: hyper_util::client::legacy::Client<UnixConnector, Empty<Bytes>> =
        hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
            .build(connector);
    let uri: hyper::Uri = Uri::new(socket, path).into();
    let req = Request::builder()
        .uri(uri)
        .body(Empty::<Bytes>::new())
        .unwrap();
    let resp = client.request(req).await.expect("http request");
    let (parts, body) = resp.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    let response_body = String::from_utf8_lossy(&bytes).into_owned();
    (parts.status, response_body)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn healthz_returns_ok() {
    let server = TestServer::spawn();
    let (status, body) = unix_get(&server.socket, "/healthz").await;
    assert!(status.is_success(), "expected 2xx, got {status}");
    assert!(body.contains("\"status\":\"ok\""), "body: {body}");
    assert!(body.contains("\"version\":\"0.0.1\""), "body: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn version_returns_build_info() {
    let server = TestServer::spawn();
    let (status, body) = unix_get(&server.socket, "/version").await;
    assert!(status.is_success(), "expected 2xx, got {status}");
    assert!(body.contains("\"version\":\"0.0.1\""), "body: {body}");
    assert!(body.contains("\"backend\":\"whisper-rs\""), "body: {body}");
    assert!(body.contains("\"build\":"), "body should contain build field: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sigterm_removes_socket_file() {
    let server = TestServer::spawn();
    let socket = server.socket.clone();
    assert!(socket.exists(), "socket should exist before SIGTERM");

    // Send SIGTERM to the child.
    let pid = i32::try_from(server.child.id()).expect("pid fits in i32");
    let result = unsafe { libc::kill(pid, libc::SIGTERM) };
    assert_eq!(result, 0, "kill returned {result}, errno={}", std::io::Error::last_os_error());

    // Wait up to 3 s for the process to exit.
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    let mut exited = false;
    while std::time::Instant::now() < deadline {
        match unsafe { libc::waitpid(pid, std::ptr::null_mut(), libc::WNOHANG) } {
            0 => std::thread::sleep(Duration::from_millis(50)),
            -1 => panic!("waitpid failed: {}", std::io::Error::last_os_error()),
            _ => { exited = true; break; }
        }
    }
    assert!(exited, "sidecar did not exit within 3 s after SIGTERM");

    // Drop the TestServer at the end of scope to avoid double-kill.
    // Socket file must be gone.
    assert!(!socket.exists(), "socket file should be removed after SIGTERM cleanup");
    std::mem::forget(server); // already reaped above; skip Drop double-kill
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn healthz_reports_stt_ready_false_when_no_backend() {
    let server = TestServer::spawn();
    let (status, body) = unix_get(&server.socket, "/healthz").await;
    assert!(status.is_success(), "got {status}");
    assert!(body.contains("\"stt_ready\":false"), "body: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn healthz_reports_stt_ready_true_with_stub_backend() {
    let server = TestServer::spawn_with_env(&[("SIDECAR_STT_BACKEND", "stub")]);
    let (status, body) = unix_get(&server.socket, "/healthz").await;
    assert!(status.is_success(), "got {status}");
    assert!(body.contains("\"stt_ready\":true"), "body: {body}");
}
