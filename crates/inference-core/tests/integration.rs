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

/// Performs a raw HTTP POST over a UNIX socket and returns (status, body).
async fn unix_post(
    socket: &std::path::Path,
    path: &str,
    content_type: &str,
    body_bytes: Vec<u8>,
) -> (hyper::StatusCode, String) {
    use hyper::body::Bytes;
    use hyper::Request;
    use hyperlocal::{UnixConnector, Uri};
    use http_body_util::{BodyExt, Full};

    let connector = UnixConnector;
    let client: hyper_util::client::legacy::Client<UnixConnector, Full<Bytes>> =
        hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
            .build(connector);
    let uri: hyper::Uri = Uri::new(socket, path).into();
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", content_type)
        .header("accept", "application/json")
        .body(Full::new(Bytes::from(body_bytes)))
        .unwrap();
    let resp = client.request(req).await.expect("http post");
    let (parts, body) = resp.into_parts();
    let bytes = body.collect().await.unwrap().to_bytes();
    let response_body = String::from_utf8_lossy(&bytes).into_owned();
    (parts.status, response_body)
}

fn synth_wav_i16_mono_16k(samples: &[i16]) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut buf = Vec::new();
    {
        let mut writer = hound::WavWriter::new(std::io::Cursor::new(&mut buf), spec).unwrap();
        for s in samples {
            writer.write_sample(*s).unwrap();
        }
        writer.finalize().unwrap();
    }
    buf
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stt_with_stub_returns_text() {
    let server = TestServer::spawn_with_env(&[("SIDECAR_STT_BACKEND", "stub")]);
    // 1600 samples = 100 ms of audio at 16k
    let pcm: Vec<i16> = vec![0; 1600];
    let wav = synth_wav_i16_mono_16k(&pcm);
    let (status, body) = unix_post(&server.socket, "/v1/stt", "audio/wav", wav).await;
    assert!(status.is_success(), "status={status} body={body}");
    assert!(body.contains("\"text\":\"[stub] 1600 samples\""), "body: {body}");
    assert!(body.contains("\"backend\":\"stub\""), "body: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn stt_503_when_no_backend_loaded() {
    let server = TestServer::spawn(); // no SIDECAR_STT_BACKEND, no model path => no backend
    let pcm: Vec<i16> = vec![0; 16];
    let wav = synth_wav_i16_mono_16k(&pcm);
    let (status, body) = unix_post(&server.socket, "/v1/stt", "audio/wav", wav).await;
    assert_eq!(status, hyper::StatusCode::SERVICE_UNAVAILABLE, "body: {body}");
    assert!(body.contains("\"error\":\"stt_unavailable\""), "body: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn models_empty_when_no_backend() {
    let server = TestServer::spawn();
    let (status, body) = unix_get(&server.socket, "/v1/models").await;
    assert!(status.is_success(), "body: {body}");
    assert!(body.contains("\"models\":[]"), "body: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn models_lists_stub_when_loaded() {
    let server = TestServer::spawn_with_env(&[("SIDECAR_STT_BACKEND", "stub")]);
    let (status, body) = unix_get(&server.socket, "/v1/models").await;
    assert!(status.is_success(), "body: {body}");
    assert!(body.contains("\"backend\":\"stub\""), "body: {body}");
    assert!(body.contains("\"loaded\":true"), "body: {body}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires SIDECAR_WHISPER_MODEL_PATH to a real ggml file + sample-30s.wav fixture"]
async fn stt_real_whisper_transcribes_sample() {
    let model_path = std::env::var("SIDECAR_WHISPER_MODEL_PATH")
        .expect("set SIDECAR_WHISPER_MODEL_PATH to a real ggml whisper model");
    assert!(std::path::Path::new(&model_path).exists(), "model not found at {model_path}");

    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/sample-30s.wav");
    let wav = std::fs::read(&fixture).expect(
        "place a 30s WAV sample at crates/inference-core/tests/fixtures/sample-30s.wav (gitignored)",
    );

    let server = TestServer::spawn_with_env(&[
        ("SIDECAR_STT_BACKEND", "whisper"),
        ("SIDECAR_WHISPER_MODEL_PATH", model_path.as_str()),
    ]);
    let (status, body) = unix_post(&server.socket, "/v1/stt", "audio/wav", wav).await;
    assert!(status.is_success(), "body: {body}");
    assert!(body.contains("\"backend\":\"whisper-rs\""), "body: {body}");
    let text_idx = body.find("\"text\":\"").expect("text field");
    let after = &body[text_idx + 8..];
    let close = after.find('"').expect("text close");
    let transcript = &after[..close];
    assert!(!transcript.is_empty(), "transcript was empty: {body}");
    eprintln!("transcript: {transcript}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn stt_returns_503_busy_on_concurrent_requests() {
    // Stub sleeps 800 ms per request. Stub's LOCK_TIMEOUT is 200 ms (in src/stub.rs).
    // First request acquires the lock and sleeps; second request fails to acquire within 200 ms.
    let server = TestServer::spawn_with_env(&[
        ("SIDECAR_STT_BACKEND", "stub"),
        ("SIDECAR_STUB_SLEEP_MS", "800"),
    ]);
    let pcm: Vec<i16> = vec![0; 16];
    let wav = synth_wav_i16_mono_16k(&pcm);

    let s1 = server.socket.clone();
    let s2 = server.socket.clone();
    let w1 = wav.clone();
    let w2 = wav.clone();

    let h1 = tokio::spawn(async move { unix_post(&s1, "/v1/stt", "audio/wav", w1).await });
    // small delay so request 1 has acquired the mutex before request 2 arrives
    tokio::time::sleep(Duration::from_millis(50)).await;
    let h2 = tokio::spawn(async move { unix_post(&s2, "/v1/stt", "audio/wav", w2).await });

    let (r1, r2) = tokio::join!(h1, h2);
    let (status1, body1) = r1.unwrap();
    let (status2, body2) = r2.unwrap();

    assert!(status1.is_success(), "first request failed: {status1} {body1}");
    assert_eq!(status2, hyper::StatusCode::SERVICE_UNAVAILABLE, "body: {body2}");
    assert!(body2.contains("\"error\":\"busy\""), "body: {body2}");
}
