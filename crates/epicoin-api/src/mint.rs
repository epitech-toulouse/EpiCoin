//! Staff and admins: distributing EPC to students.
//!
//! A distribution is one batch of on-chain `Mint` transactions, signed with the
//! distributor's custodial key. The reason code goes on chain; the optional free-text comment
//! stays in Postgres.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::routing::{get, post};
use axum::{Json, Router};
use epicoin_core::{Address, Amount, ReasonCode, TxPayload};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::auth::CurrentUser;
use crate::chain::{ChainError, TxMeta};
use crate::error::{ApiError, ApiResult};
use crate::models::{Page, Role, COHORTS};
use crate::state::AppState;
use crate::users::like_pattern;
use crate::{audit, wallets};

const MAX_RECIPIENTS: usize = 500;
const MAX_COMMENT_CHARS: usize = 280;
const MAX_IDEMPOTENCY_KEY_LEN: usize = 100;
/// How long a distribution request waits for its transactions to be included.
const WAIT: Duration = Duration::from_secs(20);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/students", get(students))
        .route("/students/resolve", post(resolve_students))
        .route("/promos", get(promos))
        .route("/quota", get(quota))
        .route("/distributions", get(history).post(distribute))
}

// --- Finding students ------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct StudentView {
    pub id: Uuid,
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    pub promo: String,
    pub activated: bool,
}

#[derive(Deserialize)]
struct StudentParams {
    q: Option<String>,
    promo: Option<String>,
    // Not `#[serde(flatten)]`: flattened query strings lose their numeric types.
    limit: Option<i64>,
    offset: Option<i64>,
}

macro_rules! student_filter {
    () => {
        "role = 'student' AND disabled_at IS NULL \
         AND ($1::text IS NULL OR promo = $1) AND ($2::text IS NULL \
         OR email ILIKE $2 OR first_name ILIKE $2 OR last_name ILIKE $2 \
         OR (first_name || ' ' || last_name) ILIKE $2)"
    };
}

async fn students(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<StudentParams>,
) -> ApiResult<Json<Value>> {
    current.require(&[Role::Staff, Role::Admin])?;
    let page = Page {
        limit: params.limit,
        offset: params.offset,
    };
    let promo = params.promo.as_deref().filter(|p| !p.is_empty());
    let pattern = params.q.as_deref().and_then(like_pattern);

    let total: i64 = sqlx::query_scalar(concat!(
        "SELECT count(*) FROM users WHERE ",
        student_filter!()
    ))
    .bind(promo)
    .bind(&pattern)
    .fetch_one(&state.db)
    .await?;
    let items = sqlx::query_as::<_, StudentView>(concat!(
        "SELECT id, email, first_name, last_name, promo, \
         (activated_at IS NOT NULL) AS activated FROM users WHERE ",
        student_filter!(),
        " ORDER BY last_name, first_name, email LIMIT $3 OFFSET $4"
    ))
    .bind(promo)
    .bind(&pattern)
    .bind(page.limit())
    .bind(page.offset())
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({ "items": items, "total": total })))
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CohortView {
    pub code: String,
    /// Active (not disabled) students in the cohort.
    pub students: i64,
}

/// Every cohort, in display order, with its number of students.
async fn promos(
    State(state): State<AppState>,
    current: CurrentUser,
) -> ApiResult<Json<Vec<CohortView>>> {
    current.require(&[Role::Staff, Role::Admin])?;
    let counts: HashMap<String, i64> = sqlx::query_as(
        "SELECT promo, count(*) FROM users \
         WHERE role = 'student' AND disabled_at IS NULL GROUP BY promo",
    )
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .collect();
    let cohorts = COHORTS
        .into_iter()
        .map(|code| CohortView {
            code: code.to_owned(),
            students: counts.get(code).copied().unwrap_or(0),
        })
        .collect();
    Ok(Json(cohorts))
}

#[derive(Deserialize)]
struct ResolveRequest {
    emails: Vec<String>,
}

/// Turns a list of emails (from a CSV chosen in the browser) into students.
async fn resolve_students(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(body): Json<ResolveRequest>,
) -> ApiResult<Json<Value>> {
    current.require(&[Role::Staff, Role::Admin])?;
    if body.emails.len() > MAX_RECIPIENTS {
        return Err(too_many_recipients());
    }
    let wanted: Vec<String> = body
        .emails
        .iter()
        .map(|email| email.trim().to_lowercase())
        .filter(|email| !email.is_empty())
        .collect();
    let found = sqlx::query_as::<_, StudentView>(
        "SELECT id, email, first_name, last_name, promo, \
         (activated_at IS NOT NULL) AS activated \
         FROM users WHERE role = 'student' AND disabled_at IS NULL AND email = ANY($1) \
         ORDER BY last_name, first_name, email",
    )
    .bind(&wanted)
    .fetch_all(&state.db)
    .await?;
    let known: HashSet<&str> = found.iter().map(|s| s.email.as_str()).collect();
    let mut seen = HashSet::new();
    let missing: Vec<&String> = wanted
        .iter()
        .filter(|email| !known.contains(email.as_str()) && seen.insert(email.as_str()))
        .collect();
    Ok(Json(json!({ "found": found, "missing": missing })))
}

// --- Quota -----------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct QuotaView {
    /// Whether the staff member is currently allowed to mint on chain.
    pub active: bool,
    pub quota: Amount,
    pub minted: Amount,
    pub remaining: Amount,
    /// End of the current quota period (RFC 3339), when the quota is active.
    pub period_ends_at: Option<String>,
}

async fn quota_of(state: &AppState, address: &Address) -> ApiResult<QuotaView> {
    let authority = state.node.account(address).await?.authority;
    Ok(match authority {
        Some(a) if a.active => QuotaView {
            active: true,
            quota: a.quota,
            minted: a.minted,
            remaining: a.remaining,
            period_ends_at: OffsetDateTime::from_unix_timestamp_nanos(
                i128::from(a.period_ends_at) * 1_000_000,
            )
            .ok()
            .and_then(|t| {
                t.format(&time::format_description::well_known::Rfc3339)
                    .ok()
            }),
        },
        _ => QuotaView {
            active: false,
            quota: 0,
            minted: 0,
            remaining: 0,
            period_ends_at: None,
        },
    })
}

async fn quota(State(state): State<AppState>, current: CurrentUser) -> ApiResult<Json<QuotaView>> {
    current.require(&Role::DISTRIBUTORS)?;
    let address = wallets::address_of(&state.db, current.id()).await?;
    Ok(Json(quota_of(&state, &address).await?))
}

// --- Distribution ----------------------------------------------------------------------

#[derive(Deserialize)]
struct DistributeRequest {
    recipient_ids: Vec<Uuid>,
    /// Amount for each recipient, in units (1 EPC = 100 units).
    amount: Amount,
    reason_code: ReasonCode,
    #[serde(default)]
    comment: String,
}

fn too_many_recipients() -> ApiError {
    ApiError::unprocessable(
        "too_many_recipients",
        format!("Une distribution est limitée à {MAX_RECIPIENTS} étudiants."),
    )
}

fn reason_name(reason: ReasonCode) -> String {
    serde_json::to_value(reason)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default()
}

async fn distribute(
    State(state): State<AppState>,
    current: CurrentUser,
    headers: HeaderMap,
    Json(body): Json<DistributeRequest>,
) -> ApiResult<Json<Value>> {
    current.require(&Role::DISTRIBUTORS)?;

    // An idempotency key makes a double click or a retry return the first result.
    let idempotency_key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|key| !key.is_empty());
    if idempotency_key.is_some_and(|key| key.len() > MAX_IDEMPOTENCY_KEY_LEN) {
        return Err(ApiError::bad_request("Clé d'idempotence trop longue."));
    }
    if let Some(key) = idempotency_key {
        if let Some(batch_id) = existing_batch(&state, current.id(), key).await? {
            return Ok(Json(batch_view(&state, batch_id).await?));
        }
    }

    // --- Validation: nothing is written or sent unless everything is acceptable.
    let mut seen = HashSet::new();
    let recipient_ids: Vec<Uuid> = body
        .recipient_ids
        .iter()
        .copied()
        .filter(|id| seen.insert(*id))
        .collect();
    if recipient_ids.is_empty() {
        return Err(ApiError::unprocessable(
            "no_recipient",
            "Sélectionnez au moins un étudiant.",
        ));
    }
    if recipient_ids.len() > MAX_RECIPIENTS {
        return Err(too_many_recipients());
    }
    if body.amount == 0 {
        return Err(ApiError::unprocessable(
            "invalid_amount",
            "Le montant doit être supérieur à zéro.",
        ));
    }
    let comment = body.comment.trim();
    if comment.chars().count() > MAX_COMMENT_CHARS {
        return Err(ApiError::unprocessable(
            "comment_too_long",
            format!("Le commentaire est limité à {MAX_COMMENT_CHARS} caractères."),
        ));
    }
    let total = body
        .amount
        .checked_mul(recipient_ids.len() as Amount)
        .filter(|total| i64::try_from(*total).is_ok())
        .ok_or_else(|| ApiError::unprocessable("invalid_amount", "Montant trop élevé."))?;

    let recipients: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT u.id, w.address FROM users u JOIN wallets w ON w.user_id = u.id \
         WHERE u.id = ANY($1) AND u.role = 'student' AND u.disabled_at IS NULL",
    )
    .bind(&recipient_ids)
    .fetch_all(&state.db)
    .await?;
    if recipients.len() != recipient_ids.len() {
        let valid: HashSet<Uuid> = recipients.iter().map(|(id, _)| *id).collect();
        let invalid: Vec<Uuid> = recipient_ids
            .iter()
            .copied()
            .filter(|id| !valid.contains(id))
            .collect();
        return Err(ApiError::unprocessable(
            "invalid_recipients",
            "Certains destinataires ne sont pas des étudiants actifs.",
        )
        .with_details(json!({ "invalid_ids": invalid })));
    }

    let keypair = wallets::load_keypair(&state.db, &state.vault, current.id()).await?;
    let quota = quota_of(&state, &keypair.address()).await?;
    if !quota.active {
        return Err(ApiError::conflict(
            "authority_inactive",
            "Votre autorisation de distribution n'est pas encore active sur la blockchain. \
             Réessayez dans une minute ou contactez un administrateur.",
        ));
    }
    if total > quota.remaining {
        return Err(ApiError::unprocessable(
            "quota_exceeded",
            "Cette distribution dépasse votre quota restant pour la période.",
        )
        .with_details(json!({ "needed": total, "remaining": quota.remaining })));
    }

    // --- Record the batch, then mint.
    let batch_id = Uuid::new_v4();
    let reason = reason_name(body.reason_code);
    let inserted: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO mint_batches \
         (id, actor_user_id, idempotency_key, amount, recipient_count, reason_code, comment) \
         VALUES ($1, $2, $3, $4, $5, $6, $7) \
         ON CONFLICT (actor_user_id, idempotency_key) DO NOTHING RETURNING id",
    )
    .bind(batch_id)
    .bind(current.id())
    .bind(idempotency_key)
    .bind(body.amount as i64)
    .bind(recipients.len() as i32)
    .bind(&reason)
    .bind(comment)
    .fetch_optional(&state.db)
    .await?;
    if inserted.is_none() {
        // A concurrent request with the same idempotency key won the race.
        let key = idempotency_key.unwrap_or_default();
        let batch_id = existing_batch(&state, current.id(), key)
            .await?
            .ok_or_else(|| ApiError::internal("idempotent batch vanished"))?;
        return Ok(Json(batch_view(&state, batch_id).await?));
    }

    // Keep the caller's order.
    let address_of: std::collections::HashMap<Uuid, &String> = recipients
        .iter()
        .map(|(id, address)| (*id, address))
        .collect();
    let mut items = Vec::with_capacity(recipient_ids.len());
    for id in &recipient_ids {
        let to: Address = address_of[id]
            .parse()
            .map_err(|_| ApiError::internal(format!("user {id} has a malformed wallet address")))?;
        items.push((
            TxPayload::Mint {
                to,
                amount: body.amount,
                reason: body.reason_code,
            },
            TxMeta {
                actor_user_id: Some(current.id()),
                subject_user_id: Some(*id),
                amount: Some(body.amount),
                reason_code: Some(reason.clone()),
                batch_id: Some(batch_id),
                order_id: None,
            },
        ));
    }
    let rows = match state.chain.send_batch(&keypair, items, WAIT).await {
        Ok(rows) => rows,
        Err(ChainError::Db(error)) => return Err(error.into()),
        Err(error) => {
            tracing::warn!(%error, "distribution could not be submitted");
            return Err(ApiError::unavailable(
                "La blockchain est momentanément indisponible. Aucun EPC n'a été distribué.",
            ));
        }
    };

    audit::record(
        &state.db,
        Some(current.id()),
        audit::action::MINT,
        Some(("mint_batch", batch_id.to_string())),
        json!({
            "recipients": recipient_ids.len(),
            "amount": body.amount,
            "reason_code": reason,
            "included": rows.iter().filter(|r| r.is_included()).count(),
        }),
    )
    .await?;
    Ok(Json(batch_view(&state, batch_id).await?))
}

async fn existing_batch(state: &AppState, actor: Uuid, key: &str) -> ApiResult<Option<Uuid>> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM mint_batches WHERE actor_user_id = $1 AND idempotency_key = $2",
    )
    .bind(actor)
    .bind(key)
    .fetch_optional(&state.db)
    .await?)
}

#[derive(sqlx::FromRow)]
struct BatchRow {
    amount: i64,
    recipient_count: i32,
    reason_code: String,
    comment: String,
    created_at: OffsetDateTime,
}

#[derive(Serialize, sqlx::FromRow)]
struct BatchResult {
    user_id: Option<Uuid>,
    name: Option<String>,
    tx_hash: String,
    status: String,
}

/// The current state of a distribution: one result per transaction, and an overall status
/// (`included`, `pending`, `partial` or `failed`).
async fn batch_view(state: &AppState, batch_id: Uuid) -> ApiResult<Value> {
    let batch = sqlx::query_as::<_, BatchRow>(
        "SELECT amount, recipient_count, reason_code, comment, created_at \
         FROM mint_batches WHERE id = $1",
    )
    .bind(batch_id)
    .fetch_one(&state.db)
    .await?;
    let results = sqlx::query_as::<_, BatchResult>(
        "SELECT c.subject_user_id AS user_id, u.first_name || ' ' || u.last_name AS name, \
                c.hash AS tx_hash, c.status \
         FROM chain_txs c LEFT JOIN users u ON u.id = c.subject_user_id \
         WHERE c.batch_id = $1 ORDER BY c.nonce",
    )
    .bind(batch_id)
    .fetch_all(&state.db)
    .await?;

    let count = |status: &str| results.iter().filter(|r| r.status == status).count();
    let requested = batch.recipient_count as usize;
    let included = count(crate::chain::STATUS_INCLUDED);
    let failed = count(crate::chain::STATUS_FAILED) + requested.saturating_sub(results.len());
    let status = if included == requested {
        "included"
    } else if failed == 0 {
        "pending"
    } else if included == 0 {
        "failed"
    } else {
        "partial"
    };
    Ok(json!({
        "batch_id": batch_id,
        "status": status,
        "amount": batch.amount,
        "reason_code": batch.reason_code,
        "comment": batch.comment,
        "created_at": batch
            .created_at
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap_or_default(),
        "requested": requested,
        "included": included,
        "results": results,
    }))
}

// --- History ---------------------------------------------------------------------------

#[derive(Serialize, sqlx::FromRow)]
struct DistributionRow {
    tx_hash: String,
    batch_id: Option<Uuid>,
    status: String,
    amount: Option<i64>,
    reason_code: Option<String>,
    comment: Option<String>,
    recipient_id: Option<Uuid>,
    recipient_name: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

/// The distributor's own distributions, newest first.
async fn history(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(page): Query<Page>,
) -> ApiResult<Json<Value>> {
    current.require(&Role::DISTRIBUTORS)?;
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM chain_txs WHERE actor_user_id = $1 AND kind = 'mint'",
    )
    .bind(current.id())
    .fetch_one(&state.db)
    .await?;
    let items = sqlx::query_as::<_, DistributionRow>(
        "SELECT c.hash AS tx_hash, c.batch_id, c.status, c.amount, c.reason_code, b.comment, \
                c.subject_user_id AS recipient_id, \
                u.first_name || ' ' || u.last_name AS recipient_name, c.created_at \
         FROM chain_txs c \
         LEFT JOIN mint_batches b ON b.id = c.batch_id \
         LEFT JOIN users u ON u.id = c.subject_user_id \
         WHERE c.actor_user_id = $1 AND c.kind = 'mint' \
         ORDER BY c.created_at DESC, c.nonce DESC LIMIT $2 OFFSET $3",
    )
    .bind(current.id())
    .bind(page.limit())
    .bind(page.offset())
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({ "items": items, "total": total })))
}
