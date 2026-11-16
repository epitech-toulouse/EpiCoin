//! Genesis configuration: the immutable parameters of a chain.

use std::collections::BTreeSet;

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::crypto::{domain, hash_borsh};
use crate::error::GenesisError;
use crate::types::{Address, Hash, PublicKey};

/// Default length of a mint-quota period: 30 days.
pub const DEFAULT_QUOTA_PERIOD_MS: u64 = 30 * 24 * 60 * 60 * 1000;

/// The parameters every node of a chain agrees on. Its hash is the parent of block 1.
///
/// Stored as a JSON file for humans; hashed through borsh like everything else.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct Genesis {
    /// Short identifier bound into every transaction signature, e.g. `epicoin-prod`.
    pub chain_id: String,
    /// Chain start, in milliseconds since the Unix epoch. Quota periods are counted from it.
    pub timestamp: u64,
    /// Proof-of-authority validators, in round-robin order.
    pub validators: Vec<PublicKey>,
    /// The only account allowed to grant or revoke mint authorities.
    pub admin: Address,
    /// The shop account: receives purchases, issues refunds.
    pub treasury: Address,
    /// Length of a mint-quota period, in milliseconds.
    pub quota_period_ms: u64,
}

impl Genesis {
    pub fn validate(&self) -> Result<(), GenesisError> {
        let id_ok = (1..=32).contains(&self.chain_id.len())
            && self
                .chain_id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
        if !id_ok {
            return Err(GenesisError::BadChainId);
        }
        if self.validators.is_empty() {
            return Err(GenesisError::NoValidators);
        }
        let unique: BTreeSet<_> = self.validators.iter().collect();
        if unique.len() != self.validators.len() {
            return Err(GenesisError::DuplicateValidator);
        }
        if self.quota_period_ms == 0 {
            return Err(GenesisError::ZeroQuotaPeriod);
        }
        if self.admin == self.treasury {
            return Err(GenesisError::AdminIsTreasury);
        }
        Ok(())
    }

    /// Identifies the chain; used as `prev_hash` of the first block.
    pub fn hash(&self) -> Hash {
        hash_borsh(domain::GENESIS, self)
    }

    /// The validator expected to sign the block at `height` (round-robin, blocks start at 1).
    ///
    /// # Panics
    /// Panics if the genesis has no validator, which [`Genesis::validate`] rules out.
    pub fn validator_at(&self, height: u64) -> PublicKey {
        let index = height.saturating_sub(1) % self.validators.len() as u64;
        self.validators[index as usize]
    }

    /// Index of the quota period containing `timestamp`.
    pub fn quota_period_at(&self, timestamp: u64) -> u64 {
        timestamp.saturating_sub(self.timestamp) / self.quota_period_ms
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn genesis() -> Genesis {
        Genesis {
            chain_id: "epicoin-test".into(),
            timestamp: 1_000,
            validators: vec![PublicKey([1; 32]), PublicKey([2; 32])],
            admin: Address([3; 32]),
            treasury: Address([4; 32]),
            quota_period_ms: 100,
        }
    }

    #[test]
    fn valid_genesis_passes() {
        assert_eq!(genesis().validate(), Ok(()));
    }

    #[test]
    fn invalid_genesis_is_rejected() {
        let with = |f: fn(&mut Genesis)| {
            let mut g = genesis();
            f(&mut g);
            g.validate()
        };
        assert_eq!(
            with(|g| g.chain_id = String::new()),
            Err(GenesisError::BadChainId)
        );
        assert_eq!(
            with(|g| g.chain_id = "Has Spaces".into()),
            Err(GenesisError::BadChainId)
        );
        assert_eq!(
            with(|g| g.chain_id = "x".repeat(33)),
            Err(GenesisError::BadChainId)
        );
        assert_eq!(
            with(|g| g.validators.clear()),
            Err(GenesisError::NoValidators)
        );
        assert_eq!(
            with(|g| g.validators.push(PublicKey([1; 32]))),
            Err(GenesisError::DuplicateValidator)
        );
        assert_eq!(
            with(|g| g.quota_period_ms = 0),
            Err(GenesisError::ZeroQuotaPeriod)
        );
        assert_eq!(
            with(|g| g.treasury = g.admin),
            Err(GenesisError::AdminIsTreasury)
        );
    }

    #[test]
    fn hash_depends_on_every_parameter() {
        let base = genesis().hash();
        let mut g = genesis();
        g.chain_id = "epicoin-other".into();
        assert_ne!(g.hash(), base);
        let mut g = genesis();
        g.quota_period_ms += 1;
        assert_ne!(g.hash(), base);
        assert_eq!(genesis().hash(), base);
    }

    #[test]
    fn validators_rotate_round_robin() {
        let g = genesis();
        assert_eq!(g.validator_at(1), PublicKey([1; 32]));
        assert_eq!(g.validator_at(2), PublicKey([2; 32]));
        assert_eq!(g.validator_at(3), PublicKey([1; 32]));
    }

    #[test]
    fn quota_periods_are_fixed_windows_from_genesis() {
        let g = genesis();
        assert_eq!(g.quota_period_at(0), 0);
        assert_eq!(g.quota_period_at(1_000), 0);
        assert_eq!(g.quota_period_at(1_099), 0);
        assert_eq!(g.quota_period_at(1_100), 1);
        assert_eq!(g.quota_period_at(1_250), 2);
    }

    #[test]
    fn json_round_trip() {
        let g = genesis();
        let json = serde_json::to_string_pretty(&g).unwrap();
        assert_eq!(serde_json::from_str::<Genesis>(&json).unwrap(), g);
    }
}
