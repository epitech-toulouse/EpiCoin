//! Full chain audit: replay every block from the genesis.
//!
//! The only trusted input is the genesis. Everything else — block hashes and links,
//! validator signatures, transaction roots and signatures, nonces, quotas, balances, state
//! roots — is recomputed.

use epicoin_core::{Amount, Block, BlockError, Chain, Genesis, GenesisError, Hash};
use serde::Serialize;

use crate::store::{Store, StoreError};

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("invalid genesis: {0}")]
    Genesis(#[from] GenesisError),
    #[error("block {height} is invalid: {source}")]
    Block { height: u64, source: BlockError },
    #[error(transparent)]
    Store(#[from] StoreError),
    #[error("the chain stops at height {replayed} but {expected} blocks were expected")]
    MissingBlocks { replayed: u64, expected: u64 },
    #[error("the stored state snapshot does not match the replayed chain")]
    SnapshotMismatch,
    #[error("the supply invariant is broken: balances sum to {balances}, supply is {supply}")]
    SupplyMismatch { balances: u128, supply: Amount },
}

/// What a successful audit established.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VerifyReport {
    pub chain_id: String,
    pub height: u64,
    pub transactions: u64,
    pub tip_hash: Hash,
    pub state_root: Hash,
    pub accounts: usize,
    pub total_minted: Amount,
    pub circulating_supply: Amount,
}

/// Replays blocks on top of a chain, counting transactions.
pub struct Replayer {
    chain: Chain,
    transactions: u64,
}

impl Replayer {
    pub fn new(genesis: &Genesis) -> Result<Self, VerifyError> {
        Ok(Self {
            chain: Chain::new(genesis.clone())?,
            transactions: 0,
        })
    }

    pub fn height(&self) -> u64 {
        self.chain.tip().height
    }

    /// Fully verifies `block` as the next block of the chain.
    pub fn apply(&mut self, block: &Block) -> Result<(), VerifyError> {
        self.chain
            .apply_block(block)
            .map_err(|source| VerifyError::Block {
                // The position in the chain, which a tampered header might lie about.
                height: self.chain.tip().height + 1,
                source,
            })?;
        self.transactions += block.txs.len() as u64;
        Ok(())
    }

    /// Final checks, then the report.
    pub fn finish(self) -> Result<(Chain, VerifyReport), VerifyError> {
        let state = self.chain.state();
        let balances: u128 = state
            .accounts()
            .values()
            .map(|account| u128::from(account.balance))
            .sum();
        if balances != u128::from(state.circulating_supply()) {
            return Err(VerifyError::SupplyMismatch {
                balances,
                supply: state.circulating_supply(),
            });
        }
        let report = VerifyReport {
            chain_id: state.genesis().chain_id.clone(),
            height: self.chain.tip().height,
            transactions: self.transactions,
            tip_hash: self.chain.tip().hash,
            state_root: state.root(),
            accounts: state.accounts().len(),
            total_minted: state.total_minted(),
            circulating_supply: state.circulating_supply(),
        };
        Ok((self.chain, report))
    }
}

/// Audits a node database: replays every stored block from `genesis` and checks that the
/// stored state snapshot (what the node serves balances from) is exactly the replayed state.
pub fn verify_store(store: &Store, genesis: &Genesis) -> Result<VerifyReport, VerifyError> {
    let expected = store.height()?;
    let mut replayer = Replayer::new(genesis)?;
    loop {
        let blocks = store.blocks(replayer.height() + 1, 200)?;
        if blocks.is_empty() {
            break;
        }
        for block in &blocks {
            replayer.apply(block)?;
        }
    }
    if replayer.height() != expected {
        return Err(VerifyError::MissingBlocks {
            replayed: replayer.height(),
            expected,
        });
    }
    let (chain, report) = replayer.finish()?;
    if store.chain_snapshot()? != chain {
        return Err(VerifyError::SnapshotMismatch);
    }
    Ok(report)
}
