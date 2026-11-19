//! Transactions.
//!
//! GDPR: a transaction only ever contains addresses, amounts, enum codes and hashes. There is
//! no free-text field, and none may be added.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::crypto::{domain, hash_borsh, Keypair};
use crate::error::TxError;
use crate::types::{Address, Amount, Hash, PublicKey, Signature};

/// Why EPC were granted. Fixed list: new codes may only be appended, existing discriminants
/// never change.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    BorshSerialize,
    BorshDeserialize,
    Serialize,
    Deserialize,
)]
#[borsh(use_discriminant = true)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum ReasonCode {
    Participation = 0,
    PeerHelp = 1,
    OutstandingProject = 2,
    Event = 3,
    CampusLife = 4,
    Challenge = 5,
    Other = 6,
}

impl ReasonCode {
    pub const ALL: [ReasonCode; 7] = [
        ReasonCode::Participation,
        ReasonCode::PeerHelp,
        ReasonCode::OutstandingProject,
        ReasonCode::Event,
        ReasonCode::CampusLife,
        ReasonCode::Challenge,
        ReasonCode::Other,
    ];
}

/// What an [`TxPayload::AuthorityUpdate`] does to a staff key.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
#[borsh(use_discriminant = true)]
#[serde(tag = "action", rename_all = "snake_case")]
#[repr(u8)]
pub enum AuthorityAction {
    /// Authorizes the key (or updates its quota): at most `quota` units minted per period.
    Grant { quota: Amount } = 0,
    /// Revokes the key. It can be granted again later.
    Revoke = 1,
}

/// The kind-specific content of a transaction.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
#[borsh(use_discriminant = true)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[repr(u8)]
pub enum TxPayload {
    /// Signed by a staff authority: creates EPC on `to`.
    Mint {
        to: Address,
        amount: Amount,
        reason: ReasonCode,
    } = 0,
    /// Signed by the buyer: moves EPC to the shop treasury. `order_ref` is an opaque hash.
    Purchase { amount: Amount, order_ref: Hash } = 1,
    /// Signed by the treasury: gives back the full amount of the `Purchase` with this hash.
    Refund { purchase: Hash } = 2,
    /// Signed by the admin key: grants or revokes a staff authority.
    AuthorityUpdate {
        authority: Address,
        action: AuthorityAction,
    } = 3,
    /// Signed by the sender: moves EPC to another account. Refused in V1, allowed since V2
    /// (never from the treasury, never to oneself).
    Transfer { to: Address, amount: Amount } = 4,
}

/// Transaction kind, without its content.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TxKind {
    Mint,
    Purchase,
    Refund,
    AuthorityUpdate,
    Transfer,
}

impl TxPayload {
    pub fn kind(&self) -> TxKind {
        match self {
            TxPayload::Mint { .. } => TxKind::Mint,
            TxPayload::Purchase { .. } => TxKind::Purchase,
            TxPayload::Refund { .. } => TxKind::Refund,
            TxPayload::AuthorityUpdate { .. } => TxKind::AuthorityUpdate,
            TxPayload::Transfer { .. } => TxKind::Transfer,
        }
    }
}

/// A signed transaction.
///
/// `from` is the hash of `pubkey`; the key travels with the transaction because an address
/// alone is not enough to verify a signature.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct Transaction {
    pub from: Address,
    pub pubkey: PublicKey,
    /// Must equal the sender's account nonce; prevents replay.
    pub nonce: u64,
    pub payload: TxPayload,
    pub signature: Signature,
}

/// The exact content covered by the signature, in canonical field order.
#[derive(BorshSerialize)]
struct SigningPreimage<'a> {
    chain_id: &'a str,
    from: &'a Address,
    pubkey: &'a PublicKey,
    nonce: u64,
    payload: &'a TxPayload,
}

impl Transaction {
    /// Builds and signs a transaction for the chain identified by `chain_id`.
    pub fn sign(chain_id: &str, keypair: &Keypair, nonce: u64, payload: TxPayload) -> Self {
        let pubkey = keypair.public_key();
        let from = pubkey.address();
        let digest = Self::digest(chain_id, &from, &pubkey, nonce, &payload);
        Self {
            from,
            pubkey,
            nonce,
            payload,
            signature: keypair.sign(&digest),
        }
    }

    fn digest(
        chain_id: &str,
        from: &Address,
        pubkey: &PublicKey,
        nonce: u64,
        payload: &TxPayload,
    ) -> Hash {
        hash_borsh(
            domain::TX_SIGNING,
            &SigningPreimage {
                chain_id,
                from,
                pubkey,
                nonce,
                payload,
            },
        )
    }

    /// The digest the sender signs. Binding the chain id prevents replaying a transaction from
    /// another EpiCoin chain (e.g. dev on prod).
    pub fn signing_hash(&self, chain_id: &str) -> Hash {
        Self::digest(
            chain_id,
            &self.from,
            &self.pubkey,
            self.nonce,
            &self.payload,
        )
    }

    /// The transaction id: hash of the full signed encoding.
    pub fn hash(&self) -> Hash {
        hash_borsh(domain::TX_ID, self)
    }

    pub fn kind(&self) -> TxKind {
        self.payload.kind()
    }

    /// Checks that `pubkey` owns `from` and that the signature is valid for `chain_id`.
    pub fn verify_signature(&self, chain_id: &str) -> Result<(), TxError> {
        if self.pubkey.address() != self.from {
            return Err(TxError::AddressMismatch);
        }
        if !self
            .pubkey
            .verify(&self.signing_hash(chain_id), &self.signature)
        {
            return Err(TxError::BadSignature);
        }
        Ok(())
    }

    /// Canonical encoding, as stored in blocks and sent over the wire.
    pub fn to_bytes(&self) -> Vec<u8> {
        borsh::to_vec(self).expect("borsh serialization into a Vec is infallible")
    }

    /// Decodes a canonical encoding. Trailing bytes are rejected.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, std::io::Error> {
        borsh::from_slice(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAIN: &str = "epicoin-test";

    fn keypair(seed: u8) -> Keypair {
        Keypair::from_secret_bytes(&[seed; 32])
    }

    fn sample() -> Transaction {
        Transaction::sign(
            CHAIN,
            &keypair(1),
            7,
            TxPayload::Mint {
                to: keypair(2).address(),
                amount: 1_500,
                reason: ReasonCode::PeerHelp,
            },
        )
    }

    #[test]
    fn signed_transaction_verifies() {
        assert_eq!(sample().verify_signature(CHAIN), Ok(()));
    }

    #[test]
    fn signature_is_bound_to_the_chain_id() {
        assert_eq!(
            sample().verify_signature("another-chain"),
            Err(TxError::BadSignature)
        );
    }

    #[test]
    fn any_field_change_invalidates_the_signature() {
        let mut tx = sample();
        tx.nonce += 1;
        assert_eq!(tx.verify_signature(CHAIN), Err(TxError::BadSignature));

        let mut tx = sample();
        tx.payload = TxPayload::Mint {
            to: keypair(2).address(),
            amount: 1_501,
            reason: ReasonCode::PeerHelp,
        };
        assert_eq!(tx.verify_signature(CHAIN), Err(TxError::BadSignature));
    }

    #[test]
    fn sender_must_own_the_public_key() {
        let mut tx = sample();
        tx.from = keypair(3).address();
        assert_eq!(tx.verify_signature(CHAIN), Err(TxError::AddressMismatch));

        // Swapping in someone else's key does not help either.
        let mut tx = sample();
        tx.pubkey = keypair(3).public_key();
        assert_eq!(tx.verify_signature(CHAIN), Err(TxError::AddressMismatch));
    }

    #[test]
    fn encoding_round_trips_and_rejects_trailing_bytes() {
        let tx = sample();
        let bytes = tx.to_bytes();
        assert_eq!(Transaction::from_bytes(&bytes).unwrap(), tx);

        let mut longer = bytes.clone();
        longer.push(0);
        assert!(Transaction::from_bytes(&longer).is_err());
        assert!(Transaction::from_bytes(&bytes[..bytes.len() - 1]).is_err());
    }

    #[test]
    fn encoding_is_deterministic_and_hash_covers_the_signature() {
        let tx = sample();
        assert_eq!(tx.to_bytes(), sample().to_bytes());
        assert_eq!(tx.hash(), sample().hash());

        let mut other = tx.clone();
        other.signature.0[0] ^= 1;
        assert_ne!(tx.hash(), other.hash());
    }

    /// The wire layout is frozen: field order, sizes and enum discriminants.
    #[test]
    fn canonical_layout_is_stable() {
        let tx = sample();
        let bytes = tx.to_bytes();
        let mut expected = Vec::new();
        expected.extend_from_slice(&tx.from.0); // from
        expected.extend_from_slice(&tx.pubkey.0); // pubkey
        expected.extend_from_slice(&7u64.to_le_bytes()); // nonce
        expected.push(0); // payload discriminant: Mint
        expected.extend_from_slice(&keypair(2).address().0); // to
        expected.extend_from_slice(&1_500u64.to_le_bytes()); // amount
        expected.push(1); // reason: PeerHelp
        expected.extend_from_slice(&tx.signature.0); // signature
        assert_eq!(bytes, expected);
    }

    #[test]
    fn payload_discriminants_are_stable() {
        let h = Hash([9; 32]);
        let a = Address([8; 32]);
        let first_byte = |p: &TxPayload| borsh::to_vec(p).unwrap()[0];
        assert_eq!(
            first_byte(&TxPayload::Mint {
                to: a,
                amount: 1,
                reason: ReasonCode::Other
            }),
            0
        );
        assert_eq!(
            first_byte(&TxPayload::Purchase {
                amount: 1,
                order_ref: h
            }),
            1
        );
        assert_eq!(first_byte(&TxPayload::Refund { purchase: h }), 2);
        assert_eq!(
            first_byte(&TxPayload::AuthorityUpdate {
                authority: a,
                action: AuthorityAction::Revoke
            }),
            3
        );
        assert_eq!(first_byte(&TxPayload::Transfer { to: a, amount: 1 }), 4);
        for (i, reason) in ReasonCode::ALL.iter().enumerate() {
            assert_eq!(borsh::to_vec(reason).unwrap(), vec![i as u8]);
        }
    }

    #[test]
    fn json_view_is_readable() {
        let json = serde_json::to_value(sample()).unwrap();
        assert_eq!(json["payload"]["kind"], "mint");
        assert_eq!(json["payload"]["reason"], "peer_help");
        assert_eq!(json["payload"]["amount"], 1_500);
        let back: Transaction = serde_json::from_value(json).unwrap();
        assert_eq!(back, sample());
    }
}
