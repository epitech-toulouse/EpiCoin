//! Administration: audit trail, statistics, and the blockchain explorer.

use std::collections::HashMap;

use axum::extract::{Path, Query, State};
use axum::routing::get;
use axum::{Json, Router};
use epicoin_core::{Address, Hash};
use epicoin_node::views::{BlockSummary, BlockView, StatusView, TxView};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::audit;
use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::models::{Page, Role};
use crate::state::AppState;

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/audit", get(audit_log))
        .route("/stats", get(stats))
}

pub fn explorer_router() -> Router<AppState> {
    Router::new()
        .route("/status", get(status))
        .route("/blocks", get(blocks))
        .route("/blocks/{id}", get(block))
        .route("/txs", get(recent_txs))
        .route("/tx/{hash}", get(tx))
        .route("/accounts", get(accounts))
        .route("/accounts/{address}", get(account))
}

async fn audit_log(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(page): Query<Page>,
) -> ApiResult<Json<Value>> {
    current.require(&[Role::Admin])?;
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM audit_log")
        .fetch_one(&state.db)
        .await?;
    let items = audit::list(&state.db, page.limit(), page.offset()).await?;
    Ok(Json(json!({ "items": items, "total": total })))
}

// --- Statistics ------------------------------------------------------------------------

#[derive(Serialize, sqlx::FromRow)]
struct ByStaff {
    user_id: Option<Uuid>,
    /// `None` when the staff member's account was deleted.
    name: Option<String>,
    total: i64,
    count: i64,
}

#[derive(Serialize, sqlx::FromRow)]
struct ByReason {
    reason_code: Option<String>,
    total: i64,
    count: i64,
}

#[derive(Serialize, sqlx::FromRow)]
struct ByRole {
    role: String,
    total: i64,
    activated: i64,
}

/// The admin dashboard: money supply (from the chain), distribution by staff member and by
/// reason, user counts.
async fn stats(State(state): State<AppState>, current: CurrentUser) -> ApiResult<Json<Value>> {
    current.require(&[Role::Admin])?;
    let chain: StatusView = state.node.status().await?;
    let treasury = state.node.account(&chain.treasury).await?;

    let by_staff = sqlx::query_as::<_, ByStaff>(
        "SELECT c.actor_user_id AS user_id, max(u.first_name || ' ' || u.last_name) AS name, \
                COALESCE(sum(c.amount), 0)::bigint AS total, count(*) AS count \
         FROM chain_txs c LEFT JOIN users u ON u.id = c.actor_user_id \
         WHERE c.kind = 'mint' AND c.status = 'included' \
         GROUP BY c.actor_user_id ORDER BY total DESC",
    )
    .fetch_all(&state.db)
    .await?;
    let by_reason = sqlx::query_as::<_, ByReason>(
        "SELECT reason_code, COALESCE(sum(amount), 0)::bigint AS total, count(*) AS count \
         FROM chain_txs WHERE kind = 'mint' AND status = 'included' \
         GROUP BY reason_code ORDER BY total DESC",
    )
    .fetch_all(&state.db)
    .await?;
    let users = sqlx::query_as::<_, ByRole>(
        "SELECT role, count(*) AS total, count(activated_at) AS activated \
         FROM users WHERE disabled_at IS NULL GROUP BY role ORDER BY role",
    )
    .fetch_all(&state.db)
    .await?;

    Ok(Json(json!({
        "supply": {
            "total_minted": chain.total_minted,
            "circulating": chain.circulating_supply,
            "treasury_balance": treasury.balance,
            "held_by_users": chain.circulating_supply.saturating_sub(treasury.balance),
        },
        "chain": { "height": chain.height, "chain_id": chain.chain_id },
        "minted_by_staff": by_staff,
        "minted_by_reason": by_reason,
        "users": users,
    })))
}

// --- Explorer --------------------------------------------------------------------------
//
// Read-only views of the chain, relayed from the node. The chain itself only knows
// addresses; for administrators, account lists are annotated with their owner.

async fn status(
    State(state): State<AppState>,
    current: CurrentUser,
) -> ApiResult<Json<StatusView>> {
    current.require(&[Role::Admin])?;
    Ok(Json(state.node.status().await?))
}

#[derive(Deserialize)]
struct BlocksParams {
    before: Option<u64>,
    limit: Option<usize>,
}

async fn blocks(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<BlocksParams>,
) -> ApiResult<Json<Vec<BlockSummary>>> {
    current.require(&[Role::Admin])?;
    let limit = params.limit.unwrap_or(20).clamp(1, 100);
    Ok(Json(state.node.blocks(params.before, limit).await?))
}

async fn block(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<String>,
) -> ApiResult<Json<BlockView>> {
    current.require(&[Role::Admin])?;
    // A height or a block hash; anything else cannot exist.
    if id.parse::<u64>().is_err() && id.parse::<Hash>().is_err() {
        return Err(ApiError::not_found("Bloc"));
    }
    state
        .node
        .block(&id)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("Bloc"))
}

#[derive(Deserialize)]
struct LimitParams {
    limit: Option<usize>,
}

async fn recent_txs(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<LimitParams>,
) -> ApiResult<Json<Vec<TxView>>> {
    current.require(&[Role::Admin])?;
    let limit = params.limit.unwrap_or(20).clamp(1, 100);
    Ok(Json(state.node.recent_txs(limit).await?))
}

async fn tx(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(hash): Path<String>,
) -> ApiResult<Json<TxView>> {
    current.require(&[Role::Admin])?;
    let hash: Hash = hash
        .parse()
        .map_err(|_| ApiError::not_found("Transaction"))?;
    state
        .node
        .tx(&hash)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found("Transaction"))
}

/// Who owns each known address: users by name, plus the chain's special accounts.
async fn labels(state: &AppState, chain: &StatusView) -> ApiResult<HashMap<String, String>> {
    let owners: Vec<(String, String)> = sqlx::query_as(
        "SELECT w.address, u.first_name || ' ' || u.last_name \
         FROM wallets w JOIN users u ON u.id = w.user_id",
    )
    .fetch_all(&state.db)
    .await?;
    let mut labels: HashMap<String, String> = owners.into_iter().collect();
    labels.insert(
        chain.treasury.to_string(),
        "Trésorerie de la boutique".into(),
    );
    labels.insert(chain.admin.to_string(), "Clé d'administration".into());
    Ok(labels)
}

async fn accounts(State(state): State<AppState>, current: CurrentUser) -> ApiResult<Json<Value>> {
    current.require(&[Role::Admin])?;
    let chain = state.node.status().await?;
    let labels = labels(&state, &chain).await?;
    let accounts: Vec<Value> = state
        .node
        .accounts()
        .await?
        .into_iter()
        .map(|account| {
            json!({
                "address": account.address,
                "balance": account.balance,
                "nonce": account.nonce,
                "label": labels.get(&account.address.to_string()),
                "authority": account.authority,
            })
        })
        .collect();
    Ok(Json(json!({ "items": accounts })))
}

async fn account(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(address): Path<String>,
) -> ApiResult<Json<Value>> {
    current.require(&[Role::Admin])?;
    let address: Address = address.parse().map_err(|_| ApiError::not_found("Compte"))?;
    let chain = state.node.status().await?;
    let labels = labels(&state, &chain).await?;
    let account = state.node.account(&address).await?;
    let txs = state.node.account_txs(&address, 50).await?;
    Ok(Json(json!({
        "address": account.address,
        "balance": account.balance,
        "nonce": account.nonce,
        "label": labels.get(&address.to_string()),
        "authority": account.authority,
        "transactions": txs,
    })))
}
