//! Transfers: students send EPC to each other.
//!
//! A transfer is a `Transfer` transaction signed with the sender's custodial key: the chain
//! sees two addresses and an amount, nothing else. The optional message stays in the
//! `transfers` table, next to the chain transaction (actor = sender, subject = recipient),
//! and is erased when either student's account is.
//!
//! Like a checkout, a transfer carries an `Idempotency-Key`: `(sender, key)` is unique, so a
//! double click or a retry after a network error finds the first transfer instead of
//! sending a second one. The transfer row and the signed transaction are written in one SQL
//! transaction, then submitted; reconciliation settles them if the API stops in between.

use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use epicoin_core::{Amount, TxError, TxPayload};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::chain::{error_code, record_tx, ChainError, TxMeta, STATUS_FAILED};
use crate::error::{ApiError, ApiResult};
use crate::models::{Page, Role};
use crate::state::AppState;
use crate::users::like_pattern;
use crate::{audit, wallets};

const MAX_MESSAGE_CHARS: usize = 140;
const MAX_IDEMPOTENCY_KEY_LEN: usize = 100;
const MAX_SEARCH_RESULTS: i64 = 20;
/// How long a transfer waits for its inclusion before answering.
const WAIT: Duration = Duration::from_secs(20);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/transfers", post(send))
        .route("/transfers/recipients", get(recipients))
        .route("/admin/transfers", get(list_all))
}

// --- Finding a classmate ---------------------------------------------------------------

#[derive(Deserialize)]
struct SearchParams {
    q: Option<String>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct Recipient {
    id: Uuid,
    first_name: String,
    last_name: String,
    email: String,
    promo: String,
}

/// Other active students whose name or email contains the search (2 characters at least).
async fn recipients(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<SearchParams>,
) -> ApiResult<Json<Vec<Recipient>>> {
    current.require(&[Role::Student])?;
    let search = params.q.unwrap_or_default();
    let Some(pattern) = like_pattern(&search).filter(|_| search.trim().chars().count() >= 2) else {
        return Ok(Json(Vec::new()));
    };
    let found = sqlx::query_as::<_, Recipient>(
        "SELECT id, first_name, last_name, email, promo FROM users \
         WHERE role = 'student' AND disabled_at IS NULL AND id <> $1 \
           AND (first_name ILIKE $2 OR last_name ILIKE $2 OR email ILIKE $2 \
                OR (first_name || ' ' || last_name) ILIKE $2) \
         ORDER BY last_name, first_name, email LIMIT $3",
    )
    .bind(current.id())
    .bind(&pattern)
    .bind(MAX_SEARCH_RESULTS)
    .fetch_all(&state.db)
    .await?;
    Ok(Json(found))
}

// --- Sending ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct TransferRequest {
    recipient_id: Uuid,
    /// In units (1 EPC = 100 units).
    amount: Amount,
    #[serde(default)]
    message: String,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
struct TransferView {
    tx_hash: String,
    /// `included`, `pending` or `failed`.
    status: String,
    /// When failed: why the chain refused it.
    #[serde(skip)]
    error: Option<String>,
    amount: i64,
    message: String,
    recipient_id: Option<Uuid>,
    recipient_name: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

/// A transfer as its sender sees it, selected by `$condition`.
macro_rules! view_select {
    ($condition:literal) => {
        concat!(
            "SELECT t.tx_hash, c.status, c.error, t.amount, t.message, \
               t.recipient_user_id AS recipient_id, \
               r.first_name || ' ' || r.last_name AS recipient_name, t.created_at \
             FROM transfers t JOIN chain_txs c ON c.hash = t.tx_hash \
             LEFT JOIN users r ON r.id = t.recipient_user_id ",
            $condition
        )
    };
}

async fn find(state: &AppState, condition: Condition<'_>) -> ApiResult<Option<TransferView>> {
    let row = match condition {
        Condition::Hash(hash) => {
            sqlx::query_as::<_, TransferView>(view_select!("WHERE t.tx_hash = $1"))
                .bind(hash)
                .fetch_optional(&state.db)
                .await?
        }
        Condition::Key { sender, key } => {
            sqlx::query_as::<_, TransferView>(view_select!(
                "WHERE t.sender_user_id = $1 AND t.idempotency_key = $2"
            ))
            .bind(sender)
            .bind(key)
            .fetch_optional(&state.db)
            .await?
        }
    };
    Ok(row)
}

enum Condition<'a> {
    Hash(&'a str),
    Key { sender: Uuid, key: &'a str },
}

fn refused(code: &str) -> ApiError {
    let message = match code {
        "insufficient_balance" => "Solde insuffisant pour cet envoi.",
        _ => "L'envoi a été refusé. Aucun EPC n'a quitté votre solde.",
    };
    ApiError::unprocessable("transfer_refused", message).with_details(json!({ "reason": code }))
}

fn chain_unavailable(error: ChainError) -> ApiError {
    match error {
        ChainError::Db(error) => error.into(),
        other => {
            tracing::warn!(error = %other, "transfer not sent");
            ApiError::unavailable(
                "La blockchain est momentanément indisponible. Aucun EPC n'a été envoyé.",
            )
        }
    }
}

/// The answer for a transfer as it stands: refused, confirmed (201) or still pending (202).
fn answer(view: TransferView) -> ApiResult<(StatusCode, Json<TransferView>)> {
    if view.status == STATUS_FAILED {
        let code = view
            .error
            .as_deref()
            .and_then(|json| serde_json::from_str::<TxError>(json).ok())
            .map_or_else(|| "rejected".to_owned(), |error| error_code(&error));
        return Err(refused(&code));
    }
    let status = if view.status == crate::chain::STATUS_INCLUDED {
        StatusCode::CREATED
    } else {
        StatusCode::ACCEPTED
    };
    Ok((status, Json(view)))
}

async fn send(
    State(state): State<AppState>,
    current: CurrentUser,
    headers: HeaderMap,
    Json(body): Json<TransferRequest>,
) -> ApiResult<(StatusCode, Json<TransferView>)> {
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
    let sender = current.id();
    if let Some(existing) = find(&state, Condition::Key { sender, key }).await? {
        return answer(existing);
    }

    // --- Validation: nothing is written or signed unless the request is acceptable.
    let amount = i64::try_from(body.amount)
        .ok()
        .filter(|amount| *amount > 0)
        .ok_or_else(|| {
            ApiError::unprocessable("invalid_amount", "Le montant doit être supérieur à zéro.")
        })?;
    let message = body.message.trim();
    if message.chars().count() > MAX_MESSAGE_CHARS {
        return Err(ApiError::unprocessable(
            "message_too_long",
            format!("Le message est limité à {MAX_MESSAGE_CHARS} caractères."),
        ));
    }
    if body.recipient_id == sender {
        return Err(ApiError::unprocessable(
            "self_transfer",
            "Vous ne pouvez pas vous envoyer des EPC.",
        ));
    }
    let recipient_ok: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users \
           WHERE id = $1 AND role = 'student' AND disabled_at IS NULL)",
    )
    .bind(body.recipient_id)
    .fetch_one(&state.db)
    .await?;
    if !recipient_ok {
        return Err(ApiError::unprocessable(
            "invalid_recipient",
            "Ce destinataire n'est pas un étudiant actif.",
        ));
    }
    let to = wallets::address_of(&state.db, body.recipient_id).await?;
    let keypair = wallets::load_keypair(&state.db, &state.vault, sender).await?;

    // --- Signing rights first (serializes the sender's transactions), then the transfer
    // and its signed transaction, atomically.
    let mut session = state
        .chain
        .begin(&keypair)
        .await
        .map_err(chain_unavailable)?;
    let mut tx = state.db.begin().await?;
    let transfer = session.sign(TxPayload::Transfer {
        to,
        amount: body.amount,
    });
    let hash = transfer.hash().to_string();
    record_tx(
        &mut tx,
        &transfer,
        &TxMeta {
            actor_user_id: Some(sender),
            subject_user_id: Some(body.recipient_id),
            amount: Some(body.amount),
            ..TxMeta::default()
        },
    )
    .await?;
    let inserted: Option<String> = sqlx::query_scalar(
        "INSERT INTO transfers \
           (tx_hash, sender_user_id, recipient_user_id, amount, message, idempotency_key) \
         VALUES ($1, $2, $3, $4, $5, $6) \
         ON CONFLICT (sender_user_id, idempotency_key) DO NOTHING RETURNING tx_hash",
    )
    .bind(&hash)
    .bind(sender)
    .bind(body.recipient_id)
    .bind(amount)
    .bind(message)
    .bind(key)
    .fetch_optional(&mut *tx)
    .await?;
    if inserted.is_none() {
        // A concurrent request with the same key won the race: forget this one.
        drop(tx);
        drop(session);
        let existing = find(&state, Condition::Key { sender, key })
            .await?
            .ok_or_else(|| ApiError::internal("idempotent transfer vanished"))?;
        return answer(existing);
    }
    audit::record(
        &mut *tx,
        Some(sender),
        audit::action::TRANSFER_SENT,
        Some(("user", body.recipient_id.to_string())),
        json!({ "amount": amount, "tx": hash }),
    )
    .await?;
    tx.commit().await?;

    // --- Submit. A refusal finalizes the transaction as failed: nothing moved.
    let submitted = session.submit(&transfer).await;
    drop(session);
    match submitted {
        Ok(()) => {
            if let Err(error) = state.chain.wait_final(&[transfer.hash()], WAIT).await {
                tracing::warn!(%error, "transfer recorded, inclusion not confirmed yet");
            }
        }
        Err(ChainError::Rejected(error)) => return Err(refused(&error_code(&error))),
        Err(ChainError::Unavailable(reason)) => {
            tracing::warn!(%reason, "transfer recorded but not delivered yet");
        }
        Err(other) => return Err(chain_unavailable(other)),
    }
    let view = find(&state, Condition::Hash(&hash))
        .await?
        .ok_or_else(|| ApiError::internal("recorded transfer vanished"))?;
    answer(view)
}

// --- Administration --------------------------------------------------------------------

#[derive(Debug, Serialize, sqlx::FromRow)]
struct AdminTransferView {
    tx_hash: String,
    status: String,
    amount: i64,
    message: String,
    sender_name: Option<String>,
    recipient_name: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

/// Every transfer, newest first, with its message (for moderation).
async fn list_all(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(page): Query<Page>,
) -> ApiResult<Json<Value>> {
    current.require(&[Role::Admin])?;
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM transfers")
        .fetch_one(&state.db)
        .await?;
    let items = sqlx::query_as::<_, AdminTransferView>(
        "SELECT t.tx_hash, c.status, t.amount, t.message, \
                s.first_name || ' ' || s.last_name AS sender_name, \
                r.first_name || ' ' || r.last_name AS recipient_name, t.created_at \
         FROM transfers t JOIN chain_txs c ON c.hash = t.tx_hash \
         LEFT JOIN users s ON s.id = t.sender_user_id \
         LEFT JOIN users r ON r.id = t.recipient_user_id \
         ORDER BY t.created_at DESC LIMIT $1 OFFSET $2",
    )
    .bind(page.limit())
    .bind(page.offset())
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({ "items": items, "total": total })))
}
