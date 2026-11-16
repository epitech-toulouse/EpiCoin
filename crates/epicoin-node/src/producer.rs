//! The block production loop of a validator.

use std::sync::Arc;
use std::time::Duration;

use tokio::time::{Instant, MissedTickBehavior};

use crate::node::Node;

/// Every `interval`, produces a block if transactions are waiting. With `heartbeat`, also
/// produces an empty block when none was produced for that long.
pub async fn run(node: Arc<Node>, interval: Duration, heartbeat: Option<Duration>) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut last_block = Instant::now();
    loop {
        ticker.tick().await;
        let allow_empty = heartbeat.is_some_and(|h| last_block.elapsed() >= h);
        match node.produce_block(allow_empty) {
            Ok(Some(block)) => {
                last_block = Instant::now();
                tracing::info!(
                    height = block.header.height,
                    txs = block.txs.len(),
                    hash = %block.hash(),
                    "block produced"
                );
            }
            Ok(None) => {}
            Err(error) => tracing::error!(%error, "block production failed"),
        }
    }
}
