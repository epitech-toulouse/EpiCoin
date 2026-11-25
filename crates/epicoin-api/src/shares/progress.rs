//! A student's progress (streak, badges, shares) and the ranking.

use std::collections::BTreeSet;

use axum::extract::State;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use time::{Date, OffsetDateTime};
use uuid::Uuid;

use super::badges::{self, Fact};
use super::calendar;
use super::{iso_date, short_name, today, valid_share, Settings};
use crate::auth::CurrentUser;
use crate::error::ApiResult;
use crate::models::Role;
use crate::state::AppState;

/// How many students the ranking shows.
const RANKING_SIZE: i64 = 100;
/// How many shares a student's page lists.
const RECENT_SHARES: i64 = 50;

/// The ranking: students by EPC earned through valid shares (bonuses included), ties
/// sharing a rank. Active students who did not opt out, with at least one valid share.
macro_rules! ranking {
    () => {
        concat!(
            "WITH scores AS ( \
               SELECT s.student_user_id AS user_id, sum(s.amount + s.bonus)::bigint AS total, \
                      count(*) AS shares \
               FROM shares s JOIN chain_txs t ON t.hash = s.tx_hash \
               WHERE s.student_user_id IS NOT NULL AND ",
            valid_share!(),
            " GROUP BY s.student_user_id), \
             ranked AS ( \
               SELECT u.id AS user_id, u.first_name, u.last_name, u.promo, sc.total, sc.shares, \
                      (l.user_id IS NOT NULL) AS legend, \
                      RANK() OVER (ORDER BY sc.total DESC) AS rank \
               FROM scores sc JOIN users u ON u.id = sc.user_id \
               LEFT JOIN share_legends l ON l.user_id = u.id \
               WHERE u.role = 'student' AND u.disabled_at IS NULL \
                 AND NOT u.hidden_from_ranking) "
        )
    };
}

#[derive(sqlx::FromRow)]
struct RankedRow {
    user_id: Uuid,
    first_name: String,
    last_name: String,
    promo: String,
    total: i64,
    shares: i64,
    legend: bool,
    rank: i64,
}

#[derive(sqlx::FromRow)]
struct FactRow {
    claimed_at: OffsetDateTime,
    claim_day: Date,
    staff_user_id: Option<Uuid>,
    amount: i64,
    standard: i64,
    mention: Option<String>,
}

#[derive(Serialize, sqlx::FromRow)]
struct MyShare {
    id: Uuid,
    topic: String,
    amount: i64,
    bonus: i64,
    streak_days: Option<i32>,
    mention: Option<String>,
    staff_name: Option<String>,
    #[serde(with = "time::serde::rfc3339")]
    claimed_at: OffsetDateTime,
    /// `pending` or `included`.
    reward_status: String,
    #[serde(with = "time::serde::rfc3339::option")]
    cancelled_at: Option<OffsetDateTime>,
    cancel_reason: String,
}

/// The student's page: streak, badges, totals, rank and recent shares.
pub(super) async fn mine(
    State(state): State<AppState>,
    current: CurrentUser,
) -> ApiResult<Json<Value>> {
    current.require(&[Role::Student])?;
    let student = current.id();
    let settings = Settings::load(&state.db).await?;
    let today = today(&state.db, &settings.time_zone).await?;
    let calendar = settings.calendar();

    let facts: Vec<Fact> = sqlx::query_as::<_, FactRow>(concat!(
        "SELECT s.claimed_at, s.claim_day, s.staff_user_id, s.amount, s.standard, s.mention \
         FROM shares s JOIN chain_txs t ON t.hash = s.tx_hash \
         WHERE s.student_user_id = $1 AND ",
        valid_share!(),
        " ORDER BY s.claimed_at, s.id"
    ))
    .bind(student)
    .fetch_all(&state.db)
    .await?
    .into_iter()
    .map(|row| Fact {
        claimed_at: row.claimed_at,
        claim_day: row.claim_day,
        staff: row.staff_user_id,
        amount: row.amount,
        standard: row.standard,
        mention: row.mention,
    })
    .collect();
    let days: BTreeSet<Date> = facts.iter().map(|fact| fact.claim_day).collect();
    let progress = calendar::progress(&days, today, &calendar, &settings.tiers);
    let legend: Option<OffsetDateTime> =
        sqlx::query_scalar("SELECT granted_at FROM share_legends WHERE user_id = $1")
            .bind(student)
            .fetch_optional(&state.db)
            .await?;
    let badges = badges::compute(&facts, &calendar, legend);

    let total: i64 = sqlx::query_scalar(concat!(
        "SELECT COALESCE(sum(s.amount + s.bonus), 0)::bigint \
         FROM shares s JOIN chain_txs t ON t.hash = s.tx_hash \
         WHERE s.student_user_id = $1 AND ",
        valid_share!()
    ))
    .bind(student)
    .fetch_one(&state.db)
    .await?;
    let rank: Option<i64> = sqlx::query_scalar(concat!(
        ranking!(),
        "SELECT rank FROM ranked WHERE user_id = $1"
    ))
    .bind(student)
    .fetch_optional(&state.db)
    .await?;
    let hidden: bool = sqlx::query_scalar("SELECT hidden_from_ranking FROM users WHERE id = $1")
        .bind(student)
        .fetch_one(&state.db)
        .await?;
    let recent = sqlx::query_as::<_, MyShare>(
        "SELECT s.id, s.topic, s.amount, s.bonus, s.streak_days, s.mention, \
                u.first_name || ' ' || u.last_name AS staff_name, s.claimed_at, \
                t.status AS reward_status, s.cancelled_at, s.cancel_reason \
         FROM shares s JOIN chain_txs t ON t.hash = s.tx_hash \
         LEFT JOIN users u ON u.id = s.staff_user_id \
         WHERE s.student_user_id = $1 AND t.status <> 'failed' \
         ORDER BY s.claimed_at DESC, s.id LIMIT $2",
    )
    .bind(student)
    .bind(RECENT_SHARES)
    .fetch_all(&state.db)
    .await?;

    Ok(Json(json!({
        "today": iso_date(today),
        "progress": progress,
        "total": total,
        "shares": facts.len(),
        "rank": rank,
        "hidden_from_ranking": hidden,
        "badges": badges,
        "recent": recent,
    })))
}

#[derive(Deserialize)]
pub(super) struct UpdateMine {
    hidden_from_ranking: bool,
}

/// A student leaves the ranking, or comes back.
pub(super) async fn update_mine(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(body): Json<UpdateMine>,
) -> ApiResult<Json<Value>> {
    current.require(&[Role::Student])?;
    sqlx::query("UPDATE users SET hidden_from_ranking = $2 WHERE id = $1")
        .bind(current.id())
        .bind(body.hidden_from_ranking)
        .execute(&state.db)
        .await?;
    Ok(Json(
        json!({ "hidden_from_ranking": body.hidden_from_ranking }),
    ))
}

/// The ranking, for everyone logged in. Students appear as "First name L.", with their
/// cohort; admins also get their ids (to give the legend badge).
pub(super) async fn ranking(
    State(state): State<AppState>,
    current: CurrentUser,
) -> ApiResult<Json<Value>> {
    let rows = sqlx::query_as::<_, RankedRow>(concat!(
        ranking!(),
        "SELECT * FROM ranked ORDER BY rank, first_name, last_name, user_id LIMIT $1"
    ))
    .bind(RANKING_SIZE)
    .fetch_all(&state.db)
    .await?;
    let admin = current.user.role == Role::Admin;
    let items: Vec<Value> = rows
        .iter()
        .map(|row| {
            json!({
                "rank": row.rank,
                "user_id": admin.then_some(row.user_id),
                "name": short_name(&row.first_name, &row.last_name),
                "promo": row.promo,
                "total": row.total,
                "shares": row.shares,
                "legend": row.legend,
                "me": row.user_id == current.id(),
            })
        })
        .collect();
    let me = if current.user.role == Role::Student {
        sqlx::query_as::<_, RankedRow>(concat!(
            ranking!(),
            "SELECT * FROM ranked WHERE user_id = $1"
        ))
        .bind(current.id())
        .fetch_optional(&state.db)
        .await?
        .map(|row| json!({ "rank": row.rank, "total": row.total, "shares": row.shares }))
    } else {
        None
    };
    Ok(Json(json!({ "items": items, "me": me })))
}
