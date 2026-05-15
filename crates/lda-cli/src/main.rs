use std::path::PathBuf;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "lda-cli", version, about = "client for the local-dictation-app sidecar")]
struct Cli {
    /// Override the sidecar UNIX socket path.
    #[arg(long, global = true)]
    socket: Option<PathBuf>,

    /// Request MsgPack instead of JSON for responses.
    #[arg(long, global = true)]
    msgpack: bool,

    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// GET /healthz
    Health,
    /// GET /version
    Version,
    /// GET /v1/models
    Models,
    /// POST /v1/stt <file.wav>
    Stt {
        /// WAV file to transcribe.
        file: PathBuf,
        /// Language hint (ISO 639-1, e.g. "en", "it"). Default = auto-detect.
        #[arg(long)]
        language: Option<String>,
        /// Translate to English.
        #[arg(long)]
        translate: bool,
        /// Include segments in the response.
        #[arg(long)]
        segments: bool,
        /// Print the full response JSON instead of just the text.
        #[arg(long)]
        json: bool,
    },
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

    // Subcommand handlers land in T16-T19.
    let _ = (socket, cli.msgpack);
    eprintln!("subcommand {:?} not yet implemented (T16-T19)", cli.cmd);
    std::process::ExitCode::from(1)
}
