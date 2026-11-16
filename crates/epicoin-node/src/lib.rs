//! EpiCoin node.
//!
//! - [`store`]: durable storage of blocks, indexes and the state snapshot (redb).
//! - [`mempool`]: pending transactions, validated on arrival.
//! - [`node`]: the chain head, block production and block application.
//! - [`http`]: the internal HTTP API.
//! - [`client`]: a client for that API.
//! - [`replica`]: read-only replication with full re-verification.
//! - [`verify`]: full audit of a chain from its genesis.
//! - [`views`]: JSON shapes shared by the server and the client.

pub mod client;
pub mod http;
pub mod logging;
pub mod mempool;
pub mod node;
pub mod producer;
pub mod replica;
pub mod store;
pub mod verify;
pub mod views;

pub use client::{ClientError, NodeClient};
pub use node::{now_ms, Node, NodeError, NodeOptions};
pub use store::{Store, StoreError};
