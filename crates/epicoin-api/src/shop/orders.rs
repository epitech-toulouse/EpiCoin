//! Orders: checkout, lifecycle, refunds.
//!
//! # Keeping stock, order and chain consistent
//!
//! A checkout touches two systems — Postgres (stock, order) and the chain (the debit) —
//! which cannot share a transaction. Consistency comes from doing things in this order:
//!
//! 1. **One SQL transaction** creates the order as `awaiting_payment`, takes the stock
//!    (`stock >= quantity` or nothing happens), and stores the signed `Purchase`. Until it
//!    commits, nothing exists; a stock shortage rolls everything back before any debit.
//! 2. The stored transaction is submitted to the node.
//! 3. When the transaction is final, [`settle`] runs in the same SQL transaction as its
//!    status change: included → the order is `pending` (paid); refused → the order is
//!    `failed` and the stock is given back.
//!
//! If the API stops between 1 and 3, the reconciler resubmits the very same bytes and step
//! 3 happens then. Since those bytes carry a fixed nonce, the debit can only happen once.
//!
//! # Idempotency
//!
//! The client sends an `Idempotency-Key` with every checkout. `(user, key)` is unique in
//! `orders`: a double click or a retry finds the existing order instead of creating — and
//! paying for — a second one.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use epicoin_core::{Amount, Hash, TxPayload};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use super::products::Product;
use super::MANAGERS;
use crate::auth::CurrentUser;
use crate::chain::{record_tx, ChainError, Outcome, TxMeta, KIND_PURCHASE, KIND_REFUND};
use crate::error::{ApiError, ApiResult};
use crate::models::{Page, Role};
use crate::state::AppState;
use crate::{audit, wallets};

const MAX_LINES: usize = 20;
const MAX_QUANTITY: i32 = 100;
const MAX_IDEMPOTENCY_KEY_LEN: usize = 100;
/// How long a checkout or a cancellation waits for its transaction to be in a block.
const WAIT: Duration = Duration::from_secs(20);
/// How long a retried checkout waits for the original payment before answering.
const RESUME_WAIT: Duration = Duration::from_secs(5);

pub const AWAITING_PAYMENT: &str = "awaiting_payment";
pub const FAILED: &str = "failed";
pub const PENDING: &str = "pending";
pub const READY: &str = "ready";
pub const DELIVERED: &str = "delivered";
pub const REFUNDING: &str = "refunding";
pub const CANCELLED: &str = "cancelled";

/// The opaque reference of an order on chain: a hash, so that the chain reveals nothing
/// about the order.
pub fn order_ref(order_id: Uuid) -> Hash {
    Hash(blake3::derive_key(
        "epicoin v1 order ref",
        order_id.as_bytes(),
    ))
}

// --- Views -----------------------------------------------------------------------------

#[derive(Debug, sqlx::FromRow)]
struct OrderRow {
    id: Uuid,
    user_id: Option<Uuid>,
    status: String,
    total: i64,
    failure: Option<String>,
    created_at: OffsetDateTime,
    updated_at: OffsetDateTime,
    buyer_name: Option<String>,
    buyer_email: Option<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ItemView {
    #[serde(skip)]
    order_id: Uuid,
    pub product_id: Uuid,
    pub name: String,
    pub unit_price: i64,
    pub quantity: i32,
}

#[derive(Debug, Serialize)]
pub struct BuyerView {
    pub id: Uuid,
    pub name: String,
    pub email: String,
}

#[derive(Debug, Serialize)]
pub struct OrderView {
    pub id: Uuid,
    pub status: String,
    pub total: i64,
    /// Why the payment failed, when `status` is `failed`.
    pub failure: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
    pub items: Vec<ItemView>,
    /// Only in the shop manager's views. `None` if the buyer's account was deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buyer: Option<BuyerView>,
}

/// The order query (with its buyer), followed by a literal suffix, as one static string.
macro_rules! concat_order_select {
    ($suffix:literal) => {
        concat!(
            "SELECT o.id, o.user_id, o.status, o.total, o.failure, o.created_at, \
             o.updated_at, u.first_name || ' ' || u.last_name AS buyer_name, \
             u.email AS buyer_email FROM orders o LEFT JOIN users u ON u.id = o.user_id",
            $suffix
        )
    };
}

/// Attaches their items to orders. `with_buyer` adds the buyer (manager views).
async fn views(
    conn: &mut PgConnection,
    rows: Vec<OrderRow>,
    with_buyer: bool,
) -> Result<Vec<OrderView>, sqlx::Error> {
    let ids: Vec<Uuid> = rows.iter().map(|row| row.id).collect();
    let items = sqlx::query_as::<_, ItemView>(
        "SELECT order_id, product_id, product_name AS name, unit_price, quantity \
         FROM order_items WHERE order_id = ANY($1) ORDER BY product_name, product_id",
    )
    .bind(&ids)
    .fetch_all(&mut *conn)
    .await?;
    let mut by_order: HashMap<Uuid, Vec<ItemView>> = HashMap::new();
    for item in items {
        by_order.entry(item.order_id).or_default().push(item);
    }
    Ok(rows
        .into_iter()
        .map(|row| OrderView {
            buyer: match (with_buyer, row.user_id, row.buyer_name, row.buyer_email) {
                (true, Some(id), Some(name), Some(email)) => Some(BuyerView { id, name, email }),
                _ => None,
            },
            items: by_order.remove(&row.id).unwrap_or_default(),
            id: row.id,
            status: row.status,
            total: row.total,
            failure: row.failure,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
        .collect())
}

async fn load(state: &AppState, id: Uuid, with_buyer: bool) -> ApiResult<OrderView> {
    let mut conn = state.db.acquire().await?;
    let row = sqlx::query_as::<_, OrderRow>(concat_order_select!(" WHERE o.id = $1"))
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(|| ApiError::not_found("Commande"))?;
    Ok(views(&mut conn, vec![row], with_buyer)
        .await?
        .pop()
        .expect("one row in, one view out"))
}

// --- Settlement ------------------------------------------------------------------------

async fn restore_stock(conn: &mut PgConnection, order_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE products p SET stock = p.stock + i.quantity, updated_at = now() \
         FROM order_items i WHERE i.order_id = $1 AND p.id = i.product_id",
    )
    .bind(order_id)
    .execute(conn)
    .await?;
    Ok(())
}

/// Moves an order from `from` to `to`; returns whether it was in `from`.
async fn transition(
    conn: &mut PgConnection,
    order_id: Uuid,
    from: &str,
    to: &str,
    failure: Option<&str>,
) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE orders SET status = $3, failure = $4, updated_at = now() \
         WHERE id = $1 AND status = $2",
    )
    .bind(order_id)
    .bind(from)
    .bind(to)
    .bind(failure)
    .execute(conn)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Applies the outcome of an order's on-chain transaction to the order and the stock.
///
/// Called by [`crate::chain::ChainService::finalize`] inside the SQL transaction that
/// records the outcome, exactly once per transaction: the order can never disagree with
/// the chain.
pub async fn settle(
    conn: &mut PgConnection,
    kind: &str,
    order_id: Uuid,
    outcome: &Outcome,
) -> Result<(), sqlx::Error> {
    match (kind, outcome) {
        // Paid.
        (KIND_PURCHASE, Outcome::Included { .. }) => {
            transition(conn, order_id, AWAITING_PAYMENT, PENDING, None).await?;
        }
        // Payment refused (typically an insufficient balance): give the stock back.
        (KIND_PURCHASE, Outcome::Failed(error)) => {
            let code = crate::chain::error_code(error);
            if transition(conn, order_id, AWAITING_PAYMENT, FAILED, Some(&code)).await? {
                restore_stock(conn, order_id).await?;
            }
        }
        // Refunded: the order is cancelled and its items go back on the shelf.
        (KIND_REFUND, Outcome::Included { .. }) => {
            if transition(conn, order_id, REFUNDING, CANCELLED, None).await? {
                restore_stock(conn, order_id).await?;
            }
        }
        // A refund should never be refused; if it is, the order is still paid.
        (KIND_REFUND, Outcome::Failed(error)) => {
            tracing::error!(%order_id, %error, "refund refused by the chain");
            transition(conn, order_id, REFUNDING, PENDING, Some("refund_failed")).await?;
        }
        _ => {}
    }
    Ok(())
}

// --- Checkout --------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct CheckoutLine {
    product_id: Uuid,
    quantity: i32,
}

#[derive(Deserialize)]
pub struct CheckoutRequest {
    items: Vec<CheckoutLine>,
}

fn chain_unavailable(error: ChainError) -> ApiError {
    match error {
        ChainError::Db(error) => error.into(),
        other => {
            tracing::warn!(error = %other, "chain unavailable");
            ApiError::unavailable(
                "La blockchain est momentanément indisponible. Réessayez dans un instant.",
            )
        }
    }
}

fn payment_refused(code: &str) -> ApiError {
    let message = match code {
        "insufficient_balance" => "Solde insuffisant pour cette commande.",
        _ => "Le paiement a été refusé. Aucun EPC n'a été débité.",
    };
    ApiError::unprocessable("payment_refused", message).with_details(json!({ "reason": code }))
}

async fn find_by_key(state: &AppState, user_id: Uuid, key: &str) -> ApiResult<Option<Uuid>> {
    Ok(
        sqlx::query_scalar("SELECT id FROM orders WHERE user_id = $1 AND idempotency_key = $2")
            .bind(user_id)
            .bind(key)
            .fetch_optional(&state.db)
            .await?,
    )
}

/// The hash of an order's transaction of `kind` that is not failed.
async fn order_tx(state: &AppState, order_id: Uuid, kind: &str) -> ApiResult<Option<Hash>> {
    let hash: Option<String> = sqlx::query_scalar(
        "SELECT hash FROM chain_txs WHERE order_id = $1 AND kind = $2 AND status <> 'failed' \
         ORDER BY created_at DESC LIMIT 1",
    )
    .bind(order_id)
    .bind(kind)
    .fetch_optional(&state.db)
    .await?;
    Ok(hash.and_then(|h| h.parse().ok()))
}

/// Answers a repeated checkout with the order created the first time, giving its payment
/// a moment to complete if it is still in flight.
async fn resume(state: &AppState, order_id: Uuid) -> ApiResult<(StatusCode, Json<OrderView>)> {
    let order = load(state, order_id, false).await?;
    if order.status == AWAITING_PAYMENT {
        if let Some(hash) = order_tx(state, order_id, KIND_PURCHASE).await? {
            let _ = state.chain.wait_final(&[hash], RESUME_WAIT).await;
        }
        return Ok((StatusCode::OK, Json(load(state, order_id, false).await?)));
    }
    Ok((StatusCode::OK, Json(order)))
}

/// Buys the cart: reserves the stock and pays on chain.
///
/// Answers 201 when the order is paid (`pending`), 202 when the payment is recorded but
/// not confirmed yet (`awaiting_payment` — it will complete or fail on its own), 200 when
/// this `Idempotency-Key` was already used (the existing order), and an error — with
/// nothing debited and no stock taken — otherwise.
pub async fn checkout(
    State(state): State<AppState>,
    current: CurrentUser,
    headers: HeaderMap,
    Json(body): Json<CheckoutRequest>,
) -> ApiResult<(StatusCode, Json<OrderView>)> {
    current.require(&[Role::Student])?;
    let key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|key| !key.is_empty() && key.len() <= MAX_IDEMPOTENCY_KEY_LEN)
        .ok_or_else(|| {
            ApiError::bad_request("En-tête Idempotency-Key manquant ou invalide.")
                .with_details(json!({ "header": "Idempotency-Key" }))
        })?;
    if let Some(order_id) = find_by_key(&state, current.id(), key).await? {
        return resume(&state, order_id).await;
    }

    // --- The cart: merged by product, in a fixed order (which also prevents deadlocks
    // between two checkouts locking the same products).
    let mut cart: BTreeMap<Uuid, i32> = BTreeMap::new();
    for line in &body.items {
        if !(1..=MAX_QUANTITY).contains(&line.quantity) {
            return Err(ApiError::unprocessable(
                "invalid_quantity",
                format!("La quantité doit être comprise entre 1 et {MAX_QUANTITY}."),
            ));
        }
        let quantity = cart.entry(line.product_id).or_insert(0);
        *quantity += line.quantity;
        if *quantity > MAX_QUANTITY {
            return Err(ApiError::unprocessable(
                "invalid_quantity",
                format!("La quantité doit être comprise entre 1 et {MAX_QUANTITY}."),
            ));
        }
    }
    if cart.is_empty() {
        return Err(ApiError::unprocessable(
            "empty_cart",
            "Votre panier est vide.",
        ));
    }
    if cart.len() > MAX_LINES {
        return Err(ApiError::unprocessable(
            "cart_too_large",
            format!("Un panier est limité à {MAX_LINES} produits différents."),
        ));
    }

    // --- Signing rights first: holding the buyer's lock serializes their checkouts.
    let keypair = wallets::load_keypair(&state.db, &state.vault, current.id()).await?;
    let mut session = state
        .chain
        .begin(&keypair)
        .await
        .map_err(chain_unavailable)?;

    // --- Step 1: order + stock + signed payment, atomically.
    let mut tx = state.db.begin().await?;
    let ids: Vec<Uuid> = cart.keys().copied().collect();
    let products = sqlx::query_as::<_, Product>(
        "SELECT * FROM products WHERE id = ANY($1) ORDER BY id FOR UPDATE",
    )
    .bind(&ids)
    .fetch_all(&mut *tx)
    .await?;
    let unavailable: Vec<Uuid> = ids
        .iter()
        .copied()
        .filter(|id| !products.iter().any(|p| p.id == *id && p.active))
        .collect();
    if !unavailable.is_empty() {
        return Err(ApiError::unprocessable(
            "product_unavailable",
            "Un produit de votre panier n'est plus en vente.",
        )
        .with_details(json!({ "product_ids": unavailable })));
    }
    let short: Vec<Value> = products
        .iter()
        .filter(|p| p.stock < cart[&p.id])
        .map(|p| json!({ "product_id": p.id, "name": p.name, "available": p.stock }))
        .collect();
    if !short.is_empty() {
        return Err(ApiError::conflict(
            "out_of_stock",
            "Stock insuffisant pour un produit de votre panier.",
        )
        .with_details(json!({ "products": short })));
    }
    let total = products
        .iter()
        .try_fold(0i64, |sum, p| {
            p.price
                .checked_mul(i64::from(cart[&p.id]))
                .and_then(|line| sum.checked_add(line))
        })
        .ok_or_else(|| ApiError::unprocessable("invalid_total", "Montant trop élevé."))?;

    // A friendly early answer; the chain remains the judge of the balance.
    let balance = state.node.account(&keypair.address()).await?.balance;
    if balance < total as Amount {
        return Err(payment_refused("insufficient_balance").with_details(
            json!({ "reason": "insufficient_balance", "balance": balance, "needed": total }),
        ));
    }

    let order_id = Uuid::new_v4();
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO orders (id, user_id, idempotency_key, status, total) \
         VALUES ($1, $2, $3, 'awaiting_payment', $4) \
         ON CONFLICT (user_id, idempotency_key) DO NOTHING RETURNING id",
    )
    .bind(order_id)
    .bind(current.id())
    .bind(key)
    .bind(total)
    .fetch_optional(&mut *tx)
    .await?;
    if inserted.is_none() {
        // Lost a race against the same key: that request owns the order.
        tx.rollback().await?;
        drop(session);
        let order_id = find_by_key(&state, current.id(), key)
            .await?
            .ok_or_else(|| ApiError::internal("idempotent order vanished"))?;
        return resume(&state, order_id).await;
    }
    for product in &products {
        let quantity = cart[&product.id];
        sqlx::query("UPDATE products SET stock = stock - $2, updated_at = now() WHERE id = $1")
            .bind(product.id)
            .bind(quantity)
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO order_items (order_id, product_id, product_name, unit_price, quantity) \
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(order_id)
        .bind(product.id)
        .bind(&product.name)
        .bind(product.price)
        .bind(quantity)
        .execute(&mut *tx)
        .await?;
    }
    let payment = session.sign(TxPayload::Purchase {
        amount: total as Amount,
        order_ref: order_ref(order_id),
    });
    record_tx(
        &mut tx,
        &payment,
        &TxMeta {
            actor_user_id: Some(current.id()),
            subject_user_id: Some(current.id()),
            amount: Some(total as Amount),
            order_id: Some(order_id),
            ..TxMeta::default()
        },
    )
    .await?;
    tx.commit().await?;

    // --- Step 2: submit. Step 3 (settlement) happens in `finalize`.
    let submitted = session.submit(&payment).await;
    drop(session);
    match submitted {
        Ok(()) => {
            state
                .chain
                .wait_final(&[payment.hash()], WAIT)
                .await
                .map_err(chain_unavailable)?;
        }
        // Refused: `finalize` already marked the order failed and gave the stock back.
        Err(ChainError::Rejected(error)) => {
            return Err(
                payment_refused(&crate::chain::error_code(&error)).with_details(json!({
                    "reason": crate::chain::error_code(&error),
                    "order_id": order_id,
                })),
            );
        }
        // Outcome unknown: the order stays `awaiting_payment` and the reconciler settles it.
        Err(ChainError::Unavailable(reason)) => {
            tracing::warn!(%order_id, %reason, "payment recorded but not delivered yet");
        }
        Err(other) => return Err(chain_unavailable(other)),
    }

    let order = load(&state, order_id, false).await?;
    match order.status.as_str() {
        PENDING => Ok((StatusCode::CREATED, Json(order))),
        FAILED => Err(
            payment_refused(order.failure.as_deref().unwrap_or("rejected")).with_details(json!({
                "reason": order.failure,
                "order_id": order_id,
            })),
        ),
        _ => Ok((StatusCode::ACCEPTED, Json(order))),
    }
}

// --- The buyer's orders ----------------------------------------------------------------

pub async fn mine(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(page): Query<Page>,
) -> ApiResult<Json<Value>> {
    let mut conn = state.db.acquire().await?;
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM orders WHERE user_id = $1")
        .bind(current.id())
        .fetch_one(&mut *conn)
        .await?;
    let rows = sqlx::query_as::<_, OrderRow>(concat_order_select!(
        " WHERE o.user_id = $1 ORDER BY o.created_at DESC, o.id LIMIT $2 OFFSET $3"
    ))
    .bind(current.id())
    .bind(page.limit())
    .bind(page.offset())
    .fetch_all(&mut *conn)
    .await?;
    let items = views(&mut conn, rows, false).await?;
    Ok(Json(json!({ "items": items, "total": total })))
}

pub async fn mine_one(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<OrderView>> {
    let owner: Option<Option<Uuid>> =
        sqlx::query_scalar("SELECT user_id FROM orders WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    // Someone else's order does not exist, as far as this user is concerned.
    if owner.flatten() != Some(current.id()) {
        return Err(ApiError::not_found("Commande"));
    }
    Ok(Json(load(&state, id, false).await?))
}

// --- The shop manager's side -----------------------------------------------------------

#[derive(Deserialize)]
pub struct QueueParams {
    status: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
}

/// Every order, optionally filtered by status. The work queues (`pending`, `ready`) come
/// oldest first; everything else newest first.
pub async fn queue(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<QueueParams>,
) -> ApiResult<Json<Value>> {
    current.require(&MANAGERS)?;
    let page = Page {
        limit: params.limit,
        offset: params.offset,
    };
    let status = params.status.as_deref().filter(|s| !s.is_empty());
    let oldest_first = matches!(status, Some(PENDING | READY));

    let mut conn = state.db.acquire().await?;
    let total: i64 =
        sqlx::query_scalar("SELECT count(*) FROM orders WHERE ($1::text IS NULL OR status = $1)")
            .bind(status)
            .fetch_one(&mut *conn)
            .await?;
    let rows = sqlx::query_as::<_, OrderRow>(concat_order_select!(
        " WHERE ($1::text IS NULL OR o.status = $1) \
          ORDER BY CASE WHEN $2 THEN o.created_at END ASC, \
                   CASE WHEN NOT $2 THEN o.created_at END DESC, o.id \
          LIMIT $3 OFFSET $4"
    ))
    .bind(status)
    .bind(oldest_first)
    .bind(page.limit())
    .bind(page.offset())
    .fetch_all(&mut *conn)
    .await?;
    let items = views(&mut conn, rows, true).await?;
    Ok(Json(json!({ "items": items, "total": total })))
}

fn invalid_transition() -> ApiError {
    ApiError::conflict(
        "invalid_transition",
        "Ce changement de statut n'est pas possible pour cette commande.",
    )
}

#[derive(Deserialize)]
pub struct StatusRequest {
    status: String,
}

/// Moves an order forward: `pending` → `ready` (prepared), `ready` → `delivered` (handed
/// over to the student).
pub async fn set_status(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<StatusRequest>,
) -> ApiResult<Json<OrderView>> {
    current.require(&MANAGERS)?;
    let from = match body.status.as_str() {
        READY => PENDING,
        DELIVERED => READY,
        _ => return Err(invalid_transition()),
    };
    let mut tx = state.db.begin().await?;
    if !transition(&mut tx, id, from, &body.status, None).await? {
        let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM orders WHERE id = $1)")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        return Err(if exists {
            invalid_transition()
        } else {
            ApiError::not_found("Commande")
        });
    }
    audit::record(
        &mut *tx,
        Some(current.id()),
        audit::action::ORDER_STATUS_CHANGED,
        Some(("order", id.to_string())),
        json!({ "from": from, "to": body.status }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(load(&state, id, true).await?))
}

/// Cancels a paid order that was not delivered yet, and refunds the student on chain.
///
/// The order is `refunding` until the `Refund` is in a block, then `cancelled`, at which
/// point its items return to stock.
pub async fn cancel(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<(StatusCode, Json<OrderView>)> {
    current.require(&MANAGERS)?;
    let mut session = state
        .chain
        .begin(&state.treasury_key)
        .await
        .map_err(chain_unavailable)?;

    let mut tx = state.db.begin().await?;
    let order: Option<(String, Option<Uuid>, i64)> =
        sqlx::query_as("SELECT status, user_id, total FROM orders WHERE id = $1 FOR UPDATE")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let (status, buyer, total) = order.ok_or_else(|| ApiError::not_found("Commande"))?;
    if status != PENDING && status != READY {
        return Err(invalid_transition());
    }
    let purchase: Option<String> = sqlx::query_scalar(
        "SELECT hash FROM chain_txs \
         WHERE order_id = $1 AND kind = 'purchase' AND status = 'included'",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let purchase: Hash = purchase.and_then(|hash| hash.parse().ok()).ok_or_else(|| {
        ApiError::internal(format!("paid order {id} has no purchase transaction"))
    })?;

    transition(&mut tx, id, &status, REFUNDING, None).await?;
    let refund = session.sign(TxPayload::Refund { purchase });
    record_tx(
        &mut tx,
        &refund,
        &TxMeta {
            actor_user_id: Some(current.id()),
            subject_user_id: buyer,
            amount: Some(total as Amount),
            order_id: Some(id),
            ..TxMeta::default()
        },
    )
    .await?;
    audit::record(
        &mut *tx,
        Some(current.id()),
        audit::action::ORDER_REFUNDED,
        Some(("order", id.to_string())),
        json!({ "amount": total, "from_status": status, "tx": refund.hash() }),
    )
    .await?;
    tx.commit().await?;

    let submitted = session.submit(&refund).await;
    drop(session);
    match submitted {
        Ok(()) => state
            .chain
            .wait_final(&[refund.hash()], WAIT)
            .await
            .map_err(chain_unavailable)?,
        // `finalize` put the order back to `pending`.
        Err(ChainError::Rejected(error)) => {
            return Err(ApiError::internal(format!(
                "refund of order {id} refused by the chain: {error}"
            )));
        }
        Err(ChainError::Unavailable(reason)) => {
            tracing::warn!(order_id = %id, %reason, "refund recorded but not delivered yet");
        }
        Err(other) => return Err(chain_unavailable(other)),
    }

    let order = load(&state, id, true).await?;
    let code = if order.status == CANCELLED {
        StatusCode::OK
    } else {
        // Still `refunding`: recorded, and guaranteed to complete.
        StatusCode::ACCEPTED
    };
    Ok((code, Json(order)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn order_refs_are_opaque_and_stable() {
        let id = Uuid::new_v4();
        assert_eq!(order_ref(id), order_ref(id));
        assert_ne!(order_ref(id), order_ref(Uuid::new_v4()));
        // The reference does not contain the order id.
        let reference = order_ref(id).to_string();
        assert!(!reference.contains(&id.simple().to_string()));
    }
}
