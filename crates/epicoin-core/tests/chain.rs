//! Block production and verification.

mod common;

use common::*;
use epicoin_core::{
    tx_root, Block, BlockError, Chain, Genesis, Keypair, Transaction, TxError, TxPayload,
};

const VALIDATOR_2_SEED: [u8; 32] = [99; 32];

/// Builds a block on top of `chain` and applies it.
fn produce(chain: &mut Chain, validator: &Keypair, txs: Vec<Transaction>) -> Block {
    let timestamp = chain.tip().timestamp + 3_000;
    let block = chain.build_block(txs, timestamp, validator).unwrap();
    chain.apply_block(&block).unwrap();
    block
}

/// A three-block chain: grant, mint, purchase.
fn sample_chain() -> (World, Chain, Vec<Block>) {
    let w = World::new();
    let mut chain = Chain::new(w.genesis.clone()).unwrap();
    let mut blocks = Vec::new();

    let t = tx(chain.state(), &w.admin, grant(w.staff[0].address(), 1_000));
    blocks.push(produce(&mut chain, &w.validator, vec![t]));

    let t1 = tx(
        chain.state(),
        &w.staff[0],
        mint(w.students[0].address(), 300),
    );
    let t2 = Transaction::sign(CHAIN_ID, &w.staff[0], 1, mint(w.students[1].address(), 200));
    blocks.push(produce(&mut chain, &w.validator, vec![t1, t2]));

    let t = tx(chain.state(), &w.students[0], purchase(100, 1));
    blocks.push(produce(&mut chain, &w.validator, vec![t]));

    (w, chain, blocks)
}

/// Replays `blocks` on a fresh chain.
fn replay(genesis: &Genesis, blocks: &[Block]) -> Result<Chain, (usize, BlockError)> {
    let mut chain = Chain::new(genesis.clone()).unwrap();
    for (i, block) in blocks.iter().enumerate() {
        chain.apply_block(block).map_err(|e| (i, e))?;
    }
    Ok(chain)
}

/// Re-signs a block after its header was modified.
fn resign(block: &mut Block, validator: &Keypair) {
    block.signature = validator.sign(&block.header.hash());
}

#[test]
fn new_chain_starts_at_the_genesis() {
    let w = World::new();
    let chain = Chain::new(w.genesis.clone()).unwrap();
    assert_eq!(chain.tip().height, 0);
    assert_eq!(chain.tip().hash, w.genesis.hash());
    assert_eq!(chain.tip().timestamp, GENESIS_TIME);
}

#[test]
fn blocks_link_and_advance_the_tip() {
    let (w, chain, blocks) = sample_chain();
    assert_eq!(chain.tip().height, 3);
    assert_eq!(chain.tip().hash, blocks[2].hash());
    assert_eq!(blocks[0].header.prev_hash, w.genesis.hash());
    assert_eq!(blocks[1].header.prev_hash, blocks[0].hash());
    assert_eq!(blocks[2].header.state_root, chain.state().root());
    assert_eq!(chain.state().balance(&w.students[0].address()), 200);
    assert_eq!(chain.state().balance(&w.treasury.address()), 100);
}

#[test]
fn replaying_the_blocks_gives_the_same_state() {
    let (w, chain, blocks) = sample_chain();
    let replayed = replay(&w.genesis, &blocks).unwrap();
    assert_eq!(replayed, chain);
    assert_eq!(replayed.state().root(), chain.state().root());
}

#[test]
fn blocks_round_trip_through_their_canonical_encoding() {
    let (_, _, blocks) = sample_chain();
    for block in &blocks {
        let bytes = block.to_bytes();
        assert_eq!(&Block::from_bytes(&bytes).unwrap(), block);
        let mut longer = bytes;
        longer.push(0);
        assert!(Block::from_bytes(&longer).is_err());
    }
}

#[test]
fn empty_blocks_are_valid_heartbeats() {
    let w = World::new();
    let mut chain = Chain::new(w.genesis.clone()).unwrap();
    let root = chain.state().root();
    let block = produce(&mut chain, &w.validator, vec![]);
    assert_eq!(block.header.state_root, root);
    assert_eq!(chain.tip().height, 1);
}

// --- Tampering -------------------------------------------------------------------------

#[test]
fn altered_transaction_is_detected() {
    let (w, _, mut blocks) = sample_chain();
    blocks[1].txs[0].payload = mint(w.students[0].address(), 999);
    assert_eq!(
        replay(&w.genesis, &blocks).unwrap_err(),
        (1, BlockError::BadTxRoot)
    );
}

#[test]
fn removed_or_reordered_transactions_are_detected() {
    let (w, _, blocks) = sample_chain();

    let mut removed = blocks.clone();
    removed[1].txs.pop();
    assert_eq!(
        replay(&w.genesis, &removed).unwrap_err(),
        (1, BlockError::BadTxRoot)
    );

    let mut reordered = blocks.clone();
    reordered[1].txs.swap(0, 1);
    assert_eq!(
        replay(&w.genesis, &reordered).unwrap_err(),
        (1, BlockError::BadTxRoot)
    );
}

#[test]
fn altered_header_is_detected() {
    let (w, _, blocks) = sample_chain();

    let mut altered = blocks.clone();
    altered[1].header.state_root.0[0] ^= 1;
    assert_eq!(
        replay(&w.genesis, &altered).unwrap_err(),
        (1, BlockError::BadSignature)
    );

    let mut altered = blocks.clone();
    altered[1].header.timestamp += 1;
    assert_eq!(
        replay(&w.genesis, &altered).unwrap_err(),
        (1, BlockError::BadSignature)
    );
}

#[test]
fn altered_signature_is_detected() {
    let (w, _, mut blocks) = sample_chain();
    blocks[2].signature.0[0] ^= 1;
    assert_eq!(
        replay(&w.genesis, &blocks).unwrap_err(),
        (2, BlockError::BadSignature)
    );
}

#[test]
fn block_signed_by_an_unknown_key_is_rejected() {
    let (w, _, mut blocks) = sample_chain();
    let intruder = Keypair::from_secret_bytes(&[66; 32]);

    // Claiming to be the validator but signing with another key.
    resign(&mut blocks[0], &intruder);
    assert_eq!(
        replay(&w.genesis, &blocks).unwrap_err(),
        (0, BlockError::BadSignature)
    );

    // Announcing the intruder's key as validator.
    blocks[0].header.validator = intruder.public_key();
    resign(&mut blocks[0], &intruder);
    assert_eq!(
        replay(&w.genesis, &blocks).unwrap_err(),
        (
            0,
            BlockError::WrongValidator {
                expected: w.validator.public_key()
            }
        )
    );
}

#[test]
fn dropping_a_block_breaks_the_chain() {
    let (w, _, blocks) = sample_chain();
    let without_middle = vec![blocks[0].clone(), blocks[2].clone()];
    assert_eq!(
        replay(&w.genesis, &without_middle).unwrap_err(),
        (
            1,
            BlockError::BadHeight {
                expected: 2,
                got: 3
            }
        )
    );
}

// --- Rules that hold even for a block correctly signed by the validator ----------------

#[test]
fn wrong_parent_hash_is_rejected() {
    let (w, _, mut blocks) = sample_chain();
    blocks[1].header.prev_hash.0[0] ^= 1;
    resign(&mut blocks[1], &w.validator);
    assert_eq!(
        replay(&w.genesis, &blocks).unwrap_err(),
        (1, BlockError::BadPrevHash)
    );
}

#[test]
fn non_increasing_timestamp_is_rejected() {
    let (w, _, mut blocks) = sample_chain();
    blocks[1].header.timestamp = blocks[0].header.timestamp;
    resign(&mut blocks[1], &w.validator);
    assert_eq!(
        replay(&w.genesis, &blocks).unwrap_err(),
        (1, BlockError::BadTimestamp)
    );

    let chain = Chain::new(w.genesis.clone()).unwrap();
    assert_eq!(
        chain
            .build_block(vec![], GENESIS_TIME, &w.validator)
            .unwrap_err(),
        BlockError::BadTimestamp
    );
}

#[test]
fn wrong_state_root_is_rejected() {
    let (w, _, mut blocks) = sample_chain();
    blocks[1].header.state_root.0[0] ^= 1;
    resign(&mut blocks[1], &w.validator);
    assert_eq!(
        replay(&w.genesis, &blocks).unwrap_err(),
        (1, BlockError::BadStateRoot)
    );
}

#[test]
fn validator_cannot_include_an_invalid_transaction() {
    let (w, chain, mut blocks) = sample_chain();

    // A mint signed by a student, slipped into an otherwise valid block.
    let forged = tx(
        chain.state(),
        &w.students[1],
        mint(w.students[1].address(), 1_000_000),
    );
    let last = blocks.last_mut().unwrap();
    last.txs.push(forged);
    last.header.tx_root = tx_root(&last.txs);
    resign(last, &w.validator);

    assert_eq!(
        replay(&w.genesis, &blocks).unwrap_err(),
        (
            2,
            BlockError::Tx {
                index: 1,
                source: TxError::NotAuthority
            }
        )
    );

    // And the producer refuses to build such a block in the first place.
    let bad = tx(
        chain.state(),
        &w.students[0],
        TxPayload::Transfer {
            to: w.students[0].address(),
            amount: 1,
        },
    );
    assert_eq!(
        chain
            .build_block(vec![bad], chain.tip().timestamp + 1, &w.validator)
            .unwrap_err(),
        BlockError::Tx {
            index: 0,
            source: TxError::SelfTransfer
        }
    );
}

#[test]
fn rejected_block_leaves_the_chain_unchanged() {
    let (_, mut chain, blocks) = sample_chain();
    let before = chain.clone();
    // Re-applying an old block: wrong height.
    assert!(chain.apply_block(&blocks[0]).is_err());
    assert_eq!(chain, before);
}

// --- Multiple validators (not active in V1, but the format supports it) ----------------

#[test]
fn validators_take_turns_round_robin() {
    let mut w = World::new();
    let second = Keypair::from_secret_bytes(&VALIDATOR_2_SEED);
    w.genesis.validators.push(second.public_key());
    let mut chain = Chain::new(w.genesis.clone()).unwrap();

    // Height 1 belongs to the first validator.
    assert_eq!(
        chain
            .build_block(vec![], GENESIS_TIME + 1, &second)
            .unwrap_err(),
        BlockError::WrongValidator {
            expected: w.validator.public_key()
        }
    );
    produce(&mut chain, &w.validator, vec![]);

    // Height 2 belongs to the second one; a block from the first is refused by replicas too.
    let mut out_of_turn = chain
        .build_block(vec![], chain.tip().timestamp + 1, &second)
        .unwrap();
    out_of_turn.header.validator = w.validator.public_key();
    resign(&mut out_of_turn, &w.validator);
    assert_eq!(
        chain.apply_block(&out_of_turn).unwrap_err(),
        BlockError::WrongValidator {
            expected: second.public_key()
        }
    );

    produce(&mut chain, &second, vec![]);
    produce(&mut chain, &w.validator, vec![]);
    assert_eq!(chain.tip().height, 3);
}
