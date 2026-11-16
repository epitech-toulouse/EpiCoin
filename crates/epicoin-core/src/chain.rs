//! The chain: a state plus the tip it corresponds to, and the block rules.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::block::{tx_root, Block, BlockHeader};
use crate::crypto::Keypair;
use crate::error::{BlockError, GenesisError};
use crate::genesis::Genesis;
use crate::state::State;
use crate::tx::Transaction;
use crate::types::Hash;

/// The last block of a chain (or the genesis, at height 0).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct ChainTip {
    pub height: u64,
    pub hash: Hash,
    pub timestamp: u64,
}

/// A verified chain head: the state after applying every block up to `tip`.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct Chain {
    state: State,
    tip: ChainTip,
}

impl Chain {
    /// A new chain with no block yet.
    pub fn new(genesis: Genesis) -> Result<Self, GenesisError> {
        let tip = ChainTip {
            height: 0,
            hash: genesis.hash(),
            timestamp: genesis.timestamp,
        };
        Ok(Self {
            state: State::new(genesis)?,
            tip,
        })
    }

    pub fn state(&self) -> &State {
        &self.state
    }

    pub fn tip(&self) -> &ChainTip {
        &self.tip
    }

    pub fn genesis(&self) -> &Genesis {
        self.state.genesis()
    }

    /// Fully verifies `block` against the current tip and, if valid, advances the chain.
    ///
    /// Checks, in order: height, parent hash, timestamp, expected validator, validator
    /// signature, transaction root, every transaction, and the resulting state root. On error
    /// the chain is unchanged.
    pub fn apply_block(&mut self, block: &Block) -> Result<(), BlockError> {
        let header = &block.header;

        let expected_height = self.tip.height + 1;
        if header.height != expected_height {
            return Err(BlockError::BadHeight {
                expected: expected_height,
                got: header.height,
            });
        }
        if header.prev_hash != self.tip.hash {
            return Err(BlockError::BadPrevHash);
        }
        if header.timestamp <= self.tip.timestamp {
            return Err(BlockError::BadTimestamp);
        }
        let expected_validator = self.genesis().validator_at(header.height);
        if header.validator != expected_validator {
            return Err(BlockError::WrongValidator {
                expected: expected_validator,
            });
        }
        let hash = header.hash();
        if !header.validator.verify(&hash, &block.signature) {
            return Err(BlockError::BadSignature);
        }
        if header.tx_root != tx_root(&block.txs) {
            return Err(BlockError::BadTxRoot);
        }

        let mut next = self.state.clone();
        for (index, tx) in block.txs.iter().enumerate() {
            next.apply_tx(tx, header.timestamp)
                .map_err(|source| BlockError::Tx { index, source })?;
        }
        if header.state_root != next.root() {
            return Err(BlockError::BadStateRoot);
        }

        self.state = next;
        self.tip = ChainTip {
            height: header.height,
            hash,
            timestamp: header.timestamp,
        };
        Ok(())
    }

    /// Builds and signs the next block from `txs`, without applying it.
    ///
    /// Every transaction must be valid in order; the caller (the mempool) is responsible for
    /// selecting them. The caller then feeds the block to [`Chain::apply_block`], which
    /// re-verifies everything — producers and replicas go through the same code path.
    pub fn build_block(
        &self,
        txs: Vec<Transaction>,
        timestamp: u64,
        validator: &Keypair,
    ) -> Result<Block, BlockError> {
        if timestamp <= self.tip.timestamp {
            return Err(BlockError::BadTimestamp);
        }
        let height = self.tip.height + 1;
        let expected_validator = self.genesis().validator_at(height);
        if validator.public_key() != expected_validator {
            return Err(BlockError::WrongValidator {
                expected: expected_validator,
            });
        }

        let mut next = self.state.clone();
        for (index, tx) in txs.iter().enumerate() {
            next.apply_tx(tx, timestamp)
                .map_err(|source| BlockError::Tx { index, source })?;
        }

        let header = BlockHeader {
            height,
            prev_hash: self.tip.hash,
            timestamp,
            tx_root: tx_root(&txs),
            state_root: next.root(),
            validator: expected_validator,
        };
        let signature = validator.sign(&header.hash());
        Ok(Block {
            header,
            signature,
            txs,
        })
    }
}
