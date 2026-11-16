//! Staff distributions, histories, statistics and the explorer.

mod common;

use std::time::Duration;

use axum::http::{Method, StatusCode};
use common::*;
use epicoin_api::chain::{ChainError, TxMeta};
use epicoin_api::models::Role;
use epicoin_core::{ReasonCode, Transaction, TxError, TxPayload};
use serde_json::{json, Value};
use sqlx::PgPool;

fn distribution(recipients: &[&Seeded], amount: u64) -> Value {
    json!({
        "recipient_ids": recipients.iter().map(|r| r.id).collect::<Vec<_>>(),
        "amount": amount,
        "reason_code": "peer_help",
        "comment": "Merci pour le coup de main sur le projet",
    })
}

async fn count(app: &TestApp, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&app.db).await.unwrap()
}

// --- Minting ---------------------------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn staff_distributes_to_a_student(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, staff_user) = app.login_as(Role::Staff).await;
    let (mut student, student_user) = app.login_as(Role::Student).await;

    let before = staff.get("/api/staff/quota").await;
    assert_eq!(before.body["active"], true);
    assert_eq!(before.body["quota"], 50_000);
    assert_eq!(before.body["remaining"], 50_000);
    assert!(before.body["period_ends_at"]
        .as_str()
        .unwrap()
        .contains('T'));

    let reply = staff
        .post(
            "/api/staff/distributions",
            distribution(&[&student_user], 1_500),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(reply.body["status"], "included");
    assert_eq!(reply.body["requested"], 1);
    assert_eq!(reply.body["included"], 1);
    assert_eq!(
        reply.body["results"][0]["user_id"],
        student_user.id.to_string()
    );
    assert_eq!(reply.body["results"][0]["status"], "included");

    // On chain, in the student's wallet, and off the staff member's quota.
    assert_eq!(app.balance(&student_user.address), 1_500);
    let wallet = student.get("/api/me/wallet").await;
    assert_eq!(wallet.body["balance"], 1_500);
    assert_eq!(wallet.body["address"], student_user.address.to_string());
    let after = staff.get("/api/staff/quota").await;
    assert_eq!(after.body["minted"], 1_500);
    assert_eq!(after.body["remaining"], 48_500);

    // The student sees a readable history line: reason, comment, who, when, how much.
    let history = student.get("/api/me/transactions").await;
    assert_eq!(history.body["total"], 1);
    let line = &history.body["items"][0];
    assert_eq!(line["kind"], "mint");
    assert_eq!(line["direction"], "credit");
    assert_eq!(line["amount"], 1_500);
    assert_eq!(line["status"], "included");
    assert_eq!(line["reason_code"], "peer_help");
    assert_eq!(line["comment"], "Merci pour le coup de main sur le projet");
    assert!(line["from_name"].as_str().unwrap().starts_with("Prenom"));
    assert!(line["created_at"].as_str().unwrap().contains('T'));

    // The staff member sees it in their own history.
    let mine = staff.get("/api/staff/distributions").await;
    assert_eq!(mine.body["total"], 1);
    assert_eq!(
        mine.body["items"][0]["recipient_id"],
        student_user.id.to_string()
    );
    assert_eq!(mine.body["items"][0]["amount"], 1_500);
    assert_eq!(
        mine.body["items"][0]["tx_hash"],
        reply.body["results"][0]["tx_hash"]
    );

    // And it is audited.
    let audited = count(
        &app,
        "SELECT count(*) FROM audit_log WHERE action = 'mint.distributed'",
    )
    .await;
    assert_eq!(audited, 1);
    let _ = staff_user;
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn group_distribution_credits_every_student(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let mut students = Vec::new();
    for _ in 0..12 {
        students.push(app.seed(Role::Student).await);
    }
    let refs: Vec<&Seeded> = students.iter().collect();

    let mut body = distribution(&refs, 300);
    // Duplicates in the selection are credited once.
    body["recipient_ids"]
        .as_array_mut()
        .unwrap()
        .push(json!(students[0].id));
    let reply = staff.post("/api/staff/distributions", body).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(reply.body["status"], "included");
    assert_eq!(reply.body["included"], 12);
    assert_eq!(reply.body["results"].as_array().unwrap().len(), 12);

    for student in &students {
        assert_eq!(app.balance(&student.address), 300);
    }
    assert_eq!(app.node.status().total_minted, 3_600);
    assert_eq!(
        staff.get("/api/staff/quota").await.body["remaining"],
        50_000 - 3_600
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn a_student_cannot_mint(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut student, me) = app.login_as(Role::Student).await;
    let friend = app.seed(Role::Student).await;

    // Through the API: forbidden, whoever the recipient is.
    for recipients in [vec![&friend], vec![&me]] {
        let reply = student
            .post(
                "/api/staff/distributions",
                distribution(&recipients, 100_000),
            )
            .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
        assert_eq!(reply.error(), "forbidden");
    }
    assert_eq!(
        student.get("/api/staff/quota").await.status,
        StatusCode::FORBIDDEN
    );

    // And even if the API were bypassed: a mint signed with a student's own key is
    // refused by the chain itself.
    let keypair = epicoin_api::wallets::load_keypair(&app.db, &app.state.vault, me.id)
        .await
        .unwrap();
    let forged = app
        .state
        .chain
        .send(
            &keypair,
            TxPayload::Mint {
                to: friend.address,
                amount: 100_000,
                reason: ReasonCode::Other,
            },
            TxMeta::default(),
            Duration::from_secs(5),
        )
        .await;
    assert!(matches!(
        forged,
        Err(ChainError::Rejected(TxError::NotAuthority))
    ));

    assert_eq!(app.balance(&friend.address), 0);
    assert_eq!(app.balance(&me.address), 0);
    assert_eq!(app.node.status().total_minted, 0);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn only_staff_can_mint(db: PgPool) {
    let app = TestApp::start(db).await;
    let student = app.seed(Role::Student).await;
    for role in [Role::Admin, Role::ShopManager] {
        let (mut client, _) = app.login_as(role).await;
        let reply = client
            .post("/api/staff/distributions", distribution(&[&student], 100))
            .await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "as {role:?}");
    }
    assert_eq!(app.balance(&student.address), 0);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn distribution_over_quota_mints_nothing(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let a = app.seed(Role::Student).await;
    let b = app.seed(Role::Student).await;

    // 2 × 300 EPC = 600 EPC, over the 500 EPC quota.
    let reply = staff
        .post("/api/staff/distributions", distribution(&[&a, &b], 30_000))
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.error(), "quota_exceeded");
    assert_eq!(reply.body["details"]["needed"], 60_000);
    assert_eq!(reply.body["details"]["remaining"], 50_000);

    // All or nothing: nobody was credited.
    assert_eq!(app.balance(&a.address), 0);
    assert_eq!(app.balance(&b.address), 0);
    assert_eq!(
        count(&app, "SELECT count(*) FROM chain_txs WHERE kind = 'mint'").await,
        0
    );

    // Exactly the quota is fine; one more unit is not.
    let reply = staff
        .post("/api/staff/distributions", distribution(&[&a, &b], 25_000))
        .await;
    assert_eq!(reply.body["status"], "included");
    let reply = staff
        .post("/api/staff/distributions", distribution(&[&a], 1))
        .await;
    assert_eq!(reply.error(), "quota_exceeded");
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn invalid_distributions_are_refused(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, me) = app.login_as(Role::Staff).await;
    let student = app.seed(Role::Student).await;
    let colleague = app.seed(Role::Staff).await;
    let disabled = app.seed(Role::Student).await;
    sqlx::query("UPDATE users SET disabled_at = now() WHERE id = $1")
        .bind(disabled.id)
        .execute(&app.db)
        .await
        .unwrap();

    // Recipients that are not active students — including oneself.
    for bad in [&colleague, &disabled, &me] {
        let reply = staff
            .post(
                "/api/staff/distributions",
                distribution(&[&student, bad], 100),
            )
            .await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(reply.error(), "invalid_recipients");
        assert_eq!(reply.body["details"]["invalid_ids"][0], bad.id.to_string());
    }
    let unknown = staff
        .post(
            "/api/staff/distributions",
            json!({ "recipient_ids": [uuid::Uuid::new_v4()], "amount": 100,
                    "reason_code": "event" }),
        )
        .await;
    assert_eq!(unknown.error(), "invalid_recipients");

    let cases = [
        (
            json!({ "recipient_ids": [], "amount": 100, "reason_code": "event" }),
            "no_recipient",
        ),
        (
            json!({ "recipient_ids": [student.id], "amount": 0, "reason_code": "event" }),
            "invalid_amount",
        ),
        (
            json!({ "recipient_ids": [student.id], "amount": 100, "reason_code": "event",
                    "comment": "x".repeat(281) }),
            "comment_too_long",
        ),
    ];
    for (body, error) in cases {
        let reply = staff.post("/api/staff/distributions", body).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(reply.error(), error);
    }

    // A reason outside the fixed list, a negative or fractional amount: not even parsed.
    for body in [
        json!({ "recipient_ids": [student.id], "amount": 100, "reason_code": "because" }),
        json!({ "recipient_ids": [student.id], "amount": -100, "reason_code": "event" }),
        json!({ "recipient_ids": [student.id], "amount": 1.5, "reason_code": "event" }),
    ] {
        let reply = staff.post("/api/staff/distributions", body).await;
        assert!(reply.status.is_client_error());
    }

    assert_eq!(app.balance(&student.address), 0);
    assert_eq!(app.node.status().total_minted, 0);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn a_double_click_mints_once(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let student = app.seed(Role::Student).await;
    let key = [("idempotency-key", "click-42")];

    let first = staff
        .send(
            Method::POST,
            "/api/staff/distributions",
            Some(distribution(&[&student], 700)),
            &key,
        )
        .await;
    let second = staff
        .send(
            Method::POST,
            "/api/staff/distributions",
            Some(distribution(&[&student], 700)),
            &key,
        )
        .await;
    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(first.body["batch_id"], second.body["batch_id"]);
    assert_eq!(second.body["status"], "included");
    assert_eq!(app.balance(&student.address), 700);

    // A different key is a different distribution.
    let third = staff
        .send(
            Method::POST,
            "/api/staff/distributions",
            Some(distribution(&[&student], 700)),
            &[("idempotency-key", "click-43")],
        )
        .await;
    assert_ne!(third.body["batch_id"], first.body["batch_id"]);
    assert_eq!(app.balance(&student.address), 1_400);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn losing_the_staff_role_stops_minting(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let (mut staff, staff_user) = app.login_as(Role::Staff).await;
    let student = app.seed(Role::Student).await;

    admin
        .patch(
            &format!("/api/admin/users/{}", staff_user.id),
            json!({ "role": "student" }),
        )
        .await;
    let reply = staff
        .post("/api/staff/distributions", distribution(&[&student], 100))
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(app.balance(&student.address), 0);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn staff_without_on_chain_authority_gets_a_clear_error(db: PgPool) {
    let app = TestApp::start(db).await;
    // Staff in the database, but the grant has not reached the chain yet.
    let user = app.seed(Role::Student).await;
    sqlx::query("UPDATE users SET role = 'staff' WHERE id = $1")
        .bind(user.id)
        .execute(&app.db)
        .await
        .unwrap();
    let mut staff = app.client();
    staff.login(&user.email, PASSWORD).await;
    let student = app.seed(Role::Student).await;

    assert_eq!(staff.get("/api/staff/quota").await.body["active"], false);
    let reply = staff
        .post("/api/staff/distributions", distribution(&[&student], 100))
        .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.error(), "authority_inactive");
    assert_eq!(count(&app, "SELECT count(*) FROM mint_batches").await, 0);
}

// --- Privacy of the chain --------------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn no_personal_data_reaches_the_chain(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, staff_user) = app.login_as(Role::Staff).await;
    let student = app.seed(Role::Student).await;
    let reply = staff
        .post("/api/staff/distributions", distribution(&[&student], 100))
        .await;
    assert_eq!(reply.body["status"], "included");

    // Everything the chain stores: raw blocks, and their JSON views.
    let mut chain_dump = String::new();
    for block in app.node.blocks(1, 1_000).unwrap() {
        chain_dump.push_str(&String::from_utf8_lossy(&block.to_bytes()));
        chain_dump.push_str(&serde_json::to_string(&block).unwrap());
    }
    assert!(chain_dump.contains(&student.address.to_string()));
    for personal in [
        staff_user.email.as_str(),
        student.email.as_str(),
        "Prenom",
        "Nom",
        "epitech",
        "coup de main",
        &staff_user.id.to_string(),
        &student.id.to_string(),
    ] {
        assert!(
            !chain_dump.contains(personal),
            "the chain contains {personal:?}"
        );
    }
}

// --- Finding students ------------------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn staff_can_search_students(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    admin
        .post(
            "/api/admin/users/import",
            json!({ "csv": "email,prenom,nom,promo,role\n\
                            ada@epitech.eu,Ada,Lovelace,2027,student\n\
                            grace@epitech.eu,Grace,Hopper,2026,student\n\
                            alan@epitech.eu,Alan,Turing,,staff\n" }),
        )
        .await;
    let (mut staff, _) = app.login_as(Role::Staff).await;

    let all = staff.get("/api/staff/students").await;
    assert_eq!(all.body["total"], 2, "staff and admins are not listed");
    assert_eq!(all.body["items"][0]["last_name"], "Hopper");
    assert!(all.body["items"][0].get("role").is_none());

    assert_eq!(
        staff.get("/api/staff/students?q=ada").await.body["total"],
        1
    );
    assert_eq!(
        staff.get("/api/staff/students?promo=2026").await.body["total"],
        1
    );
    assert_eq!(
        staff.get("/api/staff/students?q=turing").await.body["total"],
        0
    );
    assert_eq!(
        staff.get("/api/staff/promos").await.body,
        json!(["2026", "2027"])
    );

    // Emails from a CSV chosen in the browser.
    let resolved = staff
        .post(
            "/api/staff/students/resolve",
            json!({ "emails": ["ADA@epitech.eu", "ghost@epitech.eu", "alan@epitech.eu", " "] }),
        )
        .await;
    assert_eq!(resolved.body["found"].as_array().unwrap().len(), 1);
    assert_eq!(resolved.body["found"][0]["email"], "ada@epitech.eu");
    assert_eq!(
        resolved.body["missing"],
        json!(["ghost@epitech.eu", "alan@epitech.eu"])
    );

    // Students cannot browse the directory.
    let (mut student, _) = app.login_as(Role::Student).await;
    assert_eq!(
        student.get("/api/staff/students").await.status,
        StatusCode::FORBIDDEN
    );
}

// --- Reliability -----------------------------------------------------------------------

/// Inserts a signed transaction as if the API had crashed right after recording it.
async fn record_unsent(app: &TestApp, tx: &Transaction) {
    sqlx::query(
        "INSERT INTO chain_txs (hash, kind, signer_address, nonce, raw, status, created_at) \
         VALUES ($1, 'authority_update', $2, $3, $4, 'pending', now() - interval '5 minutes')",
    )
    .bind(tx.hash().to_string())
    .bind(tx.from.to_string())
    .bind(tx.nonce as i64)
    .bind(tx.to_bytes())
    .execute(&app.db)
    .await
    .unwrap();
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn reconciliation_delivers_transactions_recorded_but_never_sent(db: PgPool) {
    let app = TestApp::start(db).await;
    let staff = app.seed(Role::Student).await;
    let grant = |nonce| {
        Transaction::sign(
            &app.genesis.chain_id,
            &app.admin_key,
            nonce,
            TxPayload::AuthorityUpdate {
                authority: staff.address,
                action: epicoin_core::AuthorityAction::Grant { quota: 4_200 },
            },
        )
    };
    let delivered = grant(0);
    // Same signer, a nonce that can never be valid: the chain will refuse it.
    let hopeless = grant(7);
    record_unsent(&app, &delivered).await;
    record_unsent(&app, &hopeless).await;

    // First pass: delivered to the node (pending there) and the hopeless one refused.
    app.state.chain.reconcile(Duration::ZERO).await.unwrap();
    app.eventually("the grant on chain", || {
        app.node.account(&staff.address).authority.is_some()
    })
    .await;
    app.state.chain.reconcile(Duration::ZERO).await.unwrap();

    let status = |hash: String| {
        let db = app.db.clone();
        async move {
            sqlx::query_as::<_, (String, Option<String>)>(
                "SELECT status, error FROM chain_txs WHERE hash = $1",
            )
            .bind(hash)
            .fetch_one(&db)
            .await
            .unwrap()
        }
    };
    let (delivered_status, _) = status(delivered.hash().to_string()).await;
    assert_eq!(delivered_status, "included");
    let (hopeless_status, error) = status(hopeless.hash().to_string()).await;
    assert_eq!(hopeless_status, "failed");
    assert!(error.unwrap().contains("bad_nonce"));

    // Running it again changes nothing: the grant was applied exactly once.
    assert_eq!(app.state.chain.reconcile(Duration::ZERO).await.unwrap(), 0);
    assert_eq!(app.node.account(&app.admin_key.address()).nonce, 1);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn reconciliation_leaves_recent_transactions_alone(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let student = app.seed(Role::Student).await;
    staff
        .post("/api/staff/distributions", distribution(&[&student], 100))
        .await;
    // Everything is settled; nothing older than the threshold is pending.
    assert_eq!(
        app.state
            .chain
            .reconcile(Duration::from_secs(30))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        count(
            &app,
            "SELECT count(*) FROM chain_txs WHERE status = 'pending'"
        )
        .await,
        0
    );
}

// --- Statistics and explorer -----------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn admin_dashboard_adds_up(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let (mut staff_a, a) = app.login_as(Role::Staff).await;
    let (mut staff_b, _) = app.login_as(Role::Staff).await;
    let s1 = app.seed(Role::Student).await;
    let s2 = app.seed(Role::Student).await;

    staff_a
        .post("/api/staff/distributions", distribution(&[&s1, &s2], 1_000))
        .await;
    staff_b
        .post(
            "/api/staff/distributions",
            json!({ "recipient_ids": [s1.id], "amount": 250, "reason_code": "event" }),
        )
        .await;

    let stats = admin.get("/api/admin/stats").await;
    assert_eq!(stats.status, StatusCode::OK, "{:?}", stats.body);
    assert_eq!(stats.body["supply"]["total_minted"], 2_250);
    assert_eq!(stats.body["supply"]["circulating"], 2_250);
    assert_eq!(stats.body["supply"]["treasury_balance"], 0);
    assert_eq!(stats.body["supply"]["held_by_users"], 2_250);

    let by_staff = stats.body["minted_by_staff"].as_array().unwrap();
    assert_eq!(by_staff.len(), 2);
    assert_eq!(by_staff[0]["user_id"], a.id.to_string());
    assert_eq!(by_staff[0]["total"], 2_000);
    assert_eq!(by_staff[0]["count"], 2);
    assert_eq!(by_staff[1]["total"], 250);

    let by_reason = stats.body["minted_by_reason"].as_array().unwrap();
    assert_eq!(by_reason[0]["reason_code"], "peer_help");
    assert_eq!(by_reason[0]["total"], 2_000);
    assert_eq!(by_reason[1]["reason_code"], "event");

    let students = stats.body["users"]
        .as_array()
        .unwrap()
        .iter()
        .find(|u| u["role"] == "student")
        .unwrap();
    assert_eq!(students["total"], 2);
    assert_eq!(students["activated"], 2);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn explorer_shows_blocks_transactions_and_labelled_accounts(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let student = app.seed(Role::Student).await;
    let minted = staff
        .post("/api/staff/distributions", distribution(&[&student], 900))
        .await;
    let tx_hash = minted.body["results"][0]["tx_hash"].as_str().unwrap();

    let status = admin.get("/api/explorer/status").await;
    assert_eq!(status.body["chain_id"], "epicoin-apitest");
    let height = status.body["height"].as_u64().unwrap();
    assert!(height >= 2);

    let blocks = admin.get("/api/explorer/blocks?limit=5").await;
    assert_eq!(blocks.body[0]["height"], height);
    let block = admin.get(&format!("/api/explorer/blocks/{height}")).await;
    assert_eq!(block.status, StatusCode::OK);
    assert!(!block.body["txs"].as_array().unwrap().is_empty());
    let by_hash = admin
        .get(&format!(
            "/api/explorer/blocks/{}",
            block.body["hash"].as_str().unwrap()
        ))
        .await;
    assert_eq!(by_hash.body["height"], height);

    let tx = admin.get(&format!("/api/explorer/tx/{tx_hash}")).await;
    assert_eq!(tx.body["status"], "included");
    assert_eq!(tx.body["tx"]["payload"]["kind"], "mint");
    assert_eq!(tx.body["tx"]["payload"]["amount"], 900);
    assert_eq!(
        admin.get("/api/explorer/txs").await.body[0]["hash"],
        tx_hash
    );

    let accounts = admin.get("/api/explorer/accounts").await;
    let richest = &accounts.body["items"][0];
    assert_eq!(richest["address"], student.address.to_string());
    assert_eq!(richest["balance"], 900);
    assert!(richest["label"].as_str().unwrap().starts_with("Prenom"));

    let account = admin
        .get(&format!("/api/explorer/accounts/{}", student.address))
        .await;
    assert_eq!(account.body["balance"], 900);
    assert_eq!(account.body["transactions"][0]["hash"], tx_hash);
    let treasury = admin
        .get(&format!(
            "/api/explorer/accounts/{}",
            app.treasury_key.address()
        ))
        .await;
    assert_eq!(treasury.body["label"], "Trésorerie de la boutique");

    for missing in [
        "/api/explorer/blocks/999999",
        "/api/explorer/blocks/not-a-block",
        "/api/explorer/tx/not-a-hash",
        "/api/explorer/accounts/not-an-address",
    ] {
        assert_eq!(
            admin.get(missing).await.status,
            StatusCode::NOT_FOUND,
            "{missing}"
        );
    }
}
