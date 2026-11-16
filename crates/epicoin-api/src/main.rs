//! `epicoin-api`: the application backend and its maintenance commands.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{bail, Context};
use clap::{Parser, Subcommand};
use epicoin_api::config::{Config, MailerConfig};
use epicoin_api::mailer::{ConsoleMailer, Mailer, SmtpMailer};
use epicoin_api::models::Role;
use epicoin_api::state::AppState;
use epicoin_api::users::{self, NewUser};
use epicoin_api::vault::{MasterKey, Vault};
use epicoin_api::{auth, wallets, MIGRATOR};
use epicoin_node::logging::{self, LogFormat};
use epicoin_node::NodeClient;
use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;

#[derive(Debug, Parser)]
#[command(
    name = "epicoin-api",
    about = "EpiCoin application backend. Configured by environment variables (see .env.example)."
)]
struct Cli {
    #[arg(
        long,
        env = "EPICOIN_LOG_FORMAT",
        value_enum,
        default_value = "pretty",
        global = true
    )]
    log_format: LogFormat,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the HTTP API (the default).
    Serve,
    /// Create the first administrator and print their activation link.
    CreateAdmin {
        #[arg(long)]
        email: String,
        #[arg(long)]
        first_name: String,
        #[arg(long)]
        last_name: String,
    },
    /// Re-encrypt every wallet under EPICOIN_MASTER_KEY, reading the old ones with
    /// EPICOIN_MASTER_KEY_PREVIOUS. See docs for the full rotation procedure.
    RotateMasterKey,
    /// Print a new random master key.
    GenerateMasterKey,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    logging::init(cli.log_format);

    if let Some(Command::GenerateMasterKey) = cli.command {
        println!("{}", *MasterKey::generate_hex());
        return Ok(());
    }

    let config = Config::from_env().context("invalid configuration")?;
    let db = PgPoolOptions::new()
        .max_connections(10)
        .connect(&config.database_url)
        .await
        // Never print the URL: it contains the database password.
        .map_err(|_| {
            anyhow::anyhow!("cannot connect to the database (check EPICOIN_DATABASE_URL)")
        })?;
    MIGRATOR
        .run(&db)
        .await
        .context("database migration failed")?;

    match cli.command.unwrap_or(Command::Serve) {
        Command::Serve => serve(config, db).await,
        Command::CreateAdmin {
            email,
            first_name,
            last_name,
        } => create_admin(config, db, email, first_name, last_name).await,
        Command::RotateMasterKey => rotate_master_key(config, db).await,
        Command::GenerateMasterKey => unreachable!("handled before loading the configuration"),
    }
}

fn build_state(config: Config, db: PgPool) -> anyhow::Result<(AppState, SocketAddr)> {
    let mailer: Arc<dyn Mailer> = match &config.mailer {
        MailerConfig::Console => Arc::new(ConsoleMailer),
        MailerConfig::Smtp { url, from } => {
            Arc::new(SmtpMailer::new(url, from).context("invalid SMTP configuration")?)
        }
    };
    let node = NodeClient::new(&config.node_url).context("invalid EPICOIN_NODE_URL")?;
    let vault = Vault::new(config.master_key, config.previous_master_key);
    let state = AppState::new(
        db,
        node,
        vault,
        mailer,
        config.settings,
        config.admin_key,
        config.treasury_key,
    );
    Ok((state, config.listen))
}

async fn serve(config: Config, db: PgPool) -> anyhow::Result<()> {
    let (state, listen) = build_state(config, db)?;

    // Fail fast on keys that do not belong to the chain; tolerate a node that is merely
    // not up yet (it is retried on first use).
    match state.chain.info().await {
        Ok(info) => tracing::info!(chain_id = %info.chain_id, "connected to the node"),
        Err(error @ epicoin_api::chain::ChainError::Misconfigured(_)) => bail!("{error}"),
        Err(error) => tracing::warn!(%error, "node not reachable yet"),
    }

    let background = tokio::spawn(epicoin_api::run_background(state.clone()));
    let listener = tokio::net::TcpListener::bind(listen)
        .await
        .with_context(|| format!("cannot listen on {listen}"))?;
    tracing::info!(%listen, "HTTP API ready");
    axum::serve(
        listener,
        epicoin_api::router(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("HTTP server failed")?;
    background.abort();
    tracing::info!("API stopped");
    Ok(())
}

async fn create_admin(
    config: Config,
    db: PgPool,
    email: String,
    first_name: String,
    last_name: String,
) -> anyhow::Result<()> {
    let (state, _) = build_state(config, db)?;
    let new = users::validate(
        &NewUser {
            email,
            first_name,
            last_name,
            promo: String::new(),
            role: Role::Admin,
        },
        &state.settings,
    )
    .map_err(|message| anyhow::anyhow!("invalid user: {message}"))?;

    let mut tx = state.db.begin().await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE email = $1)")
        .bind(&new.email)
        .fetch_one(&mut *tx)
        .await?;
    if exists {
        bail!("a user with the email {} already exists", new.email);
    }
    let user = users::create(&mut tx, &state.vault, &new)
        .await
        .map_err(|e| anyhow::anyhow!("cannot create the user: {}", e.message))?;
    let token = auth::issue_token(
        &mut tx,
        user.id,
        auth::PURPOSE_INVITE,
        state.settings.invite_ttl,
    )
    .await?;
    tx.commit().await?;

    println!("Administrator {} created.", user.email);
    println!("Activation link (single use):");
    println!("  {}/activation#token={token}", state.settings.public_url);
    Ok(())
}

async fn rotate_master_key(config: Config, db: PgPool) -> anyhow::Result<()> {
    if config.previous_master_key.is_none() {
        bail!(
            "set EPICOIN_MASTER_KEY to the new key and EPICOIN_MASTER_KEY_PREVIOUS to the old one"
        );
    }
    let vault = Vault::new(config.master_key, config.previous_master_key);
    let report = wallets::rotate_master_key(&db, &vault).await?;
    println!(
        "{} wallet(s) re-encrypted, {} already under the current key ({}).",
        report.rotated,
        report.already_current,
        vault.current_key_id()
    );
    println!("You can now remove EPICOIN_MASTER_KEY_PREVIOUS and restart the API.");
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
