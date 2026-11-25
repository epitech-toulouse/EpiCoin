//! Knowledge shares: QR codes paid once on the staff member's quota, streak bonuses, daily
//! limit, cancellation, badges, ranking, settings, who may do what, and privacy.

mod common;

use axum::http::{Method, StatusCode};
use common::*;
use epicoin_api::models::Role;
use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

const TOPIC: &str = "Pourquoi le ciel est bleu";

/// 10 EPC, with a topic.
fn share(amount: u64) -> Value {
    json!({ "amount": amount, "topic": TOPIC })
}

/// Generates a QR code; returns the share id and the secret in its link.
async fn new_qr(staff: &mut Client, body: Value) -> (Uuid, String) {
    let reply = staff.post("/api/staff/shares", body).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.body);
    let link = reply.body["link"].as_str().unwrap();
    assert!(
        link.starts_with("http://localhost:8088/partage#token="),
        "{link}"
    );
    let token = link.split_once("#token=").unwrap().1.to_owned();
    (reply.body["id"].as_str().unwrap().parse().unwrap(), token)
}

async fn scan(student: &mut Client, token: &str) -> Reply {
    student
        .post("/api/shares/claim", json!({ "token": token }))
        .await
}

async fn status(client: &mut Client, id: Uuid) -> Value {
    let reply = client.get(&format!("/api/staff/shares/{id}")).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    reply.body
}

/// Settings where every day is a working day, so that tests do not depend on the date.
fn every_day(tiers: Value, daily_limit: Value) -> Value {
    json!({
        "standard": 1_000,
        "max_amount": 10_000,
        "qr_seconds": 120,
        "time_zone": "Europe/Paris",
        "weekdays": [1, 2, 3, 4, 5, 6, 7],
        "closures": [],
        "tiers": tiers,
        "daily_limit": daily_limit,
    })
}

async fn put(client: &mut Client, settings: Value) -> Reply {
    client
        .send(
            Method::PUT,
            "/api/admin/shares/settings",
            Some(settings),
            &[],
        )
        .await
}

async fn configure(app: &TestApp, settings: Value) {
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let reply = put(&mut admin, settings).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
}

/// Moves a scanned share `days` days into the past, as if it had been scanned then.
async fn backdate(app: &TestApp, id: Uuid, days: i32) {
    sqlx::query(
        "UPDATE shares SET claim_day = claim_day - $2, \
           claimed_at = claimed_at - make_interval(days => $2) WHERE id = $1",
    )
    .bind(id)
    .bind(days)
    .execute(&app.db)
    .await
    .unwrap();
}

async fn mints(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM chain_txs WHERE kind = 'mint'")
        .fetch_one(&app.db)
        .await
        .unwrap()
}

// --- Scanning --------------------------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn a_scanned_qr_code_pays_the_student_once_on_the_staff_quota(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, staff_user) = app.login_as(Role::Staff).await;
    let (mut student, me) = app.login_as(Role::Student).await;
    let (id, token) = new_qr(
        &mut staff,
        json!({ "amount": 1_500, "mention": "genius", "topic": TOPIC }),
    )
    .await;
    assert_eq!(status(&mut staff, id).await["status"], "open");

    // The student sees what they are about to get.
    let preview = student
        .post("/api/shares/preview", json!({ "token": token }))
        .await;
    assert_eq!(preview.status, StatusCode::OK, "{:?}", preview.body);
    assert_eq!(preview.body["amount"], 1_500);
    assert_eq!(preview.body["mention"], "genius");
    assert_eq!(preview.body["topic"], TOPIC);
    assert!(preview.body["staff_name"]
        .as_str()
        .unwrap()
        .starts_with("Prenom"));
    assert!(preview.body["expires_in"].as_u64().unwrap() > 100);
    assert_eq!(mints(&app).await, 0, "a preview pays nothing");

    let paid = scan(&mut student, &token).await;
    assert_eq!(paid.status, StatusCode::OK, "{:?}", paid.body);
    assert_eq!(paid.body["amount"], 1_500);
    assert_eq!(paid.body["bonus"], 0);
    assert_eq!(paid.body["reward_status"], "included");
    assert_eq!(app.balance(&me.address), 1_500);

    // Once only, for them and for anyone else.
    let again = scan(&mut student, &token).await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.error(), "share_already_yours");
    let (mut other, _) = app.login_as(Role::Student).await;
    assert_eq!(scan(&mut other, &token).await.error(), "share_used");
    assert_eq!(app.balance(&me.address), 1_500);
    assert_eq!(mints(&app).await, 1);

    // Paid by the staff member, on their quota, readable on both sides.
    assert_eq!(
        staff.get("/api/staff/quota").await.body["remaining"],
        48_500
    );
    let seen = status(&mut staff, id).await;
    assert_eq!(seen["status"], "claimed");
    assert_eq!(seen["student_id"], me.id.to_string());
    assert_eq!(seen["staff_id"], staff_user.id.to_string());
    let line = &student.get("/api/me/transactions").await.body["items"][0];
    assert_eq!(line["reason_code"], "knowledge_share");
    assert_eq!(line["comment"], format!("Partage : {TOPIC}"));
    let history = staff.get("/api/staff/shares").await;
    assert_eq!(history.body["total"], 1);
    assert_eq!(history.body["items"][0]["id"], id.to_string());
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn two_students_scanning_at_once_are_not_both_paid(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let (mut first, a) = app.login_as(Role::Student).await;
    let (mut second, b) = app.login_as(Role::Student).await;
    let (_, token) = new_qr(&mut staff, share(1_000)).await;

    let (one, two) = tokio::join!(scan(&mut first, &token), scan(&mut second, &token));
    let statuses = [one.status, two.status];
    assert_eq!(
        statuses.iter().filter(|s| **s == StatusCode::OK).count(),
        1,
        "{:?} / {:?}",
        one.body,
        two.body
    );
    assert_eq!(app.balance(&a.address) + app.balance(&b.address), 1_000);
    assert_eq!(mints(&app).await, 1);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn a_new_qr_code_replaces_the_previous_one_and_can_be_withdrawn(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let (mut student, me) = app.login_as(Role::Student).await;
    let (old_id, old) = new_qr(&mut staff, share(1_000)).await;
    let (new_id, new) = new_qr(&mut staff, share(2_000)).await;

    assert_eq!(status(&mut staff, old_id).await["status"], "invalidated");
    assert_eq!(scan(&mut student, &old).await.error(), "share_invalidated");

    let withdrawn = staff
        .post(&format!("/api/staff/shares/{new_id}/invalidate"), json!({}))
        .await;
    assert_eq!(withdrawn.status, StatusCode::NO_CONTENT);
    assert_eq!(scan(&mut student, &new).await.error(), "share_invalidated");
    assert_eq!(app.balance(&me.address), 0);

    // A scanned QR code cannot be withdrawn.
    let (id, token) = new_qr(&mut staff, share(1_000)).await;
    assert_eq!(scan(&mut student, &token).await.status, StatusCode::OK);
    let late = staff
        .post(&format!("/api/staff/shares/{id}/invalidate"), json!({}))
        .await;
    assert_eq!(late.error(), "share_used");
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn expired_or_unknown_qr_codes_pay_nothing(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let (mut student, me) = app.login_as(Role::Student).await;
    let (id, token) = new_qr(&mut staff, share(1_000)).await;
    sqlx::query("UPDATE shares SET expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(id)
        .execute(&app.db)
        .await
        .unwrap();

    assert_eq!(scan(&mut student, &token).await.error(), "share_expired");
    assert_eq!(status(&mut staff, id).await["status"], "expired");
    let unknown = scan(&mut student, &"ab".repeat(32)).await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
    assert_eq!(unknown.error(), "share_invalid");
    assert_eq!(
        scan(&mut student, "not-a-token").await.error(),
        "share_invalid"
    );
    assert_eq!(app.balance(&me.address), 0);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn amount_mention_and_topic_are_checked(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    for (body, code) in [
        (json!({ "amount": 0 }), "invalid_amount"),
        (json!({ "amount": 10_001 }), "invalid_amount"),
        (
            json!({ "amount": 100, "mention": "boring" }),
            "unknown_mention",
        ),
        (
            json!({ "amount": 100, "topic": "x".repeat(301) }),
            "topic_too_long",
        ),
        (
            json!({ "amount": 100, "mention": "legendary" }),
            "mention_reserved",
        ),
    ] {
        let reply = staff.post("/api/staff/shares", body.clone()).await;
        assert_eq!(reply.error(), code, "{body}");
    }
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT count(*) FROM shares")
            .fetch_one(&app.db)
            .await
            .unwrap(),
        0
    );
    // "Legendary" is for admins.
    new_qr(&mut admin, json!({ "amount": 100, "mention": "legendary" })).await;
}

// --- Streaks, quota and daily limit ----------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn completing_a_streak_tier_pays_its_bonus_once_in_the_same_mint(db: PgPool) {
    let app = TestApp::start(db).await;
    configure(
        &app,
        every_day(json!([{ "days": 3, "bonus": 500 }]), Value::Null),
    )
    .await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let (mut student, me) = app.login_as(Role::Student).await;

    // Two days ago, then yesterday.
    for days_ago in [2, 1] {
        let (id, token) = new_qr(&mut staff, share(1_000)).await;
        let paid = scan(&mut student, &token).await;
        assert_eq!(paid.body["bonus"], 0, "{:?}", paid.body);
        backdate(&app, id, days_ago).await;
    }
    let before = student.get("/api/shares/me").await.body;
    assert_eq!(before["progress"]["current"], 2);
    assert_eq!(before["progress"]["next"]["days"], 3);
    assert_eq!(before["progress"]["today_counted"], false);

    // Today: the third day in a row.
    let (id, token) = new_qr(&mut staff, share(1_000)).await;
    let paid = scan(&mut student, &token).await;
    assert_eq!(paid.status, StatusCode::OK, "{:?}", paid.body);
    assert_eq!(paid.body["amount"], 1_000);
    assert_eq!(paid.body["bonus"], 500);
    assert_eq!(paid.body["streak_days"], 3);
    assert_eq!(app.balance(&me.address), 3_500);
    assert_eq!(
        mints(&app).await,
        3,
        "the bonus travels in the share's mint"
    );
    assert_eq!(status(&mut staff, id).await["bonus"], 500);
    assert_eq!(
        staff.get("/api/staff/quota").await.body["remaining"],
        50_000 - 3_500
    );
    let line = &student.get("/api/me/transactions").await.body["items"][0];
    assert_eq!(line["amount"], 1_500);
    assert_eq!(
        line["comment"],
        format!("Partage : {TOPIC} · bonus de régularité (3 jours)")
    );

    // A second share the same day does not pay it again.
    let (_, token) = new_qr(&mut staff, share(1_000)).await;
    assert_eq!(scan(&mut student, &token).await.body["bonus"], 0);
    let after = student.get("/api/shares/me").await.body;
    assert_eq!(after["progress"]["current"], 3);
    assert_eq!(after["progress"]["best"], 3);
    assert_eq!(after["progress"]["today_counted"], true);
    assert_eq!(after["total"], 4_500);
    assert_eq!(after["shares"], 4);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn one_bonus_even_when_two_qr_codes_are_scanned_at_once(db: PgPool) {
    let app = TestApp::start(db).await;
    // The first share of a streak completes a one-day tier.
    configure(
        &app,
        every_day(json!([{ "days": 1, "bonus": 100 }]), Value::Null),
    )
    .await;
    let (mut alice, _) = app.login_as(Role::Staff).await;
    let (mut bob, _) = app.login_as(Role::Staff).await;
    let (_, first) = new_qr(&mut alice, share(1_000)).await;
    let (_, second) = new_qr(&mut bob, share(1_000)).await;
    let (mut phone, me) = app.login_as(Role::Student).await;
    let mut laptop = app.client();
    laptop.login(&me.email, PASSWORD).await;

    let (one, two) = tokio::join!(scan(&mut phone, &first), scan(&mut laptop, &second));
    assert_eq!(one.status, StatusCode::OK, "{:?}", one.body);
    assert_eq!(two.status, StatusCode::OK, "{:?}", two.body);
    let bonuses = one.body["bonus"].as_u64().unwrap() + two.body["bonus"].as_u64().unwrap();
    assert_eq!(bonuses, 100);
    assert_eq!(app.balance(&me.address), 2_100);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn a_scan_the_staff_quota_cannot_pay_bonus_included_writes_nothing(db: PgPool) {
    let app = TestApp::start(db).await;
    // The default quota is 500 EPC; this bonus alone is 1 000 EPC.
    configure(
        &app,
        every_day(json!([{ "days": 1, "bonus": 100_000 }]), Value::Null),
    )
    .await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let (mut student, me) = app.login_as(Role::Student).await;
    let (id, token) = new_qr(&mut staff, share(1_000)).await;

    let refused = scan(&mut student, &token).await;
    assert_eq!(refused.status, StatusCode::CONFLICT);
    assert_eq!(refused.error(), "share_unfunded");
    assert_eq!(app.balance(&me.address), 0);
    assert_eq!(mints(&app).await, 0);
    assert_eq!(status(&mut staff, id).await["status"], "open");
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn the_daily_limit_holds(db: PgPool) {
    let app = TestApp::start(db).await;
    configure(&app, every_day(json!([]), json!(1))).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let (mut student, me) = app.login_as(Role::Student).await;
    let (_, token) = new_qr(&mut staff, share(1_000)).await;
    assert_eq!(scan(&mut student, &token).await.status, StatusCode::OK);
    let (_, token) = new_qr(&mut staff, share(1_000)).await;
    let refused = scan(&mut student, &token).await;
    assert_eq!(refused.error(), "daily_limit");
    assert_eq!(app.balance(&me.address), 1_000);
}

// --- Cancellation, badges and ranking --------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn a_cancelled_share_leaves_badges_and_ranking_but_its_epc_stay(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let (mut student, me) = app.login_as(Role::Student).await;
    let (id, token) = new_qr(&mut staff, share(1_000)).await;
    assert_eq!(scan(&mut student, &token).await.status, StatusCode::OK);

    let mine = student.get("/api/shares/me").await.body;
    assert_eq!(mine["badges"][0]["code"], "first");
    assert_eq!(mine["rank"], 1);
    let ranking = student.get("/api/shares/ranking").await.body;
    assert_eq!(ranking["items"][0]["me"], true);
    assert_eq!(ranking["items"][0]["total"], 1_000);
    assert_eq!(ranking["me"]["rank"], 1);

    // Only admins cancel, with a reason.
    let path = format!("/api/admin/shares/{id}/cancel");
    let forbidden = staff.post(&path, json!({ "reason": "Doublon" })).await;
    assert_eq!(forbidden.status, StatusCode::FORBIDDEN);
    assert_eq!(
        admin.post(&path, json!({ "reason": "Non" })).await.error(),
        "invalid_reason"
    );
    let cancelled = admin
        .post(&path, json!({ "reason": "Partage déjà compté" }))
        .await;
    assert_eq!(cancelled.status, StatusCode::NO_CONTENT);
    assert_eq!(
        admin
            .post(&path, json!({ "reason": "Partage déjà compté" }))
            .await
            .error(),
        "already_cancelled"
    );

    assert_eq!(app.balance(&me.address), 1_000, "the chain keeps the EPC");
    let mine = student.get("/api/shares/me").await.body;
    assert_eq!(mine["badges"], json!([]));
    assert_eq!(mine["rank"], Value::Null);
    assert_eq!(mine["total"], 0);
    assert_eq!(mine["recent"][0]["cancel_reason"], "Partage déjà compté");
    assert!(mine["recent"][0]["cancelled_at"].is_string());
    assert_eq!(
        student.get("/api/shares/ranking").await.body["items"],
        json!([])
    );
    assert_eq!(status(&mut staff, id).await["status"], "cancelled");

    // An unscanned share cannot be cancelled.
    let (open, _) = new_qr(&mut staff, share(1_000)).await;
    let refused = admin
        .post(
            &format!("/api/admin/shares/{open}/cancel"),
            json!({ "reason": "Erreur de saisie" }),
        )
        .await;
    assert_eq!(refused.error(), "share_not_claimed");
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn the_ranking_orders_students_and_respects_who_left_it(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let (mut top, top_user) = app.login_as(Role::Student).await;
    let (mut second, _) = app.login_as(Role::Student).await;
    for (client, amount) in [(&mut top, 3_000), (&mut second, 1_000)] {
        let (_, token) = new_qr(&mut staff, share(amount)).await;
        assert_eq!(scan(client, &token).await.status, StatusCode::OK);
    }

    let ranking = second.get("/api/shares/ranking").await.body;
    let items = ranking["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["rank"], 1);
    assert_eq!(items[0]["total"], 3_000);
    assert_eq!(items[0]["promo"], "pge1");
    assert_eq!(items[1]["me"], true);
    // "First name L." only; ids for admins only.
    let name = items[0]["name"].as_str().unwrap();
    assert!(
        name.starts_with("Prenom") && name.ends_with(" N."),
        "{name}"
    );
    assert_eq!(items[0]["user_id"], Value::Null);
    let seen_by_admin = admin.get("/api/shares/ranking").await.body;
    assert_eq!(
        seen_by_admin["items"][0]["user_id"],
        top_user.id.to_string()
    );
    assert_eq!(seen_by_admin["me"], Value::Null);

    // Leaving the ranking.
    let left = top
        .patch("/api/shares/me", json!({ "hidden_from_ranking": true }))
        .await;
    assert_eq!(left.status, StatusCode::OK);
    let ranking = second.get("/api/shares/ranking").await.body;
    assert_eq!(ranking["items"].as_array().unwrap().len(), 1);
    assert_eq!(ranking["items"][0]["rank"], 1);
    let mine = top.get("/api/shares/me").await.body;
    assert_eq!(mine["hidden_from_ranking"], true);
    assert_eq!(mine["rank"], Value::Null);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn admins_give_the_legend_badge_to_students(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let (mut staff, staff_user) = app.login_as(Role::Staff).await;
    let (mut student, me) = app.login_as(Role::Student).await;
    let path = format!("/api/admin/shares/legends/{}", me.id);

    let forbidden = staff.send(Method::PUT, &path, None, &[]).await;
    assert_eq!(forbidden.status, StatusCode::FORBIDDEN);
    let not_student = admin
        .send(
            Method::PUT,
            &format!("/api/admin/shares/legends/{}", staff_user.id),
            None,
            &[],
        )
        .await;
    assert_eq!(not_student.error(), "not_a_student");

    let given = admin.send(Method::PUT, &path, None, &[]).await;
    assert_eq!(given.status, StatusCode::NO_CONTENT);
    let badges = student.get("/api/shares/me").await.body["badges"].clone();
    assert_eq!(badges[0]["code"], "legend");

    assert_eq!(admin.delete(&path).await.status, StatusCode::NO_CONTENT);
    assert_eq!(
        student.get("/api/shares/me").await.body["badges"],
        json!([])
    );
}

// --- Settings and permissions ----------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn settings_are_read_by_all_and_changed_by_admins_only(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let (mut student, _) = app.login_as(Role::Student).await;

    let defaults = student.get("/api/shares/settings").await.body;
    assert_eq!(defaults["standard"], 1_000);
    assert_eq!(defaults["weekdays"], json!([1, 2, 3, 4, 5]));
    assert_eq!(defaults["tiers"][0], json!({ "days": 5, "bonus": 2_000 }));

    let mut wanted = every_day(json!([{ "days": 2, "bonus": 300 }]), json!(3));
    wanted["closures"] = json!(["2026-12-25"]);
    assert_eq!(
        put(&mut staff, wanted.clone()).await.status,
        StatusCode::FORBIDDEN
    );
    let mut bad_zone = wanted.clone();
    bad_zone["time_zone"] = json!("Mars/Olympus");
    assert_eq!(put(&mut admin, bad_zone).await.error(), "invalid_settings");
    let mut bad_tiers = wanted.clone();
    bad_tiers["tiers"] = json!([{ "days": 2, "bonus": 1 }, { "days": 2, "bonus": 2 }]);
    assert_eq!(put(&mut admin, bad_tiers).await.error(), "invalid_settings");

    let saved = put(&mut admin, wanted.clone()).await;
    assert_eq!(saved.status, StatusCode::OK, "{:?}", saved.body);
    let read = staff.get("/api/shares/settings").await.body;
    assert_eq!(read, wanted);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn each_role_only_reaches_its_side(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut alice, _) = app.login_as(Role::Staff).await;
    let (mut bob, _) = app.login_as(Role::Staff).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let (mut student, _) = app.login_as(Role::Student).await;
    let (mut manager, _) = app.login_as(Role::ShopManager).await;
    let (id, token) = new_qr(&mut alice, share(1_000)).await;

    for client in [&mut student, &mut manager] {
        let reply = client.post("/api/staff/shares", share(1_000)).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN);
    }
    for client in [&mut alice, &mut admin, &mut manager] {
        assert_eq!(scan(client, &token).await.status, StatusCode::FORBIDDEN);
        let preview = client
            .post("/api/shares/preview", json!({ "token": token }))
            .await;
        assert_eq!(preview.status, StatusCode::FORBIDDEN);
    }
    // Another staff member does not see Alice's QR code; an admin does.
    let hidden = bob.get(&format!("/api/staff/shares/{id}")).await;
    assert_eq!(hidden.status, StatusCode::NOT_FOUND);
    assert_eq!(status(&mut admin, id).await["status"], "open");
    let withdraw = bob
        .post(&format!("/api/staff/shares/{id}/invalidate"), json!({}))
        .await;
    assert_eq!(withdraw.status, StatusCode::NOT_FOUND);
    // Everyone's history is for admins.
    assert_eq!(
        bob.get("/api/staff/shares?scope=all").await.status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(scan(&mut student, &token).await.status, StatusCode::OK);
    let all = admin.get("/api/staff/shares?scope=all").await;
    assert_eq!(all.body["total"], 1);
    assert_eq!(bob.get("/api/staff/shares").await.body["total"], 0);
    // A student's progress is theirs.
    assert_eq!(
        alice.get("/api/shares/me").await.status,
        StatusCode::FORBIDDEN
    );
}

// --- Privacy ---------------------------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn topics_never_reach_the_chain_and_leave_with_the_student(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let (mut student, me) = app.login_as(Role::Student).await;
    let (id, token) = new_qr(
        &mut staff,
        json!({ "amount": 1_000, "mention": "wtf", "topic": TOPIC }),
    )
    .await;
    assert_eq!(scan(&mut student, &token).await.status, StatusCode::OK);
    let cancel = admin
        .post(
            &format!("/api/admin/shares/{id}/cancel"),
            json!({ "reason": "Raison très personnelle" }),
        )
        .await;
    assert_eq!(cancel.status, StatusCode::NO_CONTENT);

    let mut chain_dump = String::new();
    for block in app.node.blocks(1, 1_000).unwrap() {
        chain_dump.push_str(&String::from_utf8_lossy(&block.to_bytes()));
        chain_dump.push_str(&serde_json::to_string(&block).unwrap());
    }
    assert!(chain_dump.contains("knowledge_share"));
    for private in [TOPIC, "wtf", "Raison", "Prenom", &me.id.to_string()] {
        assert!(
            !chain_dump.contains(private),
            "the chain contains {private:?}"
        );
    }

    // Erasing the student erases what they said.
    let deleted = admin.delete(&format!("/api/admin/users/{}", me.id)).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    let (topic, reason, comment): (String, String, String) = sqlx::query_as(
        "SELECT s.topic, s.cancel_reason, b.comment FROM shares s \
         JOIN chain_txs c ON c.hash = s.tx_hash JOIN mint_batches b ON b.id = c.batch_id \
         WHERE s.id = $1",
    )
    .bind(id)
    .fetch_one(&app.db)
    .await
    .unwrap();
    assert_eq!(topic, "");
    assert_eq!(reason, "");
    assert_eq!(comment, "Partage de connaissance");
}
