//! Durable storage of the chain in a single redb file.
//!
//! Tables:
//! - `blocks`: height → canonical block bytes (the source of truth)
//! - `block_heights`: block hash → height
//! - `tx_index`: transaction hash → (height, index in block)
//! - `address_txs`: address ‖ height ‖ index → () — transactions involving an address
//! - `meta`: `genesis` (canonical genesis) and `chain` (state + tip after the last block)
//!
//! A block, its indexes and the new state snapshot are written in one transaction: after a
//! crash the file holds either all of a block or none of it.

use std::path::Path;

use epicoin_core::{Address, Block, Chain, Genesis, Hash, Transaction, TxPayload};
use redb::{Database, ReadableDatabase, ReadableTable, ReadableTableMetadata, TableDefinition};

const BLOCKS: TableDefinition<u64, &[u8]> = TableDefinition::new("blocks");
const BLOCK_HEIGHTS: TableDefinition<&[u8], u64> = TableDefinition::new("block_heights");
const TX_INDEX: TableDefinition<&[u8], &[u8]> = TableDefinition::new("tx_index");
const ADDRESS_TXS: TableDefinition<&[u8], ()> = TableDefinition::new("address_txs");
const META: TableDefinition<&str, &[u8]> = TableDefinition::new("meta");

const META_GENESIS: &str = "genesis";
const META_CHAIN: &str = "chain";

/// File name of the database inside the node's data directory.
pub const DB_FILE: &str = "chain.redb";

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("database error: {0}")]
    Db(#[from] redb::Error),
    #[error("the database belongs to a different chain (genesis mismatch)")]
    GenesisMismatch,
    #[error("the database is corrupt: {0}")]
    Corrupt(String),
    #[error("block {got} does not extend the stored chain (height {height})")]
    NotNext { height: u64, got: u64 },
}

macro_rules! from_redb {
    ($($source:ty),*) => {
        $(impl From<$source> for StoreError {
            fn from(error: $source) -> Self {
                StoreError::Db(error.into())
            }
        })*
    };
}
from_redb!(
    redb::DatabaseError,
    redb::TransactionError,
    redb::TableError,
    redb::StorageError,
    redb::CommitError
);

/// Position of a transaction in the chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TxLocation {
    pub height: u64,
    pub index: u32,
}

impl TxLocation {
    fn to_bytes(self) -> [u8; 12] {
        let mut out = [0u8; 12];
        out[..8].copy_from_slice(&self.height.to_be_bytes());
        out[8..].copy_from_slice(&self.index.to_be_bytes());
        out
    }

    fn from_bytes(bytes: &[u8]) -> Result<Self, StoreError> {
        let corrupt = || StoreError::Corrupt("malformed transaction location".into());
        let height = bytes.get(..8).ok_or_else(corrupt)?;
        let index = bytes.get(8..12).ok_or_else(corrupt)?;
        Ok(Self {
            height: u64::from_be_bytes(height.try_into().map_err(|_| corrupt())?),
            index: u32::from_be_bytes(index.try_into().map_err(|_| corrupt())?),
        })
    }
}

fn address_key(address: &Address, location: TxLocation) -> [u8; 44] {
    let mut key = [0u8; 44];
    key[..32].copy_from_slice(&address.0);
    key[32..].copy_from_slice(&location.to_bytes());
    key
}

/// Addresses whose history should show `tx`. `chain` is the chain *after* the block.
fn involved_addresses(tx: &Transaction, chain: &Chain) -> Vec<Address> {
    let mut addresses = vec![tx.from];
    match &tx.payload {
        TxPayload::Mint { to, .. } | TxPayload::Transfer { to, .. } => addresses.push(*to),
        TxPayload::Purchase { .. } => addresses.push(chain.genesis().treasury),
        TxPayload::Refund { purchase } => {
            if let Some(record) = chain.state().purchases().get(purchase) {
                addresses.push(record.buyer);
            }
        }
        TxPayload::AuthorityUpdate { authority, .. } => addresses.push(*authority),
    }
    addresses.sort();
    addresses.dedup();
    addresses
}

pub struct Store {
    db: Database,
}

impl Store {
    /// Opens (or creates) the database at `path` for the chain defined by `genesis`, and
    /// returns it with the chain head it contains.
    ///
    /// Fails if the file belongs to another chain, or if the stored snapshot does not match
    /// the last stored block. This is a cheap consistency check; a full audit is
    /// [`crate::verify::verify_store`].
    pub fn open(path: &Path, genesis: &Genesis) -> Result<(Self, Chain), StoreError> {
        let fresh = Chain::new(genesis.clone())
            .map_err(|e| StoreError::Corrupt(format!("invalid genesis: {e}")))?;
        let genesis_bytes = encode(genesis);

        let db = Database::create(path)?;
        let txn = db.begin_write()?;
        let chain = {
            // Opening every table creates the missing ones.
            let blocks = txn.open_table(BLOCKS)?;
            txn.open_table(BLOCK_HEIGHTS)?;
            txn.open_table(TX_INDEX)?;
            txn.open_table(ADDRESS_TXS)?;
            let mut meta = txn.open_table(META)?;

            let stored_genesis = meta.get(META_GENESIS)?.map(|v| v.value().to_vec());
            match stored_genesis {
                None => {
                    if !blocks.is_empty()? {
                        return Err(StoreError::Corrupt("blocks without a genesis".into()));
                    }
                    meta.insert(META_GENESIS, genesis_bytes.as_slice())?;
                    meta.insert(META_CHAIN, encode(&fresh).as_slice())?;
                    fresh
                }
                Some(bytes) if bytes != genesis_bytes => return Err(StoreError::GenesisMismatch),
                Some(_) => {
                    let bytes = meta
                        .get(META_CHAIN)?
                        .ok_or_else(|| StoreError::Corrupt("missing chain snapshot".into()))?
                        .value()
                        .to_vec();
                    let chain: Chain = borsh::from_slice(&bytes)
                        .map_err(|e| StoreError::Corrupt(format!("chain snapshot: {e}")))?;
                    let last = blocks.last()?.map(|(k, v)| (k.value(), v.value().to_vec()));
                    check_snapshot(&chain, &fresh, last)?;
                    chain
                }
            }
        };
        txn.commit()?;
        Ok((Self { db }, chain))
    }

    /// Durably appends `block`; `chain` is the chain head after applying it.
    pub fn append(&self, block: &Block, chain: &Chain) -> Result<(), StoreError> {
        let height = block.header.height;
        let txn = self.db.begin_write()?;
        {
            let mut blocks = txn.open_table(BLOCKS)?;
            let stored = blocks.last()?.map_or(0, |(k, _)| k.value());
            if height != stored + 1 || chain.tip().height != height {
                return Err(StoreError::NotNext {
                    height: stored,
                    got: height,
                });
            }
            blocks.insert(height, block.to_bytes().as_slice())?;

            let mut heights = txn.open_table(BLOCK_HEIGHTS)?;
            heights.insert(block.hash().0.as_slice(), height)?;

            let mut tx_index = txn.open_table(TX_INDEX)?;
            let mut address_txs = txn.open_table(ADDRESS_TXS)?;
            for (index, tx) in block.txs.iter().enumerate() {
                let location = TxLocation {
                    height,
                    index: index as u32,
                };
                tx_index.insert(tx.hash().0.as_slice(), location.to_bytes().as_slice())?;
                for address in involved_addresses(tx, chain) {
                    address_txs.insert(address_key(&address, location).as_slice(), ())?;
                }
            }

            let mut meta = txn.open_table(META)?;
            meta.insert(META_CHAIN, encode(chain).as_slice())?;
        }
        txn.commit()?;
        Ok(())
    }

    /// Height of the last stored block (0 if none).
    pub fn height(&self) -> Result<u64, StoreError> {
        let txn = self.db.begin_read()?;
        let blocks = txn.open_table(BLOCKS)?;
        let height = blocks.last()?.map_or(0, |(k, _)| k.value());
        Ok(height)
    }

    /// Canonical bytes of the block at `height`, exactly as stored.
    pub fn block_bytes(&self, height: u64) -> Result<Option<Vec<u8>>, StoreError> {
        let txn = self.db.begin_read()?;
        let blocks = txn.open_table(BLOCKS)?;
        Ok(blocks.get(height)?.map(|v| v.value().to_vec()))
    }

    pub fn block(&self, height: u64) -> Result<Option<Block>, StoreError> {
        self.block_bytes(height)?
            .map(|bytes| decode_block(height, &bytes))
            .transpose()
    }

    /// Up to `limit` blocks starting at height `from`, in ascending order.
    pub fn blocks(&self, from: u64, limit: usize) -> Result<Vec<Block>, StoreError> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(BLOCKS)?;
        let mut out = Vec::new();
        for entry in table.range(from..)?.take(limit) {
            let (height, bytes) = entry?;
            out.push(decode_block(height.value(), bytes.value())?);
        }
        Ok(out)
    }

    /// Up to `limit` blocks at or below height `from`, most recent first.
    pub fn blocks_desc(&self, from: u64, limit: usize) -> Result<Vec<Block>, StoreError> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(BLOCKS)?;
        let mut out = Vec::new();
        for entry in table.range(..=from)?.rev().take(limit) {
            let (height, bytes) = entry?;
            out.push(decode_block(height.value(), bytes.value())?);
        }
        Ok(out)
    }

    pub fn height_of(&self, block_hash: &Hash) -> Result<Option<u64>, StoreError> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(BLOCK_HEIGHTS)?;
        Ok(table.get(block_hash.0.as_slice())?.map(|v| v.value()))
    }

    pub fn tx_location(&self, tx_hash: &Hash) -> Result<Option<TxLocation>, StoreError> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(TX_INDEX)?;
        let location = table.get(tx_hash.0.as_slice())?;
        location
            .map(|v| TxLocation::from_bytes(v.value()))
            .transpose()
    }

    /// Locations of the transactions involving `address`, most recent first. `before`
    /// (exclusive) allows paging.
    pub fn address_txs(
        &self,
        address: &Address,
        before: Option<TxLocation>,
        limit: usize,
    ) -> Result<Vec<TxLocation>, StoreError> {
        let txn = self.db.begin_read()?;
        let table = txn.open_table(ADDRESS_TXS)?;
        let start = address_key(
            address,
            TxLocation {
                height: 0,
                index: 0,
            },
        );
        let end = address_key(
            address,
            before.unwrap_or(TxLocation {
                height: u64::MAX,
                index: u32::MAX,
            }),
        );
        let mut out = Vec::new();
        for entry in table
            .range(start.as_slice()..end.as_slice())?
            .rev()
            .take(limit)
        {
            let (key, _) = entry?;
            out.push(TxLocation::from_bytes(&key.value()[32..])?);
        }
        Ok(out)
    }

    /// The chain snapshot as stored (state + tip).
    pub fn chain_snapshot(&self) -> Result<Chain, StoreError> {
        let txn = self.db.begin_read()?;
        let meta = txn.open_table(META)?;
        let bytes = meta
            .get(META_CHAIN)?
            .ok_or_else(|| StoreError::Corrupt("missing chain snapshot".into()))?;
        borsh::from_slice(bytes.value())
            .map_err(|e| StoreError::Corrupt(format!("chain snapshot: {e}")))
    }

    /// Number of indexed transactions.
    pub fn tx_count(&self) -> Result<u64, StoreError> {
        let txn = self.db.begin_read()?;
        Ok(txn.open_table(TX_INDEX)?.len()?)
    }
}

fn encode<T: borsh::BorshSerialize>(value: &T) -> Vec<u8> {
    borsh::to_vec(value).expect("borsh serialization into a Vec is infallible")
}

fn decode_block(height: u64, bytes: &[u8]) -> Result<Block, StoreError> {
    Block::from_bytes(bytes)
        .map_err(|e| StoreError::Corrupt(format!("block {height} cannot be decoded: {e}")))
}

/// Checks that the snapshot describes the chain whose last block is `last`.
fn check_snapshot(
    chain: &Chain,
    fresh: &Chain,
    last: Option<(u64, Vec<u8>)>,
) -> Result<(), StoreError> {
    let mismatch = |what: &str| StoreError::Corrupt(format!("snapshot/blocks mismatch: {what}"));
    if chain.genesis() != fresh.genesis() {
        return Err(mismatch("genesis"));
    }
    match last {
        None if chain == fresh => Ok(()),
        None => Err(mismatch("snapshot is ahead of an empty chain")),
        Some((height, bytes)) => {
            let block = decode_block(height, &bytes)?;
            if chain.tip().height != height || block.header.height != height {
                return Err(mismatch("height"));
            }
            if chain.tip().hash != block.hash() {
                return Err(mismatch("tip hash"));
            }
            if chain.state().root() != block.header.state_root {
                return Err(mismatch("state root"));
            }
            Ok(())
        }
    }
}
