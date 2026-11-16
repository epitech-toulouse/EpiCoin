//! Authentication: sessions, account activation, password reset.
//!
//! - Sessions are opaque random tokens in an `HttpOnly`, `SameSite=Lax` (and `Secure`) cookie;
//!   only their hash is stored.
//! - CSRF: every state-changing request of a logged-in user must carry the session's CSRF
//!   token in the `X-CSRF-Token` header (synchronizer token). Unauthenticated endpoints only
//!   accept JSON bodies, which a cross-site form cannot produce.
//! - Login and password-reset requests are rate limited per account and per client address.
//! - Emailed links carry their token in the URL *fragment* (`/activation#token=…`), which
//!   browsers never send to a server, and the API only receives tokens in request bodies:
//!   no token ever lands in an access log.
//!
//! A future Microsoft 365 login (OIDC) plugs in here: an `/api/auth/oidc/callback` handler
//! would resolve `user_identities (provider, subject)` to a user and call [`start_session`],
//! exactly like `login` does after checking a password.

use std::net::SocketAddr;
use std::sync::OnceLock;
use std::time::Duration;

use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::request::Parts;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;
use sqlx::PgConnection;
use time::OffsetDateTime;
use uuid::Uuid;

use crate::audit;
use crate::error::{ApiError, ApiResult};
use crate::mailer::Mail;
use crate::models::{Role, User, UserView};
use crate::secrets::{
    check_password_policy, constant_time_eq, hash_password, random_token, token_hash,
    verify_password,
};
use crate::state::AppState;

pub const SESSION_COOKIE: &str = "epicoin_session";
pub const CSRF_HEADER: &str = "x-csrf-token";

const LOGIN_WINDOW: Duration = Duration::from_secs(15 * 60);
const LOGIN_MAX_PER_ACCOUNT: u32 = 5;
// Generous: a whole campus can share one public address behind its NAT.
const LOGIN_MAX_PER_CLIENT: u32 = 200;
const RESET_WINDOW: Duration = Duration::from_secs(60 * 60);
const RESET_MAX_PER_ACCOUNT: u32 = 3;
const RESET_MAX_PER_CLIENT: u32 = 30;

pub const PURPOSE_INVITE: &str = "invite";
pub const PURPOSE_RESET: &str = "reset";

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/login", post(login))
        .route("/logout", post(logout))
        .route("/me", get(me))
        .route("/invitation", post(invitation))
        .route("/activate", post(activate))
        .route("/forgot", post(forgot))
        .route("/reset", post(reset))
}

// --- The authenticated user ------------------------------------------------------------

/// The logged-in user. Extracting it enforces a valid session and, for state-changing
/// requests, a valid CSRF token.
pub struct CurrentUser {
    pub user: User,
    csrf_token: String,
    session_hash: Vec<u8>,
}

impl CurrentUser {
    pub fn id(&self) -> Uuid {
        self.user.id
    }

    /// Fails with 403 unless the user has one of `allowed` roles.
    pub fn require(&self, allowed: &[Role]) -> ApiResult<()> {
        if allowed.contains(&self.user.role) {
            Ok(())
        } else {
            Err(ApiError::forbidden())
        }
    }
}

#[derive(sqlx::FromRow)]
struct SessionRow {
    #[sqlx(flatten)]
    user: User,
    csrf_token: String,
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, ApiError> {
        let token = cookie(&parts.headers, SESSION_COOKIE).ok_or_else(ApiError::unauthorized)?;
        let session_hash = token_hash(&token);
        let row = sqlx::query_as::<_, SessionRow>(
            "SELECT u.*, s.csrf_token FROM sessions s JOIN users u ON u.id = s.user_id \
             WHERE s.token_hash = $1 AND s.expires_at > now()",
        )
        .bind(&session_hash)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(ApiError::unauthorized)?;
        if !row.user.is_active() {
            return Err(ApiError::unauthorized());
        }

        if !parts.method.is_safe() {
            let sent = parts
                .headers
                .get(CSRF_HEADER)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("");
            if !constant_time_eq(sent, &row.csrf_token) {
                return Err(ApiError::new(
                    StatusCode::FORBIDDEN,
                    "csrf",
                    "Jeton de sécurité manquant ou invalide. Rechargez la page.",
                ));
            }
        }
        Ok(Self {
            user: row.user,
            csrf_token: row.csrf_token,
            session_hash,
        })
    }
}

fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_owned())
}

/// The client's network address, for rate limiting.
pub struct ClientAddr(pub String);

impl FromRequestParts<AppState> for ClientAddr {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let forwarded = state
            .settings
            .trust_proxy
            .then(|| {
                parts
                    .headers
                    .get("x-forwarded-for")
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.split(',').next())
                    .map(|first| first.trim().to_owned())
            })
            .flatten();
        let direct = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0.ip().to_string());
        Ok(Self(
            forwarded.or(direct).unwrap_or_else(|| "unknown".to_owned()),
        ))
    }
}

// --- Sessions --------------------------------------------------------------------------

fn session_cookie(state: &AppState, token: &str, max_age_secs: u64) -> HeaderValue {
    let secure = if state.settings.cookie_secure {
        "; Secure"
    } else {
        ""
    };
    let value = format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_secs}{secure}"
    );
    // Tokens are hex: the value is always a valid header.
    HeaderValue::from_str(&value).expect("session cookie is ASCII")
}

/// Opens a session for `user` and builds the response carrying its cookie.
pub async fn start_session(state: &AppState, user: &User) -> ApiResult<Response> {
    let token = random_token();
    let csrf_token = random_token();
    let ttl = state.settings.session_ttl;
    sqlx::query(
        "INSERT INTO sessions (token_hash, user_id, csrf_token, expires_at) \
         VALUES ($1, $2, $3, now() + make_interval(secs => $4))",
    )
    .bind(token_hash(&token))
    .bind(user.id)
    .bind(&csrf_token)
    .bind(ttl.as_secs_f64())
    .execute(&state.db)
    .await?;
    // Opportunistic cleanup of expired sessions.
    sqlx::query("DELETE FROM sessions WHERE expires_at <= now()")
        .execute(&state.db)
        .await?;

    let body = Json(json!({ "user": UserView::from(user), "csrf_token": csrf_token }));
    let mut response = body.into_response();
    response
        .headers_mut()
        .insert(SET_COOKIE, session_cookie(state, &token, ttl.as_secs()));
    Ok(response)
}

/// A dummy hash verified when the account does not exist, so that response time does not
/// reveal which emails have an account.
fn decoy_hash() -> &'static str {
    static DECOY: OnceLock<String> = OnceLock::new();
    DECOY.get_or_init(|| hash_password("decoy password, never valid").unwrap_or_default())
}

/// Argon2 is deliberately slow: keep it off the async worker threads.
async fn verify_blocking(password: String, phc: String) -> bool {
    tokio::task::spawn_blocking(move || verify_password(&password, &phc))
        .await
        .unwrap_or(false)
}

async fn hash_blocking(password: String) -> ApiResult<String> {
    tokio::task::spawn_blocking(move || hash_password(&password))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)
}

fn normalize_email(email: &str) -> String {
    email.trim().to_lowercase()
}

async fn find_user_by_email(state: &AppState, email: &str) -> ApiResult<Option<User>> {
    Ok(
        sqlx::query_as::<_, User>("SELECT * FROM users WHERE email = $1")
            .bind(email)
            .fetch_optional(&state.db)
            .await?,
    )
}

// --- Handlers --------------------------------------------------------------------------

#[derive(Deserialize)]
struct LoginRequest {
    email: String,
    password: String,
}

async fn login(
    State(state): State<AppState>,
    ClientAddr(client): ClientAddr,
    Json(body): Json<LoginRequest>,
) -> ApiResult<Response> {
    let email = normalize_email(&body.email);
    if email.len() > 254 || body.password.len() > 1024 {
        return Err(invalid_credentials());
    }
    let account_key = format!("login:account:{email}");
    let client_ok = state.limiter.hit(
        &format!("login:client:{client}"),
        LOGIN_MAX_PER_CLIENT,
        LOGIN_WINDOW,
    );
    let account_ok = state
        .limiter
        .hit(&account_key, LOGIN_MAX_PER_ACCOUNT, LOGIN_WINDOW);
    if !client_ok || !account_ok {
        return Err(ApiError::too_many_requests());
    }

    let user = find_user_by_email(&state, &email).await?;
    let phc = user
        .as_ref()
        .and_then(|u| u.password_hash.clone())
        .unwrap_or_else(|| decoy_hash().to_owned());
    let password_ok = verify_blocking(body.password, phc).await;

    let user = match user {
        Some(user) if password_ok && user.is_active() => user,
        _ => return Err(invalid_credentials()),
    };
    state.limiter.reset(&account_key);
    audit::record(
        &state.db,
        Some(user.id),
        audit::action::LOGIN,
        None,
        json!({}),
    )
    .await?;
    start_session(&state, &user).await
}

fn invalid_credentials() -> ApiError {
    ApiError::new(
        StatusCode::UNAUTHORIZED,
        "invalid_credentials",
        "Email ou mot de passe incorrect.",
    )
}

async fn logout(State(state): State<AppState>, current: CurrentUser) -> ApiResult<Response> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = $1")
        .bind(&current.session_hash)
        .execute(&state.db)
        .await?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .insert(SET_COOKIE, session_cookie(&state, "", 0));
    Ok(response)
}

async fn me(State(state): State<AppState>, current: CurrentUser) -> ApiResult<Response> {
    let address = crate::wallets::address_of(&state.db, current.id()).await?;
    Ok(Json(json!({
        "user": UserView::from(&current.user),
        "csrf_token": current.csrf_token,
        "wallet_address": address,
    }))
    .into_response())
}

/// Creates a single-use token for `user_id`, replacing any unused one of the same purpose.
pub async fn issue_token(
    conn: &mut PgConnection,
    user_id: Uuid,
    purpose: &str,
    ttl: Duration,
) -> Result<String, sqlx::Error> {
    sqlx::query("DELETE FROM auth_tokens WHERE user_id = $1 AND purpose = $2 AND used_at IS NULL")
        .bind(user_id)
        .bind(purpose)
        .execute(&mut *conn)
        .await?;
    let token = random_token();
    sqlx::query(
        "INSERT INTO auth_tokens (token_hash, user_id, purpose, expires_at) \
         VALUES ($1, $2, $3, now() + make_interval(secs => $4))",
    )
    .bind(token_hash(&token))
    .bind(user_id)
    .bind(purpose)
    .bind(ttl.as_secs_f64())
    .execute(&mut *conn)
    .await?;
    Ok(token)
}

fn invalid_token() -> ApiError {
    ApiError::new(
        StatusCode::NOT_FOUND,
        "invalid_token",
        "Ce lien est invalide ou a expiré. Demandez-en un nouveau.",
    )
}

/// The user a valid, unused token of `purpose` belongs to. Locks the token row.
async fn user_for_token(conn: &mut PgConnection, token: &str, purpose: &str) -> ApiResult<User> {
    // Lock the token first (two requests with the same token must not both succeed)…
    let user_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM auth_tokens \
         WHERE token_hash = $1 AND purpose = $2 AND used_at IS NULL AND expires_at > now() \
         FOR UPDATE",
    )
    .bind(token_hash(token))
    .bind(purpose)
    .fetch_optional(&mut *conn)
    .await?;
    let user_id = user_id.ok_or_else(invalid_token)?;
    // …then load its owner.
    sqlx::query_as::<_, User>("SELECT * FROM users WHERE id = $1 AND disabled_at IS NULL")
        .bind(user_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(invalid_token)
}

#[derive(Deserialize)]
struct TokenRequest {
    token: String,
}

/// Tells the activation page who the invitation is for.
///
/// A `POST` with the token in the body, like every endpoint that takes a token: URLs end
/// up in access logs, bodies do not.
async fn invitation(
    State(state): State<AppState>,
    Json(body): Json<TokenRequest>,
) -> ApiResult<Json<serde_json::Value>> {
    let mut conn = state.db.acquire().await?;
    let user = user_for_token(&mut conn, &body.token, PURPOSE_INVITE).await?;
    Ok(Json(
        json!({ "email": user.email, "first_name": user.first_name }),
    ))
}

#[derive(Deserialize)]
struct TokenPasswordRequest {
    token: String,
    password: String,
}

/// Sets the password of the token's owner and consumes the token.
async fn set_password_with_token(
    state: &AppState,
    body: TokenPasswordRequest,
    purpose: &str,
    action: &str,
) -> ApiResult<User> {
    check_password_policy(&body.password)
        .map_err(|message| ApiError::unprocessable("weak_password", message))?;
    let password_hash = hash_blocking(body.password).await?;

    let mut tx = state.db.begin().await?;
    let user = user_for_token(&mut tx, &body.token, purpose).await?;
    sqlx::query(
        "UPDATE users SET password_hash = $2, activated_at = COALESCE(activated_at, now()) \
         WHERE id = $1",
    )
    .bind(user.id)
    .bind(&password_hash)
    .execute(&mut *tx)
    .await?;
    sqlx::query("UPDATE auth_tokens SET used_at = now() WHERE token_hash = $1")
        .bind(token_hash(&body.token))
        .execute(&mut *tx)
        .await?;
    // A new password ends every existing session.
    sqlx::query("DELETE FROM sessions WHERE user_id = $1")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    audit::record(&mut *tx, Some(user.id), action, None, json!({})).await?;
    tx.commit().await?;

    Ok(User {
        password_hash: Some(password_hash),
        activated_at: user.activated_at.or(Some(OffsetDateTime::now_utc())),
        ..user
    })
}

/// Account activation from an invitation link: choose a password, and get logged in.
async fn activate(
    State(state): State<AppState>,
    Json(body): Json<TokenPasswordRequest>,
) -> ApiResult<Response> {
    let user = set_password_with_token(
        &state,
        body,
        PURPOSE_INVITE,
        audit::action::ACCOUNT_ACTIVATED,
    )
    .await?;
    start_session(&state, &user).await
}

#[derive(Deserialize)]
struct ForgotRequest {
    email: String,
}

/// Always answers 204, whether or not the account exists.
async fn forgot(
    State(state): State<AppState>,
    ClientAddr(client): ClientAddr,
    Json(body): Json<ForgotRequest>,
) -> ApiResult<StatusCode> {
    let email = normalize_email(&body.email);
    let client_ok = state.limiter.hit(
        &format!("reset:client:{client}"),
        RESET_MAX_PER_CLIENT,
        RESET_WINDOW,
    );
    let account_ok = email.len() <= 254
        && state.limiter.hit(
            &format!("reset:account:{email}"),
            RESET_MAX_PER_ACCOUNT,
            RESET_WINDOW,
        );
    if !client_ok {
        return Err(ApiError::too_many_requests());
    }
    if account_ok {
        if let Some(user) = find_user_by_email(&state, &email).await? {
            if user.disabled_at.is_none() {
                // Not activated yet? The useful email is the invitation.
                let result = if user.activated_at.is_some() {
                    send_reset(&state, &user).await
                } else {
                    send_invitation(&state, &user).await
                };
                if let Err(error) = result {
                    tracing::warn!(user_id = %user.id, error = %error.message, "reset email not sent");
                }
            }
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn reset(
    State(state): State<AppState>,
    Json(body): Json<TokenPasswordRequest>,
) -> ApiResult<StatusCode> {
    set_password_with_token(&state, body, PURPOSE_RESET, audit::action::PASSWORD_RESET).await?;
    Ok(StatusCode::NO_CONTENT)
}

// --- Emails ----------------------------------------------------------------------------

async fn deliver(state: &AppState, mail: Mail) -> ApiResult<()> {
    state.mailer.send(&mail).await.map_err(|error| {
        tracing::warn!(%error, "email delivery failed");
        ApiError::unavailable("L'email n'a pas pu être envoyé. Réessayez plus tard.")
    })
}

/// Issues a fresh invitation token and emails the activation link.
pub async fn send_invitation(state: &AppState, user: &User) -> ApiResult<()> {
    let mut conn = state.db.acquire().await?;
    let token = issue_token(
        &mut conn,
        user.id,
        PURPOSE_INVITE,
        state.settings.invite_ttl,
    )
    .await?;
    let days = state.settings.invite_ttl.as_secs() / 86_400;
    deliver(
        state,
        Mail {
            to: user.email.clone(),
            subject: "Bienvenue sur EpiCoin : activez votre compte".into(),
            body: format!(
                "Bonjour {first_name},\n\n\
                 Un compte EpiCoin a été créé pour vous. Pour l'activer et choisir votre mot \
                 de passe, ouvrez ce lien :\n\n\
                 {url}/activation#token={token}\n\n\
                 Ce lien est personnel, utilisable une seule fois et valable {days} jours.\n\n\
                 L'équipe EpiCoin",
                first_name = user.first_name,
                url = state.settings.public_url,
            ),
        },
    )
    .await
}

/// Issues a password-reset token and emails the link.
pub async fn send_reset(state: &AppState, user: &User) -> ApiResult<()> {
    let mut conn = state.db.acquire().await?;
    let token = issue_token(&mut conn, user.id, PURPOSE_RESET, state.settings.reset_ttl).await?;
    let minutes = state.settings.reset_ttl.as_secs() / 60;
    deliver(
        state,
        Mail {
            to: user.email.clone(),
            subject: "EpiCoin : réinitialisation de votre mot de passe".into(),
            body: format!(
                "Bonjour {first_name},\n\n\
                 Pour choisir un nouveau mot de passe, ouvrez ce lien :\n\n\
                 {url}/reinitialisation#token={token}\n\n\
                 Ce lien est utilisable une seule fois et valable {minutes} minutes. Si vous \
                 n'êtes pas à l'origine de cette demande, ignorez cet email : votre mot de \
                 passe reste inchangé.\n\n\
                 L'équipe EpiCoin",
                first_name = user.first_name,
                url = state.settings.public_url,
            ),
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_parsing() {
        let mut headers = HeaderMap::new();
        headers.insert(
            COOKIE,
            HeaderValue::from_static("theme=dark; epicoin_session=abc123; other=x=y"),
        );
        assert_eq!(cookie(&headers, SESSION_COOKIE).as_deref(), Some("abc123"));
        assert_eq!(cookie(&headers, "other").as_deref(), Some("x=y"));
        assert_eq!(cookie(&headers, "missing"), None);
        // A cookie whose name merely ends with ours must not match.
        let mut headers = HeaderMap::new();
        headers.insert(COOKIE, HeaderValue::from_static("not_epicoin_session=evil"));
        assert_eq!(cookie(&headers, SESSION_COOKIE), None);
    }

    #[test]
    fn emails_are_normalized() {
        assert_eq!(
            normalize_email("  Jean.Dupont@Epitech.EU "),
            "jean.dupont@epitech.eu"
        );
    }
}
