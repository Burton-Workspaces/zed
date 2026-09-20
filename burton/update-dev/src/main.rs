use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context as _;
use burton_update_dev::{AppState, crate_version, repo_root, serve};
use clap::Parser;
use url::Url;

#[derive(Parser, Debug)]
#[command(
    name = "burton-update-dev",
    about = "Serve Burton installers and reverse-proxy Zed's extension marketplace.",
    long_about = "Burton points server_url at this origin. Auto-update polls \
/releases/.../asset; the Extensions UI calls /extensions*. Downloads are \
followed server-side so the client never sees Zed blob URLs.\n\n\
Bind on loopback and put Caddy in front for TLS, or listen directly for local testing."
)]
struct Args {
    /// Directory to search for installers (repeatable)
    #[arg(long = "dir")]
    dirs: Vec<PathBuf>,

    /// Version string returned in the JSON body
    #[arg(long)]
    version: Option<String>,

    /// Bind host (used with --port unless --listen is set)
    #[arg(long, default_value = "127.0.0.1")]
    host: String,

    /// Bind port (used with --host unless --listen is set)
    #[arg(long, default_value_t = 4180)]
    port: u16,

    /// Bind address (overrides --host/--port)
    #[arg(long)]
    listen: Option<SocketAddr>,

    /// Public origin for release asset URLs (for example https://updates.burton.dev)
    #[arg(long)]
    public_url: Option<String>,

    /// Extension marketplace origin to reverse-proxy
    #[arg(long, default_value = "https://api.zed.dev")]
    extensions_upstream: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let root = repo_root();
    let search_dirs = if args.dirs.is_empty() {
        vec![root.join("target")]
    } else {
        args.dirs
            .into_iter()
            .map(|path| {
                if path.is_absolute() {
                    path
                } else {
                    std::env::current_dir()
                        .unwrap_or_else(|_| root.clone())
                        .join(path)
                }
            })
            .collect()
    };
    let version = match args.version {
        Some(version) => version,
        None => crate_version(&root)?,
    };
    let extensions_upstream = Url::parse(&args.extensions_upstream)
        .with_context(|| format!("invalid --extensions-upstream {}", args.extensions_upstream))?;
    if extensions_upstream.scheme() != "http" && extensions_upstream.scheme() != "https" {
        anyhow::bail!("--extensions-upstream must be http or https");
    }

    let listen = match args.listen {
        Some(address) => address,
        None => format!("{}:{}", args.host, args.port)
            .parse()
            .with_context(|| format!("invalid bind address {}:{}", args.host, args.port))?,
    };

    let state = AppState::new(search_dirs, version, args.public_url, extensions_upstream)?;
    serve(state, listen).await
}
