//! Custodial wallets: one ed25519 key per user, generated here and encrypted at rest.
//!
//! The private key exists in clear only inside [`load_keypair`]'s return value, for the time
//! it takes to sign. It is never logged, never serialized, never returned by an endpoint.

use epicoin_core::{Address, Keypair};
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::error::{ApiError, ApiResult};
use crate::vault::{Sealed, Vault, VaultError};

/// Generates a key pair for `user_id` and stores it sealed. Returns the wallet address.
pub async fn create(conn: &mut PgConnection, vault: &Vault, user_id: Uuid) -> ApiResult<Address> {
    let keypair = Keypair::generate();
    let sealed = vault.seal(keypair.secret_bytes().as_ref(), user_id.as_bytes())?;
    sqlx::query(
        "INSERT INTO wallets (user_id, address, public_key, encrypted_secret, key_id) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(user_id)
    .bind(keypair.address().to_string())
    .bind(keypair.public_key().to_string())
    .bind(&sealed.bytes)
    .bind(&sealed.key_id)
    .execute(conn)
    .await?;
    Ok(keypair.address())
}

/// The address of a user's wallet.
pub async fn address_of(db: &PgPool, user_id: Uuid) -> ApiResult<Address> {
    let address: Option<String> =
        sqlx::query_scalar("SELECT address FROM wallets WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(db)
            .await?;
    address
        .and_then(|a| a.parse().ok())
        .ok_or_else(|| ApiError::internal(format!("user {user_id} has no usable wallet")))
}

/// Decrypts a user's signing key. Call only to sign on behalf of the authenticated user.
pub async fn load_keypair(db: &PgPool, vault: &Vault, user_id: Uuid) -> ApiResult<Keypair> {
    let row: Option<(Vec<u8>, String, String)> =
        sqlx::query_as("SELECT encrypted_secret, key_id, address FROM wallets WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(db)
            .await?;
    let (bytes, key_id, address) =
        row.ok_or_else(|| ApiError::internal(format!("user {user_id} has no wallet")))?;
    let keypair = open(vault, user_id, &Sealed { key_id, bytes })?;
    if keypair.address().to_string() != address {
        return Err(ApiError::internal(format!(
            "wallet of user {user_id} does not match its stored address"
        )));
    }
    Ok(keypair)
}

fn open(vault: &Vault, user_id: Uuid, sealed: &Sealed) -> Result<Keypair, VaultError> {
    let secret = vault.open(sealed, user_id.as_bytes())?;
    let bytes: &[u8; 32] = secret
        .as_slice()
        .try_into()
        .map_err(|_| VaultError::Decrypt)?;
    Ok(Keypair::from_secret_bytes(bytes))
}

/// Outcome of a master-key rotation.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct RotationReport {
    pub rotated: usize,
    pub already_current: usize,
}

/// Re-seals every wallet under the vault's current master key.
///
/// The vault must hold the old key as "previous". All-or-nothing: on any error (for
/// instance a wallet sealed by a key the vault does not have) nothing is changed.
pub async fn rotate_master_key(db: &PgPool, vault: &Vault) -> anyhow::Result<RotationReport> {
    let mut tx = db.begin().await?;
    let wallets: Vec<(Uuid, Vec<u8>, String)> =
        sqlx::query_as("SELECT user_id, encrypted_secret, key_id FROM wallets FOR UPDATE")
            .fetch_all(&mut *tx)
            .await?;

    let mut report = RotationReport::default();
    for (user_id, bytes, key_id) in wallets {
        if key_id == vault.current_key_id() {
            report.already_current += 1;
            continue;
        }
        let secret = vault
            .open(&Sealed { key_id, bytes }, user_id.as_bytes())
            .map_err(|e| anyhow::anyhow!("wallet of user {user_id}: {e}"))?;
        let sealed = vault.seal(&secret, user_id.as_bytes())?;
        sqlx::query("UPDATE wallets SET encrypted_secret = $2, key_id = $3 WHERE user_id = $1")
            .bind(user_id)
            .bind(&sealed.bytes)
            .bind(&sealed.key_id)
            .execute(&mut *tx)
            .await?;
        report.rotated += 1;
    }
    tx.commit().await?;
    Ok(report)
}
