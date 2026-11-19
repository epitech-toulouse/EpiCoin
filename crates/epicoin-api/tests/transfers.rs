//! Transfers between students: on chain, sent once, with a message that stays off chain.

mod common;

use axum::http::{Method, StatusCode};
use common::*;
use epicoin_api::models::Role;
use serde_json::{json, Value};
use sqlx::PgPool;

/// Gives `student` `amount` units through a staff distribution.
async fn fund(app: &TestApp, student: &Seeded, amount: u64) {
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let reply = staff
        .post(
            "/api/staff/distributions",
            json!({ "recipient_ids": [student.id], "amount": amount, "reason_code": "participation" }),
        )
        .await;
    assert_eq!(reply.body["status"], "included", "{:?}", reply.body);
}

async fn transfer(
    client: &mut Client,
    to: &Seeded,
    amount: u64,
    message: &str,
    key: &str,
) -> Reply {
    client
        .send(
            Method::POST,
            "/api/transfers",
            Some(json!({ "recipient_id": to.id, "amount": amount, "message": message })),
            &[("idempotency-key", key)],
        )
        .await
}

async fn rename(app: &TestApp, user: &Seeded, first: &str, last: &str) {
    sqlx::query("UPDATE users SET first_name = $2, last_name = $3 WHERE id = $1")
        .bind(user.id)
        .bind(first)
        .bind(last)
        .execute(&app.db)
        .await
        .unwrap();
}

async fn count(app: &TestApp, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&app.db).await.unwrap()
}

fn first_line(history: &Reply) -> &Value {
    &history.body["items"][0]
}

// --- Sending ---------------------------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn a_student_sends_epc_to_a_classmate(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut ada, ada_user) = app.login_as(Role::Student).await;
    let (mut grace, grace_user) = app.login_as(Role::Student).await;
    rename(&app, &ada_user, "Ada", "Lovelace").await;
    rename(&app, &grace_user, "Grace", "Hopper").await;
    fund(&app, &ada_user, 1_000).await;

    let reply = transfer(&mut ada, &grace_user, 250, "  Merci pour le café ", "k1").await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.body);
    assert_eq!(reply.body["status"], "included");
    assert_eq!(reply.body["amount"], 250);
    assert_eq!(reply.body["recipient_name"], "Grace Hopper");
    assert_eq!(reply.body["message"], "Merci pour le café");

    // On chain: the EPC moved, none were created.
    assert_eq!(app.balance(&ada_user.address), 750);
    assert_eq!(app.balance(&grace_user.address), 250);
    assert_eq!(app.node.status().total_minted, 1_000);

    // Both histories tell who, how much and why.
    let sent = ada.get("/api/me/transactions").await;
    let line = first_line(&sent);
    assert_eq!(line["kind"], "transfer");
    assert_eq!(line["direction"], "debit");
    assert_eq!(line["amount"], 250);
    assert_eq!(line["to_name"], "Grace Hopper");
    assert_eq!(line["from_name"], Value::Null);
    assert_eq!(line["comment"], "Merci pour le café");
    let received = grace.get("/api/me/transactions").await;
    let line = first_line(&received);
    assert_eq!(line["direction"], "credit");
    assert_eq!(line["from_name"], "Ada Lovelace");
    assert_eq!(line["to_name"], Value::Null);
    assert_eq!(line["comment"], "Merci pour le café");
    assert_eq!(received.body["total"], 1);

    // The message never reaches the chain.
    let raw: Vec<u8> = sqlx::query_scalar("SELECT raw FROM chain_txs WHERE kind = 'transfer'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert!(!raw.windows(5).any(|w| w == b"Merci"));
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn a_retry_with_the_same_key_sends_once(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut ada, ada_user) = app.login_as(Role::Student).await;
    let grace = app.seed(Role::Student).await;
    fund(&app, &ada_user, 1_000).await;

    let first = transfer(&mut ada, &grace, 300, "", "same-key").await;
    let again = transfer(&mut ada, &grace, 300, "", "same-key").await;
    assert_eq!(first.status, StatusCode::CREATED);
    assert_eq!(again.status, StatusCode::CREATED);
    assert_eq!(first.body["tx_hash"], again.body["tx_hash"]);
    assert_eq!(app.balance(&grace.address), 300);
    assert_eq!(count(&app, "SELECT count(*) FROM transfers").await, 1);

    // A new key is a new transfer.
    transfer(&mut ada, &grace, 300, "", "other-key").await;
    assert_eq!(app.balance(&grace.address), 600);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn a_transfer_beyond_the_balance_moves_nothing(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut ada, ada_user) = app.login_as(Role::Student).await;
    let (mut grace, grace_user) = app.login_as(Role::Student).await;
    fund(&app, &ada_user, 100).await;

    let reply = transfer(&mut ada, &grace_user, 101, "", "k1").await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.error(), "transfer_refused");
    assert_eq!(reply.body["details"]["reason"], "insufficient_balance");
    assert_eq!(app.balance(&ada_user.address), 100);
    assert_eq!(app.balance(&grace_user.address), 0);
    assert_eq!(grace.get("/api/me/transactions").await.body["total"], 0);

    // The same attempt gives the same answer; nothing is sent twice.
    let again = transfer(&mut ada, &grace_user, 101, "", "k1").await;
    assert_eq!(again.error(), "transfer_refused");
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn invalid_transfers_are_refused_before_anything_is_signed(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut ada, ada_user) = app.login_as(Role::Student).await;
    let grace = app.seed(Role::Student).await;
    let staff = app.seed(Role::Staff).await;
    let disabled = app.seed(Role::Student).await;
    sqlx::query("UPDATE users SET disabled_at = now() WHERE id = $1")
        .bind(disabled.id)
        .execute(&app.db)
        .await
        .unwrap();
    fund(&app, &ada_user, 1_000).await;

    let cases: [(&Seeded, u64, String, &str); 5] = [
        (&ada_user, 100, String::new(), "self_transfer"),
        (&staff, 100, String::new(), "invalid_recipient"),
        (&disabled, 100, String::new(), "invalid_recipient"),
        (&grace, 0, String::new(), "invalid_amount"),
        (&grace, 100, "x".repeat(141), "message_too_long"),
    ];
    for (index, (to, amount, message, code)) in cases.iter().enumerate() {
        let reply = transfer(&mut ada, to, *amount, message, &format!("k{index}")).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{code}");
        assert_eq!(reply.error(), *code);
    }
    let without_key = ada
        .post(
            "/api/transfers",
            json!({ "recipient_id": grace.id, "amount": 100 }),
        )
        .await;
    assert_eq!(without_key.status, StatusCode::BAD_REQUEST);

    assert_eq!(
        count(
            &app,
            "SELECT count(*) FROM chain_txs WHERE kind = 'transfer'"
        )
        .await,
        0
    );
    assert_eq!(app.balance(&ada_user.address), 1_000);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn only_students_send_and_search(db: PgPool) {
    let app = TestApp::start(db).await;
    let grace = app.seed(Role::Student).await;
    for role in [Role::Staff, Role::Admin, Role::ShopManager] {
        let (mut client, _) = app.login_as(role).await;
        assert_eq!(
            transfer(&mut client, &grace, 100, "", "k").await.status,
            StatusCode::FORBIDDEN,
            "as {role:?}"
        );
        assert_eq!(
            client.get("/api/transfers/recipients?q=nom").await.status,
            StatusCode::FORBIDDEN,
            "as {role:?}"
        );
    }
}

// --- Finding a classmate ---------------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn students_find_classmates_by_name(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut ada, ada_user) = app.login_as(Role::Student).await;
    let grace = app.seed(Role::Student).await;
    let gary = app.seed(Role::Student).await;
    let gone = app.seed(Role::Student).await;
    let staff = app.seed(Role::Staff).await;
    rename(&app, &ada_user, "Ada", "Hopper").await;
    rename(&app, &grace, "Grace", "Hopper").await;
    rename(&app, &gary, "Gary", "Hopperson").await;
    rename(&app, &gone, "Gina", "Hopper").await;
    rename(&app, &staff, "Gus", "Hopper").await;
    sqlx::query("UPDATE users SET disabled_at = now() WHERE id = $1")
        .bind(gone.id)
        .execute(&app.db)
        .await
        .unwrap();

    let found = ada.get("/api/transfers/recipients?q=hopp").await;
    let names: Vec<&str> = found
        .body
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["first_name"].as_str().unwrap())
        .collect();
    // Active students only, never oneself.
    assert_eq!(names, ["Grace", "Gary"]);
    let grace_found = &found.body[0];
    assert_eq!(grace_found["id"], grace.id.to_string());
    assert_eq!(grace_found["email"], grace.email);
    assert_eq!(grace_found["promo"], "pge1");
    assert!(grace_found.get("role").is_none());

    assert_eq!(
        ada.get("/api/transfers/recipients?q=grace%20hopper")
            .await
            .body
            .as_array()
            .unwrap()
            .len(),
        1
    );
    // One letter is not a search: the list of students is not handed out.
    assert_eq!(
        ada.get("/api/transfers/recipients?q=h").await.body,
        json!([])
    );
    assert_eq!(ada.get("/api/transfers/recipients").await.body, json!([]));
}

// --- Administration and erasure --------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn admins_see_every_transfer_with_its_message(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let (mut ada, ada_user) = app.login_as(Role::Student).await;
    let grace = app.seed(Role::Student).await;
    rename(&app, &ada_user, "Ada", "Lovelace").await;
    rename(&app, &grace, "Grace", "Hopper").await;
    fund(&app, &ada_user, 1_000).await;
    transfer(&mut ada, &grace, 400, "Pour le projet", "k1").await;

    let list = admin.get("/api/admin/transfers").await;
    assert_eq!(list.status, StatusCode::OK);
    assert_eq!(list.body["total"], 1);
    let item = &list.body["items"][0];
    assert_eq!(item["sender_name"], "Ada Lovelace");
    assert_eq!(item["recipient_name"], "Grace Hopper");
    assert_eq!(item["amount"], 400);
    assert_eq!(item["message"], "Pour le projet");
    assert_eq!(item["status"], "included");
    assert_eq!(
        ada.get("/api/admin/transfers").await.status,
        StatusCode::FORBIDDEN
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn erasing_a_student_erases_the_messages_of_their_transfers(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let (mut ada, ada_user) = app.login_as(Role::Student).await;
    let grace = app.seed(Role::Student).await;
    fund(&app, &ada_user, 1_000).await;
    transfer(&mut ada, &grace, 400, "Joyeux anniversaire Grace", "k1").await;

    let deleted = admin
        .delete(&format!("/api/admin/users/{}", grace.id))
        .await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let messages: String = sqlx::query_scalar("SELECT string_agg(message, '') FROM transfers")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(messages, "");

    // Ada still sees she sent 4 EPC, to nobody nameable any more.
    let line = first_line(&ada.get("/api/me/transactions").await).clone();
    assert_eq!(line["kind"], "transfer");
    assert_eq!(line["amount"], 400);
    assert_eq!(line["to_name"], Value::Null);
    assert_eq!(line["comment"], Value::Null);
}
