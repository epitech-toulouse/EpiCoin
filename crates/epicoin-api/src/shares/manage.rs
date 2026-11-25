//! Staff and admins: QR codes, their status and history. Admins only: cancellation, the
//! "Légende du campus" badge and the settings.

use std::time::Duration;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use epicoin_core::Amount;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use time::OffsetDateTime;
use uuid::Uuid;

use super::badges::{ADMIN_MENTION, MENTIONS};
use super::{epc_fr, known_time_zone, Settings, MAX_TOPIC_CHARS};
use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::models::Role;
use crate::secrets::{random_token, token_hash};
use crate::state::AppState;
use crate::{audit, mint, wallets};

/// QR codes a staff member may generate per [`CREATE_WINDOW`].
const MAX_CREATIONS: u32 = 30;
const CREATE_WINDOW: Duration = Duration::from_secs(60);
const MIN_REASON_CHARS: usize = 5;
const MAX_REASON_CHARS: usize = 1_000;

// --- Settings --------------------------------------------------------------------------

/// The settings, for every logged-in user: the forms and pages need the amounts, tiers and
/// calendar.
pub(super) async fn settings(
    State(state): State<AppState>,
    _current: CurrentUser,
) -> ApiResult<Json<Settings>> {
    Ok(Json(Settings::load(&state.db).await?))
}

pub(super) async fn update_settings(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(body): Json<Settings>,
) -> ApiResult<Json<Settings>> {
    current.require(&[Role::Admin])?;
    let settings = body
        .validated()
        .map_err(|message| ApiError::unprocessable("invalid_settings", message))?;
    if !known_time_zone(&state.db, &settings.time_zone).await? {
        return Err(ApiError::unprocessable(
            "invalid_settings",
            "Fuseau horaire inconnu (exemple : Europe/Paris).",
        ));
    }
    let mut tx = state.db.begin().await?;
    settings.save(&mut *tx).await?;
    audit::record(
        &mut *tx,
        Some(current.id()),
        audit::action::SHARE_SETTINGS_UPDATED,
        None,
        json!({
            "standard": settings.standard,
            "max_amount": settings.max_amount,
            "qr_seconds": settings.qr_seconds,
            "tiers": settings.tiers.len(),
            "closures": settings.closures.len(),
        }),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(settings))
}

// --- QR codes --------------------------------------------------------------------------

#[derive(Deserialize)]
pub(super) struct NewShare {
    /// In units.
    amount: Amount,
    #[serde(default)]
    mention: Option<String>,
    #[serde(default)]
    topic: String,
}

/// The modules of a QR code, row by row: `size × size` characters, `1` for dark.
fn qr_matrix(text: &str) -> ApiResult<Value> {
    let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::M)
        .map_err(ApiError::internal)?;
    let modules: String = code
        .to_colors()
        .iter()
        .map(|color| {
            if *color == qrcode::Color::Dark {
                '1'
            } else {
                '0'
            }
        })
        .collect();
    Ok(json!({ "size": code.width(), "modules": modules }))
}

/// Generates a QR code for a share. The staff member's previous QR code that was not
/// scanned stops working.
pub(super) async fn create(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(body): Json<NewShare>,
) -> ApiResult<(StatusCode, Json<Value>)> {
    current.require(&Role::DISTRIBUTORS)?;
    if !state.limiter.hit(
        &format!("share-create:{}", current.id()),
        MAX_CREATIONS,
        CREATE_WINDOW,
    ) {
        return Err(ApiError::too_many_requests());
    }
    let settings = Settings::load(&state.db).await?;
    if body.amount == 0 || body.amount > settings.max_amount {
        return Err(ApiError::unprocessable(
            "invalid_amount",
            format!(
                "Le montant d'un partage va de 0,01 à {}.",
                epc_fr(settings.max_amount)
            ),
        ));
    }
    let mention = body
        .mention
        .as_deref()
        .map(str::trim)
        .filter(|mention| !mention.is_empty());
    if let Some(mention) = mention {
        if !MENTIONS.contains(&mention) {
            return Err(ApiError::unprocessable(
                "unknown_mention",
                "Mention inconnue.",
            ));
        }
        if mention == ADMIN_MENTION && current.user.role != Role::Admin {
            return Err(ApiError::new(
                StatusCode::FORBIDDEN,
                "mention_reserved",
                "Cette mention est réservée aux administrateurs.",
            ));
        }
    }
    let topic = body.topic.trim();
    if topic.chars().count() > MAX_TOPIC_CHARS {
        return Err(ApiError::unprocessable(
            "topic_too_long",
            format!("Le sujet est limité à {MAX_TOPIC_CHARS} caractères."),
        ));
    }

    // Early answer when the quota cannot pay; the scan checks it again, bonus included.
    let address = wallets::address_of(&state.db, current.id()).await?;
    let quota = mint::quota_of(&state, &address).await?;
    if !quota.active {
        return Err(ApiError::conflict(
            "authority_inactive",
            "Votre autorisation de distribution n'est pas encore active sur la blockchain. \
             Réessayez dans une minute ou contactez un administrateur.",
        ));
    }
    if body.amount > quota.remaining {
        return Err(ApiError::unprocessable(
            "quota_exceeded",
            "Ce montant dépasse votre quota restant pour la période.",
        )
        .with_details(json!({ "needed": body.amount, "remaining": quota.remaining })));
    }

    let token = random_token();
    let id = Uuid::new_v4();
    let mut tx = state.db.begin().await?;
    sqlx::query(
        "UPDATE shares SET invalidated_at = now() \
         WHERE staff_user_id = $1 AND invalidated_at IS NULL AND expires_at > now() \
           AND (tx_hash IS NULL OR EXISTS (SELECT 1 FROM chain_txs t \
                                           WHERE t.hash = shares.tx_hash AND t.status = 'failed'))",
    )
    .bind(current.id())
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "INSERT INTO shares (id, token_hash, staff_user_id, amount, standard, mention, topic, expires_at) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, now() + make_interval(secs => $8))",
    )
    .bind(id)
    .bind(token_hash(&token))
    .bind(current.id())
    .bind(body.amount as i64)
    .bind(settings.standard as i64)
    .bind(mention)
    .bind(topic)
    .bind(f64::from(settings.qr_seconds))
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    // The secret travels in the fragment: browsers never send it to a server.
    let link = format!("{}/partage#token={token}", state.settings.public_url);
    let qr = qr_matrix(&link)?;
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "link": link,
            "qr": qr,
            "amount": body.amount,
            "mention": mention,
            "topic": topic,
            "expires_in": settings.qr_seconds,
        })),
    ))
}

/// A share as staff and admins see it.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct ShareView {
    pub id: Uuid,
    /// `open`, `expired`, `invalidated`, `claimed` or `cancelled`.
    pub status: String,
    /// The payment transaction's status, once scanned: `pending`, `included` or `failed`.
    pub reward_status: Option<String>,
    pub amount: i64,
    /// Streak bonus paid with the share, in units.
    pub bonus: i64,
    pub streak_days: Option<i32>,
    pub mention: Option<String>,
    pub topic: String,
    pub staff_id: Option<Uuid>,
    pub staff_name: Option<String>,
    /// Only once claimed.
    pub student_id: Option<Uuid>,
    pub student_name: Option<String>,
    /// Seconds left to scan it.
    pub expires_in: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    #[serde(with = "time::serde::rfc3339::option")]
    pub claimed_at: Option<OffsetDateTime>,
    #[serde(with = "time::serde::rfc3339::option")]
    pub cancelled_at: Option<OffsetDateTime>,
    pub cancel_reason: String,
}

/// Selects [`ShareView`] rows; `s` is the share, `t` its payment.
macro_rules! share_views {
    () => {
        "SELECT s.id, \
                CASE WHEN s.cancelled_at IS NOT NULL THEN 'cancelled' \
                     WHEN t.status <> 'failed' THEN 'claimed' \
                     WHEN s.invalidated_at IS NOT NULL THEN 'invalidated' \
                     WHEN s.expires_at <= now() THEN 'expired' \
                     ELSE 'open' END AS status, \
                t.status AS reward_status, s.amount, s.bonus, s.streak_days, s.mention, s.topic, \
                s.staff_user_id AS staff_id, sf.first_name || ' ' || sf.last_name AS staff_name, \
                CASE WHEN t.status <> 'failed' THEN s.student_user_id END AS student_id, \
                CASE WHEN t.status <> 'failed' THEN st.first_name || ' ' || st.last_name END \
                  AS student_name, \
                GREATEST(CEIL(EXTRACT(EPOCH FROM s.expires_at - now())), 0)::bigint AS expires_in, \
                s.created_at, CASE WHEN t.status <> 'failed' THEN s.claimed_at END AS claimed_at, \
                s.cancelled_at, s.cancel_reason \
         FROM shares s \
         LEFT JOIN chain_txs t ON t.hash = s.tx_hash \
         LEFT JOIN users sf ON sf.id = s.staff_user_id \
         LEFT JOIN users st ON st.id = s.student_user_id "
    };
}

/// The share `id`, if `current` may see it: their own, or any for an admin.
async fn visible(state: &AppState, current: &CurrentUser, id: Uuid) -> ApiResult<ShareView> {
    current.require(&Role::DISTRIBUTORS)?;
    let share = sqlx::query_as::<_, ShareView>(concat!(share_views!(), "WHERE s.id = $1"))
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError::not_found("Partage"))?;
    if share.staff_id != Some(current.id()) && current.user.role != Role::Admin {
        return Err(ApiError::not_found("Partage"));
    }
    Ok(share)
}

/// Where a QR code stands: the staff member's screen polls it until it is scanned.
pub(super) async fn status(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<Json<ShareView>> {
    Ok(Json(visible(&state, &current, id).await?))
}

/// Withdraws a QR code that was not scanned. Doing it twice is harmless.
pub(super) async fn invalidate(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    visible(&state, &current, id).await?;
    let updated = sqlx::query(
        "UPDATE shares SET invalidated_at = COALESCE(invalidated_at, now()) \
         WHERE id = $1 AND (tx_hash IS NULL OR EXISTS (SELECT 1 FROM chain_txs t \
                            WHERE t.hash = shares.tx_hash AND t.status = 'failed'))",
    )
    .bind(id)
    .execute(&state.db)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(ApiError::conflict(
            "share_used",
            "Ce QR code a déjà été utilisé.",
        ));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub(super) struct HistoryParams {
    /// `all`: every staff member's shares (admins only).
    scope: Option<String>,
    // Not `#[serde(flatten)]`: flattened query strings lose their numeric types.
    limit: Option<i64>,
    offset: Option<i64>,
}

/// Scanned shares, newest first: the staff member's own, or everyone's for an admin who
/// asks for `scope=all`.
pub(super) async fn history(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<HistoryParams>,
) -> ApiResult<Json<Value>> {
    current.require(&Role::DISTRIBUTORS)?;
    let all = params.scope.as_deref() == Some("all");
    if all {
        current.require(&[Role::Admin])?;
    }
    let page = crate::models::Page {
        limit: params.limit,
        offset: params.offset,
    };
    let staff = (!all).then(|| current.id());
    let total: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM shares s JOIN chain_txs t ON t.hash = s.tx_hash \
         WHERE t.status <> 'failed' AND ($1::uuid IS NULL OR s.staff_user_id = $1)",
    )
    .bind(staff)
    .fetch_one(&state.db)
    .await?;
    let items = sqlx::query_as::<_, ShareView>(concat!(
        share_views!(),
        "WHERE t.status <> 'failed' AND ($1::uuid IS NULL OR s.staff_user_id = $1) \
         ORDER BY s.claimed_at DESC, s.id LIMIT $2 OFFSET $3"
    ))
    .bind(staff)
    .bind(page.limit())
    .bind(page.offset())
    .fetch_all(&state.db)
    .await?;
    Ok(Json(json!({ "items": items, "total": total })))
}

// --- Admins: cancellation and legend ---------------------------------------------------

#[derive(Deserialize)]
pub(super) struct CancelRequest {
    reason: String,
}

/// Takes a scanned share out of streaks, badges and the ranking. Its EPC stay: the chain
/// cannot take them back.
pub(super) async fn cancel(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<CancelRequest>,
) -> ApiResult<StatusCode> {
    current.require(&[Role::Admin])?;
    let reason = body.reason.trim();
    let length = reason.chars().count();
    if !(MIN_REASON_CHARS..=MAX_REASON_CHARS).contains(&length) {
        return Err(ApiError::unprocessable(
            "invalid_reason",
            format!(
                "Une justification de {MIN_REASON_CHARS} à {MAX_REASON_CHARS} caractères est \
                 obligatoire."
            ),
        ));
    }
    let mut tx = state.db.begin().await?;
    let cancelled: Option<(Option<Uuid>, i64, i64)> = sqlx::query_as(
        "UPDATE shares s SET cancelled_at = now(), cancelled_by = $2, cancel_reason = $3 \
         FROM chain_txs t \
         WHERE s.id = $1 AND t.hash = s.tx_hash AND t.status <> 'failed' \
           AND s.cancelled_at IS NULL \
         RETURNING s.student_user_id, s.amount, s.bonus",
    )
    .bind(id)
    .bind(current.id())
    .bind(reason)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((student, amount, bonus)) = cancelled else {
        let already: Option<bool> =
            sqlx::query_scalar("SELECT cancelled_at IS NOT NULL FROM shares WHERE id = $1")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        return Err(match already {
            None => ApiError::not_found("Partage"),
            Some(true) => ApiError::conflict("already_cancelled", "Ce partage est déjà annulé."),
            Some(false) => ApiError::conflict(
                "share_not_claimed",
                "Seul un partage récupéré peut être annulé.",
            ),
        });
    };
    audit::record(
        &mut *tx,
        Some(current.id()),
        audit::action::SHARE_CANCELLED,
        Some(("share", id.to_string())),
        json!({ "student": student, "amount": amount, "bonus": bonus }),
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Gives "Légende du campus" to a student.
pub(super) async fn grant_legend(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(user_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    current.require(&[Role::Admin])?;
    let student: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users \
           WHERE id = $1 AND role = 'student' AND disabled_at IS NULL)",
    )
    .bind(user_id)
    .fetch_one(&state.db)
    .await?;
    if !student {
        return Err(ApiError::unprocessable(
            "not_a_student",
            "Ce badge se donne à un étudiant actif.",
        ));
    }
    let mut tx = state.db.begin().await?;
    let inserted = sqlx::query(
        "INSERT INTO share_legends (user_id, granted_by) VALUES ($1, $2) \
         ON CONFLICT (user_id) DO NOTHING",
    )
    .bind(user_id)
    .bind(current.id())
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if inserted > 0 {
        audit::record(
            &mut *tx,
            Some(current.id()),
            audit::action::SHARE_LEGEND_GRANTED,
            Some(("user", user_id.to_string())),
            json!({}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(super) async fn remove_legend(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(user_id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    current.require(&[Role::Admin])?;
    let mut tx = state.db.begin().await?;
    let removed = sqlx::query("DELETE FROM share_legends WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if removed > 0 {
        audit::record(
            &mut *tx,
            Some(current.id()),
            audit::action::SHARE_LEGEND_REMOVED,
            Some(("user", user_id.to_string())),
            json!({}),
        )
        .await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_qr_matrix_is_square() {
        let qr = qr_matrix("http://localhost:8088/partage#token=00ff").unwrap();
        let size = qr["size"].as_u64().unwrap() as usize;
        let modules = qr["modules"].as_str().unwrap();
        assert!(size >= 21);
        assert_eq!(modules.len(), size * size);
        assert!(modules.chars().all(|c| c == '0' || c == '1'));
    }
}
