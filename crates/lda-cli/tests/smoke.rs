use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

fn workspace_target_debug(bin: &str) -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("..");
    p.push("..");
    p.push("target");
    p.push("debug");
    p.push(bin);
    p
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

#[test]
fn lda_cli_stt_with_stub_backend_prints_stub_text() {
    let sidecar = workspace_target_debug("inference-core");
    let cli = workspace_target_debug("lda-cli");
    assert!(sidecar.exists(), "build inference-core first: {sidecar:?}");
    assert!(cli.exists(), "build lda-cli first: {cli:?}");

    let tmp = tempfile::TempDir::new().unwrap();
    let socket = tmp.path().join("s.sock");
    let wav = tmp.path().join("input.wav");
    std::fs::write(&wav, synth_wav_i16_mono_16k(&vec![0_i16; 1600])).unwrap();

    let mut child = Command::new(&sidecar)
        .env("SIDECAR_SOCKET_PATH", &socket)
        .env("SIDECAR_STT_BACKEND", "stub")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sidecar");

    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        if socket.exists() { break; }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(socket.exists(), "sidecar socket never appeared");

    let out = Command::new(&cli)
        .arg("--socket")
        .arg(&socket)
        .arg("stt")
        .arg(&wav)
        .output()
        .expect("run lda-cli");
    let _ = child.kill();
    let _ = child.wait();

    assert!(out.status.success(), "lda-cli failed: stderr={}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("[stub] 1600 samples"), "stdout: {stdout}");
}
