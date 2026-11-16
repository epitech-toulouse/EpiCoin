//! The HTTP API end to end, and replication over it.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::*;
use epicoin_core::{Genesis, Hash, TxError};
use epicoin_node::replica::{check_upstream, sync_once, ReplicaError};
use epicoin_node::verify::Replayer;
use epicoin_node::views::{NodeMode, TxStatus};
use epicoin_node::{producer, ClientError, Node};
use tempfile::TempDir;

/// A validator node behind HTTP with a few blocks: grant, two mints, a purchase.
async fn running_node(net: &mut Net, dir: &TempDir) -> (Arc<Node>, epicoin_node::NodeClient) {
    let node = Arc::new(net.validator_node(dir.path()));
    include(&node, [net.grant(0, 10_000)]);
    include(&node, [net.mint(0, 0, 500), net.mint(0, 1, 300)]);
    include(&node, [net.purchase(0, 120, 1)]);
    let client = serve(node.clone()).await;
    (node, client)
}

#[tokio::test]
async fn read_endpoints_describe_the_chain() {
    let dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 2);
    let (node, client) = running_node(&mut net, &dir).await;

    client.health().await.unwrap();

    let status = client.status().await.unwrap();
    assert_eq!(status, node.status());
    assert_eq!(status.mode, NodeMode::Validator);
    assert_eq!(status.height, 3);
    assert_eq!(status.chain_id, CHAIN_ID);
    assert_eq!(status.treasury, net.treasury.address());

    assert_eq!(client.genesis().await.unwrap(), net.genesis);

    let student = client.account(&net.students[0].address()).await.unwrap();
    assert_eq!(student.balance, 380);
    assert_eq!(student.nonce, 1);

    let accounts = client.accounts().await.unwrap();
    assert_eq!(accounts[0].address, net.students[0].address());

    let authorities = client.authorities().await.unwrap();
    assert_eq!(authorities.len(), 1);
    assert_eq!(authorities[0].remaining, 9_200);

    let blocks = client.blocks(None, 10).await.unwrap();
    assert_eq!(
        blocks.iter().map(|b| b.height).collect::<Vec<_>>(),
        vec![3, 2, 1]
    );
    assert_eq!(blocks[1].tx_count, 2);
    let older = client.blocks(Some(2), 10).await.unwrap();
    assert_eq!(older.iter().map(|b| b.height).collect::<Vec<_>>(), vec![1]);
    assert!(client.blocks(Some(1), 10).await.unwrap().is_empty());

    let by_height = client.block("2").await.unwrap().unwrap();
    assert_eq!(by_height.txs.len(), 2);
    let by_hash = client
        .block(&by_height.summary.hash.to_string())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(by_hash, by_height);
    assert!(client.block("99").await.unwrap().is_none());

    let tx_hash = by_height.txs[0].hash;
    let tx = client.tx(&tx_hash).await.unwrap().unwrap();
    assert!(matches!(
        tx.status,
        TxStatus::Included {
            height: 2,
            index: 0,
            ..
        }
    ));
    assert!(client.tx(&Hash([1; 32])).await.unwrap().is_none());

    assert_eq!(client.recent_txs(2).await.unwrap().len(), 2);
    assert_eq!(
        client
            .account_txs(&net.students[0].address(), 10)
            .await
            .unwrap()
            .len(),
        2
    );
}

#[tokio::test]
async fn json_never_carries_what_borsh_signed_differently() {
    // The JSON view of a transaction round-trips to the very same signed transaction.
    let dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 2);
    let (_, client) = running_node(&mut net, &dir).await;
    let block = client.block("3").await.unwrap().unwrap();
    let view = &block.txs[0];
    let tx = view.tx.as_ref().unwrap();
    assert_eq!(tx.hash(), view.hash);
    assert_eq!(tx.verify_signature(CHAIN_ID), Ok(()));
}

#[tokio::test]
async fn submit_then_wait_for_inclusion() {
    let dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 2);
    let (node, client) = running_node(&mut net, &dir).await;
    tokio::spawn(producer::run(node.clone(), Duration::from_millis(20), None));

    // Without waiting: pending.
    let mint = net.mint(0, 1, 50);
    let view = client.submit(&mint, None).await.unwrap();
    assert_eq!(view.hash, mint.hash());
    assert_eq!(view.status, TxStatus::Pending);

    // Waiting: the call returns once the transaction is in a block.
    let purchase = net.purchase(1, 30, 2);
    let view = client
        .submit(&purchase, Some(Duration::from_secs(10)))
        .await
        .unwrap();
    assert!(matches!(view.status, TxStatus::Included { .. }));
    assert_eq!(node.account(&net.students[1].address()).balance, 320);

    // Retrying the same transaction is harmless.
    let again = client
        .submit(&purchase, Some(Duration::from_secs(10)))
        .await
        .unwrap();
    assert_eq!(again.status, view.status);
    assert_eq!(node.account(&net.students[1].address()).balance, 320);
}

#[tokio::test]
async fn invalid_submissions_are_rejected_with_a_reason() {
    let dir = TempDir::new().unwrap();
    let mut net = Net::new(2, 2);
    let (_, client) = running_node(&mut net, &dir).await;

    // A student trying to mint: students are not authorities.
    let forged = epicoin_core::Transaction::sign(
        CHAIN_ID,
        &net.students[0],
        1,
        epicoin_core::TxPayload::Mint {
            to: net.students[0].address(),
            amount: 1_000_000,
            reason: epicoin_core::ReasonCode::Other,
        },
    );
    assert!(matches!(
        client.submit(&forged, None).await.unwrap_err(),
        ClientError::Rejected(TxError::NotAuthority)
    ));

    // Not enough balance.
    let too_much = net.purchase(1, 5_000, 9);
    assert!(matches!(
        client.submit(&too_much, None).await.unwrap_err(),
        ClientError::Rejected(TxError::InsufficientBalance {
            balance: 300,
            needed: 5_000
        })
    ));
}

#[tokio::test]
async fn malformed_requests_get_clean_errors() {
    let dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 2);
    let (node, client) = running_node(&mut net, &dir).await;
    let base = format!("http://{}", serve_address(node).await);
    let http = reqwest::Client::new();

    let response = http
        .post(format!("{base}/tx"))
        .body("definitely not borsh")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"], "bad_request");

    let response = http.get(format!("{base}/tx/zzzz")).send().await.unwrap();
    assert_eq!(response.status(), 400);

    let response = http
        .get(format!("{base}/accounts/not-an-address"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);

    let response = http
        .get(format!("{base}/tx/{}", Hash([3; 32])))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"], "not_found");

    // Oversized bodies are cut off.
    let response = http
        .post(format!("{base}/tx"))
        .body(vec![0u8; 64 * 1024])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 413);

    // The client still works afterwards.
    client.health().await.unwrap();
}

/// Serves `node` and returns the socket address (for raw HTTP requests).
async fn serve_address(node: Arc<Node>) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, epicoin_node::http::router(node))
            .await
            .unwrap();
    });
    address
}

// --- Replication -----------------------------------------------------------------------

#[tokio::test]
async fn replica_syncs_and_reverifies_the_whole_chain() {
    let dir = TempDir::new().unwrap();
    let replica_dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 2);
    let (node, client) = running_node(&mut net, &dir).await;
    let replica = Arc::new(net.replica_node(replica_dir.path()));

    check_upstream(&replica, &client).await.unwrap();
    assert_eq!(sync_once(&replica, &client).await.unwrap(), 3);
    assert_eq!(replica.status().state_root, node.status().state_root);
    assert_eq!(replica.status().tip_hash, node.status().tip_hash);
    assert_eq!(replica.account(&net.students[0].address()).balance, 380);

    // Nothing new: nothing applied.
    assert_eq!(sync_once(&replica, &client).await.unwrap(), 0);

    // The chain grows; the replica catches up.
    include(&node, [net.mint(0, 1, 7)]);
    include(&node, [net.purchase(1, 5, 2)]);
    assert_eq!(sync_once(&replica, &client).await.unwrap(), 2);
    assert_eq!(replica.status().state_root, node.status().state_root);

    // The replica serves the same data over its own API, but refuses writes.
    let replica_client = serve(replica.clone()).await;
    let status = replica_client.status().await.unwrap();
    assert_eq!(status.mode, NodeMode::Replica);
    assert_eq!(status.height, 5);
    // (signed outside the fixture's nonce bookkeeping: it will never be included)
    let never_included = epicoin_core::Transaction::sign(
        CHAIN_ID,
        &net.admin,
        1,
        epicoin_core::TxPayload::AuthorityUpdate {
            authority: net.staff[0].address(),
            action: epicoin_core::AuthorityAction::Revoke,
        },
    );
    let refused = replica_client
        .submit(&never_included, None)
        .await
        .unwrap_err();
    assert!(matches!(refused, ClientError::Api { status: 403, .. }));
}

#[tokio::test]
async fn replica_resumes_after_a_restart() {
    let dir = TempDir::new().unwrap();
    let replica_dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 2);
    let (node, client) = running_node(&mut net, &dir).await;

    {
        let replica = net.replica_node(replica_dir.path());
        assert_eq!(sync_once(&replica, &client).await.unwrap(), 3);
    }
    include(&node, [net.mint(0, 0, 1)]);

    let replica = net.replica_node(replica_dir.path());
    assert_eq!(replica.height(), 3);
    assert_eq!(sync_once(&replica, &client).await.unwrap(), 1);
    assert_eq!(replica.status().state_root, node.status().state_root);
}

#[tokio::test]
async fn replica_syncs_more_than_one_batch() {
    let dir = TempDir::new().unwrap();
    let replica_dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 1);
    let node = Arc::new(net.validator_node(dir.path()));
    include(&node, [net.grant(0, 1_000_000)]);
    // More blocks than a single sync request returns.
    let blocks = epicoin_node::http::MAX_SYNC_BATCH as u64 + 30;
    for _ in 1..blocks {
        include(&node, [net.mint(0, 0, 1)]);
    }
    let client = serve(node.clone()).await;
    let replica = net.replica_node(replica_dir.path());
    assert_eq!(sync_once(&replica, &client).await.unwrap(), blocks);
    assert_eq!(replica.status().state_root, node.status().state_root);
}

#[tokio::test]
async fn replica_refuses_another_chain() {
    let dir = TempDir::new().unwrap();
    let replica_dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 2);
    let (_, client) = running_node(&mut net, &dir).await;

    let mut other = Net::new(1, 2);
    other.genesis = Genesis {
        chain_id: "epicoin-other".into(),
        ..net.genesis.clone()
    };
    let replica = other.replica_node(replica_dir.path());
    assert!(matches!(
        check_upstream(&replica, &client).await.unwrap_err(),
        ReplicaError::WrongChain
    ));
    // Even without the handshake, no block of the other chain can be applied.
    assert!(matches!(
        sync_once(&replica, &client).await.unwrap_err(),
        ReplicaError::InvalidBlock { height: 1, .. }
    ));
    assert_eq!(replica.height(), 0);
}

#[tokio::test]
async fn replica_stops_at_a_tampered_block() {
    let dir = TempDir::new().unwrap();
    let replica_dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 2);
    {
        let node = net.validator_node(dir.path());
        include(&node, [net.grant(0, 10_000)]);
        include(&node, [net.mint(0, 0, 500)]);
        include(&node, [net.mint(0, 1, 300)]);
    }
    // The upstream's disk is tampered with while it is stopped. Its cheap startup check
    // only looks at the tip, so it starts and serves the altered history.
    let forged = net.mint(0, 0, 999_999);
    tamper_block(dir.path(), 2, |block| block.txs[0] = forged);
    let upstream = Arc::new(net.validator_node(dir.path()));
    let client = serve(upstream).await;

    let replica = net.replica_node(replica_dir.path());
    assert!(matches!(
        sync_once(&replica, &client).await.unwrap_err(),
        ReplicaError::InvalidBlock { height: 2, .. }
    ));
    // The replica kept only what it could verify.
    assert_eq!(replica.height(), 1);
    assert_eq!(replica.account(&net.students[0].address()).balance, 0);
}

#[tokio::test]
async fn chain_can_be_audited_remotely() {
    // What `epicoin-cli verify --node` does: download raw blocks and replay them.
    let dir = TempDir::new().unwrap();
    let mut net = Net::new(1, 2);
    let (node, client) = running_node(&mut net, &dir).await;

    let mut replayer = Replayer::new(&net.genesis).unwrap();
    loop {
        let blocks = client.sync_blocks(replayer.height() + 1, 2).await.unwrap();
        if blocks.is_empty() {
            break;
        }
        for block in &blocks {
            replayer.apply(block).unwrap();
        }
    }
    let (_, report) = replayer.finish().unwrap();
    assert_eq!(report.height, 3);
    assert_eq!(report.transactions, 4);
    assert_eq!(report.state_root, node.status().state_root);
}
