//! Shared fixtures for the node integration tests.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use epicoin_core::{
    Address, Amount, AuthorityAction, Block, Genesis, Hash, Keypair, ReasonCode, Transaction,
    TxPayload, DEFAULT_QUOTA_PERIOD_MS,
};
use epicoin_node::store::DB_FILE;
use epicoin_node::{now_ms, Node, NodeClient, NodeOptions};
use redb::{Database, ReadableDatabase, TableDefinition};

pub const CHAIN_ID: &str = "epicoin-test";

/// A small EpiCoin "network": the keys of every role and the genesis binding them.
pub struct Net {
    pub validator: Keypair,
    pub admin: Keypair,
    pub treasury: Keypair,
    pub staff: Vec<Keypair>,
    pub students: Vec<Keypair>,
    pub genesis: Genesis,
    nonces: HashMap<Address, u64>,
}

fn key(seed: u16) -> Keypair {
    let mut bytes = [0u8; 32];
    bytes[..2].copy_from_slice(&seed.to_le_bytes());
    bytes[31] = 0xEC;
    Keypair::from_secret_bytes(&bytes)
}

impl Net {
    pub fn new(staff: u16, students: u16) -> Self {
        let validator = key(1);
        let admin = key(2);
        let treasury = key(3);
        let genesis = Genesis {
            chain_id: CHAIN_ID.into(),
            timestamp: now_ms() - 1_000,
            validators: vec![validator.public_key()],
            admin: admin.address(),
            treasury: treasury.address(),
            quota_period_ms: DEFAULT_QUOTA_PERIOD_MS,
        };
        Self {
            validator,
            admin,
            treasury,
            staff: (0..staff).map(|i| key(100 + i)).collect(),
            students: (0..students).map(|i| key(1_000 + i)).collect(),
            genesis,
            nonces: HashMap::new(),
        }
    }

    /// Opens a block-producing node on `dir`.
    pub fn validator_node(&self, dir: &Path) -> Node {
        Node::open(
            dir,
            &self.genesis,
            Some(self.validator.clone()),
            NodeOptions::default(),
        )
        .expect("node opens")
    }

    /// Opens a read-only replica on `dir`.
    pub fn replica_node(&self, dir: &Path) -> Node {
        Node::open(dir, &self.genesis, None, NodeOptions::default()).expect("replica opens")
    }

    /// Signs `payload` with the signer's next nonce, tracked locally.
    pub fn sign(&mut self, signer: &Keypair, payload: TxPayload) -> Transaction {
        let nonce = self.nonces.entry(signer.address()).or_insert(0);
        let tx = Transaction::sign(CHAIN_ID, signer, *nonce, payload);
        *nonce += 1;
        tx
    }

    pub fn grant(&mut self, staff: usize, quota: Amount) -> Transaction {
        let admin = self.admin.clone();
        let authority = self.staff[staff].address();
        self.sign(
            &admin,
            TxPayload::AuthorityUpdate {
                authority,
                action: AuthorityAction::Grant { quota },
            },
        )
    }

    pub fn mint(&mut self, staff: usize, student: usize, amount: Amount) -> Transaction {
        let signer = self.staff[staff].clone();
        let to = self.students[student].address();
        self.sign(
            &signer,
            TxPayload::Mint {
                to,
                amount,
                reason: ReasonCode::ALL[(staff + student) % ReasonCode::ALL.len()],
            },
        )
    }

    pub fn purchase(&mut self, student: usize, amount: Amount, order: u64) -> Transaction {
        let signer = self.students[student].clone();
        let mut order_ref = [0u8; 32];
        order_ref[..8].copy_from_slice(&order.to_le_bytes());
        self.sign(
            &signer,
            TxPayload::Purchase {
                amount,
                order_ref: Hash(order_ref),
            },
        )
    }

    pub fn refund(&mut self, purchase: Hash) -> Transaction {
        let treasury = self.treasury.clone();
        self.sign(&treasury, TxPayload::Refund { purchase })
    }
}

/// Submits `txs` and produces one block containing them.
pub fn include(node: &Node, txs: impl IntoIterator<Item = Transaction>) -> Block {
    for tx in txs {
        node.submit(tx).expect("transaction accepted");
    }
    node.produce_block(false)
        .expect("block production works")
        .expect("a block is produced")
}

/// Serves `node` over HTTP on an ephemeral local port and returns a client for it.
pub async fn serve(node: Arc<Node>) -> NodeClient {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let address = listener.local_addr().expect("local address");
    tokio::spawn(async move {
        axum::serve(listener, epicoin_node::http::router(node))
            .await
            .expect("server runs");
    });
    NodeClient::new(&format!("http://{address}")).expect("client")
}

const BLOCKS: TableDefinition<u64, &[u8]> = TableDefinition::new("blocks");
const META: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");

/// Rewrites a stored block behind the node's back, as an attacker with disk access would.
pub fn tamper_block(dir: &Path, height: u64, edit: impl FnOnce(&mut Block)) {
    let db = Database::create(dir.join(DB_FILE)).expect("open database");
    let bytes = {
        let txn = db.begin_read().expect("read");
        let table = txn.open_table(BLOCKS).expect("blocks table");
        let bytes = table
            .get(height)
            .expect("get")
            .expect("block exists")
            .value()
            .to_vec();
        bytes
    };
    let mut block = Block::from_bytes(&bytes).expect("decode");
    edit(&mut block);
    let txn = db.begin_write().expect("write");
    {
        let mut table = txn.open_table(BLOCKS).expect("blocks table");
        table
            .insert(height, block.to_bytes().as_slice())
            .expect("insert");
    }
    txn.commit().expect("commit");
}

/// Deletes a stored block.
pub fn delete_block(dir: &Path, height: u64) {
    let db = Database::create(dir.join(DB_FILE)).expect("open database");
    let txn = db.begin_write().expect("write");
    {
        let mut table = txn.open_table(BLOCKS).expect("blocks table");
        table.remove(height).expect("remove");
    }
    txn.commit().expect("commit");
}

/// Reads the raw state snapshot.
pub fn read_snapshot(dir: &Path) -> Vec<u8> {
    let db = Database::create(dir.join(DB_FILE)).expect("open database");
    let txn = db.begin_read().expect("read");
    let table = txn.open_table(META).expect("meta table");
    let bytes = table
        .get("chain")
        .expect("get")
        .expect("snapshot exists")
        .value()
        .to_vec();
    bytes
}

/// Overwrites the raw state snapshot.
pub fn write_snapshot(dir: &Path, bytes: &[u8]) {
    let db = Database::create(dir.join(DB_FILE)).expect("open database");
    let txn = db.begin_write().expect("write");
    {
        let mut table = txn.open_table(META).expect("meta table");
        table.insert("chain", bytes).expect("insert");
    }
    txn.commit().expect("commit");
}
