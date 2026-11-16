//! User accounts: creation, CSV import, administration.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use epicoin_core::Address;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sqlx::PgConnection;
use uuid::Uuid;

use crate::auth::{send_invitation, CurrentUser};
use crate::config::Settings;
use crate::error::{ApiError, ApiResult};
use crate::models::{Page, Role, User, UserView};
use crate::state::AppState;
use crate::vault::Vault;
use crate::{audit, authority, wallets};

const MAX_IMPORT_ROWS: usize = 2_000;
const MAX_FIELD_LEN: usize = 100;

pub fn admin_router() -> Router<AppState> {
    Router::new()
        .route("/users", get(list_users).post(create_one))
        .route("/users/import", post(import))
        .route("/users/{id}", patch(update_user).delete(delete_user))
        .route("/users/{id}/invitation", post(resend_invitation))
}

// --- Creation --------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct NewUser {
    pub email: String,
    pub first_name: String,
    pub last_name: String,
    #[serde(default)]
    pub promo: String,
    pub role: Role,
}

/// Normalizes and validates a user to create. The error is a message for the admin.
pub fn validate(new: &NewUser, settings: &Settings) -> Result<NewUser, String> {
    let email = new.email.trim().to_lowercase();
    let first_name = new.first_name.trim().to_owned();
    let last_name = new.last_name.trim().to_owned();
    let promo = new.promo.trim().to_owned();

    let (local, domain) = email
        .split_once('@')
        .ok_or_else(|| "adresse email invalide".to_owned())?;
    let local_ok = !local.is_empty()
        && local
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._%+-".contains(c));
    if !local_ok || email.len() > 254 {
        return Err("adresse email invalide".into());
    }
    if domain != settings.allowed_email_domain {
        return Err(format!(
            "seules les adresses @{} sont acceptées",
            settings.allowed_email_domain
        ));
    }
    if first_name.is_empty() || last_name.is_empty() {
        return Err("le prénom et le nom sont obligatoires".into());
    }
    if [&first_name, &last_name, &promo]
        .iter()
        .any(|field| field.chars().count() > MAX_FIELD_LEN)
    {
        return Err(format!("un champ dépasse {MAX_FIELD_LEN} caractères"));
    }
    Ok(NewUser {
        email,
        first_name,
        last_name,
        promo,
        role: new.role,
    })
}

/// Inserts a (validated) user and their custodial wallet.
pub async fn create(conn: &mut PgConnection, vault: &Vault, new: &NewUser) -> ApiResult<User> {
    let user = sqlx::query_as::<_, User>(
        "INSERT INTO users (id, email, first_name, last_name, promo, role) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING *",
    )
    .bind(Uuid::new_v4())
    .bind(&new.email)
    .bind(&new.first_name)
    .bind(&new.last_name)
    .bind(&new.promo)
    .bind(new.role.as_str())
    .fetch_one(&mut *conn)
    .await?;
    wallets::create(conn, vault, user.id).await?;
    Ok(user)
}

// --- CSV import ------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CsvError {
    /// 1-based line in the file (the header is line 1); 0 for a problem with the file itself.
    pub line: u64,
    pub message: String,
}

fn header_field(name: &str) -> Option<&'static str> {
    match name
        .trim()
        .trim_start_matches('\u{feff}')
        .to_lowercase()
        .as_str()
    {
        "email" | "e-mail" | "mail" | "adresse email" => Some("email"),
        "first_name" | "firstname" | "prenom" | "prénom" => Some("first_name"),
        "last_name" | "lastname" | "nom" => Some("last_name"),
        "promo" | "promotion" => Some("promo"),
        "role" | "rôle" => Some("role"),
        _ => None,
    }
}

/// Parses a users CSV. Columns: `email`, `first_name`/`prenom`, `last_name`/`nom`, and
/// optionally `promo` and `role` (student by default). Separator `,` or `;`.
///
/// Returns either every user, validated, or every error — never a partial result.
pub fn parse_csv(text: &str, settings: &Settings) -> Result<Vec<NewUser>, Vec<CsvError>> {
    let file_error = |message: &str| {
        vec![CsvError {
            line: 0,
            message: message.to_owned(),
        }]
    };
    let text = text.trim_start_matches('\u{feff}');
    let first_line = text.lines().next().unwrap_or("");
    let delimiter = if first_line.matches(';').count() > first_line.matches(',').count() {
        b';'
    } else {
        b','
    };
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delimiter)
        .trim(csv::Trim::All)
        .flexible(true)
        .from_reader(text.as_bytes());

    let headers = reader
        .headers()
        .map_err(|_| file_error("fichier illisible"))?
        .clone();
    let mut columns: HashMap<&'static str, usize> = HashMap::new();
    for (index, name) in headers.iter().enumerate() {
        if let Some(field) = header_field(name) {
            columns.entry(field).or_insert(index);
        }
    }
    for required in ["email", "first_name", "last_name"] {
        if !columns.contains_key(required) {
            return Err(file_error(
                "colonnes attendues : email, prenom, nom (et en option promo, role)",
            ));
        }
    }

    let mut users = Vec::new();
    let mut errors = Vec::new();
    let mut seen = HashSet::new();
    for (index, record) in reader.records().enumerate() {
        // Line 1 is the header; fall back on counting when the parser has no position.
        let fallback_line = index as u64 + 2;
        let record = match record {
            Ok(record) => record,
            Err(error) => {
                errors.push(CsvError {
                    line: error
                        .position()
                        .map_or(fallback_line, |position| position.line()),
                    message: "ligne illisible".into(),
                });
                continue;
            }
        };
        let line = record
            .position()
            .map_or(fallback_line, |position| position.line());
        if record.iter().all(str::is_empty) {
            continue;
        }
        if users.len() + errors.len() >= MAX_IMPORT_ROWS {
            return Err(file_error("fichier trop volumineux (2000 lignes maximum)"));
        }
        let field = |name: &str| {
            columns
                .get(name)
                .and_then(|index| record.get(*index))
                .unwrap_or("")
        };
        let role_text = field("role");
        let role = if role_text.is_empty() {
            Some(Role::Student)
        } else {
            Role::parse(role_text)
        };
        let Some(role) = role else {
            errors.push(CsvError {
                line,
                message: format!(
                    "rôle inconnu « {role_text} » (student, staff, shop_manager ou admin)"
                ),
            });
            continue;
        };
        let candidate = NewUser {
            email: field("email").to_owned(),
            first_name: field("first_name").to_owned(),
            last_name: field("last_name").to_owned(),
            promo: field("promo").to_owned(),
            role,
        };
        match validate(&candidate, settings) {
            Ok(user) if !seen.insert(user.email.clone()) => errors.push(CsvError {
                line,
                message: format!("email en double dans le fichier : {}", user.email),
            }),
            Ok(user) => users.push(user),
            Err(message) => errors.push(CsvError { line, message }),
        }
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    if users.is_empty() {
        return Err(file_error("le fichier ne contient aucun utilisateur"));
    }
    Ok(users)
}

#[derive(Deserialize)]
struct ImportRequest {
    csv: String,
}

/// Imports users from a CSV. Existing emails are skipped; any invalid line cancels the
/// whole import. Invitations and on-chain authorities follow in the background.
async fn import(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(body): Json<ImportRequest>,
) -> ApiResult<Json<Value>> {
    current.require(&[Role::Admin])?;
    let candidates = parse_csv(&body.csv, &state.settings).map_err(|errors| {
        ApiError::unprocessable(
            "invalid_csv",
            "Le fichier contient des erreurs. Rien n'a été importé.",
        )
        .with_details(json!({ "errors": errors }))
    })?;

    let mut tx = state.db.begin().await?;
    let emails: Vec<String> = candidates.iter().map(|u| u.email.clone()).collect();
    let existing: HashSet<String> =
        sqlx::query_scalar::<_, String>("SELECT email FROM users WHERE email = ANY($1)")
            .bind(&emails)
            .fetch_all(&mut *tx)
            .await?
            .into_iter()
            .collect();

    let mut created = Vec::new();
    for candidate in candidates.iter().filter(|u| !existing.contains(&u.email)) {
        created.push(create(&mut tx, &state.vault, candidate).await?);
    }
    let mut by_role: HashMap<&str, usize> = HashMap::new();
    for user in &created {
        *by_role.entry(user.role.as_str()).or_default() += 1;
    }
    audit::record(
        &mut *tx,
        Some(current.id()),
        audit::action::USERS_IMPORTED,
        None,
        json!({ "created": created.len(), "skipped_existing": existing.len(), "by_role": by_role }),
    )
    .await?;
    tx.commit().await?;

    let summary = json!({
        "created": created.len(),
        "skipped_existing": existing.len(),
        "by_role": by_role,
    });
    // Emails and chain transactions can take a while for a whole promotion: do not make
    // the admin's request wait for them.
    tokio::spawn(async move {
        let has_staff = created.iter().any(|u| u.role == Role::Staff);
        for user in &created {
            if let Err(error) = send_invitation(&state, user).await {
                tracing::warn!(user_id = %user.id, error = %error.message, "invitation not sent");
            }
        }
        if has_staff {
            if let Err(error) = authority::sync_all(&state).await {
                tracing::warn!(%error, "authority sync after import failed; will be retried");
            }
        }
    });
    Ok(Json(summary))
}

// --- Administration --------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize)]
pub struct AdminUserView {
    #[serde(flatten)]
    pub user: UserView,
    pub wallet_address: String,
}

#[derive(sqlx::FromRow)]
struct UserWithWallet {
    #[sqlx(flatten)]
    user: User,
    address: String,
}

impl From<UserWithWallet> for AdminUserView {
    fn from(row: UserWithWallet) -> Self {
        Self {
            user: UserView::from(&row.user),
            wallet_address: row.address,
        }
    }
}

/// Turns a search term into an `ILIKE` pattern matching it literally anywhere.
pub fn like_pattern(term: &str) -> Option<String> {
    let term = term.trim();
    if term.is_empty() {
        return None;
    }
    let escaped = term
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_");
    Some(format!("%{escaped}%"))
}

/// The `WHERE` clause shared by the count and the page queries: `$1` role, `$2` pattern.
macro_rules! user_filter {
    () => {
        "($1::text IS NULL OR u.role = $1) AND ($2::text IS NULL \
         OR u.email ILIKE $2 OR u.first_name ILIKE $2 OR u.last_name ILIKE $2 \
         OR u.promo ILIKE $2 OR (u.first_name || ' ' || u.last_name) ILIKE $2)"
    };
}

#[derive(Deserialize)]
struct ListParams {
    q: Option<String>,
    role: Option<Role>,
    // Not `#[serde(flatten)]`: flattened query strings lose their numeric types.
    limit: Option<i64>,
    offset: Option<i64>,
}

async fn list_users(
    State(state): State<AppState>,
    current: CurrentUser,
    Query(params): Query<ListParams>,
) -> ApiResult<Json<Value>> {
    current.require(&[Role::Admin])?;
    let page = Page {
        limit: params.limit,
        offset: params.offset,
    };
    let pattern = params.q.as_deref().and_then(like_pattern);
    let role = params.role.map(Role::as_str);

    let total: i64 = sqlx::query_scalar(concat!(
        "SELECT count(*) FROM users u WHERE ",
        user_filter!()
    ))
    .bind(role)
    .bind(&pattern)
    .fetch_one(&state.db)
    .await?;
    let rows = sqlx::query_as::<_, UserWithWallet>(concat!(
        "SELECT u.*, w.address FROM users u JOIN wallets w ON w.user_id = u.id WHERE ",
        user_filter!(),
        " ORDER BY u.last_name, u.first_name, u.email LIMIT $3 OFFSET $4"
    ))
    .bind(role)
    .bind(&pattern)
    .bind(page.limit())
    .bind(page.offset())
    .fetch_all(&state.db)
    .await?;

    let items: Vec<AdminUserView> = rows.into_iter().map(Into::into).collect();
    Ok(Json(json!({ "items": items, "total": total })))
}

async fn load_with_wallet(state: &AppState, id: Uuid) -> ApiResult<(User, Address)> {
    let row = sqlx::query_as::<_, UserWithWallet>(
        "SELECT u.*, w.address FROM users u JOIN wallets w ON w.user_id = u.id WHERE u.id = $1",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::not_found("Utilisateur"))?;
    let address = row
        .address
        .parse()
        .map_err(|_| ApiError::internal(format!("user {id} has a malformed wallet address")))?;
    Ok((row.user, address))
}

/// Applies the on-chain side of a user change. A failure is logged, not returned: the
/// change is saved, and the periodic authority sync will catch up.
async fn sync_authority(state: &AppState, user: &User, address: Address, actor: Uuid) {
    if let Err(error) = authority::sync_user(state, user, address, Some(actor)).await {
        tracing::warn!(user_id = %user.id, %error, "authority not synced yet; will be retried");
    }
}

async fn create_one(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(body): Json<NewUser>,
) -> ApiResult<(StatusCode, Json<AdminUserView>)> {
    current.require(&[Role::Admin])?;
    let new = validate(&body, &state.settings)
        .map_err(|message| ApiError::unprocessable("invalid_user", capitalize(&message)))?;

    let mut tx = state.db.begin().await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE email = $1)")
        .bind(&new.email)
        .fetch_one(&mut *tx)
        .await?;
    if exists {
        return Err(ApiError::conflict(
            "email_taken",
            "Un compte existe déjà avec cette adresse email.",
        ));
    }
    let user = create(&mut tx, &state.vault, &new).await?;
    audit::record(
        &mut *tx,
        Some(current.id()),
        audit::action::USER_CREATED,
        Some(("user", user.id.to_string())),
        json!({ "role": user.role }),
    )
    .await?;
    tx.commit().await?;

    let (user, address) = load_with_wallet(&state, user.id).await?;
    sync_authority(&state, &user, address, current.id()).await;
    if let Err(error) = send_invitation(&state, &user).await {
        tracing::warn!(user_id = %user.id, error = %error.message, "invitation not sent");
    }
    Ok((
        StatusCode::CREATED,
        Json(AdminUserView {
            user: UserView::from(&user),
            wallet_address: address.to_string(),
        }),
    ))
}

fn capitalize(message: &str) -> String {
    let mut chars = message.chars();
    match chars.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

/// Fields an admin can change. An absent field is left untouched; `mint_quota: null`
/// resets the personal quota to the default.
#[derive(Deserialize)]
struct UpdateRequest {
    role: Option<Role>,
    #[serde(default, deserialize_with = "double_option")]
    mint_quota: Option<Option<i64>>,
    disabled: Option<bool>,
}

/// Distinguishes "field absent" (`None`) from "field set to null" (`Some(None)`).
fn double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

async fn update_user(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateRequest>,
) -> ApiResult<Json<AdminUserView>> {
    current.require(&[Role::Admin])?;
    let (before, address) = load_with_wallet(&state, id).await?;

    let role = body.role.unwrap_or(before.role);
    let disabled = body.disabled.unwrap_or(before.disabled_at.is_some());
    let mint_quota = body.mint_quota.unwrap_or(before.mint_quota);
    if id == current.id() && (role != Role::Admin || disabled) {
        return Err(ApiError::unprocessable(
            "self_lockout",
            "Vous ne pouvez pas retirer vos propres droits d'administration.",
        ));
    }
    if mint_quota.is_some_and(|quota| quota < 0) {
        return Err(ApiError::unprocessable(
            "invalid_quota",
            "Le quota ne peut pas être négatif.",
        ));
    }

    let mut tx = state.db.begin().await?;
    let after = sqlx::query_as::<_, User>(
        "UPDATE users SET role = $2, mint_quota = $3, \
         disabled_at = CASE WHEN $4 THEN COALESCE(disabled_at, now()) ELSE NULL END \
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(role.as_str())
    .bind(mint_quota)
    .bind(disabled)
    .fetch_one(&mut *tx)
    .await?;
    if disabled {
        sqlx::query("DELETE FROM sessions WHERE user_id = $1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    audit::record(
        &mut *tx,
        Some(current.id()),
        audit::action::USER_UPDATED,
        Some(("user", id.to_string())),
        json!({
            "role": { "from": before.role, "to": after.role },
            "disabled": { "from": before.disabled_at.is_some(), "to": after.disabled_at.is_some() },
            "mint_quota": { "from": before.mint_quota, "to": after.mint_quota },
        }),
    )
    .await?;
    tx.commit().await?;

    sync_authority(&state, &after, address, current.id()).await;
    Ok(Json(AdminUserView {
        user: UserView::from(&after),
        wallet_address: address.to_string(),
    }))
}

async fn resend_invitation(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    current.require(&[Role::Admin])?;
    let (user, _) = load_with_wallet(&state, id).await?;
    if user.activated_at.is_some() {
        return Err(ApiError::conflict(
            "already_activated",
            "Ce compte est déjà activé.",
        ));
    }
    send_invitation(&state, &user).await?;
    audit::record(
        &state.db,
        Some(current.id()),
        audit::action::INVITATION_SENT,
        Some(("user", id.to_string())),
        json!({}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Erases a user (GDPR): their identity, sessions and wallet key are deleted from
/// Postgres. The chain keeps an address that nothing links to a person any more; the EPC
/// left on it become unspendable.
async fn delete_user(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    current.require(&[Role::Admin])?;
    if id == current.id() {
        return Err(ApiError::unprocessable(
            "self_lockout",
            "Vous ne pouvez pas supprimer votre propre compte.",
        ));
    }
    let (user, address) = load_with_wallet(&state, id).await?;

    let mut tx = state.db.begin().await?;
    sqlx::query("DELETE FROM users WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    audit::record(
        &mut *tx,
        Some(current.id()),
        audit::action::USER_DELETED,
        Some(("user", id.to_string())),
        json!({ "role": user.role }),
    )
    .await?;
    tx.commit().await?;

    if user.role == Role::Staff {
        if let Err(error) = authority::revoke(&state, address, Some(current.id())).await {
            tracing::warn!(%error, "authority of a deleted user not revoked yet; will be retried");
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        Settings::development()
    }

    fn new_user(email: &str) -> NewUser {
        NewUser {
            email: email.into(),
            first_name: "Ada".into(),
            last_name: "Lovelace".into(),
            promo: "2027".into(),
            role: Role::Student,
        }
    }

    #[test]
    fn validation_normalizes_and_checks_the_domain() {
        let user = validate(&new_user("  Ada.Lovelace@Epitech.EU "), &settings()).unwrap();
        assert_eq!(user.email, "ada.lovelace@epitech.eu");

        for bad in [
            "ada@gmail.com",
            "ada@epitech.eu.evil.com",
            "ada@sub.epitech.eu",
            "@epitech.eu",
            "ada",
            "ada lovelace@epitech.eu",
            "ada@@epitech.eu",
        ] {
            assert!(validate(&new_user(bad), &settings()).is_err(), "{bad}");
        }

        let mut nameless = new_user("ada@epitech.eu");
        nameless.first_name = "  ".into();
        assert!(validate(&nameless, &settings()).is_err());

        let mut long = new_user("ada@epitech.eu");
        long.last_name = "x".repeat(101);
        assert!(validate(&long, &settings()).is_err());
    }

    #[test]
    fn csv_with_english_headers_and_commas() {
        let csv = "email,first_name,last_name,promo,role\n\
                   ada@epitech.eu,Ada,Lovelace,2027,student\n\
                   alan@epitech.eu,Alan,Turing,,staff\n";
        let users = parse_csv(csv, &settings()).unwrap();
        assert_eq!(users.len(), 2);
        assert_eq!(users[0], new_user("ada@epitech.eu"));
        assert_eq!(users[1].role, Role::Staff);
        assert_eq!(users[1].promo, "");
    }

    #[test]
    fn csv_with_french_headers_semicolons_bom_and_default_role() {
        let csv = "\u{feff}Email;Prénom;Nom;Promo\r\n\
                   Ada@Epitech.eu ; Ada ; Lovelace ; 2027\r\n\
                   \r\n\
                   grace@epitech.eu;Grace;Hopper;2026\r\n";
        let users = parse_csv(csv, &settings()).unwrap();
        assert_eq!(users.len(), 2);
        assert_eq!(users[0], new_user("ada@epitech.eu"));
        assert_eq!(users[1].role, Role::Student);
    }

    #[test]
    fn csv_roles_accept_french_names() {
        let csv = "email,prenom,nom,role\n\
                   a@epitech.eu,A,A,encadrant\n\
                   b@epitech.eu,B,B,boutique\n\
                   c@epitech.eu,C,C,admin\n\
                   d@epitech.eu,D,D,étudiant\n";
        let roles: Vec<Role> = parse_csv(csv, &settings())
            .unwrap()
            .into_iter()
            .map(|u| u.role)
            .collect();
        assert_eq!(
            roles,
            vec![Role::Staff, Role::ShopManager, Role::Admin, Role::Student]
        );
    }

    #[test]
    fn csv_errors_are_all_reported_with_their_line() {
        let csv = "email,prenom,nom,role\n\
                   ada@epitech.eu,Ada,Lovelace,student\n\
                   bob@gmail.com,Bob,Martin,student\n\
                   eve@epitech.eu,Eve,,student\n\
                   ada@epitech.eu,Ada,Bis,student\n\
                   max@epitech.eu,Max,Payne,wizard\n";
        let errors = parse_csv(csv, &settings()).unwrap_err();
        assert_eq!(
            errors.iter().map(|e| e.line).collect::<Vec<_>>(),
            vec![3, 4, 5, 6]
        );
        assert!(errors[0].message.contains("@epitech.eu"));
        assert!(errors[2].message.contains("double"));
        assert!(errors[3].message.contains("wizard"));
    }

    #[test]
    fn csv_without_required_columns_or_rows_is_refused() {
        assert!(parse_csv("email,nom\nada@epitech.eu,Lovelace\n", &settings()).is_err());
        assert!(parse_csv("email,prenom,nom\n", &settings()).is_err());
        assert!(parse_csv("", &settings()).is_err());
    }

    #[test]
    fn like_patterns_escape_wildcards() {
        assert_eq!(like_pattern("  "), None);
        assert_eq!(like_pattern("ada").as_deref(), Some("%ada%"));
        assert_eq!(like_pattern("100%_").as_deref(), Some("%100\\%\\_%"));
    }

    #[test]
    fn update_request_distinguishes_absent_from_null() {
        let absent: UpdateRequest = serde_json::from_str("{}").unwrap();
        assert_eq!(absent.mint_quota, None);
        let null: UpdateRequest = serde_json::from_str(r#"{"mint_quota": null}"#).unwrap();
        assert_eq!(null.mint_quota, Some(None));
        let set: UpdateRequest = serde_json::from_str(r#"{"mint_quota": 1200}"#).unwrap();
        assert_eq!(set.mint_quota, Some(Some(1200)));
    }
}
