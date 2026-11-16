//! Drives the real `epicoin-cli` binary.

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Output};
use std::sync::Arc;
use std::time::Duration;

use epicoin_core::{
    AuthorityAction, Block, Genesis, Keypair, ReasonCode, Transaction, TxPayload, UNITS_PER_EPC,
};
use epicoin_node::store::DB_FILE;
use epicoin_node::{producer, Node, NodeOptions};
use tempfile::TempDir;

fn cli(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_epicoin-cli"));
    command
        .args(args)
        .env_remove("EPICOIN_ADMIN_KEY")
        .env_remove("EPICOIN_NODE_URL")
        .env_remove("EPICOIN_GENESIS_FILE");
    command
}

fn run(args: &[&str]) -> Output {
    cli(args).output().expect("the CLI runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A chain created by `epicoin-cli genesis`.
struct Chain {
    dir: TempDir,
    genesis: Genesis,
    keys: HashMap<String, String>,
}

impl Chain {
    fn create() -> Self {
        let dir = TempDir::new().unwrap();
        let output = run(&[
            "genesis",
            "--chain-id",
            "epicoin-clitest",
            "--out-dir",
            dir.path().to_str().unwrap(),
        ]);
        assert!(output.status.success(), "{}", stderr(&output));

        let genesis: Genesis = serde_json::from_str(
            &std::fs::read_to_string(dir.path().join("genesis.json")).unwrap(),
        )
        .unwrap();
        let keys = std::fs::read_to_string(dir.path().join("keys.env"))
            .unwrap()
            .lines()
            .filter(|line| !line.starts_with('#'))
            .filter_map(|line| line.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect();
        Self { dir, genesis, keys }
    }

    fn key(&self, name: &str) -> Keypair {
        Keypair::from_secret_hex(&self.keys[name]).unwrap()
    }

    fn genesis_path(&self) -> String {
        self.dir
            .path()
            .join("genesis.json")
            .to_str()
            .unwrap()
            .to_owned()
    }

    fn data_dir(&self) -> std::path::PathBuf {
        self.dir.path().join("node")
    }

    fn node(&self) -> Node {
        Node::open(
            &self.data_dir(),
            &self.genesis,
            Some(self.key("EPICOIN_VALIDATOR_KEY")),
            NodeOptions::default(),
        )
        .unwrap()
    }

    /// Builds three blocks: a grant, two mints, a purchase.
    fn populate(&self, node: &Node) -> Keypair {
        let admin = self.key("EPICOIN_ADMIN_KEY");
        let staff = Keypair::from_secret_bytes(&[11; 32]);
        let student = Keypair::from_secret_bytes(&[12; 32]);
        let id = &self.genesis.chain_id;
        let block = |txs: Vec<Transaction>| -> Block {
            for tx in txs {
                node.submit(tx).unwrap();
            }
            node.produce_block(false).unwrap().unwrap()
        };
        block(vec![Transaction::sign(
            id,
            &admin,
            0,
            TxPayload::AuthorityUpdate {
                authority: staff.address(),
                action: AuthorityAction::Grant {
                    quota: 500 * UNITS_PER_EPC,
                },
            },
        )]);
        let mint = |nonce, amount| {
            Transaction::sign(
                id,
                &staff,
                nonce,
                TxPayload::Mint {
                    to: student.address(),
                    amount,
                    reason: ReasonCode::Challenge,
                },
            )
        };
        block(vec![mint(0, 1_000), mint(1, 250)]);
        block(vec![Transaction::sign(
            id,
            &student,
            0,
            TxPayload::Purchase {
                amount: 300,
                order_ref: epicoin_core::Hash([5; 32]),
            },
        )]);
        student
    }
}

async fn serve(node: Arc<Node>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, epicoin_node::http::router(node))
            .await
            .unwrap();
    });
    format!("http://{address}")
}

/// Runs the CLI without blocking the async runtime that serves the node.
async fn run_async(args: Vec<String>, admin_key: Option<String>) -> Output {
    tokio::task::spawn_blocking(move || {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let mut command = cli(&args);
        if let Some(key) = admin_key {
            command.env("EPICOIN_ADMIN_KEY", key);
        }
        command.output().expect("the CLI runs")
    })
    .await
    .unwrap()
}

// --- genesis / keygen ------------------------------------------------------------------

#[test]
fn genesis_creates_a_valid_chain_definition() {
    let chain = Chain::create();
    assert_eq!(chain.genesis.validate(), Ok(()));
    assert_eq!(chain.genesis.chain_id, "epicoin-clitest");
    assert_eq!(chain.genesis.quota_period_ms, 30 * 24 * 60 * 60 * 1000);

    // The secret keys match the public material of the genesis.
    assert_eq!(
        chain.genesis.validators,
        vec![chain.key("EPICOIN_VALIDATOR_KEY").public_key()]
    );
    assert_eq!(
        chain.genesis.admin,
        chain.key("EPICOIN_ADMIN_KEY").address()
    );
    assert_eq!(
        chain.genesis.treasury,
        chain.key("EPICOIN_TREASURY_KEY").address()
    );

    // No secret in the genesis file.
    let genesis_text = std::fs::read_to_string(chain.dir.path().join("genesis.json")).unwrap();
    for secret in chain.keys.values() {
        assert!(!genesis_text.contains(secret));
    }
}

#[cfg(unix)]
#[test]
fn key_file_is_private() {
    use std::os::unix::fs::PermissionsExt;
    let chain = Chain::create();
    let mode = std::fs::metadata(chain.dir.path().join("keys.env"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o777, 0o600);
}

#[test]
fn genesis_does_not_print_or_overwrite_secrets() {
    let dir = TempDir::new().unwrap();
    let out_dir = dir.path().to_str().unwrap();
    let first = run(&["genesis", "--chain-id", "epicoin-a", "--out-dir", out_dir]);
    assert!(first.status.success());
    let keys = std::fs::read_to_string(dir.path().join("keys.env")).unwrap();
    for line in keys.lines().filter(|l| !l.starts_with('#')) {
        let secret = line.split_once('=').unwrap().1;
        assert!(!stdout(&first).contains(secret), "secret printed on stdout");
    }

    // A second run must not replace the keys of an existing chain.
    let second = run(&["genesis", "--chain-id", "epicoin-b", "--out-dir", out_dir]);
    assert!(!second.status.success());
    assert!(stderr(&second).contains("already exists"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("keys.env")).unwrap(),
        keys
    );
}

#[test]
fn genesis_rejects_a_bad_chain_id() {
    let dir = TempDir::new().unwrap();
    let output = run(&[
        "genesis",
        "--chain-id",
        "Not Valid!",
        "--out-dir",
        dir.path().to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert!(!dir.path().join("keys.env").exists());
    assert!(!dir.path().join("genesis.json").exists());
}

#[test]
fn keygen_prints_a_usable_key() {
    let output = run(&["keygen"]);
    assert!(output.status.success());
    let text = stdout(&output);
    let field = |name: &str| {
        text.lines()
            .find(|l| l.starts_with(name))
            .and_then(|l| l.split_once(": "))
            .map(|(_, v)| v.trim().to_owned())
            .unwrap()
    };
    let keypair = Keypair::from_secret_hex(&field("secret key")).unwrap();
    assert_eq!(keypair.public_key().to_string(), field("public key"));
    assert_eq!(keypair.address().to_string(), field("address"));
}

// --- verify ----------------------------------------------------------------------------

fn verify_data_dir(chain: &Chain) -> Output {
    run(&[
        "verify",
        "--genesis",
        &chain.genesis_path(),
        "--data-dir",
        chain.data_dir().to_str().unwrap(),
    ])
}

#[test]
fn verify_accepts_an_intact_database() {
    let chain = Chain::create();
    chain.populate(&chain.node());

    let output = verify_data_dir(&chain);
    assert!(output.status.success(), "{}", stderr(&output));
    let text = stdout(&output);
    assert!(text.contains("OK"));
    assert!(text.contains("height        : 3"));
    assert!(text.contains("transactions  : 4"));
    assert!(text.contains("total minted  : 12.50 EPC"));
}

#[test]
fn verify_detects_a_tampered_database() {
    let chain = Chain::create();
    chain.populate(&chain.node());
    tamper(&chain.data_dir(), 2);

    let output = verify_data_dir(&chain);
    assert!(!output.status.success());
    let text = stderr(&output);
    assert!(text.contains("VERIFICATION FAILED"), "{text}");
    assert!(text.contains("block 2"), "{text}");
    assert!(!stdout(&output).contains("OK"));
}

#[test]
fn verify_refuses_the_wrong_genesis() {
    let chain = Chain::create();
    chain.populate(&chain.node());
    let other = Chain::create();

    let output = run(&[
        "verify",
        "--genesis",
        &other.genesis_path(),
        "--data-dir",
        chain.data_dir().to_str().unwrap(),
    ]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("different chain"));
}

#[test]
fn verify_needs_a_target() {
    let chain = Chain::create();
    let output = run(&["verify", "--genesis", &chain.genesis_path()]);
    assert!(!output.status.success());
}

/// Flips the amount of the first transaction of a stored block.
fn tamper(data_dir: &Path, height: u64) {
    use redb::{Database, ReadableDatabase, TableDefinition};
    const BLOCKS: TableDefinition<u64, &[u8]> = TableDefinition::new("blocks");
    let db = Database::create(data_dir.join(DB_FILE)).unwrap();
    let bytes = {
        let txn = db.begin_read().unwrap();
        let table = txn.open_table(BLOCKS).unwrap();
        let bytes = table.get(height).unwrap().unwrap().value().to_vec();
        bytes
    };
    let mut block = Block::from_bytes(&bytes).unwrap();
    if let TxPayload::Mint { amount, .. } = &mut block.txs[0].payload {
        *amount = 9_999_999;
    } else {
        panic!("expected a mint");
    }
    let txn = db.begin_write().unwrap();
    {
        let mut table = txn.open_table(BLOCKS).unwrap();
        table.insert(height, block.to_bytes().as_slice()).unwrap();
    }
    txn.commit().unwrap();
}

#[tokio::test]
async fn verify_audits_a_running_node() {
    let chain = Chain::create();
    let node = Arc::new(chain.node());
    chain.populate(&node);
    let url = serve(node).await;

    let output = run_async(
        vec![
            "verify".into(),
            "--genesis".into(),
            chain.genesis_path(),
            "--node".into(),
            url,
        ],
        None,
    )
    .await;
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("height        : 3"));
}

#[tokio::test]
async fn verify_detects_a_running_node_serving_a_tampered_chain() {
    let chain = Chain::create();
    chain.populate(&chain.node());
    tamper(&chain.data_dir(), 2);
    // The node starts (its startup check only looks at the tip) and serves the altered block.
    let url = serve(Arc::new(chain.node())).await;

    let output = run_async(
        vec![
            "verify".into(),
            "--genesis".into(),
            chain.genesis_path(),
            "--node".into(),
            url,
        ],
        None,
    )
    .await;
    assert!(!output.status.success());
    assert!(stderr(&output).contains("VERIFICATION FAILED"));
}

// --- status / balance / authority ------------------------------------------------------

#[tokio::test]
async fn status_and_balance_read_the_node() {
    let chain = Chain::create();
    let node = Arc::new(chain.node());
    let student = chain.populate(&node);
    let url = serve(node).await;

    let output = run_async(vec!["status".into(), "--node".into(), url.clone()], None).await;
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("height        : 3"));
    assert!(stdout(&output).contains("epicoin-clitest"));

    let output = run_async(
        vec![
            "balance".into(),
            "--node".into(),
            url,
            student.address().to_string(),
        ],
        None,
    )
    .await;
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stdout(&output).contains("balance : 9.50 EPC"));
}

#[tokio::test]
async fn authority_commands_change_the_chain() {
    let chain = Chain::create();
    let node = Arc::new(chain.node());
    tokio::spawn(producer::run(node.clone(), Duration::from_millis(20), None));
    let url = serve(node.clone()).await;
    let staff = Keypair::from_secret_bytes(&[42; 32]).address();
    let admin_key = chain.keys["EPICOIN_ADMIN_KEY"].clone();
    let args = |verb: &str, extra: &[&str]| -> Vec<String> {
        let mut args = vec![
            "authority".to_owned(),
            verb.to_owned(),
            "--node".to_owned(),
            url.clone(),
        ];
        args.extend(extra.iter().map(|s| (*s).to_owned()));
        args
    };

    // Without the admin key: refused before anything is sent.
    let output = run_async(args("grant", &[&staff.to_string(), "--quota", "500"]), None).await;
    assert!(!output.status.success());
    assert!(stderr(&output).contains("EPICOIN_ADMIN_KEY"));

    // With a key that is not the admin's: refused.
    let wrong = Keypair::generate().secret_hex().to_string();
    let output = run_async(
        args("grant", &[&staff.to_string(), "--quota", "500"]),
        Some(wrong),
    )
    .await;
    assert!(!output.status.success());
    assert!(node.authorities().is_empty());

    // Grant.
    let output = run_async(
        args("grant", &[&staff.to_string(), "--quota", "500"]),
        Some(admin_key.clone()),
    )
    .await;
    assert!(output.status.success(), "{}", stderr(&output));
    let authority = node.account(&staff).authority.unwrap();
    assert!(authority.active);
    assert_eq!(authority.quota, 500 * UNITS_PER_EPC);

    let output = run_async(args("list", &[]), None).await;
    assert!(stdout(&output).contains(&staff.to_string()));
    assert!(stdout(&output).contains("500.00 EPC"));

    // Revoke.
    let output = run_async(args("revoke", &[&staff.to_string()]), Some(admin_key)).await;
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(!node.account(&staff).authority.unwrap().active);
}
