//! The node: one chain head, one mempool, one store, behind a single lock.
//!
//! Every mutation (accepting a transaction, producing or applying a block) takes the lock,
//! so there is a single, total order of events. Critical sections are short and never span
//! an `.await`; at this scale that is far simpler than anything finer-grained.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use epicoin_core::{
    Address, Block, BlockError, Chain, Genesis, Hash, Keypair, Transaction, TxError,
};
use tokio::sync::watch;
use tokio::time::Instant;

use crate::mempool::{Mempool, MempoolError};
use crate::store::{Store, StoreError, TxLocation, DB_FILE};
use crate::views::{AccountView, AuthorityView, NodeMode, StatusView, TxStatus, TxView};

/// How many dropped transactions the node remembers the reason for.
const DROPPED_MEMORY: usize = 4096;

#[derive(Debug, Clone)]
pub struct NodeOptions {
    pub max_block_txs: usize,
    pub mempool_capacity: usize,
}

impl Default for NodeOptions {
    fn default() -> Self {
        Self {
            max_block_txs: 1_000,
            mempool_capacity: 20_000,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NodeError {
    #[error("this node is a read-only replica")]
    ReadOnly,
    #[error("the validator key is not a validator of this chain")]
    NotAValidator,
    #[error("mempool is full")]
    MempoolFull,
    #[error("invalid transaction: {0}")]
    InvalidTx(#[from] TxError),
    #[error("invalid block: {0}")]
    InvalidBlock(#[from] BlockError),
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("cannot create the data directory: {0}")]
    DataDir(#[from] std::io::Error),
}

struct Inner {
    chain: Chain,
    mempool: Mempool,
    store: Store,
    dropped: Dropped,
}

/// Bounded memory of why transactions were dropped from the mempool.
#[derive(Default)]
struct Dropped {
    order: VecDeque<Hash>,
    reasons: HashMap<Hash, TxError>,
}

impl Dropped {
    fn record(&mut self, entries: impl IntoIterator<Item = (Hash, TxError)>) {
        for (hash, error) in entries {
            if self.reasons.insert(hash, error).is_none() {
                self.order.push_back(hash);
            }
            if self.order.len() > DROPPED_MEMORY {
                if let Some(oldest) = self.order.pop_front() {
                    self.reasons.remove(&oldest);
                }
            }
        }
    }
}

pub struct Node {
    inner: Mutex<Inner>,
    validator: Option<Keypair>,
    options: NodeOptions,
    /// Bumped whenever a block is committed or transactions are dropped.
    events: watch::Sender<u64>,
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

impl Node {
    /// Opens the node's database in `data_dir`. With a `validator` key the node produces
    /// blocks; without one it is a read-only replica.
    pub fn open(
        data_dir: &Path,
        genesis: &Genesis,
        validator: Option<Keypair>,
        options: NodeOptions,
    ) -> Result<Self, NodeError> {
        if let Some(key) = &validator {
            if !genesis.validators.contains(&key.public_key()) {
                return Err(NodeError::NotAValidator);
            }
        }
        std::fs::create_dir_all(data_dir)?;
        let (store, chain) = Store::open(&data_dir.join(DB_FILE), genesis)?;
        let mempool = Mempool::new(chain.state().clone(), options.mempool_capacity);
        Ok(Self {
            inner: Mutex::new(Inner {
                chain,
                mempool,
                store,
                dropped: Dropped::default(),
            }),
            validator,
            options,
            events: watch::channel(0).0,
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // A panic while holding the lock could leave the chain head and the store out of
        // step; refusing to continue is the only safe reaction.
        self.inner.lock().expect("node state lock poisoned")
    }

    fn notify(&self) {
        self.events.send_modify(|generation| *generation += 1);
    }

    pub fn mode(&self) -> NodeMode {
        if self.validator.is_some() {
            NodeMode::Validator
        } else {
            NodeMode::Replica
        }
    }

    // --- Writes ------------------------------------------------------------------------

    /// Accepts a transaction into the mempool.
    ///
    /// Idempotent: submitting a transaction that is already queued or already in a block
    /// returns its current status instead of an error, so clients can retry safely.
    pub fn submit(&self, tx: Transaction) -> Result<(Hash, TxStatus), NodeError> {
        if self.validator.is_none() {
            return Err(NodeError::ReadOnly);
        }
        let hash = tx.hash();
        let mut inner = self.lock();
        if let Some(location) = inner.store.tx_location(&hash)? {
            return Ok((hash, included(&inner.store, location)?));
        }
        let now = now_ms().max(inner.chain.tip().timestamp + 1);
        match inner.mempool.add(tx, now) {
            Ok(_) => Ok((hash, TxStatus::Pending)),
            Err(MempoolError::Full) => Err(NodeError::MempoolFull),
            Err(MempoolError::Invalid(error)) => Err(NodeError::InvalidTx(error)),
        }
    }

    /// Produces a block from the mempool. Returns `None` when there is nothing to include,
    /// unless `allow_empty` asks for a heartbeat block.
    pub fn produce_block(&self, allow_empty: bool) -> Result<Option<Block>, NodeError> {
        let validator = self.validator.as_ref().ok_or(NodeError::ReadOnly)?;
        let mut inner = self.lock();
        let timestamp = now_ms().max(inner.chain.tip().timestamp + 1);

        let (txs, rejected) =
            inner
                .mempool
                .select(inner.chain.state(), timestamp, self.options.max_block_txs);
        let mut remove: HashSet<Hash> = rejected.iter().map(|(hash, _)| *hash).collect();

        if txs.is_empty() && !allow_empty {
            if !rejected.is_empty() {
                let state = inner.chain.state().clone();
                let dropped = inner.mempool.rebase(&state, &remove, timestamp);
                inner.dropped.record(rejected.into_iter().chain(dropped));
                drop(inner);
                self.notify();
            }
            return Ok(None);
        }

        let block = inner.chain.build_block(txs, timestamp, validator)?;
        // Same verification path as a replica, then disk before memory: if the write
        // fails, the in-memory chain has not moved.
        let mut next = inner.chain.clone();
        next.apply_block(&block)?;
        inner.store.append(&block, &next)?;
        inner.chain = next;

        remove.extend(block.txs.iter().map(Transaction::hash));
        let state = inner.chain.state().clone();
        let dropped = inner.mempool.rebase(&state, &remove, timestamp);
        inner.dropped.record(rejected.into_iter().chain(dropped));
        drop(inner);
        self.notify();
        Ok(Some(block))
    }

    /// Verifies and appends a block produced elsewhere (replica mode).
    pub fn apply_block(&self, block: &Block) -> Result<(), NodeError> {
        let mut inner = self.lock();
        let mut next = inner.chain.clone();
        next.apply_block(block)?;
        inner.store.append(block, &next)?;
        inner.chain = next;

        let remove: HashSet<Hash> = block.txs.iter().map(Transaction::hash).collect();
        let state = inner.chain.state().clone();
        let dropped = inner
            .mempool
            .rebase(&state, &remove, block.header.timestamp);
        inner.dropped.record(dropped);
        drop(inner);
        self.notify();
        Ok(())
    }

    // --- Reads -------------------------------------------------------------------------

    pub fn height(&self) -> u64 {
        self.lock().chain.tip().height
    }

    pub fn genesis(&self) -> Genesis {
        self.lock().chain.genesis().clone()
    }

    /// A copy of the current chain head (state + tip).
    pub fn chain(&self) -> Chain {
        self.lock().chain.clone()
    }

    pub fn status(&self) -> StatusView {
        let inner = self.lock();
        let state = inner.chain.state();
        let genesis = state.genesis();
        let tip = inner.chain.tip();
        StatusView {
            chain_id: genesis.chain_id.clone(),
            genesis_hash: genesis.hash(),
            mode: self.mode(),
            height: tip.height,
            tip_hash: tip.hash,
            tip_timestamp: tip.timestamp,
            state_root: state.root(),
            mempool_size: inner.mempool.len(),
            total_minted: state.total_minted(),
            total_burned: state.total_burned(),
            circulating_supply: state.circulating_supply(),
            treasury: genesis.treasury,
            admin: genesis.admin,
            quota_period_ms: genesis.quota_period_ms,
        }
    }

    pub fn tx_status(&self, hash: &Hash) -> Result<TxStatus, NodeError> {
        let inner = self.lock();
        tx_status(&inner, hash)
    }

    /// The transaction with its status, or `None` if this node has never seen it.
    pub fn tx(&self, hash: &Hash) -> Result<Option<TxView>, NodeError> {
        let inner = self.lock();
        if let Some(location) = inner.store.tx_location(hash)? {
            return Ok(Some(tx_at(&inner.store, location)?));
        }
        let status = tx_status(&inner, hash)?;
        if status == TxStatus::Unknown {
            return Ok(None);
        }
        Ok(Some(TxView {
            hash: *hash,
            status,
            tx: inner.mempool.get(hash).cloned(),
        }))
    }

    /// Waits until `hash` is no longer pending, or until `timeout` elapses, and returns its
    /// status at that point.
    pub async fn wait_for_tx(&self, hash: &Hash, timeout: Duration) -> Result<TxStatus, NodeError> {
        // Subscribe before the first check so that no event can be missed in between.
        let mut events = self.events.subscribe();
        let deadline = Instant::now() + timeout;
        loop {
            let status = self.tx_status(hash)?;
            if status != TxStatus::Pending {
                return Ok(status);
            }
            match tokio::time::timeout_at(deadline, events.changed()).await {
                Ok(Ok(())) => continue,
                _ => return self.tx_status(hash),
            }
        }
    }

    pub fn account(&self, address: &Address) -> AccountView {
        let inner = self.lock();
        let account = inner.chain.state().account(address);
        AccountView {
            address: *address,
            balance: account.balance,
            nonce: account.nonce,
            pending_nonce: inner.mempool.pending_nonce(address),
            authority: authority_view(&inner.chain, address),
        }
    }

    /// Every account with a balance or a nonce, richest first.
    pub fn accounts(&self) -> Vec<AccountView> {
        let inner = self.lock();
        let mut accounts: Vec<AccountView> = inner
            .chain
            .state()
            .accounts()
            .iter()
            .map(|(address, account)| AccountView {
                address: *address,
                balance: account.balance,
                nonce: account.nonce,
                pending_nonce: inner.mempool.pending_nonce(address),
                authority: authority_view(&inner.chain, address),
            })
            .collect();
        accounts.sort_by(|a, b| b.balance.cmp(&a.balance).then(a.address.cmp(&b.address)));
        accounts
    }

    pub fn authorities(&self) -> Vec<AuthorityView> {
        let inner = self.lock();
        inner
            .chain
            .state()
            .authorities()
            .keys()
            .filter_map(|address| authority_view(&inner.chain, address))
            .collect()
    }

    pub fn block(&self, height: u64) -> Result<Option<Block>, NodeError> {
        Ok(self.lock().store.block(height)?)
    }

    pub fn block_by_hash(&self, hash: &Hash) -> Result<Option<Block>, NodeError> {
        let inner = self.lock();
        match inner.store.height_of(hash)? {
            Some(height) => Ok(inner.store.block(height)?),
            None => Ok(None),
        }
    }

    /// Blocks in ascending order from `from`, for replicas.
    pub fn blocks(&self, from: u64, limit: usize) -> Result<Vec<Block>, NodeError> {
        Ok(self.lock().store.blocks(from, limit)?)
    }

    /// Blocks in descending order from `from` (or from the tip), for the explorer.
    pub fn blocks_desc(&self, from: Option<u64>, limit: usize) -> Result<Vec<Block>, NodeError> {
        let inner = self.lock();
        let from = from.unwrap_or(inner.chain.tip().height);
        Ok(inner.store.blocks_desc(from, limit)?)
    }

    /// The most recent transactions of the chain, newest first.
    pub fn recent_txs(&self, limit: usize) -> Result<Vec<TxView>, NodeError> {
        let inner = self.lock();
        let mut out = Vec::new();
        let mut height = inner.chain.tip().height;
        while out.len() < limit && height > 0 {
            let blocks = inner.store.blocks_desc(height, 20)?;
            let Some(last) = blocks.last() else { break };
            height = last.header.height - 1;
            for block in &blocks {
                let view = crate::views::BlockView::from(block);
                out.extend(view.txs.into_iter().rev());
            }
        }
        out.truncate(limit);
        Ok(out)
    }

    /// Transactions involving `address`, newest first.
    pub fn address_txs(
        &self,
        address: &Address,
        before: Option<TxLocation>,
        limit: usize,
    ) -> Result<Vec<TxView>, NodeError> {
        let inner = self.lock();
        inner
            .store
            .address_txs(address, before, limit)?
            .into_iter()
            .map(|location| tx_at(&inner.store, location))
            .collect()
    }
}

fn tx_status(inner: &Inner, hash: &Hash) -> Result<TxStatus, NodeError> {
    if let Some(location) = inner.store.tx_location(hash)? {
        return included(&inner.store, location);
    }
    if inner.mempool.contains(hash) {
        return Ok(TxStatus::Pending);
    }
    Ok(match inner.dropped.reasons.get(hash) {
        Some(error) => TxStatus::Rejected {
            error: error.clone(),
        },
        None => TxStatus::Unknown,
    })
}

fn included(store: &Store, location: TxLocation) -> Result<TxStatus, NodeError> {
    let block = block_at(store, location.height)?;
    Ok(TxStatus::Included {
        height: location.height,
        index: location.index,
        timestamp: block.header.timestamp,
    })
}

fn block_at(store: &Store, height: u64) -> Result<Block, NodeError> {
    store.block(height)?.ok_or_else(|| {
        StoreError::Corrupt(format!("index points to missing block {height}")).into()
    })
}

fn tx_at(store: &Store, location: TxLocation) -> Result<TxView, NodeError> {
    let block = block_at(store, location.height)?;
    let tx = block.txs.get(location.index as usize).ok_or_else(|| {
        StoreError::Corrupt(format!(
            "index points to missing transaction {}/{}",
            location.height, location.index
        ))
    })?;
    Ok(TxView {
        hash: tx.hash(),
        status: TxStatus::Included {
            height: location.height,
            index: location.index,
            timestamp: block.header.timestamp,
        },
        tx: Some(tx.clone()),
    })
}

fn authority_view(chain: &Chain, address: &Address) -> Option<AuthorityView> {
    let state = chain.state();
    let authority = state.authorities().get(address)?;
    let genesis = state.genesis();
    // "Now" for quota purposes: wall clock, but never behind the chain.
    let now = now_ms().max(chain.tip().timestamp);
    let period = genesis.quota_period_at(now);
    let minted = if authority.period == period {
        authority.minted
    } else {
        0
    };
    Some(AuthorityView {
        address: *address,
        active: authority.active,
        quota: authority.quota,
        minted,
        remaining: state.mint_quota_remaining(address, now).unwrap_or(0),
        period_ends_at: genesis
            .timestamp
            .saturating_add((period + 1).saturating_mul(genesis.quota_period_ms)),
    })
}
