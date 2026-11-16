//! Node behaviour: mempool admission, block production, persistence.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use epicoin_core::{Genesis, Hash, Keypair, TxError};
use epicoin_node::store::TxLocation;
use epicoin_node::views::{NodeMode, TxStatus};
use epicoin_node::{Node, NodeError, NodeOptions, StoreError};
use tempfile::TempDir;

#[test]
fn transaction_goes_from_pending_to_included() {
    let dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 2);
    let node = net.validator_node(dir.path());
    assert_eq!(node.mode(), NodeMode::Validator);
    assert_eq!(node.height(), 0);

    let grant = net.grant(0, 10_000);
    let (hash, status) = node.submit(grant.clone()).unwrap();
    assert_eq!(hash, grant.hash());
    assert_eq!(status, TxStatus::Pending);
    assert_eq!(node.tx_status(&hash).unwrap(), TxStatus::Pending);
    assert_eq!(node.status().mempool_size, 1);
    // A pending transaction is visible with its body.
    assert_eq!(node.tx(&hash).unwrap().unwrap().tx, Some(grant));

    let block = node.produce_block(false).unwrap().unwrap();
    assert_eq!(block.header.height, 1);
    assert_eq!(block.txs.len(), 1);
    assert_eq!(
        node.tx_status(&hash).unwrap(),
        TxStatus::Included {
            height: 1,
            index: 0,
            timestamp: block.header.timestamp
        }
    );
    let status = node.status();
    assert_eq!(status.height, 1);
    assert_eq!(status.tip_hash, block.hash());
    assert_eq!(status.mempool_size, 0);
}

#[test]
fn no_block_without_transactions_unless_heartbeat() {
    let dir = TempDir::new().unwrap();
    let net = Net::new(0, 0);
    let node = net.validator_node(dir.path());
    assert!(node.produce_block(false).unwrap().is_none());
    assert_eq!(node.height(), 0);

    let heartbeat = node.produce_block(true).unwrap().unwrap();
    assert!(heartbeat.txs.is_empty());
    assert_eq!(node.height(), 1);
}

#[test]
fn balances_supply_and_quota_are_reported() {
    let dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 2);
    let node = net.validator_node(dir.path());
    include(&node, [net.grant(0, 1_000), net.mint(0, 0, 300)]);
    let purchase = net.purchase(0, 120, 1);
    include(&node, [purchase.clone()]);

    let student = node.account(&net.students[0].address());
    assert_eq!(student.balance, 180);
    assert_eq!(student.nonce, 1);
    assert_eq!(student.pending_nonce, 1);
    assert!(student.authority.is_none());

    let staff = node.account(&net.staff[0].address());
    let authority = staff.authority.unwrap();
    assert!(authority.active);
    assert_eq!(authority.quota, 1_000);
    assert_eq!(authority.minted, 300);
    assert_eq!(authority.remaining, 700);
    assert!(authority.period_ends_at > net.genesis.timestamp);

    let status = node.status();
    assert_eq!(status.total_minted, 300);
    assert_eq!(status.circulating_supply, 300);
    assert_eq!(node.account(&net.treasury.address()).balance, 120);
    assert_eq!(node.authorities().len(), 1);

    // Richest first: student 180, treasury 120, then zero-balance accounts.
    let accounts = node.accounts();
    assert_eq!(accounts[0].address, net.students[0].address());
    assert_eq!(accounts[1].address, net.treasury.address());
}

#[test]
fn pending_nonce_counts_queued_transactions() {
    let dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 3);
    let node = net.validator_node(dir.path());
    include(&node, [net.grant(0, 1_000)]);

    for student in 0..3 {
        node.submit(net.mint(0, student, 10)).unwrap();
    }
    let staff = node.account(&net.staff[0].address());
    assert_eq!(staff.nonce, 0);
    assert_eq!(staff.pending_nonce, 3);

    let block = node.produce_block(false).unwrap().unwrap();
    assert_eq!(block.txs.len(), 3);
    assert_eq!(node.account(&net.staff[0].address()).nonce, 3);
}

#[test]
fn invalid_transactions_are_refused_with_their_reason() {
    let dir = TempDir::new().unwrap();
    let mut net = Net::new(2, 2);
    let node = net.validator_node(dir.path());
    include(&node, [net.grant(0, 100)]);

    // staff[1] was never granted.
    let err = node.submit(net.mint(1, 0, 10)).unwrap_err();
    assert!(matches!(err, NodeError::InvalidTx(TxError::NotAuthority)));

    // Over quota.
    let err = node.submit(net.mint(0, 0, 101)).unwrap_err();
    assert!(matches!(
        err,
        NodeError::InvalidTx(TxError::QuotaExceeded { .. })
    ));

    // No balance.
    let err = node.submit(net.purchase(1, 5, 1)).unwrap_err();
    assert!(matches!(
        err,
        NodeError::InvalidTx(TxError::InsufficientBalance { .. })
    ));

    assert_eq!(node.status().mempool_size, 0);
    assert!(node.produce_block(false).unwrap().is_none());
}

#[test]
fn submitting_twice_is_idempotent() {
    let dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 1);
    let node = net.validator_node(dir.path());
    include(&node, [net.grant(0, 1_000)]);

    let mint = net.mint(0, 0, 50);
    assert_eq!(node.submit(mint.clone()).unwrap().1, TxStatus::Pending);
    assert_eq!(node.submit(mint.clone()).unwrap().1, TxStatus::Pending);
    assert_eq!(node.status().mempool_size, 1);

    node.produce_block(false).unwrap().unwrap();
    // Retrying after inclusion reports the inclusion instead of a nonce error…
    let (_, status) = node.submit(mint).unwrap();
    assert!(matches!(status, TxStatus::Included { height: 2, .. }));
    // …and nothing was minted twice.
    assert_eq!(node.account(&net.students[0].address()).balance, 50);
    assert!(node.produce_block(false).unwrap().is_none());
}

#[test]
fn blocks_are_capped_and_the_rest_waits() {
    let dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 1);
    let node = Node::open(
        dir.path(),
        &net.genesis,
        Some(net.validator.clone()),
        NodeOptions {
            max_block_txs: 3,
            mempool_capacity: 5,
        },
    )
    .unwrap();

    node.submit(net.grant(0, 1_000)).unwrap();
    for _ in 0..4 {
        node.submit(net.mint(0, 0, 1)).unwrap();
    }
    let overflow = node.submit(net.mint(0, 0, 1)).unwrap_err();
    assert!(matches!(overflow, NodeError::MempoolFull));

    assert_eq!(node.produce_block(false).unwrap().unwrap().txs.len(), 3);
    assert_eq!(node.status().mempool_size, 2);
    assert_eq!(node.produce_block(false).unwrap().unwrap().txs.len(), 2);
    assert_eq!(node.account(&net.students[0].address()).balance, 4);
}

#[test]
fn unknown_transaction_has_unknown_status() {
    let dir = TempDir::new().unwrap();
    let net = Net::new(0, 0);
    let node = net.validator_node(dir.path());
    let hash = Hash([7; 32]);
    assert_eq!(node.tx_status(&hash).unwrap(), TxStatus::Unknown);
    assert!(node.tx(&hash).unwrap().is_none());
}

#[test]
fn history_is_indexed_by_address() {
    let dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 2);
    let node = net.validator_node(dir.path());
    include(&node, [net.grant(0, 1_000)]);
    include(&node, [net.mint(0, 0, 300), net.mint(0, 1, 200)]);
    let purchase = net.purchase(0, 100, 1);
    include(&node, [purchase.clone()]);
    include(&node, [net.refund(purchase.hash())]);

    // Student 0: mint, purchase, refund — newest first.
    let history = node
        .address_txs(&net.students[0].address(), None, 10)
        .unwrap();
    let heights: Vec<u64> = history
        .iter()
        .map(|view| match view.status {
            TxStatus::Included { height, .. } => height,
            _ => panic!("history only contains included transactions"),
        })
        .collect();
    assert_eq!(heights, vec![4, 3, 2]);

    // Student 1 only received a mint.
    assert_eq!(
        node.address_txs(&net.students[1].address(), None, 10)
            .unwrap()
            .len(),
        1
    );
    // The treasury sees the purchase and its refund.
    assert_eq!(
        node.address_txs(&net.treasury.address(), None, 10)
            .unwrap()
            .len(),
        2
    );
    // The staff member: granted, then two mints.
    assert_eq!(
        node.address_txs(&net.staff[0].address(), None, 10)
            .unwrap()
            .len(),
        3
    );

    // Paging.
    let first = node
        .address_txs(&net.students[0].address(), None, 2)
        .unwrap();
    assert_eq!(first.len(), 2);
    let rest = node
        .address_txs(
            &net.students[0].address(),
            Some(TxLocation {
                height: 3,
                index: 0,
            }),
            2,
        )
        .unwrap();
    assert_eq!(rest.len(), 1);
    assert_eq!(rest[0].hash, history[2].hash);

    // Recent transactions across the chain, newest first.
    let recent = node.recent_txs(3).unwrap();
    assert_eq!(recent.len(), 3);
    assert_eq!(recent[1].hash, purchase.hash());
    assert_eq!(node.recent_txs(100).unwrap().len(), 5);

    // Blocks, newest first.
    let blocks = node.blocks_desc(None, 2).unwrap();
    assert_eq!(blocks[0].header.height, 4);
    assert_eq!(blocks[1].header.height, 3);
    assert_eq!(
        node.block_by_hash(&blocks[1].hash())
            .unwrap()
            .unwrap()
            .header
            .height,
        3
    );
}

// --- Persistence -----------------------------------------------------------------------

#[test]
fn restart_keeps_the_chain_and_the_state() {
    let dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 2);

    let purchase = net.purchase(0, 100, 1);
    let (status, chain) = {
        let node = net.validator_node(dir.path());
        include(&node, [net.grant(0, 1_000), net.mint(0, 0, 300)]);
        include(&node, [purchase.clone()]);
        (node.status(), node.chain())
    };

    let node = net.validator_node(dir.path());
    assert_eq!(node.status(), status);
    assert_eq!(node.chain(), chain);
    assert_eq!(node.account(&net.students[0].address()).balance, 200);
    assert_eq!(node.account(&net.treasury.address()).balance, 100);
    assert!(matches!(
        node.tx_status(&purchase.hash()).unwrap(),
        TxStatus::Included { height: 2, .. }
    ));

    // And the chain keeps growing from where it stopped.
    let block = include(&node, [net.refund(purchase.hash())]);
    assert_eq!(block.header.height, 3);
    assert_eq!(block.header.prev_hash, status.tip_hash);
    assert_eq!(node.account(&net.students[0].address()).balance, 300);
}

#[test]
fn pending_transactions_do_not_survive_a_restart() {
    // The mempool is in memory only: callers resubmit (submission is idempotent).
    let dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 1);
    let grant = net.grant(0, 1_000);
    {
        let node = net.validator_node(dir.path());
        node.submit(grant.clone()).unwrap();
    }
    let node = net.validator_node(dir.path());
    assert_eq!(node.status().mempool_size, 0);
    assert_eq!(node.tx_status(&grant.hash()).unwrap(), TxStatus::Unknown);
    assert_eq!(node.submit(grant).unwrap().1, TxStatus::Pending);
}

#[test]
fn database_of_another_chain_is_refused() {
    let dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 1);
    {
        let node = net.validator_node(dir.path());
        include(&node, [net.grant(0, 1_000)]);
    }
    let other = Genesis {
        chain_id: "epicoin-other".into(),
        ..net.genesis.clone()
    };
    let result = Node::open(
        dir.path(),
        &other,
        Some(net.validator.clone()),
        NodeOptions::default(),
    );
    assert!(matches!(
        result,
        Err(NodeError::Store(StoreError::GenesisMismatch))
    ));
}

// --- Roles -----------------------------------------------------------------------------

#[test]
fn only_a_declared_validator_can_run_a_validator_node() {
    let dir = TempDir::new().unwrap();
    let net = Net::new(0, 0);
    let intruder = Keypair::from_secret_bytes(&[0x66; 32]);
    let result = Node::open(
        dir.path(),
        &net.genesis,
        Some(intruder),
        NodeOptions::default(),
    );
    assert!(matches!(result, Err(NodeError::NotAValidator)));
}

#[test]
fn replica_is_read_only() {
    let dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 1);
    let replica = net.replica_node(dir.path());
    assert_eq!(replica.mode(), NodeMode::Replica);
    assert!(matches!(
        replica.submit(net.grant(0, 1)).unwrap_err(),
        NodeError::ReadOnly
    ));
    assert!(matches!(
        replica.produce_block(true).unwrap_err(),
        NodeError::ReadOnly
    ));
}

#[test]
fn replica_applies_valid_blocks_and_rejects_others() {
    let producer_dir = TempDir::new().unwrap();
    let replica_dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 1);
    let producer = net.validator_node(producer_dir.path());
    let replica = net.replica_node(replica_dir.path());

    let b1 = include(&producer, [net.grant(0, 1_000)]);
    let b2 = include(&producer, [net.mint(0, 0, 42)]);

    // Out of order: refused, nothing stored.
    assert!(matches!(
        replica.apply_block(&b2).unwrap_err(),
        NodeError::InvalidBlock(_)
    ));
    assert_eq!(replica.height(), 0);

    replica.apply_block(&b1).unwrap();

    // Tampered: refused.
    let mut forged = b2.clone();
    forged.txs[0] = net.mint(0, 0, 999_999);
    assert!(matches!(
        replica.apply_block(&forged).unwrap_err(),
        NodeError::InvalidBlock(_)
    ));
    assert_eq!(replica.height(), 1);

    replica.apply_block(&b2).unwrap();
    assert_eq!(replica.status().state_root, producer.status().state_root);
    assert_eq!(replica.account(&net.students[0].address()).balance, 42);
}

// --- Waiting for inclusion -------------------------------------------------------------

#[tokio::test]
async fn wait_for_tx_resolves_when_the_block_is_produced() {
    let dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 1);
    let node = Arc::new(net.validator_node(dir.path()));

    let (hash, _) = node.submit(net.grant(0, 1_000)).unwrap();

    // Nothing produced yet: the wait times out and reports "pending".
    let status = node
        .wait_for_tx(&hash, Duration::from_millis(50))
        .await
        .unwrap();
    assert_eq!(status, TxStatus::Pending);

    let producer = node.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        producer.produce_block(false).unwrap();
    });
    let status = node
        .wait_for_tx(&hash, Duration::from_secs(10))
        .await
        .unwrap();
    assert!(matches!(status, TxStatus::Included { height: 1, .. }));

    // An unknown transaction is reported at once, without waiting.
    let status = node
        .wait_for_tx(&Hash([9; 32]), Duration::from_secs(10))
        .await
        .unwrap();
    assert_eq!(status, TxStatus::Unknown);
}
