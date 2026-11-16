//! `epicoin-cli`: genesis and key generation, full chain verification, administration.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, ensure, Context};
use clap::{Args, Parser, Subcommand};
use epicoin_core::{
    format_epc, parse_epc, Address, Amount, AuthorityAction, Genesis, Keypair, Transaction,
    TxPayload,
};
use epicoin_node::store::DB_FILE;
use epicoin_node::verify::{verify_store, Replayer, VerifyReport};
use epicoin_node::{now_ms, NodeClient, Store};

/// Environment variable holding the admin secret key. Never a command-line flag: flags are
/// visible in process listings and shell history.
const ADMIN_KEY_ENV: &str = "EPICOIN_ADMIN_KEY";

const GENESIS_FILE: &str = "genesis.json";
const KEYS_FILE: &str = "keys.env";

#[derive(Debug, Parser)]
#[command(name = "epicoin-cli", about = "EpiCoin command-line tools", version)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a new chain: generates the validator, admin and treasury keys and the genesis
    /// file.
    Genesis(GenesisArgs),
    /// Generate a key pair and print it.
    Keygen,
    /// Replay the whole chain from the genesis and check every hash, signature, state root
    /// and balance.
    Verify(VerifyArgs),
    /// Show the status of a node.
    Status(NodeArgs),
    /// Show the balance and nonce of an address.
    Balance {
        #[command(flatten)]
        node: NodeArgs,
        address: Address,
    },
    /// Manage mint authorities directly on chain (signs with EPICOIN_ADMIN_KEY).
    #[command(subcommand)]
    Authority(AuthorityCommand),
}

#[derive(Debug, Args)]
struct GenesisArgs {
    /// Short chain identifier, e.g. epicoin-prod (lowercase letters, digits, dashes).
    #[arg(long)]
    chain_id: String,
    /// Directory where genesis.json and keys.env are written.
    #[arg(long, default_value = ".")]
    out_dir: PathBuf,
    /// Length of a mint-quota period, in days.
    #[arg(long, default_value_t = 30)]
    quota_period_days: u64,
}

#[derive(Debug, Args)]
struct NodeArgs {
    /// URL of the node API.
    #[arg(
        long,
        env = "EPICOIN_NODE_URL",
        default_value = "http://127.0.0.1:7000"
    )]
    node: String,
}

#[derive(Debug, Args)]
struct VerifyArgs {
    /// The genesis file: the only input that is trusted.
    #[arg(long, env = "EPICOIN_GENESIS_FILE", default_value = "./genesis.json")]
    genesis: PathBuf,
    /// Verify the database of a stopped node (the directory containing chain.redb).
    #[arg(long, conflicts_with = "node")]
    data_dir: Option<PathBuf>,
    /// Verify a running node over HTTP.
    #[arg(long)]
    node: Option<String>,
}

#[derive(Debug, Subcommand)]
enum AuthorityCommand {
    /// List the mint authorities and their quotas.
    List(NodeArgs),
    /// Authorize an address to mint, or change its quota.
    Grant {
        #[command(flatten)]
        node: NodeArgs,
        address: Address,
        /// Maximum amount of EPC the address may mint per quota period, e.g. 500.
        #[arg(long, value_parser = parse_epc_arg)]
        quota: Amount,
    },
    /// Revoke an address.
    Revoke {
        #[command(flatten)]
        node: NodeArgs,
        address: Address,
    },
}

fn parse_epc_arg(text: &str) -> Result<Amount, String> {
    parse_epc(text).ok_or_else(|| format!("`{text}` is not an EPC amount (e.g. 500 or 12.50)"))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Genesis(args) => genesis(&args),
        Command::Keygen => {
            keygen();
            Ok(())
        }
        Command::Verify(args) => verify(&args).await,
        Command::Status(node) => status(&node).await,
        Command::Balance { node, address } => balance(&node, &address).await,
        Command::Authority(command) => authority(command).await,
    }
}

// --- genesis / keygen ------------------------------------------------------------------

fn genesis(args: &GenesisArgs) -> anyhow::Result<()> {
    ensure!(
        args.quota_period_days > 0,
        "the quota period must be at least one day"
    );
    let genesis_path = args.out_dir.join(GENESIS_FILE);
    let keys_path = args.out_dir.join(KEYS_FILE);
    for path in [&genesis_path, &keys_path] {
        ensure!(
            !path.exists(),
            "{} already exists; refusing to overwrite a chain's genesis or keys",
            path.display()
        );
    }

    let validator = Keypair::generate();
    let admin = Keypair::generate();
    let treasury = Keypair::generate();
    let genesis = Genesis {
        chain_id: args.chain_id.clone(),
        timestamp: now_ms(),
        validators: vec![validator.public_key()],
        admin: admin.address(),
        treasury: treasury.address(),
        quota_period_ms: args
            .quota_period_days
            .checked_mul(24 * 60 * 60 * 1000)
            .context("quota period too long")?,
    };
    genesis.validate().context("invalid genesis parameters")?;

    fs::create_dir_all(&args.out_dir)
        .with_context(|| format!("cannot create {}", args.out_dir.display()))?;

    let keys = format!(
        "# EpiCoin secret keys for chain `{chain}`.\n\
         # Keep this file secret and backed up offline: these keys cannot be recovered.\n\
         # - the validator key goes to the node,\n\
         # - the admin and treasury keys go to the API.\n\
         EPICOIN_VALIDATOR_KEY={validator}\n\
         EPICOIN_ADMIN_KEY={admin}\n\
         EPICOIN_TREASURY_KEY={treasury}\n",
        chain = genesis.chain_id,
        validator = *validator.secret_hex(),
        admin = *admin.secret_hex(),
        treasury = *treasury.secret_hex(),
    );
    write_secret(&keys_path, &keys)?;

    let json = serde_json::to_string_pretty(&genesis).context("cannot encode the genesis")?;
    fs::write(&genesis_path, format!("{json}\n"))
        .with_context(|| format!("cannot write {}", genesis_path.display()))?;

    println!("Chain `{}` created.", genesis.chain_id);
    println!("  genesis hash : {}", genesis.hash());
    println!("  validator    : {}", validator.public_key());
    println!("  admin        : {}", admin.address());
    println!("  treasury     : {}", treasury.address());
    println!("  genesis file : {}", genesis_path.display());
    println!(
        "  secret keys  : {} (keep secret, back up offline)",
        keys_path.display()
    );
    Ok(())
}

/// Writes a file readable by its owner only.
fn write_secret(path: &Path, content: &str) -> anyhow::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .with_context(|| format!("cannot create {}", path.display()))?;
    file.write_all(content.as_bytes())
        .with_context(|| format!("cannot write {}", path.display()))?;
    Ok(())
}

fn keygen() {
    let keypair = Keypair::generate();
    println!("secret key : {}", *keypair.secret_hex());
    println!("public key : {}", keypair.public_key());
    println!("address    : {}", keypair.address());
}

// --- verify ----------------------------------------------------------------------------

fn read_genesis(path: &Path) -> anyhow::Result<Genesis> {
    let json = fs::read_to_string(path)
        .with_context(|| format!("cannot read the genesis file {}", path.display()))?;
    let genesis: Genesis = serde_json::from_str(&json).context("the genesis file is not valid")?;
    genesis
        .validate()
        .context("the genesis file is not valid")?;
    Ok(genesis)
}

async fn verify(args: &VerifyArgs) -> anyhow::Result<()> {
    let genesis = read_genesis(&args.genesis)?;
    let report = match (&args.data_dir, &args.node) {
        (Some(dir), None) => verify_data_dir(dir, &genesis)?,
        (None, Some(url)) => verify_node(url, &genesis).await?,
        _ => bail!("choose what to verify: --data-dir <DIR> (stopped node) or --node <URL>"),
    };
    println!("OK — the chain is valid.");
    println!("  chain id      : {}", report.chain_id);
    println!("  height        : {}", report.height);
    println!("  transactions  : {}", report.transactions);
    println!("  tip hash      : {}", report.tip_hash);
    println!("  state root    : {}", report.state_root);
    println!("  accounts      : {}", report.accounts);
    println!("  total minted  : {} EPC", format_epc(report.total_minted));
    println!(
        "  in circulation: {} EPC",
        format_epc(report.circulating_supply)
    );
    Ok(())
}

fn verify_data_dir(dir: &Path, genesis: &Genesis) -> anyhow::Result<VerifyReport> {
    let path = dir.join(DB_FILE);
    ensure!(path.exists(), "no chain database at {}", path.display());
    let (store, _) = Store::open(&path, genesis).with_context(|| {
        format!(
            "cannot open {} (is the node still running? stop it or use --node)",
            path.display()
        )
    })?;
    verify_store(&store, genesis).context("VERIFICATION FAILED")
}

/// Downloads every block from a running node, replays them, then compares what the node
/// claims (tip, state root, every balance) with the replayed state.
async fn verify_node(url: &str, genesis: &Genesis) -> anyhow::Result<VerifyReport> {
    let client = NodeClient::new(url)?;
    let mut replayer = Replayer::new(genesis)?;
    loop {
        let blocks = client
            .sync_blocks(replayer.height() + 1, epicoin_node::http::MAX_SYNC_BATCH)
            .await
            .context("cannot download blocks")?;
        if blocks.is_empty() {
            break;
        }
        for block in &blocks {
            replayer.apply(block).context("VERIFICATION FAILED")?;
        }
    }
    let (chain, report) = replayer.finish().context("VERIFICATION FAILED")?;

    // The node may have produced blocks while we were replaying; compare at our height.
    let status = client.status().await?;
    ensure!(
        status.height >= report.height,
        "VERIFICATION FAILED: the node reports height {} but served {} blocks",
        status.height,
        report.height
    );
    if status.height == report.height {
        ensure!(
            status.tip_hash == report.tip_hash && status.state_root == report.state_root,
            "VERIFICATION FAILED: the node's tip or state root differs from the replayed chain"
        );
        for account in client.accounts().await? {
            let replayed = chain.state().account(&account.address);
            ensure!(
                replayed.balance == account.balance && replayed.nonce == account.nonce,
                "VERIFICATION FAILED: the node reports a wrong balance for {}",
                account.address
            );
        }
    }
    Ok(report)
}

// --- status / balance ------------------------------------------------------------------

async fn status(args: &NodeArgs) -> anyhow::Result<()> {
    let status = NodeClient::new(&args.node)?.status().await?;
    println!("chain id      : {}", status.chain_id);
    println!("mode          : {:?}", status.mode);
    println!("height        : {}", status.height);
    println!("tip hash      : {}", status.tip_hash);
    println!("state root    : {}", status.state_root);
    println!("mempool       : {} pending", status.mempool_size);
    println!("total minted  : {} EPC", format_epc(status.total_minted));
    println!(
        "in circulation: {} EPC",
        format_epc(status.circulating_supply)
    );
    println!("admin         : {}", status.admin);
    println!("treasury      : {}", status.treasury);
    Ok(())
}

async fn balance(args: &NodeArgs, address: &Address) -> anyhow::Result<()> {
    let account = NodeClient::new(&args.node)?.account(address).await?;
    println!("address : {}", account.address);
    println!("balance : {} EPC", format_epc(account.balance));
    println!("nonce   : {}", account.nonce);
    if let Some(authority) = account.authority {
        println!(
            "mint authority: {} — quota {} EPC, {} EPC left this period",
            if authority.active {
                "active"
            } else {
                "revoked"
            },
            format_epc(authority.quota),
            format_epc(authority.remaining)
        );
    }
    Ok(())
}

// --- authority -------------------------------------------------------------------------

async fn authority(command: AuthorityCommand) -> anyhow::Result<()> {
    match command {
        AuthorityCommand::List(node) => {
            let authorities = NodeClient::new(&node.node)?.authorities().await?;
            if authorities.is_empty() {
                println!("No mint authority.");
            }
            for authority in authorities {
                println!(
                    "{}  {:<7}  quota {:>10} EPC  left {:>10} EPC",
                    authority.address,
                    if authority.active {
                        "active"
                    } else {
                        "revoked"
                    },
                    format_epc(authority.quota),
                    format_epc(authority.remaining)
                );
            }
            Ok(())
        }
        AuthorityCommand::Grant {
            node,
            address,
            quota,
        } => update_authority(&node, address, AuthorityAction::Grant { quota }).await,
        AuthorityCommand::Revoke { node, address } => {
            update_authority(&node, address, AuthorityAction::Revoke).await
        }
    }
}

async fn update_authority(
    node: &NodeArgs,
    authority: Address,
    action: AuthorityAction,
) -> anyhow::Result<()> {
    let secret = std::env::var(ADMIN_KEY_ENV)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .with_context(|| format!("set {ADMIN_KEY_ENV} to the admin secret key"))?;
    let admin = Keypair::from_secret_hex(&secret)
        .ok()
        .with_context(|| format!("{ADMIN_KEY_ENV} must be 64 hex characters"))?;

    let client = NodeClient::new(&node.node)?;
    let status = client.status().await?;
    ensure!(
        admin.address() == status.admin,
        "{ADMIN_KEY_ENV} is not the admin key of chain `{}`",
        status.chain_id
    );
    let nonce = client.account(&admin.address()).await?.pending_nonce;
    let tx = Transaction::sign(
        &status.chain_id,
        &admin,
        nonce,
        TxPayload::AuthorityUpdate { authority, action },
    );
    let result = client
        .submit(&tx, Some(Duration::from_secs(20)))
        .await
        .context("the node refused the transaction")?;
    println!("transaction {} : {:?}", result.hash, result.status);
    Ok(())
}
