//! The "!" of the menu: news since the last visit for students, to-dos for the others.

mod common;

use axum::http::{Method, StatusCode};
use common::*;
use epicoin_api::models::Role;
use serde_json::{json, Value};
use sqlx::PgPool;

async fn counts(client: &mut Client) -> Value {
    let reply = client.get("/api/me/notifications").await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    reply.body
}

async fn seen(client: &mut Client, section: &str) {
    let reply = client
        .post("/api/me/notifications/seen", json!({ "section": section }))
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT, "{:?}", reply.body);
}

async fn distribute(staff: &mut Client, student: &Seeded, amount: u64) {
    let reply = staff
        .post(
            "/api/staff/distributions",
            json!({ "recipient_ids": [student.id], "amount": amount, "reason_code": "event" }),
        )
        .await;
    assert_eq!(reply.body["status"], "included", "{:?}", reply.body);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn each_role_gets_the_sections_of_its_menu(db: PgPool) {
    let app = TestApp::start(db).await;
    let expected = [
        (
            Role::Student,
            json!({ "balance": 0, "orders": 0, "challenges": 0 }),
        ),
        (Role::Staff, json!({ "challenge_reviews": 0 })),
        (Role::ShopManager, json!({ "shop_orders": 0 })),
        (
            Role::Admin,
            json!({ "shop_orders": 0, "challenge_reviews": 0 }),
        ),
    ];
    for (role, sections) in expected {
        let (mut client, _) = app.login_as(role).await;
        assert_eq!(counts(&mut client).await, sections, "{role:?}");
    }
    // Only students have pages to mark as seen.
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let reply = staff
        .post(
            "/api/me/notifications/seen",
            json!({ "section": "balance" }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn epc_received_are_news_until_the_balance_page_is_opened(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let (mut ada, ada_user) = app.login_as(Role::Student).await;
    let (mut grace, grace_user) = app.login_as(Role::Student).await;

    distribute(&mut staff, &ada_user, 500).await;
    assert_eq!(counts(&mut ada).await["balance"], 1);
    seen(&mut ada, "balance").await;
    assert_eq!(counts(&mut ada).await["balance"], 0);
    distribute(&mut staff, &ada_user, 200).await;
    assert_eq!(counts(&mut ada).await["balance"], 1);
    seen(&mut ada, "balance").await;

    // Sending is not news for the sender; receiving is, for the recipient.
    let sent = ada
        .send(
            Method::POST,
            "/api/transfers",
            Some(json!({ "recipient_id": grace_user.id, "amount": 100 })),
            &[("idempotency-key", "t1")],
        )
        .await;
    assert_eq!(sent.status, StatusCode::CREATED, "{:?}", sent.body);
    assert_eq!(counts(&mut ada).await["balance"], 0);
    assert_eq!(counts(&mut grace).await["balance"], 1);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn orders_to_prepare_then_ready_to_collect(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut manager, _) = app.login_as(Role::ShopManager).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let (mut student, student_user) = app.login_as(Role::Student).await;
    distribute(&mut staff, &student_user, 5_000).await;
    let product = manager
        .post(
            "/api/shop/manage/products",
            json!({ "name": "Mug", "description": "", "price": 1_500, "stock": 5 }),
        )
        .await;
    let product_id = product.body["id"].as_str().unwrap().to_owned();

    let order = student
        .send(
            Method::POST,
            "/api/shop/orders",
            Some(json!({ "items": [{ "product_id": product_id, "quantity": 1 }] })),
            &[("idempotency-key", "o1")],
        )
        .await;
    assert_eq!(order.status, StatusCode::CREATED, "{:?}", order.body);
    let order_id = order.body["id"].as_str().unwrap().to_owned();

    // A purchase: to prepare for the shop, not news for the buyer.
    assert_eq!(counts(&mut manager).await["shop_orders"], 1);
    assert_eq!(counts(&mut admin).await["shop_orders"], 1);
    assert_eq!(counts(&mut student).await["orders"], 0);

    let ready = manager
        .post(
            &format!("/api/shop/manage/orders/{order_id}/status"),
            json!({ "status": "ready" }),
        )
        .await;
    assert_eq!(ready.status, StatusCode::OK, "{:?}", ready.body);
    assert_eq!(counts(&mut manager).await["shop_orders"], 0);
    assert_eq!(counts(&mut student).await["orders"], 1);
    seen(&mut student, "orders").await;
    assert_eq!(counts(&mut student).await["orders"], 0);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn new_challenges_and_review_decisions(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let (mut other_staff, _) = app.login_as(Role::Staff).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let (mut student, _) = app.login_as(Role::Student).await; // pge1

    // Not for the student's cohort: no news.
    let bachelors = staff
        .post(
            "/api/staff/challenges",
            json!({ "title": "Bachelor", "reward": 100, "mode": "review", "cohorts": ["bach1"] }),
        )
        .await;
    assert_eq!(bachelors.status, StatusCode::CREATED);
    assert_eq!(counts(&mut student).await["challenges"], 0);

    let workshop = staff
        .post(
            "/api/staff/challenges",
            json!({ "title": "Atelier", "reward": 100, "mode": "review" }),
        )
        .await;
    let id = workshop.body["id"].as_str().unwrap().to_owned();
    assert_eq!(counts(&mut student).await["challenges"], 1);
    seen(&mut student, "challenges").await;
    assert_eq!(counts(&mut student).await["challenges"], 0);

    // A participation: to review for its creator and the admins, not for other staff.
    let sent = student
        .post(
            &format!("/api/challenges/{id}/submission"),
            json!({ "proof": "Fait mardi" }),
        )
        .await;
    assert_eq!(sent.status, StatusCode::CREATED);
    assert_eq!(counts(&mut staff).await["challenge_reviews"], 1);
    assert_eq!(counts(&mut admin).await["challenge_reviews"], 1);
    assert_eq!(counts(&mut other_staff).await["challenge_reviews"], 0);
    assert_eq!(counts(&mut student).await["challenges"], 0);

    // The decision is news for the student.
    let list = staff
        .get(&format!("/api/staff/challenges/{id}/submissions"))
        .await;
    let submission = list.body[0]["id"].as_str().unwrap().to_owned();
    staff
        .post(
            &format!("/api/staff/challenges/{id}/submissions/{submission}"),
            json!({ "approve": false, "note": "Il manque une preuve" }),
        )
        .await;
    assert_eq!(counts(&mut staff).await["challenge_reviews"], 0);
    assert_eq!(counts(&mut student).await["challenges"], 1);
    seen(&mut student, "challenges").await;
    assert_eq!(counts(&mut student).await["challenges"], 0);
}
