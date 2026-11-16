//! JSON shapes of the node's HTTP API, shared by the server and the client.
//!
//! These are read-only views. Nothing here is ever hashed or signed.

use epicoin_core::{Address, Amount, Block, Hash, PublicKey, Signature, Transaction, TxError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeMode {
    /// Produces blocks.
    Validator,
    /// Read-only: follows another node and re-verifies every block.
    Replica,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusView {
    pub chain_id: String,
    pub genesis_hash: Hash,
    pub mode: NodeMode,
    pub height: u64,
    pub tip_hash: Hash,
    pub tip_timestamp: u64,
    pub state_root: Hash,
    pub mempool_size: usize,
    pub total_minted: Amount,
    pub total_burned: Amount,
    pub circulating_supply: Amount,
    pub treasury: Address,
    pub admin: Address,
    pub quota_period_ms: u64,
}

/// Where a transaction stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TxStatus {
    /// Accepted in the mempool, waiting for the next block.
    Pending,
    /// In a block: final.
    Included {
        height: u64,
        index: u32,
        timestamp: u64,
    },
    /// Was accepted in the mempool but dropped before inclusion.
    Rejected { error: TxError },
    /// Never seen by this node (or forgotten after a restart).
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmitView {
    pub hash: Hash,
    #[serde(flatten)]
    pub status: TxStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TxView {
    pub hash: Hash,
    #[serde(flatten)]
    pub status: TxStatus,
    /// The transaction itself; absent for a rejected transaction, which is not kept.
    pub tx: Option<Transaction>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockSummary {
    pub height: u64,
    pub hash: Hash,
    pub prev_hash: Hash,
    pub timestamp: u64,
    pub tx_count: usize,
    pub validator: PublicKey,
}

impl From<&Block> for BlockSummary {
    fn from(block: &Block) -> Self {
        Self {
            height: block.header.height,
            hash: block.hash(),
            prev_hash: block.header.prev_hash,
            timestamp: block.header.timestamp,
            tx_count: block.txs.len(),
            validator: block.header.validator,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockView {
    #[serde(flatten)]
    pub summary: BlockSummary,
    pub tx_root: Hash,
    pub state_root: Hash,
    pub signature: Signature,
    pub txs: Vec<TxView>,
}

impl From<&Block> for BlockView {
    fn from(block: &Block) -> Self {
        let txs = block
            .txs
            .iter()
            .enumerate()
            .map(|(index, tx)| TxView {
                hash: tx.hash(),
                status: TxStatus::Included {
                    height: block.header.height,
                    index: index as u32,
                    timestamp: block.header.timestamp,
                },
                tx: Some(tx.clone()),
            })
            .collect();
        Self {
            summary: block.into(),
            tx_root: block.header.tx_root,
            state_root: block.header.state_root,
            signature: block.signature,
            txs,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityView {
    pub address: Address,
    pub active: bool,
    /// Maximum mint per quota period.
    pub quota: Amount,
    /// Minted in the current period.
    pub minted: Amount,
    /// What can still be minted in the current period (0 if revoked).
    pub remaining: Amount,
    /// When the current quota period ends (milliseconds since the Unix epoch).
    pub period_ends_at: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountView {
    pub address: Address,
    pub balance: Amount,
    /// Nonce on chain.
    pub nonce: u64,
    /// Nonce to use for the next transaction, counting those waiting in the mempool.
    pub pending_nonce: u64,
    pub authority: Option<AuthorityView>,
}

/// Body of every error response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorView {
    /// Stable machine-readable code, e.g. `invalid_transaction`, `not_found`, `read_only`.
    pub error: String,
    pub message: String,
    /// The state-machine reason, when a transaction is refused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<TxError>,
}
