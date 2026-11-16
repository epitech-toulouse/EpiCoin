//! The node's internal HTTP API.
//!
//! Not meant to be exposed publicly: it is reached by `epicoin-api`, by replicas and by the
//! CLI over a private network. Writes are self-authenticating (a transaction is only
//! accepted with a valid signature), so there is no API token.
//!
//! Transactions travel as canonical borsh bytes; JSON is only used for read-only views.

use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use epicoin_core::{Address, Genesis, Hash, Transaction, TxError};
use serde::Deserialize;
use tower_http::trace::TraceLayer;

use crate::node::{Node, NodeError};
use crate::store::TxLocation;
use crate::views::{
    AccountView, AuthorityView, BlockSummary, BlockView, ErrorView, StatusView, SubmitView,
    TxStatus, TxView,
};

/// Longest time a `POST /tx?wait_ms=` request may wait for inclusion.
pub const MAX_WAIT: Duration = Duration::from_secs(30);
/// Largest page of JSON items.
const MAX_PAGE: usize = 100;
/// Largest batch of raw blocks served to a replica.
pub const MAX_SYNC_BATCH: usize = 200;
/// A transaction is about 250 bytes; anything much larger is garbage.
const MAX_TX_BYTES: usize = 4 * 1024;

pub fn router(node: Arc<Node>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/status", get(status))
        .route("/genesis", get(genesis))
        .route("/tx", post(submit_tx))
        .route("/tx/{hash}", get(get_tx))
        .route("/txs", get(recent_txs))
        .route("/blocks", get(list_blocks))
        .route("/blocks/{id}", get(get_block))
        .route("/sync/blocks", get(sync_blocks))
        .route("/accounts", get(list_accounts))
        .route("/accounts/{address}", get(get_account))
        .route("/accounts/{address}/txs", get(account_txs))
        .route("/authorities", get(list_authorities))
        .layer(DefaultBodyLimit::max(MAX_TX_BYTES))
        .layer(TraceLayer::new_for_http())
        .with_state(node)
}

// --- Errors ----------------------------------------------------------------------------

#[derive(Debug)]
pub struct ApiError {
    status: StatusCode,
    body: ErrorView,
}

impl ApiError {
    fn new(status: StatusCode, code: &str, message: impl Into<String>) -> Self {
        Self {
            status,
            body: ErrorView {
                error: code.into(),
                message: message.into(),
                detail: None,
            },
        }
    }

    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", message)
    }

    fn not_found(what: &str) -> Self {
        Self::new(
            StatusCode::NOT_FOUND,
            "not_found",
            format!("{what} not found"),
        )
    }

    fn invalid_tx(error: TxError) -> Self {
        Self {
            status: StatusCode::UNPROCESSABLE_ENTITY,
            body: ErrorView {
                error: "invalid_transaction".into(),
                message: error.to_string(),
                detail: Some(error),
            },
        }
    }
}

impl From<NodeError> for ApiError {
    fn from(error: NodeError) -> Self {
        match error {
            NodeError::InvalidTx(error) => Self::invalid_tx(error),
            NodeError::ReadOnly => Self::new(StatusCode::FORBIDDEN, "read_only", error.to_string()),
            NodeError::MempoolFull => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "mempool_full",
                error.to_string(),
            ),
            other => {
                tracing::error!(error = %other, "internal error");
                Self::new(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal",
                    "internal error",
                )
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.status, Json(self.body)).into_response()
    }
}

type ApiResult<T> = Result<Json<T>, ApiError>;

fn parse<T: FromStr>(value: &str, what: &str) -> Result<T, ApiError> {
    value
        .parse()
        .map_err(|_| ApiError::bad_request(format!("invalid {what}")))
}

fn page(limit: Option<usize>, default: usize) -> usize {
    limit.unwrap_or(default).clamp(1, MAX_PAGE)
}

// --- Handlers --------------------------------------------------------------------------

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "status": "ok" }))
}

async fn status(State(node): State<Arc<Node>>) -> Json<StatusView> {
    Json(node.status())
}

async fn genesis(State(node): State<Arc<Node>>) -> Json<Genesis> {
    Json(node.genesis())
}

#[derive(Deserialize)]
struct SubmitParams {
    /// Wait up to this long for the transaction to be included in a block.
    wait_ms: Option<u64>,
}

async fn submit_tx(
    State(node): State<Arc<Node>>,
    Query(params): Query<SubmitParams>,
    body: Bytes,
) -> ApiResult<SubmitView> {
    let tx = Transaction::from_bytes(&body)
        .map_err(|_| ApiError::bad_request("the body is not a canonical transaction"))?;
    let (hash, mut status) = node.submit(tx)?;
    if let (TxStatus::Pending, Some(wait_ms)) = (&status, params.wait_ms) {
        let wait = Duration::from_millis(wait_ms).min(MAX_WAIT);
        status = node.wait_for_tx(&hash, wait).await?;
    }
    Ok(Json(SubmitView { hash, status }))
}

async fn get_tx(State(node): State<Arc<Node>>, Path(hash): Path<String>) -> ApiResult<TxView> {
    let hash: Hash = parse(&hash, "transaction hash")?;
    node.tx(&hash)?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("transaction"))
}

#[derive(Deserialize)]
struct LimitParams {
    limit: Option<usize>,
}

async fn recent_txs(
    State(node): State<Arc<Node>>,
    Query(params): Query<LimitParams>,
) -> ApiResult<Vec<TxView>> {
    Ok(Json(node.recent_txs(page(params.limit, 20))?))
}

#[derive(Deserialize)]
struct BlocksParams {
    /// Only blocks strictly below this height (for paging backwards).
    before: Option<u64>,
    limit: Option<usize>,
}

async fn list_blocks(
    State(node): State<Arc<Node>>,
    Query(params): Query<BlocksParams>,
) -> ApiResult<Vec<BlockSummary>> {
    let from = match params.before {
        Some(0) | Some(1) => return Ok(Json(Vec::new())),
        Some(before) => Some(before - 1),
        None => None,
    };
    let blocks = node.blocks_desc(from, page(params.limit, 20))?;
    Ok(Json(blocks.iter().map(BlockSummary::from).collect()))
}

/// `id` is either a height or a block hash.
async fn get_block(State(node): State<Arc<Node>>, Path(id): Path<String>) -> ApiResult<BlockView> {
    let block = if let Ok(height) = id.parse::<u64>() {
        node.block(height)?
    } else {
        node.block_by_hash(&parse(&id, "block height or hash")?)?
    };
    block
        .map(|block| Json(BlockView::from(&block)))
        .ok_or_else(|| ApiError::not_found("block"))
}

#[derive(Deserialize)]
struct SyncParams {
    from: u64,
    limit: Option<usize>,
}

/// Raw blocks for replicas: the borsh encoding of a `Vec<Block>`, ascending from `from`.
async fn sync_blocks(
    State(node): State<Arc<Node>>,
    Query(params): Query<SyncParams>,
) -> Result<Response, ApiError> {
    let limit = params
        .limit
        .unwrap_or(MAX_SYNC_BATCH)
        .clamp(1, MAX_SYNC_BATCH);
    let blocks = node.blocks(params.from, limit)?;
    let bytes = borsh::to_vec(&blocks).map_err(|e| {
        tracing::error!(error = %e, "cannot encode blocks");
        ApiError::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "internal error",
        )
    })?;
    Ok(([(header::CONTENT_TYPE, "application/octet-stream")], bytes).into_response())
}

async fn list_accounts(State(node): State<Arc<Node>>) -> Json<Vec<AccountView>> {
    Json(node.accounts())
}

async fn get_account(
    State(node): State<Arc<Node>>,
    Path(address): Path<String>,
) -> ApiResult<AccountView> {
    let address: Address = parse(&address, "address")?;
    Ok(Json(node.account(&address)))
}

#[derive(Deserialize)]
struct AccountTxsParams {
    limit: Option<usize>,
    /// Paging cursor: only transactions strictly before this position.
    before_height: Option<u64>,
    before_index: Option<u32>,
}

async fn account_txs(
    State(node): State<Arc<Node>>,
    Path(address): Path<String>,
    Query(params): Query<AccountTxsParams>,
) -> ApiResult<Vec<TxView>> {
    let address: Address = parse(&address, "address")?;
    let before = params.before_height.map(|height| TxLocation {
        height,
        index: params.before_index.unwrap_or(0),
    });
    Ok(Json(node.address_txs(
        &address,
        before,
        page(params.limit, 20),
    )?))
}

async fn list_authorities(State(node): State<Arc<Node>>) -> Json<Vec<AuthorityView>> {
    Json(node.authorities())
}
