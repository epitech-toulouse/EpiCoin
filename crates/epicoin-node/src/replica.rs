//! Read-only replication: follow an upstream node over HTTP and re-verify every block.
//!
//! A replica trusts nothing but its own genesis file. Each block goes through
//! `Chain::apply_block` — hashes, validator signature, every transaction, state root — before
//! being stored, so a replica that is in sync is also a continuous audit of the chain.

use std::sync::Arc;
use std::time::Duration;

use crate::client::{ClientError, NodeClient};
use crate::http::MAX_SYNC_BATCH;
use crate::node::{Node, NodeError};

#[derive(Debug, thiserror::Error)]
pub enum ReplicaError {
    #[error("upstream unreachable: {0}")]
    Upstream(#[from] ClientError),
    #[error("upstream follows a different chain (genesis hash mismatch)")]
    WrongChain,
    #[error("upstream served an invalid block at height {height}: {source}")]
    InvalidBlock { height: u64, source: NodeError },
}

/// Checks that the upstream node serves the same chain as ours.
pub async fn check_upstream(node: &Node, upstream: &NodeClient) -> Result<(), ReplicaError> {
    let status = upstream.status().await?;
    if status.genesis_hash != node.genesis().hash() {
        return Err(ReplicaError::WrongChain);
    }
    Ok(())
}

/// Fetches and applies every block the upstream has beyond our tip. Returns how many blocks
/// were applied.
pub async fn sync_once(node: &Node, upstream: &NodeClient) -> Result<u64, ReplicaError> {
    let mut applied = 0;
    loop {
        let blocks = upstream
            .sync_blocks(node.height() + 1, MAX_SYNC_BATCH)
            .await?;
        if blocks.is_empty() {
            return Ok(applied);
        }
        for block in &blocks {
            node.apply_block(block)
                .map_err(|source| ReplicaError::InvalidBlock {
                    height: block.header.height,
                    source,
                })?;
            applied += 1;
        }
    }
}

/// Follows `upstream` forever, polling every `poll`.
pub async fn run(node: Arc<Node>, upstream: NodeClient, poll: Duration) {
    let mut chain_checked = false;
    loop {
        if !chain_checked {
            match check_upstream(&node, &upstream).await {
                Ok(()) => chain_checked = true,
                Err(error) => tracing::warn!(%error, "cannot validate the upstream node"),
            }
        }
        if chain_checked {
            match sync_once(&node, &upstream).await {
                Ok(0) => {}
                Ok(applied) => {
                    tracing::info!(applied, height = node.height(), "synced blocks");
                }
                Err(error @ ReplicaError::InvalidBlock { .. }) => {
                    // Never store what we cannot verify; keep serving our verified prefix.
                    tracing::error!(%error, "replication halted on an invalid block");
                }
                Err(error) => tracing::warn!(%error, "sync failed, will retry"),
            }
        }
        tokio::time::sleep(poll).await;
    }
}
