//! What every logged-in user can see about themselves: wallet balance and history.

use axum::extract::{Query, State};
use axum::routing::get;
use axum::{Json, Router};
use serde::Serialize;
use serde_json::{json, Value};
use time::OffsetDateTime;

use crate::auth::CurrentUser;
use crate::error::ApiResult;
use crate::models::Page;
use crate::state::AppState;
use crate::wallets;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/wallet", get(wallet))
        .route("/transactions", get(transactions))
}

/// The user's address and confirmed on-chain balance, in units.
async fn wallet(State(state): State<AppState>, current: CurrentUser) -> ApiResult<Json<Value>> {
    let address = wallets::address_of(&state.db, current.id()).await?;
    let account = state.node.account(&address).await?;
    Ok(Json(json!({
        "address": address,
        "balance": account.balance,
    })))
}

#[derive(Serialize, sqlx::FromRow)]
struct HistoryRow {
    tx_hash: String,
    /// `mint`, `purchase` or `refund`.
    kind: String,
    /// `included` or `pending`.
    status: String,
    amount: Option<i64>,
    /// `credit` when the balance goes up, `debit` when it goes down.
    direction: String,
    reason_code: Option<String>,
    comment: Option<String>,
    /// Who granted or sent the EPC, for a mint or a received transfer.
    from_name: Option<String>,
    /// Who received the EPC, for a sent transfer.
    to_name: Option<String>,
    /// The shop order, for a purchase or a refund.
    order_id: Option<uuid::Uuid>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

/// The transactions that moved the balance of user `$1`: what they received or paid, and the
/// transfers they sent. Failed ones never touched it.
macro_rules! mine {
    () => {
        "((c.subject_user_id = $1 AND c.kind IN ('mint', 'purchase', 'refund', 'transfer')) \
          OR (c.kind = 'transfer' AND c.actor_user_id = $1)) AND c.status <> 'failed'"
    };
}

/// Everything that moved the user's balance, newest first. Failed transactions never
/// touched the balance and are not listed.
async fn transactions(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(page): Query<Page>,
) -> ApiResult<Json<Value>> {
    let total: i64 =
        sqlx::query_scalar(concat!("SELECT count(*) FROM chain_txs c WHERE ", mine!()))
            .bind(current.id())
            .fetch_one(&state.db)
            .await?;
    let items = sqlx::query_as::<_, HistoryRow>(concat!(
        "SELECT c.hash AS tx_hash, c.kind, c.status, c.amount, \
                CASE WHEN c.kind = 'purchase' OR (c.kind = 'transfer' AND c.actor_user_id = $1) \
                     THEN 'debit' ELSE 'credit' END AS direction, \
                c.reason_code, COALESCE(b.comment, NULLIF(tr.message, '')) AS comment, \
                CASE WHEN c.kind IN ('mint', 'transfer') AND c.subject_user_id = $1 \
                     THEN a.first_name || ' ' || a.last_name END AS from_name, \
                CASE WHEN c.kind = 'transfer' AND c.actor_user_id = $1 \
                     THEN s.first_name || ' ' || s.last_name END AS to_name, \
                c.order_id, c.created_at \
         FROM chain_txs c \
         LEFT JOIN mint_batches b ON b.id = c.batch_id \
         LEFT JOIN transfers tr ON tr.tx_hash = c.hash \
         LEFT JOIN users a ON a.id = c.actor_user_id \
         LEFT JOIN users s ON s.id = c.subject_user_id \
         WHERE ",
        mine!(),
        " ORDER BY c.created_at DESC, c.nonce DESC LIMIT $2 OFFSET $3"
    ))
    .bind(current.id())
    .bind(page.limit())
    .bind(page.offset())
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({ "items": items, "total": total })))
}
