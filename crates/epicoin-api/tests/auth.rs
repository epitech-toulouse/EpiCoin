//! Authentication: login, sessions, CSRF, rate limiting, activation, password reset.

mod common;

use axum::http::{Method, StatusCode};
use common::*;
use epicoin_api::config::Settings;
use epicoin_api::models::Role;
use serde_json::json;
use sqlx::PgPool;

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn health_reports_database_and_node(db: PgPool) {
    let app = TestApp::start(db).await;
    let reply = app.client().get("/api/health").await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body["status"], "ok");
    assert_eq!(reply.body["database"], true);
    assert_eq!(reply.body["node"], true);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn protected_endpoints_require_a_session(db: PgPool) {
    let app = TestApp::start(db).await;
    let mut anonymous = app.client();
    let id = uuid::Uuid::new_v4();
    let endpoints: Vec<(Method, String)> = vec![
        (Method::GET, "/api/auth/me".into()),
        (Method::POST, "/api/auth/logout".into()),
        (Method::GET, "/api/me/wallet".into()),
        (Method::GET, "/api/me/transactions".into()),
        (Method::GET, "/api/staff/students".into()),
        (Method::GET, "/api/staff/promos".into()),
        (Method::GET, "/api/staff/quota".into()),
        (Method::GET, "/api/staff/distributions".into()),
        (Method::POST, "/api/staff/distributions".into()),
        (Method::POST, "/api/staff/students/resolve".into()),
        (Method::GET, "/api/staff/challenges".into()),
        (Method::POST, "/api/staff/challenges".into()),
        (Method::PATCH, format!("/api/staff/challenges/{id}")),
        (
            Method::GET,
            format!("/api/staff/challenges/{id}/submissions"),
        ),
        (
            Method::POST,
            format!("/api/staff/challenges/{id}/submissions/{id}"),
        ),
        (Method::GET, "/api/challenges".into()),
        (Method::POST, "/api/transfers".into()),
        (Method::GET, "/api/transfers/recipients".into()),
        (Method::GET, "/api/admin/transfers".into()),
        (Method::POST, format!("/api/challenges/{id}/answer")),
        (Method::POST, format!("/api/challenges/{id}/submission")),
        (Method::GET, "/api/admin/users".into()),
        (Method::POST, "/api/admin/users".into()),
        (Method::POST, "/api/admin/users/import".into()),
        (Method::PATCH, format!("/api/admin/users/{id}")),
        (Method::DELETE, format!("/api/admin/users/{id}")),
        (Method::POST, format!("/api/admin/users/{id}/invitation")),
        (Method::GET, "/api/admin/audit".into()),
        (Method::GET, "/api/admin/stats".into()),
        (Method::GET, "/api/explorer/status".into()),
        (Method::GET, "/api/explorer/blocks".into()),
        (Method::GET, "/api/explorer/blocks/1".into()),
        (Method::GET, "/api/explorer/txs".into()),
        (Method::GET, "/api/explorer/accounts".into()),
    ];
    for (method, path) in endpoints {
        let body = (method != Method::GET && method != Method::DELETE).then(|| json!({}));
        let reply = anonymous.send(method.clone(), &path, body, &[]).await;
        assert_eq!(
            reply.status,
            StatusCode::UNAUTHORIZED,
            "{method} {path} must require a session"
        );
        assert_eq!(reply.error(), "unauthorized");
    }
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn unknown_routes_answer_json_404(db: PgPool) {
    let app = TestApp::start(db).await;
    let reply = app.client().get("/api/does-not-exist").await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
    assert_eq!(reply.error(), "not_found");
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn login_opens_a_session_with_a_hardened_cookie(db: PgPool) {
    let app = TestApp::start(db).await;
    let student = app.seed(Role::Student).await;
    let mut client = app.client();

    let reply = client
        .post(
            "/api/auth/login",
            json!({ "email": format!("  {} ", student.email.to_uppercase()), "password": PASSWORD }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body["user"]["email"], student.email);
    assert_eq!(reply.body["user"]["role"], "student");
    assert!(reply.body["csrf_token"].as_str().unwrap().len() >= 32);

    let cookie = reply.headers["set-cookie"].to_str().unwrap();
    assert!(cookie.starts_with("epicoin_session="));
    assert!(cookie.contains("HttpOnly"));
    assert!(cookie.contains("SameSite=Lax"));
    assert!(cookie.contains("Path=/"));
    assert!(cookie.contains("Max-Age="));
    // Development settings: plain HTTP, so no Secure flag (see the next test).
    assert!(!cookie.contains("Secure"));

    let me = client.get("/api/auth/me").await;
    assert_eq!(me.status, StatusCode::OK);
    assert_eq!(me.body["user"]["id"], student.id.to_string());
    assert_eq!(me.body["wallet_address"], student.address.to_string());
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn cookie_is_secure_in_production_settings(db: PgPool) {
    let settings = Settings {
        cookie_secure: true,
        public_url: "https://epicoin.example".into(),
        ..Settings::development()
    };
    let app = TestApp::start_with(db, settings).await;
    let student = app.seed(Role::Student).await;
    let reply = app
        .client()
        .post(
            "/api/auth/login",
            json!({ "email": student.email, "password": PASSWORD }),
        )
        .await;
    let cookie = reply.headers["set-cookie"].to_str().unwrap();
    assert!(cookie.contains("; Secure"));
    assert!(cookie.contains("HttpOnly"));
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn session_token_is_stored_hashed(db: PgPool) {
    let app = TestApp::start(db).await;
    let (client, _) = app.login_as(Role::Student).await;
    let token = client.cookie.clone().unwrap();
    let stored: Vec<Vec<u8>> = sqlx::query_scalar("SELECT token_hash FROM sessions")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_ne!(stored[0], token.as_bytes());
    assert_ne!(hex::encode(&stored[0]), token);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn bad_credentials_are_indistinguishable(db: PgPool) {
    let app = TestApp::start(db).await;
    let student = app.seed(Role::Student).await;
    let mut client = app.client();

    let wrong_password = client
        .post(
            "/api/auth/login",
            json!({ "email": student.email, "password": "not the password" }),
        )
        .await;
    let unknown_account = client
        .post(
            "/api/auth/login",
            json!({ "email": "nobody@epitech.eu", "password": PASSWORD }),
        )
        .await;
    assert_eq!(wrong_password.status, StatusCode::UNAUTHORIZED);
    assert_eq!(unknown_account.status, StatusCode::UNAUTHORIZED);
    assert_eq!(wrong_password.body, unknown_account.body);
    assert!(client.cookie.is_none());
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn inactive_accounts_cannot_log_in(db: PgPool) {
    let app = TestApp::start(db).await;

    // Never activated: no password yet, and even a known password would not do.
    let pending = app.seed(Role::Student).await;
    sqlx::query("UPDATE users SET activated_at = NULL WHERE id = $1")
        .bind(pending.id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        app.client().login(&pending.email, PASSWORD).await,
        StatusCode::UNAUTHORIZED
    );

    // Disabled: refused at login, and an existing session stops working at once.
    let (mut client, disabled) = app.login_as(Role::Student).await;
    sqlx::query("UPDATE users SET disabled_at = now() WHERE id = $1")
        .bind(disabled.id)
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        client.get("/api/auth/me").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.client().login(&disabled.email, PASSWORD).await,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn state_changing_requests_need_the_csrf_token(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut client, _) = app.login_as(Role::Admin).await;
    let good = client.csrf.clone().unwrap();
    let new_user = json!({
        "email": "csrf@epitech.eu", "first_name": "C", "last_name": "Srf", "role": "student"
    });

    // A cross-site request rides the cookie but cannot know the token.
    client.csrf = None;
    let reply = client.post("/api/admin/users", new_user.clone()).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.error(), "csrf");

    client.csrf = Some("0".repeat(64));
    let reply = client.post("/api/admin/users", new_user.clone()).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(reply.error(), "csrf");

    // Reads are not affected.
    assert_eq!(client.get("/api/admin/users").await.status, StatusCode::OK);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM users WHERE email = 'csrf@epitech.eu'")
            .fetch_one(&app.db)
            .await
            .unwrap();
    assert_eq!(count, 0, "a request without the token must change nothing");

    client.csrf = Some(good);
    let reply = client.post("/api/admin/users", new_user).await;
    assert_eq!(reply.status, StatusCode::CREATED);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn another_sessions_csrf_token_is_refused(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut alice, _) = app.login_as(Role::Admin).await;
    let (bob, _) = app.login_as(Role::Admin).await;
    alice.csrf = bob.csrf.clone();
    let reply = alice.post("/api/auth/logout", json!({})).await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn logout_ends_the_session(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut client, _) = app.login_as(Role::Student).await;
    let token = client.cookie.clone().unwrap();

    let reply = client.post("/api/auth/logout", json!({})).await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert!(reply.headers["set-cookie"]
        .to_str()
        .unwrap()
        .contains("Max-Age=0"));
    assert!(client.cookie.is_none());

    // The old cookie is dead on the server too, not just forgotten by the browser.
    client.cookie = Some(token);
    assert_eq!(
        client.get("/api/auth/me").await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn expired_or_forged_sessions_are_refused(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut client, _) = app.login_as(Role::Student).await;

    sqlx::query("UPDATE sessions SET expires_at = now() - interval '1 second'")
        .execute(&app.db)
        .await
        .unwrap();
    assert_eq!(
        client.get("/api/auth/me").await.status,
        StatusCode::UNAUTHORIZED
    );

    client.cookie = Some("f".repeat(64));
    assert_eq!(
        client.get("/api/auth/me").await.status,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn login_is_rate_limited_per_account(db: PgPool) {
    let app = TestApp::start(db).await;
    let victim = app.seed(Role::Student).await;
    let other = app.seed(Role::Student).await;
    let mut attacker = app.client();

    for _ in 0..5 {
        assert_eq!(
            attacker.login(&victim.email, "guess").await,
            StatusCode::UNAUTHORIZED
        );
    }
    // The sixth attempt is refused without even checking — the right password included.
    let reply = attacker
        .post(
            "/api/auth/login",
            json!({ "email": victim.email, "password": PASSWORD }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(reply.error(), "too_many_requests");

    // Other accounts are not affected.
    assert_eq!(
        app.client().login(&other.email, PASSWORD).await,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn successful_login_resets_the_attempt_counter(db: PgPool) {
    let app = TestApp::start(db).await;
    let user = app.seed(Role::Student).await;
    let mut client = app.client();
    for _ in 0..3 {
        for _ in 0..4 {
            assert_eq!(
                client.login(&user.email, "typo").await,
                StatusCode::UNAUTHORIZED
            );
        }
        assert_eq!(client.login(&user.email, PASSWORD).await, StatusCode::OK);
    }
}

// --- Activation ------------------------------------------------------------------------

/// An admin creates `email`; returns the invitation token from the email.
async fn invite(app: &TestApp, email: &str) -> String {
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let reply = admin
        .post(
            "/api/admin/users",
            json!({ "email": email, "first_name": "Ada", "last_name": "Lovelace",
                    "promo": "pge1", "role": "student" }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.body);
    token_in(&app.mail_to(email).await)
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn invitation_link_activates_the_account(db: PgPool) {
    let app = TestApp::start(db).await;
    let token = invite(&app, "ada@epitech.eu").await;
    let mail = app.mail_to("ada@epitech.eu").await;
    assert!(mail
        .body
        .contains(&format!("http://localhost:8088/activation#token={token}")));
    assert!(mail.body.contains("Bonjour Ada"));

    let mut client = app.client();
    // Before activation: no way in.
    assert_eq!(
        client.login("ada@epitech.eu", PASSWORD).await,
        StatusCode::UNAUTHORIZED
    );

    let info = client
        .post("/api/auth/invitation", json!({ "token": token }))
        .await;
    assert_eq!(info.status, StatusCode::OK);
    assert_eq!(info.body["email"], "ada@epitech.eu");
    assert_eq!(info.body["first_name"], "Ada");

    let weak = client
        .post(
            "/api/auth/activate",
            json!({ "token": token, "password": "short" }),
        )
        .await;
    assert_eq!(weak.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(weak.error(), "weak_password");

    // Activation logs the user in directly.
    let reply = client
        .post(
            "/api/auth/activate",
            json!({ "token": token, "password": PASSWORD }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(reply.body["user"]["activated"], true);
    assert_eq!(client.get("/api/auth/me").await.status, StatusCode::OK);

    // And the password works from then on.
    assert_eq!(
        app.client().login("ada@epitech.eu", PASSWORD).await,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn invitation_link_is_single_use(db: PgPool) {
    let app = TestApp::start(db).await;
    let token = invite(&app, "ada@epitech.eu").await;
    let mut client = app.client();
    let body = json!({ "token": token, "password": PASSWORD });
    assert_eq!(
        client.post("/api/auth/activate", body.clone()).await.status,
        StatusCode::OK
    );

    // Someone who got hold of the link afterwards cannot take the account over.
    let again = app
        .client()
        .post(
            "/api/auth/activate",
            json!({ "token": token, "password": "another password!" }),
        )
        .await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
    assert_eq!(again.error(), "invalid_token");
    assert_eq!(
        app.client()
            .post("/api/auth/invitation", json!({ "token": token }))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        app.client().login("ada@epitech.eu", PASSWORD).await,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn expired_or_unknown_tokens_are_refused(db: PgPool) {
    let app = TestApp::start(db).await;
    let token = invite(&app, "ada@epitech.eu").await;
    sqlx::query("UPDATE auth_tokens SET expires_at = now() - interval '1 second'")
        .execute(&app.db)
        .await
        .unwrap();
    let mut client = app.client();
    let reply = client
        .post(
            "/api/auth/activate",
            json!({ "token": token, "password": PASSWORD }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);

    let reply = client
        .post(
            "/api/auth/activate",
            json!({ "token": "a".repeat(64), "password": PASSWORD }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn tokens_are_stored_hashed(db: PgPool) {
    let app = TestApp::start(db).await;
    let token = invite(&app, "ada@epitech.eu").await;
    let stored: Vec<Vec<u8>> = sqlx::query_scalar("SELECT token_hash FROM auth_tokens")
        .fetch_all(&app.db)
        .await
        .unwrap();
    assert_eq!(stored.len(), 1);
    assert_ne!(hex::encode(&stored[0]), token);
}

// --- Password reset --------------------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn password_reset_flow(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut old_session, user) = app.login_as(Role::Student).await;
    let mut client = app.client();

    let reply = client
        .post("/api/auth/forgot", json!({ "email": user.email }))
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let mail = app.mail_to(&user.email).await;
    assert!(mail.body.contains("/reinitialisation#token="));
    let token = token_in(&mail);

    // A reset token is not an invitation token.
    let wrong_use = client
        .post(
            "/api/auth/activate",
            json!({ "token": token, "password": "new password 123" }),
        )
        .await;
    assert_eq!(wrong_use.status, StatusCode::NOT_FOUND);

    let reply = client
        .post(
            "/api/auth/reset",
            json!({ "token": token, "password": "new password 123" }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);

    // Old password and old sessions are dead; the new password works.
    assert_eq!(
        app.client().login(&user.email, PASSWORD).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        old_session.get("/api/auth/me").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.client().login(&user.email, "new password 123").await,
        StatusCode::OK
    );

    // Single use.
    let again = client
        .post(
            "/api/auth/reset",
            json!({ "token": token, "password": "yet another one" }),
        )
        .await;
    assert_eq!(again.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn forgot_password_does_not_reveal_accounts(db: PgPool) {
    let app = TestApp::start(db).await;
    let user = app.seed(Role::Student).await;
    let mut client = app.client();

    let known = client
        .post("/api/auth/forgot", json!({ "email": user.email }))
        .await;
    let unknown = client
        .post("/api/auth/forgot", json!({ "email": "nobody@epitech.eu" }))
        .await;
    assert_eq!(known.status, StatusCode::NO_CONTENT);
    assert_eq!(unknown.status, StatusCode::NO_CONTENT);
    assert_eq!(known.body, unknown.body);
    assert!(app.mailer.last_to("nobody@epitech.eu").is_none());
    assert_eq!(app.mailer.sent().len(), 1);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn forgot_password_is_rate_limited(db: PgPool) {
    let app = TestApp::start(db).await;
    let user = app.seed(Role::Student).await;
    let mut client = app.client();
    for _ in 0..6 {
        let reply = client
            .post("/api/auth/forgot", json!({ "email": user.email }))
            .await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT);
    }
    // Only the first three requests produced an email.
    assert_eq!(app.mailer.sent().len(), 3);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn forgot_password_before_activation_resends_the_invitation(db: PgPool) {
    let app = TestApp::start(db).await;
    let first_token = invite(&app, "ada@epitech.eu").await;

    app.client()
        .post("/api/auth/forgot", json!({ "email": "ada@epitech.eu" }))
        .await;
    app.eventually("a second email", || app.mailer.sent().len() == 2)
        .await;
    let mail = app.mail_to("ada@epitech.eu").await;
    assert!(mail.body.contains("/activation#token="));
    let second_token = token_in(&mail);
    assert_ne!(first_token, second_token);

    // The newer link replaces the older one.
    let old = app
        .client()
        .post("/api/auth/invitation", json!({ "token": first_token }))
        .await;
    assert_eq!(old.status, StatusCode::NOT_FOUND);
    let new = app
        .client()
        .post("/api/auth/invitation", json!({ "token": second_token }))
        .await;
    assert_eq!(new.status, StatusCode::OK);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn disabled_users_get_no_reset_email(db: PgPool) {
    let app = TestApp::start(db).await;
    let user = app.seed(Role::Student).await;
    sqlx::query("UPDATE users SET disabled_at = now() WHERE id = $1")
        .bind(user.id)
        .execute(&app.db)
        .await
        .unwrap();
    let reply = app
        .client()
        .post("/api/auth/forgot", json!({ "email": user.email }))
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    assert!(app.mailer.sent().is_empty());
}
