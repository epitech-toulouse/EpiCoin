//! Keeps on-chain mint authorities in line with the `staff` role.
//!
//! Postgres is the source of intent: a user who is `staff` and not disabled must be an
//! active authority with their quota; everyone else must not be. [`sync_user`] applies that
//! for one user right after a change, and [`sync_all`] runs periodically so that a change
//! made while the node was unreachable is caught up — the system converges by itself.

use std::collections::HashMap;
use std::time::Duration;

use epicoin_core::{Address, Amount, AuthorityAction, TxPayload};
use serde_json::json;
use uuid::Uuid;

use crate::audit;
use crate::chain::{ChainError, TxMeta};
use crate::config::Settings;
use crate::models::{Role, User};
use crate::state::AppState;

/// How long to wait for an authority update to be included.
const WAIT: Duration = Duration::from_secs(20);

/// The quota `user` should have on chain, or `None` if they must not be an authority.
pub fn desired_quota(user: &User, settings: &Settings) -> Option<Amount> {
    (user.role == Role::Staff && user.disabled_at.is_none()).then(|| {
        user.mint_quota
            .map_or(settings.default_mint_quota, |q| q.max(0) as Amount)
    })
}

/// Brings one address to the desired state. Returns whether a transaction was sent.
async fn apply(
    state: &AppState,
    address: Address,
    desired: Option<Amount>,
    current: Option<(bool, Amount)>,
    user_id: Option<Uuid>,
    actor: Option<Uuid>,
) -> Result<bool, ChainError> {
    let (action, audit_action) = match (desired, current) {
        (Some(quota), Some((true, current_quota))) if quota == current_quota => return Ok(false),
        (Some(quota), _) => (
            AuthorityAction::Grant { quota },
            audit::action::AUTHORITY_GRANTED,
        ),
        (None, Some((true, _))) => (AuthorityAction::Revoke, audit::action::AUTHORITY_REVOKED),
        (None, _) => return Ok(false),
    };
    let tx = state
        .chain
        .send(
            &state.admin_key,
            TxPayload::AuthorityUpdate {
                authority: address,
                action,
            },
            TxMeta {
                actor_user_id: actor,
                subject_user_id: user_id,
                amount: desired,
                ..TxMeta::default()
            },
            WAIT,
        )
        .await?;
    audit::record(
        &state.db,
        actor,
        audit_action,
        user_id.map(|id| ("user", id.to_string())),
        json!({ "address": address, "quota": desired, "tx": tx.hash }),
    )
    .await?;
    Ok(true)
}

async fn on_chain(
    state: &AppState,
    address: &Address,
) -> Result<Option<(bool, Amount)>, ChainError> {
    Ok(state
        .node
        .account(address)
        .await?
        .authority
        .map(|a| (a.active, a.quota)))
}

/// Grants, updates or revokes the on-chain authority of `user` so that it matches their
/// role, quota and status. `actor` is the admin who caused the change, if any.
pub async fn sync_user(
    state: &AppState,
    user: &User,
    address: Address,
    actor: Option<Uuid>,
) -> Result<bool, ChainError> {
    let desired = desired_quota(user, &state.settings);
    let current = on_chain(state, &address).await?;
    apply(state, address, desired, current, Some(user.id), actor).await
}

/// Revokes whatever authority `address` has (used when its owner is deleted).
pub async fn revoke(
    state: &AppState,
    address: Address,
    actor: Option<Uuid>,
) -> Result<bool, ChainError> {
    let current = on_chain(state, &address).await?;
    apply(state, address, None, current, None, actor).await
}

#[derive(sqlx::FromRow)]
struct StaffRow {
    #[sqlx(flatten)]
    user: User,
    address: String,
}

/// Reconciles every authority: grants what is missing, fixes quotas, revokes authorities
/// that no longer correspond to an active staff member (including deleted users). Returns
/// the number of transactions sent.
pub async fn sync_all(state: &AppState) -> Result<usize, ChainError> {
    let staff = sqlx::query_as::<_, StaffRow>(
        "SELECT u.*, w.address FROM users u JOIN wallets w ON w.user_id = u.id \
         WHERE u.role = 'staff'",
    )
    .fetch_all(&state.db)
    .await?;
    let mut current: HashMap<Address, (bool, Amount)> = state
        .node
        .authorities()
        .await?
        .into_iter()
        .map(|a| (a.address, (a.active, a.quota)))
        .collect();

    let mut changes = 0;
    for row in staff {
        let Ok(address) = row.address.parse::<Address>() else {
            continue;
        };
        let desired = desired_quota(&row.user, &state.settings);
        let existing = current.remove(&address);
        if apply(state, address, desired, existing, Some(row.user.id), None).await? {
            changes += 1;
        }
    }
    // What is left on chain belongs to nobody who should mint.
    for (address, existing) in current {
        if apply(state, address, None, Some(existing), None, None).await? {
            changes += 1;
        }
    }
    Ok(changes)
}
