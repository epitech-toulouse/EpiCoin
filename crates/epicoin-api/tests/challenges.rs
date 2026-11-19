//! Challenges: answer and review modes, paying each winner exactly once, cohorts, deadline
//! and winner limit, who manages what.

mod common;

use axum::http::StatusCode;
use common::*;
use epicoin_api::models::Role;
use serde_json::{json, Value};
use sqlx::PgPool;

/// 5 EPC to whoever finds "Rust".
fn riddle() -> Value {
    json!({
        "title": "Énigme du lundi",
        "description": "Quel langage fait tourner la blockchain du campus ?",
        "reward": 500,
        "mode": "answer",
        "answer": "Rust",
    })
}

/// 10 EPC, checked by a staff member.
fn workshop() -> Value {
    json!({
        "title": "Animer un atelier",
        "description": "Animez un atelier pour les premières années.",
        "reward": 1_000,
        "mode": "review",
    })
}

async fn create(client: &mut Client, body: Value) -> String {
    let reply = client.post("/api/staff/challenges", body).await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.body);
    reply.body["id"].as_str().unwrap().to_owned()
}

async fn answer(client: &mut Client, id: &str, answer: &str) -> Reply {
    client
        .post(
            &format!("/api/challenges/{id}/answer"),
            json!({ "answer": answer }),
        )
        .await
}

/// The challenge `id` as the client sees it in their list, if they see it.
async fn listed(client: &mut Client, path: &str, id: &str) -> Option<Value> {
    let reply = client.get(path).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    reply
        .body
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .cloned()
}

async fn mints(app: &TestApp) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM chain_txs WHERE kind = 'mint'")
        .fetch_one(&app.db)
        .await
        .unwrap()
}

// --- Answer mode -----------------------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn a_right_answer_is_paid_at_once_and_only_once(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let (mut student, me) = app.login_as(Role::Student).await;
    let id = create(&mut staff, riddle()).await;

    let seen = listed(&mut student, "/api/challenges", &id).await.unwrap();
    assert_eq!(seen["open"], true);
    assert_eq!(seen["reward"], 500);
    assert_eq!(seen["mode"], "answer");
    assert_eq!(seen["submission"], Value::Null);
    assert!(seen["creator_name"].as_str().unwrap().starts_with("Prenom"));

    let wrong = answer(&mut student, &id, "Python").await;
    assert_eq!(wrong.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(wrong.error(), "wrong_answer");
    assert_eq!(app.balance(&me.address), 0);

    // Case and spaces do not matter.
    let right = answer(&mut student, &id, "  rUST ").await;
    assert_eq!(right.status, StatusCode::OK, "{:?}", right.body);
    assert_eq!(right.body["reward_status"], "included");
    assert_eq!(right.body["amount"], 500);
    assert_eq!(app.balance(&me.address), 500);

    let again = answer(&mut student, &id, "Rust").await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.error(), "already_completed");
    assert_eq!(app.balance(&me.address), 500);
    assert_eq!(mints(&app).await, 1);

    let seen = listed(&mut student, "/api/challenges", &id).await.unwrap();
    assert_eq!(seen["submission"]["status"], "approved");
    assert_eq!(seen["submission"]["reward_status"], "included");
    assert_eq!(seen["winners"], 1);

    // Paid by the creator, on their quota, and readable in the student's history.
    assert_eq!(
        staff.get("/api/staff/quota").await.body["remaining"],
        49_500
    );
    let line = &student.get("/api/me/transactions").await.body["items"][0];
    assert_eq!(line["reason_code"], "challenge");
    assert_eq!(line["comment"], "Défi : Énigme du lundi");
    assert_eq!(line["amount"], 500);
    let mine = staff.get("/api/staff/distributions").await;
    assert_eq!(mine.body["items"][0]["recipient_id"], me.id.to_string());
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn answers_cannot_be_guessed_endlessly(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let (mut student, me) = app.login_as(Role::Student).await;
    let id = create(&mut staff, riddle()).await;

    for guess in ["Go", "C", "Java", "Zig", "OCaml"] {
        assert_eq!(
            answer(&mut student, &id, guess).await.status,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }
    // Even the right answer waits for the end of the window.
    let blocked = answer(&mut student, &id, "Rust").await;
    assert_eq!(blocked.status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(app.balance(&me.address), 0);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn simultaneous_right_answers_pay_once(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let (mut first_tab, me) = app.login_as(Role::Student).await;
    let mut second_tab = app.client();
    assert_eq!(second_tab.login(&me.email, PASSWORD).await, StatusCode::OK);
    let id = create(&mut staff, riddle()).await;

    let (a, b) = tokio::join!(
        answer(&mut first_tab, &id, "Rust"),
        answer(&mut second_tab, &id, "Rust")
    );
    let mut statuses = [a.status, b.status];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::OK, StatusCode::CONFLICT]);
    assert_eq!(app.balance(&me.address), 500);
    assert_eq!(mints(&app).await, 1);
}

// --- Review mode -----------------------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn a_reviewed_participation_is_paid_on_approval(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let (mut student, me) = app.login_as(Role::Student).await;
    let id = create(&mut staff, workshop()).await;
    let submit = |proof: &str| {
        (
            format!("/api/challenges/{id}/submission"),
            json!({ "proof": proof }),
        )
    };

    assert_eq!(answer(&mut student, &id, "?").await.error(), "wrong_mode");
    let (path, body) = submit("  ");
    assert_eq!(student.post(&path, body).await.error(), "invalid_proof");
    let (path, body) = submit("Atelier Git mardi, 12 participants");
    assert_eq!(student.post(&path, body).await.status, StatusCode::CREATED);
    let (path, body) = submit("Encore");
    assert_eq!(student.post(&path, body).await.error(), "already_submitted");

    let managed = listed(&mut staff, "/api/staff/challenges", &id)
        .await
        .unwrap();
    assert_eq!(managed["pending"], 1);
    let list = staff
        .get(&format!("/api/staff/challenges/{id}/submissions"))
        .await;
    let submission = &list.body[0];
    assert_eq!(submission["status"], "pending");
    assert_eq!(submission["proof"], "Atelier Git mardi, 12 participants");
    assert_eq!(submission["student_email"], me.email);
    let decide = format!(
        "/api/staff/challenges/{id}/submissions/{}",
        submission["id"].as_str().unwrap()
    );

    // Rejected with a reason: the student reads it and tries again.
    let rejected = staff
        .post(
            &decide,
            json!({ "approve": false, "note": "Il manque la feuille de présence" }),
        )
        .await;
    assert_eq!(rejected.status, StatusCode::OK, "{:?}", rejected.body);
    assert_eq!(rejected.body["status"], "rejected");
    let seen = listed(&mut student, "/api/challenges", &id).await.unwrap();
    assert_eq!(seen["submission"]["status"], "rejected");
    assert_eq!(
        seen["submission"]["review_note"],
        "Il manque la feuille de présence"
    );
    let (path, body) = submit("Feuille de présence : https://intra.example/atelier-git");
    assert_eq!(student.post(&path, body).await.status, StatusCode::CREATED);

    // Approved: paid at once, by the creator.
    let approved = staff.post(&decide, json!({ "approve": true })).await;
    assert_eq!(approved.status, StatusCode::OK, "{:?}", approved.body);
    assert_eq!(approved.body["status"], "approved");
    assert_eq!(approved.body["reward_status"], "included");
    assert_eq!(app.balance(&me.address), 1_000);

    let twice = staff.post(&decide, json!({ "approve": true })).await;
    assert_eq!(twice.error(), "already_decided");
    assert_eq!(app.balance(&me.address), 1_000);
}

// --- Who takes part, and until when ----------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn challenges_follow_cohorts_deadline_and_winner_limit(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let (mut ada, _) = app.login_as(Role::Student).await; // pge1
    let (mut grace, grace_user) = app.login_as(Role::Student).await;

    // Only for Bachelor 1.
    let mut bachelors = riddle();
    bachelors["cohorts"] = json!(["Bachelor 1"]);
    let bachelors = create(&mut staff, bachelors).await;
    assert!(listed(&mut ada, "/api/challenges", &bachelors)
        .await
        .is_none());
    assert_eq!(
        answer(&mut ada, &bachelors, "Rust").await.status,
        StatusCode::NOT_FOUND
    );
    let managed = listed(&mut staff, "/api/staff/challenges", &bachelors)
        .await
        .unwrap();
    assert_eq!(managed["cohorts"], json!(["bach1"]));

    // A single winner.
    let mut first_only = riddle();
    first_only["max_winners"] = json!(1);
    let first_only = create(&mut staff, first_only).await;
    assert_eq!(
        answer(&mut ada, &first_only, "Rust").await.status,
        StatusCode::OK
    );
    assert!(listed(&mut grace, "/api/challenges", &first_only)
        .await
        .is_none());
    assert_eq!(
        answer(&mut grace, &first_only, "Rust").await.error(),
        "challenge_full"
    );
    assert_eq!(app.balance(&grace_user.address), 0);

    // Closed by hand, or past its deadline.
    let closed = create(&mut staff, riddle()).await;
    let reply = staff
        .patch(
            &format!("/api/staff/challenges/{closed}"),
            json!({ "closed": true }),
        )
        .await;
    assert_eq!(reply.body["closed"], true);
    assert_eq!(reply.body["open"], false);
    assert_eq!(
        answer(&mut grace, &closed, "Rust").await.error(),
        "challenge_closed"
    );
    let mut dated = riddle();
    dated["closes_at"] = json!("2999-01-01T18:00:00+01:00");
    let dated = create(&mut staff, dated).await;
    sqlx::query("UPDATE challenges SET closes_at = now() - interval '1 minute' WHERE id = $1")
        .bind(uuid::Uuid::parse_str(&dated).unwrap())
        .execute(&app.db)
        .await
        .unwrap();
    assert!(listed(&mut grace, "/api/challenges", &dated)
        .await
        .is_none());
    assert_eq!(
        answer(&mut grace, &dated, "Rust").await.error(),
        "challenge_closed"
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn invalid_challenges_are_refused(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    for (field, value) in [
        ("title", json!(" ")),
        ("reward", json!(0)),
        ("answer", json!("   ")),
        ("cohorts", json!(["pge9"])),
        ("max_winners", json!(0)),
        ("closes_at", json!("2020-01-01T00:00:00Z")),
    ] {
        let mut body = riddle();
        body[field] = value;
        let reply = staff.post("/api/staff/challenges", body).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{field}");
        assert_eq!(reply.error(), "invalid_challenge", "{field}");
    }
}

// --- Who manages what ------------------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn only_creators_and_admins_manage_challenges(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut alan, _) = app.login_as(Role::Staff).await;
    let (mut other_staff, _) = app.login_as(Role::Staff).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let id = create(&mut alan, riddle()).await;
    let path = format!("/api/staff/challenges/{id}");

    assert!(listed(&mut other_staff, "/api/staff/challenges", &id)
        .await
        .is_none());
    assert_eq!(
        other_staff
            .patch(&path, json!({ "title": "À moi" }))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        other_staff.get(&format!("{path}/submissions")).await.status,
        StatusCode::NOT_FOUND
    );

    // Admins see and manage every challenge, and create their own.
    assert!(listed(&mut admin, "/api/staff/challenges", &id)
        .await
        .is_some());
    let renamed = admin.patch(&path, json!({ "title": "Énigme" })).await;
    assert_eq!(renamed.status, StatusCode::OK, "{:?}", renamed.body);
    assert_eq!(renamed.body["title"], "Énigme");
    create(&mut admin, workshop()).await;

    for role in [Role::Student, Role::ShopManager] {
        let (mut client, _) = app.login_as(role).await;
        assert_eq!(
            client.post("/api/staff/challenges", riddle()).await.status,
            StatusCode::FORBIDDEN,
            "as {role:?}"
        );
        assert_eq!(
            client.get("/api/staff/challenges").await.status,
            StatusCode::FORBIDDEN,
            "as {role:?}"
        );
    }
    assert_eq!(
        alan.get("/api/challenges").await.status,
        StatusCode::FORBIDDEN
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn the_answer_never_leaves_the_server(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut staff, _) = app.login_as(Role::Staff).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let (mut student, _) = app.login_as(Role::Student).await;
    let id = create(&mut staff, riddle()).await;
    let changed = staff
        .patch(
            &format!("/api/staff/challenges/{id}"),
            json!({ "answer": "Rust 2024" }),
        )
        .await;
    assert_eq!(changed.status, StatusCode::OK);

    for (client, path) in [
        (&mut staff, "/api/staff/challenges"),
        (&mut admin, "/api/staff/challenges"),
        (&mut student, "/api/challenges"),
    ] {
        let text = client.get(path).await.body.to_string().to_lowercase();
        assert!(!text.contains("rust"), "{path}: {text}");
        assert!(!text.contains("argon2"), "{path}: {text}");
        assert!(!text.contains("answer_hash"), "{path}: {text}");
    }
    // The new answer replaced the old one.
    assert_eq!(
        answer(&mut student, &id, "Rust").await.error(),
        "wrong_answer"
    );
    assert_eq!(
        answer(&mut student, &id, "rust 2024").await.status,
        StatusCode::OK
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn a_creator_who_cannot_pay_refuses_without_recording_a_winner(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let (mut staff, staff_user) = app.login_as(Role::Staff).await;
    let (mut student, me) = app.login_as(Role::Student).await;
    let id = create(&mut staff, riddle()).await;
    let staff_path = format!("/api/admin/users/{}", staff_user.id);

    // The creator is no longer staff: their authority is revoked.
    admin.patch(&staff_path, json!({ "role": "student" })).await;
    let refused = answer(&mut student, &id, "Rust").await;
    assert_eq!(refused.status, StatusCode::CONFLICT);
    assert_eq!(refused.error(), "challenge_unfunded");
    assert_eq!(app.balance(&me.address), 0);
    assert_eq!(mints(&app).await, 0);
    let seen = listed(&mut student, "/api/challenges", &id).await.unwrap();
    assert_eq!(seen["submission"], Value::Null);

    // Back to staff: the same student can now be paid.
    admin.patch(&staff_path, json!({ "role": "staff" })).await;
    assert_eq!(
        answer(&mut student, &id, "Rust").await.status,
        StatusCode::OK
    );
    assert_eq!(app.balance(&me.address), 500);
}
