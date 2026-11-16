//! Audit trail of sensitive actions.
//!
//! `details` must never contain personal data (names, emails): only ids, counts and codes.
//! That way erasing a user (GDPR) does not require rewriting the trail.

use serde::Serialize;
use serde_json::Value;
use sqlx::PgExecutor;
use time::OffsetDateTime;
use uuid::Uuid;

pub mod action {
    pub const LOGIN: &str = "auth.login";
    pub const ACCOUNT_ACTIVATED: &str = "auth.account_activated";
    pub const PASSWORD_RESET: &str = "auth.password_reset";
    pub const USERS_IMPORTED: &str = "users.imported";
    pub const USER_CREATED: &str = "users.created";
    pub const USER_UPDATED: &str = "users.updated";
    pub const USER_DELETED: &str = "users.deleted";
    pub const INVITATION_SENT: &str = "users.invitation_sent";
    pub const AUTHORITY_GRANTED: &str = "authority.granted";
    pub const AUTHORITY_REVOKED: &str = "authority.revoked";
    pub const MINT: &str = "mint.distributed";
}

/// Records an action. `actor` is `None` for actions of the system itself.
pub async fn record<'e>(
    db: impl PgExecutor<'e>,
    actor: Option<Uuid>,
    action: &str,
    target: Option<(&str, String)>,
    details: Value,
) -> Result<(), sqlx::Error> {
    let (target_type, target_id) = match target {
        Some((kind, id)) => (Some(kind), Some(id)),
        None => (None, None),
    };
    sqlx::query(
        "INSERT INTO audit_log (actor_user_id, action, target_type, target_id, details) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(actor)
    .bind(action)
    .bind(target_type)
    .bind(target_id)
    .bind(details)
    .execute(db)
    .await?;
    Ok(())
}

/// An audit entry as shown to administrators, with the actor's name when they still exist.
#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct AuditEntry {
    pub id: i64,
    #[serde(with = "time::serde::rfc3339")]
    pub at: OffsetDateTime,
    pub actor_user_id: Option<Uuid>,
    pub actor_name: Option<String>,
    pub action: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub details: Value,
}

pub async fn list<'e>(
    db: impl PgExecutor<'e>,
    limit: i64,
    offset: i64,
) -> Result<Vec<AuditEntry>, sqlx::Error> {
    sqlx::query_as::<_, AuditEntry>(
        "SELECT a.id, a.at, a.actor_user_id, \
                u.first_name || ' ' || u.last_name AS actor_name, \
                a.action, a.target_type, a.target_id, a.details \
         FROM audit_log a LEFT JOIN users u ON u.id = a.actor_user_id \
         ORDER BY a.id DESC LIMIT $1 OFFSET $2",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(db)
    .await
}
