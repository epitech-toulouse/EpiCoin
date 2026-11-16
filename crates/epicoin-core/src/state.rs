//! The state machine: accounts, mint authorities, purchases, and the rules that change them.

use std::collections::BTreeMap;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::crypto::{domain, hash_borsh};
use crate::error::{GenesisError, TxError};
use crate::genesis::Genesis;
use crate::tx::{AuthorityAction, Transaction, TxPayload};
use crate::types::{Address, Amount, Hash};

/// Balance and replay-protection counter of an address.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
)]
pub struct Account {
    pub balance: Amount,
    /// Number of transactions this account has sent; the next one must carry this value.
    pub nonce: u64,
}

/// A staff key allowed to mint, and how much of its quota it has used.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct Authority {
    /// False once revoked. The record is kept so that revoking then re-granting within a
    /// period does not reset the usage.
    pub active: bool,
    /// Maximum amount this key may mint per quota period.
    pub quota: Amount,
    /// Quota period in which this key last minted.
    pub period: u64,
    /// Amount minted during `period`.
    pub minted: Amount,
}

/// What the chain remembers about a purchase, so it can be refunded exactly once.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize,
)]
pub struct PurchaseRecord {
    pub buyer: Address,
    pub amount: Amount,
    pub refunded: bool,
}

/// The full chain state. Maps are ordered, so the borsh encoding — and therefore the state
/// root — is deterministic.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize)]
pub struct State {
    genesis: Genesis,
    accounts: BTreeMap<Address, Account>,
    authorities: BTreeMap<Address, Authority>,
    purchases: BTreeMap<Hash, PurchaseRecord>,
    total_minted: Amount,
    total_burned: Amount,
}

impl State {
    /// The empty state of a new chain.
    pub fn new(genesis: Genesis) -> Result<Self, GenesisError> {
        genesis.validate()?;
        Ok(Self {
            genesis,
            accounts: BTreeMap::new(),
            authorities: BTreeMap::new(),
            purchases: BTreeMap::new(),
            total_minted: 0,
            total_burned: 0,
        })
    }

    /// Hash of the whole state. At this scale (hundreds of accounts) hashing everything is
    /// simpler and safer than maintaining a Merkle tree.
    pub fn root(&self) -> Hash {
        hash_borsh(domain::STATE, self)
    }

    pub fn genesis(&self) -> &Genesis {
        &self.genesis
    }

    pub fn account(&self, address: &Address) -> Account {
        self.accounts.get(address).copied().unwrap_or_default()
    }

    pub fn balance(&self, address: &Address) -> Amount {
        self.account(address).balance
    }

    pub fn nonce(&self, address: &Address) -> u64 {
        self.account(address).nonce
    }

    pub fn accounts(&self) -> &BTreeMap<Address, Account> {
        &self.accounts
    }

    pub fn authorities(&self) -> &BTreeMap<Address, Authority> {
        &self.authorities
    }

    pub fn purchases(&self) -> &BTreeMap<Hash, PurchaseRecord> {
        &self.purchases
    }

    pub fn total_minted(&self) -> Amount {
        self.total_minted
    }

    /// Always zero in V1 (no burn transaction yet); part of the state so the supply invariant
    /// `sum(balances) == total_minted - total_burned` keeps its shape when burning is added.
    pub fn total_burned(&self) -> Amount {
        self.total_burned
    }

    /// EPC currently in existence.
    pub fn circulating_supply(&self) -> Amount {
        self.total_minted - self.total_burned
    }

    /// How much `address` may still mint at `timestamp`, or `None` if it is not an active
    /// authority.
    pub fn mint_quota_remaining(&self, address: &Address, timestamp: u64) -> Option<Amount> {
        let authority = self.authorities.get(address).filter(|a| a.active)?;
        let used = self.minted_in_period(authority, self.genesis.quota_period_at(timestamp));
        Some(authority.quota.saturating_sub(used))
    }

    fn minted_in_period(&self, authority: &Authority, period: u64) -> Amount {
        if authority.period == period {
            authority.minted
        } else {
            0
        }
    }

    /// Validates `tx` and applies it, as part of a block stamped `block_timestamp`.
    ///
    /// Atomic: every check and every checked computation happens before the first write, so
    /// a rejected transaction leaves the state untouched.
    pub fn apply_tx(&mut self, tx: &Transaction, block_timestamp: u64) -> Result<(), TxError> {
        tx.verify_signature(&self.genesis.chain_id)?;

        let sender = self.account(&tx.from);
        if tx.nonce != sender.nonce {
            return Err(TxError::BadNonce {
                expected: sender.nonce,
                got: tx.nonce,
            });
        }
        let next_nonce = sender.nonce.checked_add(1).ok_or(TxError::Overflow)?;

        match &tx.payload {
            TxPayload::Mint { to, amount, .. } => {
                let authority = *self
                    .authorities
                    .get(&tx.from)
                    .filter(|a| a.active)
                    .ok_or(TxError::NotAuthority)?;
                if *amount == 0 {
                    return Err(TxError::ZeroAmount);
                }
                if *to == tx.from {
                    return Err(TxError::SelfMint);
                }
                let period = self.genesis.quota_period_at(block_timestamp);
                let used = self.minted_in_period(&authority, period);
                let new_used = used.checked_add(*amount).ok_or(TxError::Overflow)?;
                if new_used > authority.quota {
                    return Err(TxError::QuotaExceeded {
                        quota: authority.quota,
                        used,
                        requested: *amount,
                    });
                }
                let new_balance = self
                    .balance(to)
                    .checked_add(*amount)
                    .ok_or(TxError::Overflow)?;
                let new_total = self
                    .total_minted
                    .checked_add(*amount)
                    .ok_or(TxError::Overflow)?;

                self.authorities.insert(
                    tx.from,
                    Authority {
                        period,
                        minted: new_used,
                        ..authority
                    },
                );
                self.accounts.entry(*to).or_default().balance = new_balance;
                self.total_minted = new_total;
            }

            TxPayload::Purchase { amount, .. } => {
                let treasury = self.genesis.treasury;
                if tx.from == treasury {
                    return Err(TxError::TreasuryCannotPurchase);
                }
                if *amount == 0 {
                    return Err(TxError::ZeroAmount);
                }
                let new_sender_balance =
                    sender
                        .balance
                        .checked_sub(*amount)
                        .ok_or(TxError::InsufficientBalance {
                            balance: sender.balance,
                            needed: *amount,
                        })?;
                let new_treasury_balance = self
                    .balance(&treasury)
                    .checked_add(*amount)
                    .ok_or(TxError::Overflow)?;

                self.accounts.entry(tx.from).or_default().balance = new_sender_balance;
                self.accounts.entry(treasury).or_default().balance = new_treasury_balance;
                self.purchases.insert(
                    tx.hash(),
                    PurchaseRecord {
                        buyer: tx.from,
                        amount: *amount,
                        refunded: false,
                    },
                );
            }

            TxPayload::Refund { purchase } => {
                if tx.from != self.genesis.treasury {
                    return Err(TxError::NotTreasury);
                }
                let record = *self
                    .purchases
                    .get(purchase)
                    .ok_or(TxError::UnknownPurchase)?;
                if record.refunded {
                    return Err(TxError::AlreadyRefunded);
                }
                let new_treasury_balance = sender.balance.checked_sub(record.amount).ok_or(
                    TxError::InsufficientBalance {
                        balance: sender.balance,
                        needed: record.amount,
                    },
                )?;
                // The buyer is never the treasury (purchases by the treasury are rejected).
                let new_buyer_balance = self
                    .balance(&record.buyer)
                    .checked_add(record.amount)
                    .ok_or(TxError::Overflow)?;

                self.accounts.entry(tx.from).or_default().balance = new_treasury_balance;
                self.accounts.entry(record.buyer).or_default().balance = new_buyer_balance;
                self.purchases.insert(
                    *purchase,
                    PurchaseRecord {
                        refunded: true,
                        ..record
                    },
                );
            }

            TxPayload::AuthorityUpdate { authority, action } => {
                if tx.from != self.genesis.admin {
                    return Err(TxError::NotAdmin);
                }
                let existing = self.authorities.get(authority).copied();
                let updated = match action {
                    AuthorityAction::Grant { quota } => Authority {
                        active: true,
                        quota: *quota,
                        period: existing.map_or(0, |a| a.period),
                        minted: existing.map_or(0, |a| a.minted),
                    },
                    AuthorityAction::Revoke => Authority {
                        active: false,
                        ..existing
                            .filter(|a| a.active)
                            .ok_or(TxError::AuthorityNotFound)?
                    },
                };
                self.authorities.insert(*authority, updated);
            }

            // Reserved for a later version: the format carries it, the rules refuse it.
            TxPayload::Transfer { .. } => return Err(TxError::TransfersDisabled),
        }

        self.accounts.entry(tx.from).or_default().nonce = next_nonce;
        Ok(())
    }
}
