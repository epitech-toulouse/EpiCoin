//! EpiCoin chain rules.
//!
//! This crate is a pure library: no I/O, no clock, no global state. Everything that is hashed
//! or signed goes through borsh, which is canonical and deterministic. JSON (serde) is only
//! implemented for read-only views and for the genesis file.
//!
//! - [`types`]: fixed-size byte types (hashes, addresses, keys, signatures) and amounts.
//! - [`crypto`]: domain-separated BLAKE3 hashing and ed25519 key pairs.
//! - [`tx`]: transactions and their payloads.
//! - [`genesis`]: the immutable parameters of a chain.
//! - [`state`]: the state machine — every validation rule lives in [`State::apply_tx`].
//! - [`block`]: blocks and Merkle roots.
//! - [`chain`]: block validation and production on top of a state.

pub mod block;
pub mod chain;
pub mod crypto;
pub mod error;
pub mod genesis;
pub mod state;
pub mod tx;
pub mod types;

pub use block::{merkle_root, tx_root, Block, BlockHeader};
pub use chain::{Chain, ChainTip};
pub use crypto::Keypair;
pub use error::{BlockError, GenesisError, TxError};
pub use genesis::{Genesis, DEFAULT_QUOTA_PERIOD_MS};
pub use state::{Account, Authority, PurchaseRecord, State};
pub use tx::{AuthorityAction, ReasonCode, Transaction, TxKind, TxPayload};
pub use types::{Address, Amount, Hash, ParseHexError, PublicKey, Signature, UNITS_PER_EPC};
