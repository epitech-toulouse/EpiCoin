//! `epicoin-node`: runs a validator or a read-only replica.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context};
use clap::Parser;
use epicoin_core::{Genesis, Keypair};
use epicoin_node::logging::{self, LogFormat};
use epicoin_node::{http, producer, replica, Node, NodeClient, NodeOptions};

/// Name of the environment variable holding the validator secret key (64 hex characters).
/// It is deliberately not a command-line flag, which would show up in process listings.
const VALIDATOR_KEY_ENV: &str = "EPICOIN_VALIDATOR_KEY";

#[derive(Debug, Parser)]
#[command(
    name = "epicoin-node",
    about = "EpiCoin node. Produces blocks when given a validator key (EPICOIN_VALIDATOR_KEY), \
             or follows another node as a read-only replica when given --upstream."
)]
struct Args {
    /// Directory holding the chain database.
    #[arg(long, env = "EPICOIN_NODE_DATA_DIR", default_value = "./data/node")]
    data_dir: PathBuf,

    /// Genesis file of the chain (JSON).
    #[arg(long, env = "EPICOIN_GENESIS_FILE", default_value = "./genesis.json")]
    genesis: PathBuf,

    /// Address the HTTP API listens on. Keep it on a private network.
    #[arg(long, env = "EPICOIN_NODE_LISTEN", default_value = "127.0.0.1:7400")]
    listen: SocketAddr,

    /// Replica mode: URL of the node to follow, e.g. http://node:7400.
    #[arg(long, env = "EPICOIN_NODE_UPSTREAM")]
    upstream: Option<String>,

    /// How often a block is produced when transactions are waiting.
    #[arg(long, env = "EPICOIN_BLOCK_INTERVAL_MS", default_value_t = 3000)]
    block_interval_ms: u64,

    /// Produce an empty block after this many seconds without one (0 disables heartbeats).
    #[arg(long, env = "EPICOIN_HEARTBEAT_SECS", default_value_t = 0)]
    heartbeat_secs: u64,

    /// How often a replica polls its upstream.
    #[arg(long, env = "EPICOIN_SYNC_INTERVAL_MS", default_value_t = 2000)]
    sync_interval_ms: u64,

    #[arg(long, env = "EPICOIN_LOG_FORMAT", value_enum, default_value = "pretty")]
    log_format: LogFormat,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    logging::init(args.log_format);

    let genesis_json = std::fs::read_to_string(&args.genesis)
        .with_context(|| format!("cannot read the genesis file {}", args.genesis.display()))?;
    let genesis: Genesis =
        serde_json::from_str(&genesis_json).context("the genesis file is not valid")?;

    let validator = match std::env::var(VALIDATOR_KEY_ENV) {
        Ok(secret) if !secret.trim().is_empty() => Some(
            Keypair::from_secret_hex(&secret)
                .ok()
                .with_context(|| format!("{VALIDATOR_KEY_ENV} must be 64 hex characters"))?,
        ),
        _ => None,
    };
    match (&validator, &args.upstream) {
        (Some(_), Some(_)) => {
            bail!("choose one: a validator key (block production) or --upstream (replica)")
        }
        (None, None) => {
            bail!("set {VALIDATOR_KEY_ENV} to produce blocks, or --upstream to run a replica")
        }
        _ => {}
    }

    let node = Arc::new(
        Node::open(&args.data_dir, &genesis, validator, NodeOptions::default())
            .context("cannot open the node")?,
    );
    let status = node.status();
    tracing::info!(
        chain_id = %status.chain_id,
        mode = ?status.mode,
        height = status.height,
        state_root = %status.state_root,
        "node opened"
    );

    let background = match &args.upstream {
        Some(url) => {
            let upstream = NodeClient::new(url).context("invalid upstream URL")?;
            tracing::info!(upstream = %url, "following upstream node");
            tokio::spawn(replica::run(
                node.clone(),
                upstream,
                Duration::from_millis(args.sync_interval_ms.max(100)),
            ))
        }
        None => tokio::spawn(producer::run(
            node.clone(),
            Duration::from_millis(args.block_interval_ms.max(10)),
            (args.heartbeat_secs > 0).then(|| Duration::from_secs(args.heartbeat_secs)),
        )),
    };

    let listener = tokio::net::TcpListener::bind(args.listen)
        .await
        .with_context(|| format!("cannot listen on {}", args.listen))?;
    tracing::info!(listen = %args.listen, "HTTP API ready");
    axum::serve(listener, http::router(node))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .context("HTTP server failed")?;

    background.abort();
    tracing::info!("node stopped");
    Ok(())
}

/// Resolves on Ctrl-C or SIGTERM.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}
