//! Blocks and the Merkle root of their transactions.

use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

use crate::crypto::{domain, hash_borsh, hash_bytes};
use crate::tx::Transaction;
use crate::types::{Hash, PublicKey, Signature};

/// The signed part of a block.
#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct BlockHeader {
    /// 1 for the first block; the genesis itself is "height 0".
    pub height: u64,
    /// Hash of the parent block header, or of the genesis for block 1.
    pub prev_hash: Hash,
    /// Milliseconds since the Unix epoch; strictly increasing along the chain.
    pub timestamp: u64,
    /// Merkle root of the transactions.
    pub tx_root: Hash,
    /// Root of the state after applying this block.
    pub state_root: Hash,
    /// The validator that produced the block.
    pub validator: PublicKey,
}

impl BlockHeader {
    /// The block hash: what the validator signs and what the next block points to.
    pub fn hash(&self) -> Hash {
        hash_borsh(domain::BLOCK, self)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, BorshSerialize, BorshDeserialize, Serialize, Deserialize)]
pub struct Block {
    pub header: BlockHeader,
    /// Validator signature over the header hash.
    pub signature: Signature,
    pub txs: Vec<Transaction>,
}

impl Block {
    pub fn hash(&self) -> Hash {
        self.header.hash()
    }

    /// Canonical encoding, as stored on disk and served to replicas.
    pub fn to_bytes(&self) -> Vec<u8> {
        borsh::to_vec(self).expect("borsh serialization into a Vec is infallible")
    }

    /// Decodes a canonical encoding. Trailing bytes are rejected.
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, std::io::Error> {
        borsh::from_slice(bytes)
    }
}

/// Merkle root of a list of transactions.
pub fn tx_root(txs: &[Transaction]) -> Hash {
    let leaves: Vec<Hash> = txs.iter().map(Transaction::hash).collect();
    merkle_root(&leaves)
}

/// Binary Merkle root over `leaves`.
///
/// Leaves and inner nodes are hashed under different domains, and an odd node is promoted
/// unchanged rather than duplicated, so two different lists can never share a root.
pub fn merkle_root(leaves: &[Hash]) -> Hash {
    if leaves.is_empty() {
        return hash_bytes(domain::MERKLE_EMPTY, &[]);
    }
    let mut level: Vec<Hash> = leaves
        .iter()
        .map(|leaf| hash_bytes(domain::MERKLE_LEAF, &leaf.0))
        .collect();
    while level.len() > 1 {
        level = level
            .chunks(2)
            .map(|pair| match pair {
                [left, right] => {
                    let mut data = [0u8; 64];
                    data[..32].copy_from_slice(&left.0);
                    data[32..].copy_from_slice(&right.0);
                    hash_bytes(domain::MERKLE_NODE, &data)
                }
                [single] => *single,
                _ => unreachable!("chunks(2) yields one or two items"),
            })
            .collect();
    }
    level[0]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaves(n: u8) -> Vec<Hash> {
        (0..n).map(|i| Hash([i; 32])).collect()
    }

    #[test]
    fn merkle_root_is_deterministic() {
        assert_eq!(merkle_root(&leaves(5)), merkle_root(&leaves(5)));
    }

    #[test]
    fn merkle_root_depends_on_content_order_and_length() {
        let base = merkle_root(&leaves(4));

        let mut changed = leaves(4);
        changed[2].0[0] ^= 1;
        assert_ne!(merkle_root(&changed), base);

        let mut swapped = leaves(4);
        swapped.swap(0, 1);
        assert_ne!(merkle_root(&swapped), base);

        assert_ne!(merkle_root(&leaves(3)), base);
        assert_ne!(merkle_root(&leaves(5)), base);
    }

    #[test]
    fn duplicating_the_last_leaf_changes_the_root() {
        // The classic Merkle weakness when odd nodes are duplicated.
        let three = leaves(3);
        let mut four = three.clone();
        four.push(three[2]);
        assert_ne!(merkle_root(&three), merkle_root(&four));
    }

    #[test]
    fn empty_and_single_roots_are_distinct_from_their_inputs() {
        let single = Hash([7; 32]);
        assert_ne!(merkle_root(&[]), Hash::ZERO);
        assert_ne!(merkle_root(&[single]), single);
        assert_ne!(merkle_root(&[]), merkle_root(&[single]));
    }
}
