//! Error types of the chain rules.

use serde::{Deserialize, Serialize};

use crate::types::{Amount, PublicKey};

/// Why a transaction is rejected by the state machine.
///
/// Serializes to JSON as `{"code": "bad_nonce", "expected": 1, "got": 0}` so that callers of
/// the node API can react to a precise cause.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[serde(tag = "code", rename_all = "snake_case")]
pub enum TxError {
    #[error("the public key does not match the sender address")]
    AddressMismatch,
    #[error("invalid signature")]
    BadSignature,
    #[error("bad nonce: expected {expected}, got {got}")]
    BadNonce { expected: u64, got: u64 },
    #[error("amount must be greater than zero")]
    ZeroAmount,
    #[error("insufficient balance: have {balance}, need {needed}")]
    InsufficientBalance { balance: Amount, needed: Amount },
    #[error("sender is not an active mint authority")]
    NotAuthority,
    #[error("mint quota exceeded: quota {quota}, already minted {used} this period, requested {requested}")]
    QuotaExceeded {
        quota: Amount,
        used: Amount,
        requested: Amount,
    },
    #[error("an authority cannot mint to itself")]
    SelfMint,
    #[error("only the admin key can update authorities")]
    NotAdmin,
    #[error("authority not found or already revoked")]
    AuthorityNotFound,
    #[error("only the shop treasury can issue refunds")]
    NotTreasury,
    #[error("the shop treasury cannot make purchases")]
    TreasuryCannotPurchase,
    #[error("unknown purchase")]
    UnknownPurchase,
    #[error("purchase already refunded")]
    AlreadyRefunded,
    /// No longer produced: transfers were refused in V1 and are allowed since V2. Kept so
    /// that every code ever stored still reads.
    #[error("transfers between accounts are disabled")]
    TransfersDisabled,
    #[error("arithmetic overflow")]
    Overflow,
    #[error("an account cannot transfer to itself")]
    SelfTransfer,
    #[error("the shop treasury cannot make transfers")]
    TreasuryCannotTransfer,
}

/// Why a genesis configuration is invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GenesisError {
    #[error("chain id must be 1 to 32 characters among [a-z0-9-]")]
    BadChainId,
    #[error("at least one validator is required")]
    NoValidators,
    #[error("duplicate validator key")]
    DuplicateValidator,
    #[error("the quota period must be greater than zero")]
    ZeroQuotaPeriod,
    #[error("the admin and treasury accounts must be different")]
    AdminIsTreasury,
}

/// Why a block is rejected.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BlockError {
    #[error("bad height: expected {expected}, got {got}")]
    BadHeight { expected: u64, got: u64 },
    #[error("previous block hash mismatch")]
    BadPrevHash,
    #[error("timestamp must be greater than the parent's")]
    BadTimestamp,
    #[error("wrong validator for this height: expected {expected}")]
    WrongValidator { expected: PublicKey },
    #[error("invalid block signature")]
    BadSignature,
    #[error("transaction root mismatch")]
    BadTxRoot,
    #[error("state root mismatch")]
    BadStateRoot,
    #[error("invalid transaction at index {index}: {source}")]
    Tx { index: usize, source: TxError },
}
