use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use hyper::body::Bytes;
use hyper::Request;
use http_body_util::{BodyExt, Empty};
use hyperlocal::{UnixConnector, Uri};
use serde::Deserialize;

#[derive(Parser, Debug)]
#[command(name = "lda-cli", version, about = "client for the local-dictation-app sidecar")]
struct Cli {
    #[arg(long, global = true)]
    socket: Option<PathBuf>,

    #[arg(long, global = true)]
    msgpack: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    Health,
    Version,
    Models,
    Stt {
        file: PathBuf,
        #[arg(long)]
        language: Option<String>,
        #[arg(long)]
        translate: bool,
        #[arg(long)]
        segments: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Debug, Deserialize)]
struct HealthBody {
    status: String,
    version: String,
    uptime_ms: u128,
    stt_ready: bool,
}

fn resolve_socket(arg: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = arg {
        return Ok(p);
    }
    if let Ok(s) = std::env::var("SIDECAR_SOCKET_PATH") {
        return Ok(PathBuf::from(s));
    }
    let home = std::env::var("HOME").map_err(|_| anyhow!("$HOME not set"))?;
    Ok(PathBuf::from(format!(
        "{home}/Library/Application Support/app/sidecar.sock"
    )))
}

fn accept_header(msgpack: bool) -> &'static str {
    if msgpack { "application/msgpack" } else { "application/json" }
}

async fn unix_get_bytes(socket: &Path, path: &str, accept: &str) -> Result<(hyper::StatusCode, Vec<u8>)> {
    let connector = UnixConnector;
    let client: hyper_util::client::legacy::Client<UnixConnector, Empty<Bytes>> =
        hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
            .build(connector);
    let uri: hyper::Uri = Uri::new(socket, path).into();
    let req = Request::builder()
        .uri(uri)
        .header("accept", accept)
        .body(Empty::<Bytes>::new())?;
    let resp = client.request(req).await.context("request failed")?;
    let (parts, body) = resp.into_parts();
    let bytes = body.collect().await.context("body collect failed")?.to_bytes().to_vec();
    Ok((parts.status, bytes))
}

fn decode_body<T: for<'de> Deserialize<'de>>(bytes: &[u8], msgpack: bool) -> Result<T> {
    if msgpack {
        rmp_serde::from_slice(bytes).context("msgpack decode failed")
    } else {
        serde_json::from_slice(bytes).context("json decode failed")
    }
}

async fn cmd_health(socket: &Path, msgpack: bool) -> Result<i32> {
    let (status, bytes) = unix_get_bytes(socket, "/healthz", accept_header(msgpack)).await?;
    if !status.is_success() {
        eprintln!("{} {}", status, String::from_utf8_lossy(&bytes));
        return Ok(if status.is_client_error() { 3 } else { 4 });
    }
    let h: HealthBody = decode_body(&bytes, msgpack)?;
    println!(
        "status={}  version={}  uptime_ms={}  stt_ready={}",
        h.status, h.version, h.uptime_ms, h.stt_ready
    );
    Ok(0)
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("LDA_CLI_LOG_LEVEL").unwrap_or_else(|_| "warn".to_string()),
        )
        .with_writer(std::io::stderr)
        .init();

    let socket = match resolve_socket(cli.socket) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to resolve socket: {e}");
            return std::process::ExitCode::from(2);
        }
    };
    if !socket.exists() {
        eprintln!("socket not found: {}", socket.display());
        return std::process::ExitCode::from(2);
    }

    let code = match cli.cmd {
        Cmd::Health => cmd_health(&socket, cli.msgpack).await,
        Cmd::Version | Cmd::Models | Cmd::Stt { .. } => {
            eprintln!("subcommand not yet implemented");
            Ok(1)
        }
    };
    match code {
        Ok(c) => std::process::ExitCode::from(c as u8),
        Err(e) => {
            eprintln!("error: {e:?}");
            std::process::ExitCode::from(4)
        }
    }
}
