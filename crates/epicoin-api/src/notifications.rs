//! The "!" next to a menu entry: something new since the user last opened that page, or
//! something waiting for them.
//!
//! - **News**, for students: EPC received (`balance`), an order ready or refunded
//!   (`orders`), a new challenge or a staff decision on a participation (`challenges`).
//!   "New" means after the student's last visit to that page, recorded by
//!   `POST /me/notifications/seen`. Their own actions (a purchase, a transfer sent, a
//!   participation sent) never count; EPC they earn, a challenge reward included, do.
//! - **To-dos**: orders to prepare (`shop_orders`, for shop managers and admins) and
//!   participations to review (`challenge_reviews`, for staff and admins). They stay until
//!   they are handled.
//!
//! `GET /me/notifications` returns a count per section of the user's menu, `0` included.

use std::collections::BTreeMap;

use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::auth::CurrentUser;
use crate::error::ApiResult;
use crate::models::Role;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/me/notifications", get(notifications))
        .route("/me/notifications/seen", post(seen))
}

/// A page whose news a student marks as seen by opening it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Section {
    Balance,
    Orders,
    Challenges,
}

impl Section {
    fn as_str(self) -> &'static str {
        match self {
            Section::Balance => "balance",
            Section::Orders => "orders",
            Section::Challenges => "challenges",
        }
    }
}

/// When the student `$1` last opened the page `$section` (an SQL expression); never: the
/// beginning of time.
macro_rules! last_visit {
    ($section:literal) => {
        concat!(
            "COALESCE((SELECT seen_at FROM section_visits WHERE user_id = $1 AND section = '",
            $section,
            "'), '-infinity')"
        )
    };
}

async fn notifications(
    State(state): State<AppState>,
    current: CurrentUser,
) -> ApiResult<Json<BTreeMap<&'static str, i64>>> {
    let me = current.id();
    let role = current.user.role;
    let mut counts = BTreeMap::new();

    if role == Role::Student {
        // EPC received: distributions, challenge rewards, transfers, refunds.
        let balance: i64 = sqlx::query_scalar(concat!(
            "SELECT count(*) FROM chain_txs \
             WHERE subject_user_id = $1 AND status = 'included' \
               AND kind IN ('mint', 'refund', 'transfer') AND created_at > ",
            last_visit!("balance")
        ))
        .bind(me)
        .fetch_one(&state.db)
        .await?;
        // Orders the shop made ready, or cancelled and refunded.
        let orders: i64 = sqlx::query_scalar(concat!(
            "SELECT count(*) FROM orders \
             WHERE user_id = $1 AND status IN ('ready', 'cancelled') AND updated_at > ",
            last_visit!("orders")
        ))
        .bind(me)
        .fetch_one(&state.db)
        .await?;
        // New open challenges for the student's cohort, places left, not taken part in;
        // and decisions of a reviewer.
        let challenges: i64 = sqlx::query_scalar(concat!(
            "SELECT \
               (SELECT count(*) FROM challenges ch \
                WHERE ch.closed_at IS NULL AND (ch.closes_at IS NULL OR ch.closes_at > now()) \
                  AND (cardinality(ch.cohorts) = 0 OR $2 = ANY(ch.cohorts)) \
                  AND ch.created_at > ",
            last_visit!("challenges"),
            "     AND NOT EXISTS (SELECT 1 FROM challenge_submissions s \
                                  WHERE s.challenge_id = ch.id AND s.student_user_id = $1) \
                  AND (ch.max_winners IS NULL OR ch.max_winners > ( \
                        SELECT count(*) FROM challenge_submissions w \
                        JOIN chain_txs wt ON wt.hash = w.tx_hash \
                        WHERE w.challenge_id = ch.id AND w.status = 'approved' \
                          AND wt.status <> 'failed'))) \
             + (SELECT count(*) FROM challenge_submissions s \
                WHERE s.student_user_id = $1 AND s.reviewer_user_id IS NOT NULL \
                  AND s.decided_at > ",
            last_visit!("challenges"),
            ")"
        ))
        .bind(me)
        .bind(&current.user.promo)
        .fetch_one(&state.db)
        .await?;
        counts.insert(Section::Balance.as_str(), balance);
        counts.insert(Section::Orders.as_str(), orders);
        counts.insert(Section::Challenges.as_str(), challenges);
    }

    if matches!(role, Role::ShopManager | Role::Admin) {
        let to_prepare: i64 =
            sqlx::query_scalar("SELECT count(*) FROM orders WHERE status = 'pending'")
                .fetch_one(&state.db)
                .await?;
        counts.insert("shop_orders", to_prepare);
    }

    if role.distributes() {
        // Admins review every challenge; staff, their own.
        let creator = (role != Role::Admin).then_some(me);
        let to_review: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM challenge_submissions s \
             JOIN challenges ch ON ch.id = s.challenge_id \
             WHERE s.status = 'pending' AND ($1::uuid IS NULL OR ch.creator_user_id = $1)",
        )
        .bind(creator)
        .fetch_one(&state.db)
        .await?;
        counts.insert("challenge_reviews", to_review);
    }

    Ok(Json(counts))
}

#[derive(Deserialize)]
struct SeenRequest {
    section: Section,
}

/// The student opened `section`: what it holds is no longer new.
async fn seen(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(body): Json<SeenRequest>,
) -> ApiResult<StatusCode> {
    current.require(&[Role::Student])?;
    sqlx::query(
        "INSERT INTO section_visits (user_id, section, seen_at) VALUES ($1, $2, now()) \
         ON CONFLICT (user_id, section) DO UPDATE SET seen_at = now()",
    )
    .bind(current.id())
    .bind(body.section.as_str())
    .execute(&state.db)
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
