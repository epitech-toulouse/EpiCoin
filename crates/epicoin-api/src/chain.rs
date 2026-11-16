//! Sending transactions to the chain, reliably.
//!
//! Every transaction the API signs is written to `chain_txs` **before** it is submitted to
//! the node, with its exact signed bytes. From then on its fate is settled by
//! reconciliation: the same bytes are resubmitted until the node reports them included or
//! refuses them for good. Since the bytes (and therefore the nonce) never change, a retry —
//! after a timeout, a crash or a node restart — can never apply the transaction twice.
//!
//! Nonces are allocated under a per-signer lock held from signature to submission, so a
//! signer's transactions reach the node in nonce order. The lock is in-process: the API is
//! meant to run as a single instance.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use epicoin_core::{Address, Amount, Hash, Keypair, Transaction, TxError, TxKind, TxPayload};
use epicoin_node::views::{StatusView, TxStatus};
use epicoin_node::{ClientError, NodeClient};
use sqlx::PgPool;
use time::OffsetDateTime;
use tokio::sync::OnceCell;
use uuid::Uuid;

/// Business context stored next to a transaction, for histories and statistics.
#[derive(Debug, Clone, Default)]
pub struct TxMeta {
    /// The user who triggered the transaction.
    pub actor_user_id: Option<Uuid>,
    /// The user whose balance changes (mint recipient, buyer…).
    pub subject_user_id: Option<Uuid>,
    pub amount: Option<Amount>,
    pub reason_code: Option<String>,
    pub batch_id: Option<Uuid>,
}

/// A `chain_txs` row.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChainTx {
    pub hash: String,
    pub kind: String,
    pub signer_address: String,
    pub nonce: i64,
    pub raw: Vec<u8>,
    pub status: String,
    /// When `failed`: the state-machine error, as JSON (`{"code": "bad_nonce", …}`).
    pub error: Option<String>,
    pub block_height: Option<i64>,
    pub block_time: Option<OffsetDateTime>,
    pub actor_user_id: Option<Uuid>,
    pub subject_user_id: Option<Uuid>,
    pub amount: Option<i64>,
    pub reason_code: Option<String>,
    pub batch_id: Option<Uuid>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

pub const STATUS_PENDING: &str = "pending";
pub const STATUS_INCLUDED: &str = "included";
pub const STATUS_FAILED: &str = "failed";

impl ChainTx {
    pub fn is_included(&self) -> bool {
        self.status == STATUS_INCLUDED
    }

    pub fn is_failed(&self) -> bool {
        self.status == STATUS_FAILED
    }

    /// Why the chain refused the transaction, if it did.
    pub fn failure(&self) -> Option<TxError> {
        if !self.is_failed() {
            return None;
        }
        self.error
            .as_deref()
            .and_then(|json| serde_json::from_str(json).ok())
            // A failed row always carries its reason; this is only a safe fallback.
            .or(Some(TxError::BadSignature))
    }

    /// The stable code of the failure (`quota_exceeded`, `insufficient_balance`…).
    pub fn failure_code(&self) -> Option<String> {
        self.failure().as_ref().map(error_code)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    /// The state machine refused the transaction. Nothing was applied.
    #[error("transaction rejected: {0}")]
    Rejected(TxError),
    /// The node cannot be reached. Transactions already recorded will be retried.
    #[error("node unavailable: {0}")]
    Unavailable(String),
    #[error("the configured keys do not match the chain: {0}")]
    Misconfigured(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

impl From<ClientError> for ChainError {
    fn from(error: ClientError) -> Self {
        match error {
            ClientError::Rejected(error) => ChainError::Rejected(error),
            other => ChainError::Unavailable(other.to_string()),
        }
    }
}

/// What the API needs to know about the chain it talks to.
#[derive(Debug, Clone)]
pub struct ChainInfo {
    pub chain_id: String,
    pub admin: Address,
    pub treasury: Address,
}

pub struct ChainService {
    db: PgPool,
    node: NodeClient,
    info: OnceCell<ChainInfo>,
    expected_admin: Address,
    expected_treasury: Address,
    locks: Mutex<HashMap<Address, Arc<tokio::sync::Mutex<()>>>>,
}

fn kind_name(kind: TxKind) -> &'static str {
    match kind {
        TxKind::Mint => "mint",
        TxKind::Purchase => "purchase",
        TxKind::Refund => "refund",
        TxKind::AuthorityUpdate => "authority_update",
        TxKind::Transfer => "transfer",
    }
}

/// The stable code of a state-machine error (`bad_nonce`, `quota_exceeded`…).
pub fn error_code(error: &TxError) -> String {
    serde_json::to_value(error)
        .ok()
        .and_then(|value| {
            value
                .get("code")
                .and_then(|c| c.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_else(|| "rejected".to_owned())
}

fn block_time(timestamp_ms: u64) -> Option<OffsetDateTime> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(timestamp_ms) * 1_000_000).ok()
}

impl ChainService {
    /// `admin` and `treasury` are the addresses of the keys this API holds; they are checked
    /// against the chain on first contact.
    pub fn new(db: PgPool, node: NodeClient, admin: Address, treasury: Address) -> Self {
        Self {
            db,
            node,
            info: OnceCell::new(),
            expected_admin: admin,
            expected_treasury: treasury,
            locks: Mutex::new(HashMap::new()),
        }
    }

    pub fn node(&self) -> &NodeClient {
        &self.node
    }

    /// Chain parameters, fetched from the node once and then cached. Fails if the node is
    /// unreachable or if the API's admin/treasury keys are not this chain's.
    pub async fn info(&self) -> Result<&ChainInfo, ChainError> {
        self.info
            .get_or_try_init(|| async {
                let status: StatusView = self.node.status().await?;
                if status.admin != self.expected_admin {
                    return Err(ChainError::Misconfigured(
                        "EPICOIN_ADMIN_KEY is not the admin key of the chain".into(),
                    ));
                }
                if status.treasury != self.expected_treasury {
                    return Err(ChainError::Misconfigured(
                        "EPICOIN_TREASURY_KEY is not the treasury key of the chain".into(),
                    ));
                }
                Ok(ChainInfo {
                    chain_id: status.chain_id,
                    admin: status.admin,
                    treasury: status.treasury,
                })
            })
            .await
    }

    fn signer_lock(&self, signer: &Address) -> Arc<tokio::sync::Mutex<()>> {
        self.locks
            .lock()
            .expect("signer lock table poisoned")
            .entry(*signer)
            .or_default()
            .clone()
    }

    /// Signs, records and submits one transaction, then waits up to `wait` for its
    /// inclusion. A transaction refused by the chain is an error.
    ///
    /// The returned row is `included`, or still `pending` if the wait elapsed (the
    /// reconciler will settle it).
    pub async fn send(
        &self,
        signer: &Keypair,
        payload: TxPayload,
        meta: TxMeta,
        wait: Duration,
    ) -> Result<ChainTx, ChainError> {
        let mut rows = self.send_batch(signer, vec![(payload, meta)], wait).await?;
        let row = rows.pop().expect("one transaction in, one row out");
        match row.failure() {
            Some(error) => Err(ChainError::Rejected(error)),
            None => Ok(row),
        }
    }

    /// Like [`Self::send`] for several transactions of the same signer, with consecutive
    /// nonces. Returns one row per transaction that was recorded, in order.
    ///
    /// If the chain refuses one of them, that row comes back `failed` and the following
    /// ones are not sent (their nonces would be wrong); the caller sees fewer rows than it
    /// asked for. If the node becomes unreachable midway, what was recorded stays `pending`
    /// and is retried by the reconciler.
    pub async fn send_batch(
        &self,
        signer: &Keypair,
        items: Vec<(TxPayload, TxMeta)>,
        wait: Duration,
    ) -> Result<Vec<ChainTx>, ChainError> {
        if items.is_empty() {
            return Ok(Vec::new());
        }
        let chain_id = self.info().await?.chain_id.clone();
        let address = signer.address();
        let lock = self.signer_lock(&address);
        let guard = lock.lock().await;

        // Next nonce: what the node expects, unless we hold transactions it has not seen
        // yet (recorded but not delivered — the reconciler will deliver them first).
        let on_node = self.node.account(&address).await?.pending_nonce;
        let ours: Option<i64> = sqlx::query_scalar(
            "SELECT max(nonce) FROM chain_txs WHERE signer_address = $1 AND status = 'pending'",
        )
        .bind(address.to_string())
        .fetch_one(&self.db)
        .await?;
        let first_nonce = on_node.max(ours.map_or(0, |n| n as u64 + 1));

        let mut hashes: Vec<Hash> = Vec::with_capacity(items.len());
        let mut node_down = None;
        for (nonce, (payload, meta)) in (first_nonce..).zip(items) {
            let tx = Transaction::sign(&chain_id, signer, nonce, payload);
            let hash = tx.hash();
            self.record(&tx, &meta).await?;
            hashes.push(hash);

            match self.node.submit(&tx, None).await {
                Ok(_) => {}
                Err(ClientError::Rejected(error)) => {
                    self.mark_failed(&hash, &error).await?;
                    break;
                }
                Err(other) => {
                    // Unknown outcome: keep the row pending, stop here.
                    node_down = Some(other.to_string());
                    break;
                }
            }
        }
        drop(guard);

        if let Some(reason) = node_down {
            tracing::warn!(%reason, "node unreachable while submitting; will retry");
        } else {
            self.settle(&hashes, wait).await?;
        }
        self.rows(&hashes).await
    }

    /// Waits up to `wait` for `hashes` to be included, updating their rows.
    async fn settle(&self, hashes: &[Hash], wait: Duration) -> Result<(), ChainError> {
        let deadline = tokio::time::Instant::now() + wait;
        let mut remaining: Vec<Hash> = hashes.to_vec();
        loop {
            let mut still_pending = Vec::new();
            for hash in &remaining {
                match self.node.tx(hash).await {
                    Ok(Some(view)) => match view.status {
                        TxStatus::Included {
                            height, timestamp, ..
                        } => self.mark_included(hash, height, timestamp).await?,
                        TxStatus::Rejected { error } => self.mark_failed(hash, &error).await?,
                        TxStatus::Pending | TxStatus::Unknown => still_pending.push(*hash),
                    },
                    // Unknown to the node or node unreachable: leave it to the reconciler.
                    Ok(None) | Err(_) => still_pending.push(*hash),
                }
            }
            remaining = still_pending;
            let now = tokio::time::Instant::now();
            if remaining.is_empty() || now >= deadline {
                return Ok(());
            }
            tokio::time::sleep((deadline - now).min(Duration::from_millis(250))).await;
        }
    }

    async fn record(&self, tx: &Transaction, meta: &TxMeta) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO chain_txs \
             (hash, kind, signer_address, nonce, raw, status, actor_user_id, subject_user_id, \
              amount, reason_code, batch_id) \
             VALUES ($1, $2, $3, $4, $5, 'pending', $6, $7, $8, $9, $10)",
        )
        .bind(tx.hash().to_string())
        .bind(kind_name(tx.kind()))
        .bind(tx.from.to_string())
        .bind(tx.nonce as i64)
        .bind(tx.to_bytes())
        .bind(meta.actor_user_id)
        .bind(meta.subject_user_id)
        .bind(meta.amount.map(|a| a as i64))
        .bind(meta.reason_code.as_deref())
        .bind(meta.batch_id)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    async fn mark_included(
        &self,
        hash: &Hash,
        height: u64,
        timestamp_ms: u64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE chain_txs SET status = 'included', block_height = $2, block_time = $3, \
             error = NULL, updated_at = now() WHERE hash = $1 AND status = 'pending'",
        )
        .bind(hash.to_string())
        .bind(height as i64)
        .bind(block_time(timestamp_ms))
        .execute(&self.db)
        .await?;
        Ok(())
    }

    async fn mark_failed(&self, hash: &Hash, error: &TxError) -> Result<(), sqlx::Error> {
        let json = serde_json::to_string(error).unwrap_or_else(|_| "{}".to_owned());
        sqlx::query(
            "UPDATE chain_txs SET status = 'failed', error = $2, updated_at = now() \
             WHERE hash = $1 AND status = 'pending'",
        )
        .bind(hash.to_string())
        .bind(json)
        .execute(&self.db)
        .await?;
        Ok(())
    }

    async fn rows(&self, hashes: &[Hash]) -> Result<Vec<ChainTx>, ChainError> {
        let mut rows = Vec::with_capacity(hashes.len());
        for hash in hashes {
            let row = sqlx::query_as::<_, ChainTx>("SELECT * FROM chain_txs WHERE hash = $1")
                .bind(hash.to_string())
                .fetch_one(&self.db)
                .await?;
            rows.push(row);
        }
        Ok(rows)
    }

    /// Settles the transactions that are still `pending` and older than `min_age`: asks the
    /// node about each, resubmits those it does not know, and records the final outcome.
    /// Returns how many rows reached a final status.
    ///
    /// Runs periodically and at startup; this is what makes submission crash-safe.
    pub async fn reconcile(&self, min_age: Duration) -> Result<usize, ChainError> {
        let pending = sqlx::query_as::<_, ChainTx>(
            "SELECT * FROM chain_txs WHERE status = 'pending' \
             AND created_at <= now() - make_interval(secs => $1) \
             ORDER BY signer_address, nonce",
        )
        .bind(min_age.as_secs_f64())
        .fetch_all(&self.db)
        .await?;

        let mut settled = 0;
        for row in pending {
            let Ok(tx) = Transaction::from_bytes(&row.raw) else {
                tracing::error!(hash = %row.hash, "stored transaction cannot be decoded");
                continue;
            };
            let hash = tx.hash();
            let lock = self.signer_lock(&tx.from);
            let _guard = lock.lock().await;

            let status = match self.node.tx(&hash).await? {
                Some(view) => view.status,
                None => TxStatus::Unknown,
            };
            let status = match status {
                // The node has never seen it (or forgot it in a restart): deliver it again.
                TxStatus::Unknown => match self.node.submit(&tx, None).await {
                    Ok(view) => view.status,
                    Err(ClientError::Rejected(error)) => TxStatus::Rejected { error },
                    Err(other) => return Err(other.into()),
                },
                known => known,
            };
            match status {
                TxStatus::Included {
                    height, timestamp, ..
                } => {
                    self.mark_included(&hash, height, timestamp).await?;
                    settled += 1;
                }
                TxStatus::Rejected { error } => {
                    tracing::warn!(hash = %row.hash, %error, "transaction refused by the chain");
                    self.mark_failed(&hash, &error).await?;
                    settled += 1;
                }
                TxStatus::Pending | TxStatus::Unknown => {}
            }
        }
        Ok(settled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_match_the_state_machine() {
        assert_eq!(error_code(&TxError::NotAuthority), "not_authority");
        assert_eq!(
            error_code(&TxError::BadNonce {
                expected: 1,
                got: 2
            }),
            "bad_nonce"
        );
        assert_eq!(
            error_code(&TxError::InsufficientBalance {
                balance: 0,
                needed: 1
            }),
            "insufficient_balance"
        );
    }

    #[test]
    fn block_time_converts_milliseconds() {
        let time = block_time(1_700_000_000_123).unwrap();
        assert_eq!(time.unix_timestamp(), 1_700_000_000);
        assert_eq!(time.millisecond(), 123);
    }
}
