//! The student side of a QR code: what it gives, then the payment. Read the module
//! documentation of [`super`] (paying exactly once) before changing [`claim`].

use std::collections::BTreeSet;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use epicoin_core::{Amount, ReasonCode, TxPayload};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::PgExecutor;
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use super::calendar::completed_tier;
use super::{today, valid_share, Settings};
use crate::auth::CurrentUser;
use crate::chain::{record_tx, ChainError, TxMeta, STATUS_FAILED};
use crate::error::{ApiError, ApiResult};
use crate::models::{Role, User};
use crate::secrets::token_hash;
use crate::state::AppState;
use crate::{audit, authority, mint, wallets};

/// Previews and scans a student may make per [`SCAN_WINDOW`].
const MAX_SCANS: u32 = 30;
const SCAN_WINDOW: Duration = Duration::from_secs(60);
/// How long a payment waits for its inclusion before answering.
const WAIT: Duration = Duration::from_secs(20);

#[derive(Deserialize)]
pub(super) struct TokenBody {
    token: String,
}

/// A share as seen through its QR code.
#[derive(sqlx::FromRow)]
struct Scanned {
    id: Uuid,
    staff_user_id: Option<Uuid>,
    student_user_id: Option<Uuid>,
    amount: i64,
    mention: Option<String>,
    topic: String,
    invalidated_at: Option<OffsetDateTime>,
    /// The status of its payment transaction, if it has one.
    tx_status: Option<String>,
    expired: bool,
    seconds_left: f64,
    staff_name: Option<String>,
}

/// Columns of [`Scanned`], for a share `s`.
macro_rules! scanned_columns {
    () => {
        "SELECT s.id, s.staff_user_id, s.student_user_id, s.amount, s.mention, s.topic, \
                s.invalidated_at, t.status AS tx_status, (s.expires_at <= now()) AS expired, \
                GREATEST(EXTRACT(EPOCH FROM s.expires_at - now()), 0)::float8 AS seconds_left, \
                u.first_name || ' ' || u.last_name AS staff_name \
         FROM shares s \
         LEFT JOIN chain_txs t ON t.hash = s.tx_hash \
         LEFT JOIN users u ON u.id = s.staff_user_id "
    };
}

fn invalid() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "share_invalid",
        "Ce QR code est invalide.",
    )
}

/// Looks the share up by its secret.
async fn find(db: impl PgExecutor<'_>, token: &str) -> ApiResult<Scanned> {
    let token = token.trim();
    if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    sqlx::query_as::<_, Scanned>(concat!(scanned_columns!(), "WHERE s.token_hash = $1"))
        .bind(token_hash(token))
        .fetch_optional(db)
        .await?
        .ok_or_else(invalid)
}

/// Whether `student` may still scan the share.
fn check_usable(share: &Scanned, student: Uuid) -> ApiResult<()> {
    if share
        .tx_status
        .as_deref()
        .is_some_and(|status| status != STATUS_FAILED)
    {
        return Err(if share.student_user_id == Some(student) {
            ApiError::conflict("share_already_yours", "Vous avez déjà récupéré ce partage.")
        } else {
            ApiError::conflict("share_used", "Ce QR code a déjà été utilisé.")
        });
    }
    if share.invalidated_at.is_some() {
        return Err(ApiError::conflict(
            "share_invalidated",
            "Ce QR code a été remplacé ou retiré par l'encadrant. Demandez-lui d'en afficher un \
             nouveau.",
        ));
    }
    if share.expired {
        return Err(ApiError::conflict(
            "share_expired",
            "Ce QR code a expiré. Demandez à l'encadrant d'en générer un nouveau.",
        ));
    }
    Ok(())
}

fn rate_limit(state: &AppState, student: Uuid) -> ApiResult<()> {
    if state
        .limiter
        .hit(&format!("share-scan:{student}"), MAX_SCANS, SCAN_WINDOW)
    {
        Ok(())
    } else {
        Err(ApiError::too_many_requests())
    }
}

/// What a QR code gives, before the student takes it.
pub(super) async fn preview(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(body): Json<TokenBody>,
) -> ApiResult<Json<Value>> {
    current.require(&[Role::Student])?;
    rate_limit(&state, current.id())?;
    let share = find(&state.db, &body.token).await?;
    check_usable(&share, current.id())?;
    Ok(Json(json!({
        "amount": share.amount,
        "mention": share.mention,
        "topic": share.topic,
        "staff_name": share.staff_name,
        "expires_in": share.seconds_left.ceil() as u64,
    })))
}

fn chain_unavailable(error: ChainError) -> ApiError {
    match error {
        ChainError::Db(error) => error.into(),
        other => {
            tracing::warn!(error = %other, "share payment not sent");
            ApiError::unavailable(
                "La blockchain est momentanément indisponible. Réessayez dans un instant.",
            )
        }
    }
}

/// Scans the QR code: pays the share, and the streak bonus it completes, in one mint signed
/// by the staff member who generated it.
pub(super) async fn claim(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(body): Json<TokenBody>,
) -> ApiResult<Json<Value>> {
    current.require(&[Role::Student])?;
    rate_limit(&state, current.id())?;
    let student = current.id();
    let share = find(&state.db, &body.token).await?;
    check_usable(&share, student)?;
    let settings = Settings::load(&state.db).await?;

    // --- The staff member must be able to pay; the chain checks it again.
    let unfunded = || {
        ApiError::conflict(
            "share_unfunded",
            "Le quota de distribution de l'encadrant ne suffit pas pour ce partage (bonus de \
             régularité compris). Rien n'a été versé : prévenez-le, puis réessayez.",
        )
    };
    let staff_id = share.staff_user_id.ok_or_else(unfunded)?;
    let staff = sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1")
        .bind(staff_id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(unfunded)?;
    if authority::desired_quota(&staff, &state.settings).is_none() {
        return Err(unfunded());
    }
    let keypair = wallets::load_keypair(&state.db, &state.vault, staff_id).await?;
    let to = wallets::address_of(&state.db, student).await?;

    // --- Signing rights, then the student and the share, locked; then everything at once.
    let mut session = state
        .chain
        .begin(&keypair)
        .await
        .map_err(chain_unavailable)?;
    let mut tx = state.db.begin().await?;
    sqlx::query("SELECT id FROM users WHERE id = $1 FOR UPDATE")
        .bind(student)
        .execute(&mut *tx)
        .await?;
    let locked = sqlx::query_as::<_, Scanned>(concat!(
        scanned_columns!(),
        "WHERE s.id = $1 FOR UPDATE OF s"
    ))
    .bind(share.id)
    .fetch_one(&mut *tx)
    .await?;
    check_usable(&locked, student)?;

    let today = today(&mut *tx, &settings.time_zone).await?;
    if let Some(limit) = settings.daily_limit {
        let scanned_today: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM shares s JOIN chain_txs t ON t.hash = s.tx_hash \
             WHERE s.student_user_id = $1 AND s.claim_day = $2 AND t.status <> 'failed'",
        )
        .bind(student)
        .bind(today)
        .fetch_one(&mut *tx)
        .await?;
        if scanned_today >= i64::from(limit) {
            return Err(ApiError::conflict(
                "daily_limit",
                format!(
                    "Vous avez atteint la limite de {limit} partage{} par jour. À demain !",
                    if limit > 1 { "s" } else { "" }
                ),
            ));
        }
    }

    // --- The streak bonus, if this scan completes a tier not paid yet.
    let days: BTreeSet<Date> = sqlx::query_scalar::<_, Date>(concat!(
        "SELECT s.claim_day FROM shares s JOIN chain_txs t ON t.hash = s.tx_hash \
         WHERE s.student_user_id = $1 AND ",
        valid_share!()
    ))
    .bind(student)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .collect();
    let mut bonus: Amount = 0;
    let mut streak = None;
    if let Some((reached, tier)) =
        completed_tier(&days, today, &settings.calendar(), &settings.tiers)
    {
        let paid: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM shares s JOIN chain_txs t ON t.hash = s.tx_hash \
               WHERE s.student_user_id = $1 AND s.streak_start = $2 AND s.streak_days = $3 \
                 AND s.bonus > 0 AND t.status <> 'failed')",
        )
        .bind(student)
        .bind(reached.start)
        .bind(reached.days as i32)
        .fetch_one(&mut *tx)
        .await?;
        if !paid {
            bonus = tier.bonus;
            streak = Some(reached);
        }
    }
    let amount = locked.amount as Amount;
    let total = amount
        .checked_add(bonus)
        .filter(|total| i64::try_from(*total).is_ok())
        .ok_or_else(|| ApiError::internal("share amount overflow"))?;
    let quota = mint::quota_of(&state, &keypair.address()).await?;
    if !quota.active || quota.remaining < total {
        return Err(unfunded());
    }

    let batch_id = Uuid::new_v4();
    let reason = mint::reason_name(ReasonCode::KnowledgeShare);
    let mut comment = if locked.topic.is_empty() {
        "Partage de connaissance".to_owned()
    } else {
        format!("Partage : {}", locked.topic)
    };
    if let Some(streak) = streak {
        comment.push_str(&format!(" · bonus de régularité ({} jours)", streak.days));
    }
    sqlx::query(
        "INSERT INTO mint_batches (id, actor_user_id, amount, recipient_count, reason_code, comment) \
         VALUES ($1, $2, $3, 1, $4, $5)",
    )
    .bind(batch_id)
    .bind(staff_id)
    .bind(total as i64)
    .bind(&reason)
    .bind(&comment)
    .execute(&mut *tx)
    .await?;
    let mint = session.sign(TxPayload::Mint {
        to,
        amount: total,
        reason: ReasonCode::KnowledgeShare,
    });
    record_tx(
        &mut tx,
        &mint,
        &TxMeta {
            actor_user_id: Some(staff_id),
            subject_user_id: Some(student),
            amount: Some(total),
            reason_code: Some(reason),
            batch_id: Some(batch_id),
            order_id: None,
        },
    )
    .await?;
    let hash = mint.hash();
    sqlx::query(
        "UPDATE shares SET student_user_id = $2, claimed_at = now(), claim_day = $3, bonus = $4, \
           streak_start = $5, streak_days = $6, tx_hash = $7 \
         WHERE id = $1",
    )
    .bind(locked.id)
    .bind(student)
    .bind(today)
    .bind(bonus as i64)
    .bind(streak.map(|s| s.start))
    .bind(streak.map(|s| s.days as i32))
    .bind(hash.to_string())
    .execute(&mut *tx)
    .await?;
    audit::record(
        &mut *tx,
        Some(student),
        audit::action::SHARE_CLAIMED,
        Some(("share", locked.id.to_string())),
        json!({ "staff": staff_id, "amount": amount, "bonus": bonus, "tx": hash }),
    )
    .await?;
    tx.commit().await?;

    // --- Submit. A refusal leaves the share unclaimed: it can be scanned again.
    let submitted = session.submit(&mint).await;
    drop(session);
    match submitted {
        Ok(()) => {
            if let Err(error) = state.chain.wait_final(&[hash], WAIT).await {
                tracing::warn!(%error, "share paid, inclusion not confirmed yet");
            }
        }
        Err(ChainError::Rejected(error)) => {
            return Err(ApiError::conflict(
                "reward_refused",
                "La blockchain a refusé le versement. Réessayez dans un instant.",
            )
            .with_details(json!({ "reason": crate::chain::error_code(&error) })));
        }
        Err(ChainError::Unavailable(reason)) => {
            tracing::warn!(%reason, "share payment recorded but not delivered yet");
        }
        Err(other) => return Err(chain_unavailable(other)),
    }
    let row = state.chain.row(&hash).await?;
    Ok(Json(json!({
        "amount": amount,
        "bonus": bonus,
        "streak_days": streak.map(|s| s.days),
        "mention": locked.mention,
        "staff_name": locked.staff_name,
        "reward_status": row.status,
        "tx_hash": row.hash,
    })))
}
