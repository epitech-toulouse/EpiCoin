//! Custodial keys: encrypted at rest, never in a response, never in a log.

mod common;

use axum::http::StatusCode;
use common::*;
use epicoin_api::models::Role;
use epicoin_api::vault::{MasterKey, Sealed, Vault};
use epicoin_api::wallets;
use epicoin_core::Keypair;
use serde_json::json;
use sqlx::PgPool;
use uuid::Uuid;

/// Every wallet's secret key, decrypted, as hex.
async fn wallet_secrets(app: &TestApp) -> Vec<String> {
    let rows: Vec<(Uuid, Vec<u8>, String)> =
        sqlx::query_as("SELECT user_id, encrypted_secret, key_id FROM wallets")
            .fetch_all(&app.db)
            .await
            .unwrap();
    rows.into_iter()
        .map(|(user_id, bytes, key_id)| {
            let secret = app
                .state
                .vault
                .open(&Sealed { key_id, bytes }, user_id.as_bytes())
                .unwrap();
            hex::encode(secret.as_slice())
        })
        .collect()
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn private_keys_never_appear_in_responses_or_logs(db: PgPool) {
    let app = TestApp::start(db).await;

    // --- Exercise every part of the API, happy paths and errors.
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let imported = admin
        .post(
            "/api/admin/users/import",
            json!({ "csv": "email,prenom,nom,promo,role\n\
                            ada@epitech.eu,Ada,Lovelace,pge1,student\n\
                            alan@epitech.eu,Alan,Turing,,staff\n" }),
        )
        .await;
    assert_eq!(imported.status, StatusCode::OK);

    let mut tokens = Vec::new();
    let mut ada = app.client();
    let mut alan = app.client();
    for (client, email) in [(&mut ada, "ada@epitech.eu"), (&mut alan, "alan@epitech.eu")] {
        let token = token_in(&app.mail_to(email).await);
        client
            .post("/api/auth/invitation", json!({ "token": token }))
            .await;
        let reply = client
            .post(
                "/api/auth/activate",
                json!({ "token": token, "password": PASSWORD }),
            )
            .await;
        assert_eq!(reply.status, StatusCode::OK);
        tokens.push(token);
    }
    let users = admin.get("/api/admin/users").await;
    let id_of = |email: &str| {
        users.body["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|u| u["email"] == email)
            .map(|u| u["id"].as_str().unwrap().to_owned())
            .unwrap()
    };
    let (ada_id, alan_id) = (id_of("ada@epitech.eu"), id_of("alan@epitech.eu"));

    app.eventually("alan's authority", || {
        app.node
            .authorities()
            .iter()
            .any(|authority| authority.active)
    })
    .await;
    let minted = alan
        .post(
            "/api/staff/distributions",
            json!({ "recipient_ids": [ada_id], "amount": 1_000, "reason_code": "challenge",
                    "comment": "Bravo" }),
        )
        .await;
    assert_eq!(minted.body["status"], "included", "{:?}", minted.body);
    let tx_hash = minted.body["results"][0]["tx_hash"]
        .as_str()
        .unwrap()
        .to_owned();

    for path in [
        "/api/auth/me",
        "/api/me/wallet",
        "/api/me/transactions",
        "/api/staff/quota",
        "/api/staff/distributions",
        "/api/staff/students",
    ] {
        ada.get(path).await;
        alan.get(path).await;
    }
    let ada_address = ada.get("/api/me/wallet").await.body["address"]
        .as_str()
        .unwrap()
        .to_owned();
    for path in [
        "/api/admin/users".to_owned(),
        "/api/admin/users?q=ada&role=student&limit=5".to_owned(),
        "/api/admin/audit".to_owned(),
        "/api/admin/stats".to_owned(),
        "/api/explorer/status".to_owned(),
        "/api/explorer/blocks".to_owned(),
        "/api/explorer/blocks/1".to_owned(),
        "/api/explorer/txs".to_owned(),
        format!("/api/explorer/tx/{tx_hash}"),
        "/api/explorer/accounts".to_owned(),
        format!("/api/explorer/accounts/{ada_address}"),
        "/api/health".to_owned(),
    ] {
        let reply = admin.get(&path).await;
        assert_eq!(reply.status, StatusCode::OK, "{path}: {:?}", reply.body);
    }
    admin
        .patch(
            &format!("/api/admin/users/{alan_id}"),
            json!({ "mint_quota": 80_000 }),
        )
        .await;
    admin
        .patch(
            &format!("/api/admin/users/{ada_id}"),
            json!({ "role": "staff" }),
        )
        .await;
    // Errors too: they are the classic place for a debug dump to leak.
    admin
        .patch(&format!("/api/admin/users/{ada_id}"), json!({ "role": 42 }))
        .await;
    admin
        .post("/api/admin/users", json!({ "nonsense": true }))
        .await;
    alan.post(
        "/api/staff/distributions",
        json!({ "recipient_ids": "all" }),
    )
    .await;
    ada.post(
        "/api/staff/distributions",
        json!({ "recipient_ids": [ada_id], "amount": 1, "reason_code": "other" }),
    )
    .await;
    app.client().login("ada@epitech.eu", "wrong password").await;
    app.client()
        .post("/api/auth/forgot", json!({ "email": "ada@epitech.eu" }))
        .await;
    let reset_token = token_in(&app.mail_to("ada@epitech.eu").await);
    app.client()
        .post(
            "/api/auth/reset",
            json!({ "token": reset_token, "password": "a brand new password" }),
        )
        .await;
    tokens.push(reset_token);
    epicoin_api::authority::sync_all(&app.state).await.unwrap();
    app.state
        .chain
        .reconcile(std::time::Duration::ZERO)
        .await
        .unwrap();

    // --- Everything that must stay secret.
    let mut secrets: Vec<(String, String)> = Vec::new();
    for secret in wallet_secrets(&app).await {
        secrets.push(("a wallet private key".into(), secret));
    }
    assert_eq!(secrets.len(), 3);
    for (name, keypair) in [
        ("the admin key", &app.admin_key),
        ("the treasury key", &app.treasury_key),
        ("the validator key", &app.validator_key),
    ] {
        secrets.push((name.into(), keypair.secret_hex().to_string()));
    }
    secrets.push(("the master key".into(), app.master_key_hex.clone()));
    let sealed: Vec<Vec<u8>> = sqlx::query_scalar("SELECT encrypted_secret FROM wallets")
        .fetch_all(&app.db)
        .await
        .unwrap();
    for blob in sealed {
        secrets.push(("an encrypted wallet blob".into(), hex::encode(blob)));
    }
    let hashes: Vec<String> =
        sqlx::query_scalar("SELECT password_hash FROM users WHERE password_hash IS NOT NULL")
            .fetch_all(&app.db)
            .await
            .unwrap();
    for hash in hashes {
        secrets.push(("a password hash".into(), hash));
    }
    secrets.push(("a password".into(), PASSWORD.to_owned()));
    secrets.push(("a password".into(), "a brand new password".to_owned()));

    let transcript = app.transcript();
    let logs = logs();
    assert!(transcript.len() > 5_000, "the scenario produced responses");
    assert!(logs.len() > 1_000, "logs are being captured");
    // Request URLs are logged: that is why no token may ever travel in one.
    assert!(
        logs.contains("/api/auth/activate"),
        "request logs are captured"
    );
    for (name, secret) in &secrets {
        for variant in [secret.clone(), secret.to_uppercase()] {
            assert!(
                !transcript.contains(&variant),
                "{name} appears in an HTTP response"
            );
            assert!(!logs.contains(&variant), "{name} appears in the logs");
        }
    }
    // Single-use tokens and session cookies are credentials too: not in the logs.
    for token in &tokens {
        assert!(
            !logs.contains(token),
            "an emailed token appears in the logs"
        );
    }
    for client in [&admin, &ada, &alan] {
        let cookie = client.cookie.as_ref().unwrap();
        assert!(
            !logs.contains(cookie),
            "a session cookie appears in the logs"
        );
    }
    // And no response even has a field for such things.
    for field in [
        "password_hash",
        "encrypted_secret",
        "secret",
        "private",
        "key_id",
        "token_hash",
    ] {
        assert!(
            !transcript.contains(&format!("\"{field}")),
            "a response exposes a `{field}…` field"
        );
    }
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn wallet_keys_are_encrypted_at_rest(db: PgPool) {
    let app = TestApp::start(db).await;
    let user = app.seed(Role::Student).await;
    let keypair = wallets::load_keypair(&app.db, &app.state.vault, user.id)
        .await
        .unwrap();
    assert_eq!(keypair.address(), user.address);

    let (blob, key_id): (Vec<u8>, String) =
        sqlx::query_as("SELECT encrypted_secret, key_id FROM wallets WHERE user_id = $1")
            .bind(user.id)
            .fetch_one(&app.db)
            .await
            .unwrap();
    let secret = keypair.secret_bytes();
    assert!(
        !blob
            .windows(8)
            .any(|window| secret.windows(8).any(|s| s == window)),
        "the stored blob contains part of the raw key"
    );
    assert_eq!(key_id, app.state.vault.current_key_id());
    assert_ne!(key_id, app.master_key_hex);

    // Another master key cannot read it.
    let other = Vault::new(
        MasterKey::from_hex(&MasterKey::generate_hex()).unwrap(),
        None,
    );
    assert!(wallets::load_keypair(&app.db, &other, user.id)
        .await
        .is_err());
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn a_wallet_blob_moved_to_another_user_does_not_decrypt(db: PgPool) {
    let app = TestApp::start(db).await;
    let rich = app.seed(Role::Staff).await;
    let attacker = app.seed(Role::Student).await;

    // With write access to the database, an attacker copies the staff member's encrypted
    // key (and address) onto their own account, hoping the API will sign with it.
    sqlx::query(
        "UPDATE wallets w SET encrypted_secret = src.encrypted_secret, key_id = src.key_id \
         FROM wallets src WHERE w.user_id = $1 AND src.user_id = $2",
    )
    .bind(attacker.id)
    .bind(rich.id)
    .execute(&app.db)
    .await
    .unwrap();

    assert!(
        wallets::load_keypair(&app.db, &app.state.vault, attacker.id)
            .await
            .is_err(),
        "the ciphertext is bound to its owner"
    );
    let mut client = app.client();
    client.login(&attacker.email, PASSWORD).await;
    // The API fails closed instead of signing with someone else's key.
    let wallet = client.get("/api/me/wallet").await;
    assert_eq!(wallet.status, StatusCode::OK); // reading the address needs no key
    assert_eq!(wallet.body["address"], attacker.address.to_string());
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn master_key_rotation_keeps_every_wallet_usable(db: PgPool) {
    let app = TestApp::start(db).await;
    let staff = app.seed(Role::Staff).await;
    let student = app.seed(Role::Student).await;
    let before: Vec<Keypair> = vec![
        wallets::load_keypair(&app.db, &app.state.vault, staff.id)
            .await
            .unwrap(),
        wallets::load_keypair(&app.db, &app.state.vault, student.id)
            .await
            .unwrap(),
    ];
    let old_key = || MasterKey::from_hex(&app.master_key_hex).unwrap();
    let new_hex = MasterKey::generate_hex().to_string();
    let new_key = || MasterKey::from_hex(&new_hex).unwrap();

    // Forgetting to provide the old key: nothing is changed.
    let incomplete = Vault::new(new_key(), None);
    assert!(wallets::rotate_master_key(&app.db, &incomplete)
        .await
        .is_err());
    assert!(wallets::load_keypair(&app.db, &app.state.vault, staff.id)
        .await
        .is_ok());

    // The documented procedure: new key as current, old key as previous.
    let rotating = Vault::new(new_key(), Some(old_key()));
    let report = wallets::rotate_master_key(&app.db, &rotating)
        .await
        .unwrap();
    assert_eq!(report.rotated, 2);
    assert_eq!(report.already_current, 0);
    // Running it twice is harmless.
    let again = wallets::rotate_master_key(&app.db, &rotating)
        .await
        .unwrap();
    assert_eq!(again.rotated, 0);
    assert_eq!(again.already_current, 2);

    // The old key alone no longer opens anything; the new key alone opens everything,
    // and the keys themselves (hence the addresses and balances) are unchanged.
    assert!(wallets::load_keypair(&app.db, &app.state.vault, staff.id)
        .await
        .is_err());
    let rotated = Vault::new(new_key(), None);
    for (user, original) in [&staff, &student].into_iter().zip(&before) {
        let keypair = wallets::load_keypair(&app.db, &rotated, user.id)
            .await
            .unwrap();
        assert_eq!(keypair.address(), original.address());
        assert_eq!(*keypair.secret_bytes(), *original.secret_bytes());
    }

    // The API restarted with only the new key still signs transactions.
    let restarted = app.restarted_with(rotated);
    let mut client = restarted.client();
    assert_eq!(client.login(&staff.email, PASSWORD).await, StatusCode::OK);
    let reply = client
        .post(
            "/api/staff/distributions",
            json!({ "recipient_ids": [student.id], "amount": 400, "reason_code": "event" }),
        )
        .await;
    assert_eq!(reply.body["status"], "included", "{:?}", reply.body);
    assert_eq!(restarted.balance(&student.address), 400);
}
