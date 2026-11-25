//! EpiCoin application backend.
//!
//! - [`auth`]: sessions, activation, password reset, CSRF, the `CurrentUser` extractor.
//! - [`users`]: accounts, CSV import, administration of roles and quotas.
//! - [`wallets`] / [`vault`]: custodial keys, encrypted at rest.
//! - [`chain`]: reliable submission of signed transactions to the node.
//! - [`authority`]: keeps on-chain mint authorities in line with the distributing roles.
//! - [`mint`]: distributions by staff and admins.
//! - [`challenges`]: challenges whose winners are paid at once.
//! - [`transfers`]: students sending EPC to each other.
//! - [`shares`]: knowledge shares paid by QR code, streaks, badges and ranking.
//! - [`notifications`]: what shows a "!" in each user's menu.
//! - [`me`]: balance and history of the logged-in user.
//! - [`shop`]: products, orders paid on chain, refunds.
//! - [`admin`]: audit trail, statistics, blockchain explorer.

pub mod admin;
pub mod audit;
pub mod auth;
pub mod authority;
pub mod chain;
pub mod challenges;
pub mod config;
pub mod demo;
pub mod error;
pub mod mailer;
pub mod me;
pub mod mint;
pub mod models;
pub mod notifications;
pub mod ratelimit;
pub mod secrets;
pub mod shares;
pub mod shop;
pub mod state;
pub mod transfers;
pub mod users;
pub mod vault;
pub mod wallets;

use std::time::Duration;

use axum::extract::State;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{json, Value};
use tower_http::trace::TraceLayer;

use crate::error::ApiError;
use crate::state::AppState;

/// The database migrations, embedded in the binary.
pub static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// The whole HTTP API, under `/api`.
pub fn router(state: AppState) -> Router {
    let api = Router::new()
        .route("/health", get(health))
        .nest("/auth", auth::router())
        .nest("/me", me::router())
        .nest("/staff", mint::router())
        .nest("/admin", users::admin_router().merge(admin::admin_router()))
        .nest("/explorer", admin::explorer_router())
        .nest("/shop", shop::router())
        .merge(challenges::router())
        .merge(transfers::router())
        .merge(shares::router())
        .merge(notifications::router())
        .fallback(|| async { ApiError::not_found("Ressource") });
    Router::new()
        .nest("/api", api)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Liveness and dependencies: 200 when Postgres and the node both answer, 503 otherwise.
async fn health(State(state): State<AppState>) -> (axum::http::StatusCode, Json<Value>) {
    let database = sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&state.db)
        .await
        .is_ok();
    let node = state.node.health().await.is_ok();
    let status = if database && node {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(json!({
            "status": if database && node { "ok" } else { "degraded" },
            "database": database,
            "node": node,
        })),
    )
}

/// Background upkeep: settles pending chain transactions and keeps mint authorities in
/// sync. Runs until the task is aborted.
pub async fn run_background(state: AppState) {
    const RECONCILE_EVERY: Duration = Duration::from_secs(5);
    /// Leave in-flight requests the time to settle their own transactions.
    const RECONCILE_MIN_AGE: Duration = Duration::from_secs(30);
    const AUTHORITY_SYNC_EVERY: u32 = 60; // × RECONCILE_EVERY = 5 minutes

    let mut tick: u32 = 0;
    loop {
        if let Err(error) = state.chain.reconcile(RECONCILE_MIN_AGE).await {
            tracing::warn!(%error, "reconciliation failed; will retry");
        }
        if tick.is_multiple_of(AUTHORITY_SYNC_EVERY) {
            match authority::sync_all(&state).await {
                Ok(0) => {}
                Ok(changes) => tracing::info!(changes, "mint authorities synchronized"),
                Err(error) => tracing::warn!(%error, "authority sync failed; will retry"),
            }
        }
        tick = tick.wrapping_add(1);
        tokio::time::sleep(RECONCILE_EVERY).await;
    }
}
