//! Phase 2 acceptance: a 10 000-transaction chain is verified end to end, and any tampering
//! with the stored chain is detected.

mod common;

use common::*;
use epicoin_core::{BlockError, Chain, Hash};
use epicoin_node::store::DB_FILE;
use epicoin_node::verify::{verify_store, VerifyError};
use epicoin_node::{Node, NodeError, NodeOptions, Store, StoreError};
use tempfile::TempDir;

/// Opens the store of a stopped node and audits it.
fn audit(net: &Net, dir: &TempDir) -> Result<epicoin_node::verify::VerifyReport, VerifyError> {
    let (store, _) = Store::open(&dir.path().join(DB_FILE), &net.genesis)?;
    verify_store(&store, &net.genesis)
}

/// A short chain with every kind of transaction, then the node is stopped.
fn small_chain() -> (Net, TempDir) {
    let dir = TempDir::new().unwrap();
    let mut net = Net::new(2, 4);
    {
        let node = net.validator_node(dir.path());
        include(&node, [net.grant(0, 10_000), net.grant(1, 10_000)]);
        include(
            &node,
            [
                net.mint(0, 0, 500),
                net.mint(0, 1, 400),
                net.mint(1, 2, 300),
            ],
        );
        let p1 = net.purchase(0, 150, 1);
        let p2 = net.purchase(1, 50, 2);
        include(&node, [p1.clone(), p2]);
        include(&node, [net.refund(p1.hash())]);
        include(&node, [net.mint(1, 3, 20)]);
    }
    (net, dir)
}

#[test]
fn chain_of_10_000_transactions_is_verified_end_to_end() {
    const STAFF: usize = 5;
    const STUDENTS: usize = 150;
    const MINTS: usize = 4_000;
    const PURCHASES: usize = 4_000;
    const REFUNDS: usize = 1_995;
    const TOTAL: usize = STAFF + MINTS + PURCHASES + REFUNDS;
    const BLOCK_EVERY: usize = 400;
    assert_eq!(TOTAL, 10_000);

    let dir = TempDir::new().unwrap();
    let mut net = Net::new(STAFF as u16, STUDENTS as u16);
    let mut submitted = 0usize;
    let mut purchases: Vec<Hash> = Vec::new();

    let status = {
        let node = net.validator_node(dir.path());
        let mut submit = |node: &Node, tx| {
            node.submit(tx).expect("transaction accepted");
            submitted += 1;
            if submitted.is_multiple_of(BLOCK_EVERY) {
                node.produce_block(false).unwrap().unwrap();
            }
        };

        for staff in 0..STAFF {
            submit(&node, net.grant(staff, 10_000_000));
        }
        for i in 0..MINTS {
            submit(&node, net.mint(i % STAFF, i % STUDENTS, 1_000));
        }
        for i in 0..PURCHASES {
            let tx = net.purchase(i % STUDENTS, 100 + (i % 7) as u64, i as u64);
            purchases.push(tx.hash());
            submit(&node, tx);
        }
        for hash in purchases.iter().take(REFUNDS) {
            submit(&node, net.refund(*hash));
        }
        // Whatever is left in the mempool.
        node.produce_block(false).unwrap();
        assert_eq!(node.status().mempool_size, 0);
        node.status()
    };
    assert_eq!(submitted, TOTAL);

    let report = audit(&net, &dir).expect("the chain verifies");
    assert_eq!(report.transactions, TOTAL as u64);
    assert_eq!(report.height, status.height);
    assert_eq!(report.height, (TOTAL / BLOCK_EVERY) as u64);
    assert_eq!(report.tip_hash, status.tip_hash);
    assert_eq!(report.state_root, status.state_root);
    assert_eq!(report.total_minted, MINTS as u64 * 1_000);
    assert_eq!(report.circulating_supply, report.total_minted);

    // Independent bookkeeping of what the treasury should hold.
    let spent: u64 = (0..PURCHASES).map(|i| 100 + (i % 7) as u64).sum();
    let refunded: u64 = (0..REFUNDS).map(|i| 100 + (i % 7) as u64).sum();
    let node = net.validator_node(dir.path());
    assert_eq!(
        node.account(&net.treasury.address()).balance,
        spent - refunded
    );
    let students: u64 = net
        .students
        .iter()
        .map(|s| node.account(&s.address()).balance)
        .sum();
    assert_eq!(students, report.total_minted - (spent - refunded));
}

#[test]
fn untouched_chain_verifies() {
    let (net, dir) = small_chain();
    let report = audit(&net, &dir).unwrap();
    assert_eq!(report.height, 5);
    assert_eq!(report.transactions, 9);
    assert_eq!(report.total_minted, 1_220);
    assert_eq!(report.chain_id, CHAIN_ID);
}

#[test]
fn empty_chain_verifies() {
    let dir = TempDir::new().unwrap();
    let net = Net::new(0, 0);
    drop(net.validator_node(dir.path()));
    let report = audit(&net, &dir).unwrap();
    assert_eq!(report.height, 0);
    assert_eq!(report.transactions, 0);
}

#[test]
fn altered_amount_is_detected() {
    let (mut net, dir) = small_chain();
    // Someone edits a mint in block 2 to give themselves more.
    let forged = net.mint(0, 0, 999_999);
    tamper_block(dir.path(), 2, |block| block.txs[0] = forged);
    assert!(matches!(
        audit(&net, &dir),
        Err(VerifyError::Block {
            height: 2,
            source: BlockError::BadTxRoot
        })
    ));
}

#[test]
fn single_flipped_bit_is_detected() {
    let (net, dir) = small_chain();
    tamper_block(dir.path(), 3, |block| block.txs[1].signature.0[63] ^= 1);
    assert!(matches!(
        audit(&net, &dir),
        Err(VerifyError::Block { height: 3, .. })
    ));
}

#[test]
fn altered_header_is_detected() {
    let (net, dir) = small_chain();
    tamper_block(dir.path(), 4, |block| block.header.timestamp += 1);
    assert!(matches!(
        audit(&net, &dir),
        Err(VerifyError::Block {
            height: 4,
            source: BlockError::BadSignature
        })
    ));
}

#[test]
fn removed_transaction_is_detected() {
    let (net, dir) = small_chain();
    tamper_block(dir.path(), 2, |block| {
        block.txs.pop();
    });
    assert!(matches!(
        audit(&net, &dir),
        Err(VerifyError::Block {
            height: 2,
            source: BlockError::BadTxRoot
        })
    ));
}

#[test]
fn block_swapped_for_another_height_is_detected() {
    let (net, dir) = small_chain();
    let (store, _) = Store::open(&dir.path().join(DB_FILE), &net.genesis).unwrap();
    let block_4 = store.block(4).unwrap().unwrap();
    drop(store);
    tamper_block(dir.path(), 3, |block| *block = block_4);
    assert!(matches!(
        audit(&net, &dir),
        Err(VerifyError::Block {
            height: 3,
            source: BlockError::BadHeight { .. }
        })
    ));
}

#[test]
fn missing_middle_block_is_detected() {
    let (net, dir) = small_chain();
    delete_block(dir.path(), 3);
    assert!(matches!(
        audit(&net, &dir),
        Err(VerifyError::Block {
            height: 3,
            source: BlockError::BadHeight { .. }
        })
    ));
}

#[test]
fn truncated_chain_is_refused_at_startup() {
    let (net, dir) = small_chain();
    delete_block(dir.path(), 5);
    // The snapshot is now ahead of the blocks: the node refuses to start on it.
    let result = Node::open(
        dir.path(),
        &net.genesis,
        Some(net.validator.clone()),
        NodeOptions::default(),
    );
    assert!(matches!(
        result,
        Err(NodeError::Store(StoreError::Corrupt(_)))
    ));
}

#[test]
fn forged_state_snapshot_is_refused_at_startup() {
    let (net, dir) = small_chain();

    // A snapshot from a different history of the same chain (other balances).
    let other_dir = TempDir::new().unwrap();
    let mut other = Net::new(2, 4);
    other.genesis = net.genesis.clone();
    {
        let node = other.validator_node(other_dir.path());
        for _ in 0..5 {
            include(&node, [other.grant(0, 1)]);
        }
    }
    write_snapshot(dir.path(), &read_snapshot(other_dir.path()));
    assert!(matches!(
        Store::open(&dir.path().join(DB_FILE), &net.genesis),
        Err(StoreError::Corrupt(_))
    ));

    // Garbage instead of a snapshot.
    write_snapshot(dir.path(), b"not a snapshot");
    assert!(matches!(
        Store::open(&dir.path().join(DB_FILE), &net.genesis),
        Err(StoreError::Corrupt(_))
    ));
}

#[test]
fn rolled_back_snapshot_is_refused_at_startup() {
    let dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 1);
    let old_snapshot = {
        let node = net.validator_node(dir.path());
        include(&node, [net.grant(0, 1_000)]);
        drop(node);
        let snapshot = read_snapshot(dir.path());
        let node = net.validator_node(dir.path());
        include(&node, [net.mint(0, 0, 500)]);
        snapshot
    };
    // Restoring the older state would erase the mint from the balances.
    write_snapshot(dir.path(), &old_snapshot);
    assert!(matches!(
        Store::open(&dir.path().join(DB_FILE), &net.genesis),
        Err(StoreError::Corrupt(_))
    ));
    // Sanity: the old snapshot really was a valid chain head.
    assert!(borsh::from_slice::<Chain>(&old_snapshot).is_ok());
}
