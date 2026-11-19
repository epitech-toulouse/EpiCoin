//! User administration: CSV import, roles and their on-chain authority, deletion, audit.

mod common;

use axum::http::StatusCode;
use common::*;
use epicoin_api::models::Role;
use serde_json::{json, Value};
use sqlx::PgPool;

const CSV: &str = "email;prenom;nom;promo;role\n\
                   ada@epitech.eu;Ada;Lovelace;pge1;student\n\
                   grace@epitech.eu;Grace;Hopper;MSc 2027;etudiant\n\
                   alan@epitech.eu;Alan;Turing;;encadrant\n\
                   linus@epitech.eu;Linus;Torvalds;;boutique\n";

async fn count(app: &TestApp, sql: &'static str) -> i64 {
    sqlx::query_scalar(sql).fetch_one(&app.db).await.unwrap()
}

fn authority_of(app: &TestApp, address: &str) -> Option<(bool, u64)> {
    app.node
        .account(&address.parse().unwrap())
        .authority
        .map(|a| (a.active, a.quota))
}

async fn find(admin: &mut Client, email: &str) -> Value {
    let reply = admin.get(&format!("/api/admin/users?q={email}")).await;
    assert_eq!(reply.status, StatusCode::OK);
    reply.body["items"][0].clone()
}

// --- Import ----------------------------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn csv_import_creates_accounts_wallets_and_invitations(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;

    let reply = admin
        .post("/api/admin/users/import", json!({ "csv": CSV }))
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(reply.body["created"], 4);
    assert_eq!(reply.body["updated"], 0);
    assert_eq!(reply.body["unchanged"], 0);
    assert_eq!(reply.body["by_role"]["student"], 2);
    assert_eq!(reply.body["by_role"]["staff"], 1);

    // Everyone has a wallet, nobody is activated yet.
    assert_eq!(count(&app, "SELECT count(*) FROM wallets").await, 5);
    let ada = find(&mut admin, "ada@epitech.eu").await;
    assert_eq!(ada["first_name"], "Ada");
    assert_eq!(ada["promo"], "pge1");
    let grace = find(&mut admin, "grace@epitech.eu").await;
    assert_eq!(grace["promo"], "msc2027", "stored as its code");
    assert_eq!(ada["role"], "student");
    assert_eq!(ada["activated"], false);
    assert_eq!(ada["wallet_address"].as_str().unwrap().len(), 64);

    // Each one receives a personal activation link (sent in the background).
    for email in [
        "ada@epitech.eu",
        "grace@epitech.eu",
        "alan@epitech.eu",
        "linus@epitech.eu",
    ] {
        let mail = app.mail_to(email).await;
        assert!(mail.body.contains("/activation#token="));
    }

    // The staff member becomes a mint authority on chain, with the default quota.
    let alan = find(&mut admin, "alan@epitech.eu").await;
    let address = alan["wallet_address"].as_str().unwrap().to_owned();
    app.eventually("the staff authority on chain", || {
        authority_of(&app, &address) == Some((true, 50_000))
    })
    .await;
    // Students are not.
    assert_eq!(
        authority_of(&app, ada["wallet_address"].as_str().unwrap()),
        None
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn reimporting_a_file_only_creates_the_new_accounts(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    admin
        .post("/api/admin/users/import", json!({ "csv": CSV }))
        .await;

    let more = format!("{CSV}margaret@epitech.eu;Margaret;Hamilton;pge1;student\n");
    let reply = admin
        .post("/api/admin/users/import", json!({ "csv": more }))
        .await;
    assert_eq!(reply.status, StatusCode::OK);
    assert_eq!(reply.body["created"], 1);
    assert_eq!(reply.body["updated"], 0);
    assert_eq!(reply.body["unchanged"], 4);
    assert_eq!(count(&app, "SELECT count(*) FROM users").await, 6);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn import_moves_existing_accounts_to_their_new_cohort_and_nothing_else(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    admin
        .post("/api/admin/users/import", json!({ "csv": CSV }))
        .await;

    // Next year: Ada moves up. The file also gives her another name and role, and leaves
    // Grace's cohort empty.
    let next_year = "email;prenom;nom;cohorte;role\n\
                     ada@epitech.eu;Ada;Byron;PGE 2;admin\n\
                     grace@epitech.eu;Grace;Hopper;;etudiant\n";
    let reply = admin
        .post("/api/admin/users/import", json!({ "csv": next_year }))
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(reply.body["created"], 0);
    assert_eq!(reply.body["updated"], 1);
    assert_eq!(reply.body["unchanged"], 1);

    let ada = find(&mut admin, "ada@epitech.eu").await;
    assert_eq!(ada["promo"], "pge2");
    assert_eq!(
        ada["last_name"], "Lovelace",
        "names are not changed by an import"
    );
    assert_eq!(ada["role"], "student", "roles are not changed by an import");
    let grace = find(&mut admin, "grace@epitech.eu").await;
    assert_eq!(
        grace["promo"], "msc2027",
        "an empty cohort leaves the current one"
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn a_file_of_emails_and_cohorts_reassigns_cohorts(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    admin
        .post("/api/admin/users/import", json!({ "csv": CSV }))
        .await;

    let reply = admin
        .post(
            "/api/admin/users/import",
            json!({ "csv": "email,cohorte\nada@epitech.eu,bach1\nGrace@epitech.eu,Bachelor 2\n" }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(reply.body["created"], 0);
    assert_eq!(reply.body["updated"], 2);
    assert_eq!(find(&mut admin, "ada@epitech.eu").await["promo"], "bach1");
    assert_eq!(find(&mut admin, "grace@epitech.eu").await["promo"], "bach2");
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn a_file_of_emails_and_cohorts_cannot_create_accounts(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    admin
        .post("/api/admin/users/import", json!({ "csv": CSV }))
        .await;

    let reply = admin
        .post(
            "/api/admin/users/import",
            json!({ "csv": "email;cohorte\nada@epitech.eu;bach1\nghost@epitech.eu;bach1\n" }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.error(), "invalid_csv");
    let errors = reply.body["details"]["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["line"], 3);
    assert!(errors[0]["message"]
        .as_str()
        .unwrap()
        .contains("ghost@epitech.eu"));
    // All or nothing: Ada keeps her cohort.
    assert_eq!(find(&mut admin, "ada@epitech.eu").await["promo"], "pge1");
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn import_with_an_invalid_line_imports_nothing(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let csv = "email,prenom,nom,role\n\
               ada@epitech.eu,Ada,Lovelace,student\n\
               mallory@gmail.com,Mallory,External,student\n\
               eve@epitech.eu,Eve,Nobody,superadmin\n";

    let reply = admin
        .post("/api/admin/users/import", json!({ "csv": csv }))
        .await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.error(), "invalid_csv");
    let errors = reply.body["details"]["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 2);
    assert_eq!(errors[0]["line"], 3);
    assert_eq!(errors[1]["line"], 4);

    assert_eq!(count(&app, "SELECT count(*) FROM users").await, 1);
    assert!(app.mailer.sent().is_empty());
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn only_admins_manage_users(db: PgPool) {
    let app = TestApp::start(db).await;
    let target = app.seed(Role::Student).await;
    for role in [Role::Student, Role::Staff, Role::ShopManager] {
        let (mut client, _) = app.login_as(role).await;
        let attempts = [
            client.get("/api/admin/users").await.status,
            client
                .post("/api/admin/users/import", json!({ "csv": CSV }))
                .await
                .status,
            client
                .post(
                    "/api/admin/users",
                    json!({ "email": "x@epitech.eu", "first_name": "X", "last_name": "Y",
                            "role": "admin" }),
                )
                .await
                .status,
            client
                .patch(
                    &format!("/api/admin/users/{}", target.id),
                    json!({ "role": "admin" }),
                )
                .await
                .status,
            client
                .delete(&format!("/api/admin/users/{}", target.id))
                .await
                .status,
            client.get("/api/admin/audit").await.status,
            client.get("/api/admin/stats").await.status,
            client.get("/api/explorer/accounts").await.status,
        ];
        for status in attempts {
            assert_eq!(status, StatusCode::FORBIDDEN, "as {role:?}");
        }
    }
    assert_eq!(app.user(target.id).await.unwrap().role, Role::Student);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn single_creation_validates_and_refuses_duplicates(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let user = |email: &str| json!({ "email": email, "first_name": "Ada", "last_name": "Lovelace", "role": "student" });

    let outside = admin.post("/api/admin/users", user("ada@gmail.com")).await;
    assert_eq!(outside.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(outside.error(), "invalid_user");

    let created = admin.post("/api/admin/users", user("Ada@Epitech.eu")).await;
    assert_eq!(created.status, StatusCode::CREATED);
    assert_eq!(created.body["email"], "ada@epitech.eu");

    let duplicate = admin.post("/api/admin/users", user("ada@epitech.eu")).await;
    assert_eq!(duplicate.status, StatusCode::CONFLICT);
    assert_eq!(duplicate.error(), "email_taken");

    let mut unknown_cohort = user("grace@epitech.eu");
    unknown_cohort["promo"] = json!("2027");
    let reply = admin.post("/api/admin/users", unknown_cohort).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.error(), "invalid_user");
    assert!(reply.body["message"]
        .as_str()
        .unwrap()
        .contains("Cohorte inconnue"));
}

// --- Listing ---------------------------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn users_can_be_searched_filtered_and_paged(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    admin
        .post("/api/admin/users/import", json!({ "csv": CSV }))
        .await;

    let all = admin.get("/api/admin/users").await;
    assert_eq!(all.body["total"], 5);

    let students = admin.get("/api/admin/users?role=student").await;
    assert_eq!(students.body["total"], 2);

    let by_name = admin.get("/api/admin/users?q=hopp").await;
    assert_eq!(by_name.body["total"], 1);
    assert_eq!(by_name.body["items"][0]["email"], "grace@epitech.eu");

    let by_full_name = admin.get("/api/admin/users?q=alan%20turing").await;
    assert_eq!(by_full_name.body["total"], 1);

    // A wildcard is searched literally, not as "everything".
    let wildcard = admin.get("/api/admin/users?q=%25").await;
    assert_eq!(wildcard.body["total"], 0);

    let page = admin.get("/api/admin/users?limit=2&offset=4").await;
    assert_eq!(page.body["total"], 5);
    assert_eq!(page.body["items"].as_array().unwrap().len(), 1);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn users_can_be_filtered_by_cohort(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let csv = format!("{CSV}eve@epitech.eu;Eve;Nobody;;student\nmax@epitech.eu;Max;Old;;student\n");
    admin
        .post("/api/admin/users/import", json!({ "csv": csv }))
        .await;
    // A free-text value from before cohorts were a fixed list.
    sqlx::query("UPDATE users SET promo = '2026' WHERE email = 'max@epitech.eu'")
        .execute(&app.db)
        .await
        .unwrap();

    let emails = |reply: &Reply| -> Vec<String> {
        reply.body["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|u| u["email"].as_str().unwrap().to_owned())
            .collect()
    };
    let pge1 = admin.get("/api/admin/users?promo=pge1").await;
    assert_eq!(emails(&pge1), ["ada@epitech.eu"]);
    let msc = admin.get("/api/admin/users?promo=msc2027&q=grace").await;
    assert_eq!(emails(&msc), ["grace@epitech.eu"]);
    assert_eq!(
        admin.get("/api/admin/users?promo=bach3").await.body["total"],
        0
    );

    // Students without a valid cohort; staff and admins never have one.
    let none = admin.get("/api/admin/users?promo=none").await;
    assert_eq!(emails(&none), ["eve@epitech.eu", "max@epitech.eu"]);
    assert_eq!(none.body["total"], 2);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn admins_change_the_cohort_of_a_user(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let user = app.seed(Role::Student).await;
    let path = format!("/api/admin/users/{}", user.id);

    let reply = admin.patch(&path, json!({ "promo": "PGE 3" })).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(reply.body["promo"], "pge3");

    // Another change leaves the cohort alone.
    let reply = admin.patch(&path, json!({ "disabled": false })).await;
    assert_eq!(reply.body["promo"], "pge3");

    let unknown = admin.patch(&path, json!({ "promo": "pge9" })).await;
    assert_eq!(unknown.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(unknown.error(), "invalid_promo");
    assert_eq!(app.user(user.id).await.unwrap().promo, "pge3");

    let reply = admin.patch(&path, json!({ "promo": "" })).await;
    assert_eq!(reply.body["promo"], "");
}

// --- Roles and on-chain authority ------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn staff_role_drives_the_on_chain_authority(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let user = app.seed(Role::Student).await;
    let address = user.address.to_string();
    let path = format!("/api/admin/users/{}", user.id);
    assert_eq!(authority_of(&app, &address), None);

    // Promoted to staff: granted with the default quota (500 EPC).
    let reply = admin.patch(&path, json!({ "role": "staff" })).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(reply.body["role"], "staff");
    assert_eq!(authority_of(&app, &address), Some((true, 50_000)));

    // A personal quota is applied on chain…
    let reply = admin.patch(&path, json!({ "mint_quota": 120_000 })).await;
    assert_eq!(reply.body["mint_quota"], 120_000);
    assert_eq!(authority_of(&app, &address), Some((true, 120_000)));
    // …and removing it falls back to the default.
    let reply = admin.patch(&path, json!({ "mint_quota": null })).await;
    assert_eq!(reply.body["mint_quota"], Value::Null);
    assert_eq!(authority_of(&app, &address), Some((true, 50_000)));

    // Disabled: revoked, even though the role is unchanged.
    admin.patch(&path, json!({ "disabled": true })).await;
    assert_eq!(authority_of(&app, &address), Some((false, 50_000)));
    admin.patch(&path, json!({ "disabled": false })).await;
    assert_eq!(authority_of(&app, &address), Some((true, 50_000)));

    // Back to student: revoked.
    admin.patch(&path, json!({ "role": "student" })).await;
    assert_eq!(authority_of(&app, &address), Some((false, 50_000)));
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn disabling_a_user_ends_their_sessions(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let (mut student, user) = app.login_as(Role::Student).await;

    let reply = admin
        .patch(
            &format!("/api/admin/users/{}", user.id),
            json!({ "disabled": true }),
        )
        .await;
    assert_eq!(reply.body["disabled"], true);
    assert_eq!(
        student.get("/api/auth/me").await.status,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        app.client().login(&user.email, PASSWORD).await,
        StatusCode::UNAUTHORIZED
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn admins_cannot_lock_themselves_out(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, me) = app.login_as(Role::Admin).await;
    let path = format!("/api/admin/users/{}", me.id);

    for body in [json!({ "role": "student" }), json!({ "disabled": true })] {
        let reply = admin.patch(&path, body).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(reply.error(), "self_lockout");
    }
    assert_eq!(
        admin.delete(&path).await.status,
        StatusCode::UNPROCESSABLE_ENTITY
    );
    assert_eq!(app.user(me.id).await.unwrap().role, Role::Admin);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn invalid_updates_are_refused(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let user = app.seed(Role::Staff).await;

    let negative = admin
        .patch(
            &format!("/api/admin/users/{}", user.id),
            json!({ "mint_quota": -5 }),
        )
        .await;
    assert_eq!(negative.status, StatusCode::UNPROCESSABLE_ENTITY);

    let unknown = admin
        .patch(
            &format!("/api/admin/users/{}", uuid::Uuid::new_v4()),
            json!({ "role": "staff" }),
        )
        .await;
    assert_eq!(unknown.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn periodic_sync_repairs_authorities(db: PgPool) {
    let app = TestApp::start(db).await;
    let staff = app.seed(Role::Staff).await;
    let demoted = app.seed(Role::Staff).await;

    // Changes made behind the API's back (or while the node was unreachable).
    sqlx::query("UPDATE users SET mint_quota = 7700 WHERE id = $1")
        .bind(staff.id)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("UPDATE users SET role = 'student' WHERE id = $1")
        .bind(demoted.id)
        .execute(&app.db)
        .await
        .unwrap();
    let promoted = app.seed(Role::Student).await;
    sqlx::query("UPDATE users SET role = 'staff' WHERE id = $1")
        .bind(promoted.id)
        .execute(&app.db)
        .await
        .unwrap();

    let changes = epicoin_api::authority::sync_all(&app.state).await.unwrap();
    assert_eq!(changes, 3);
    assert_eq!(
        authority_of(&app, &staff.address.to_string()),
        Some((true, 7_700))
    );
    assert_eq!(
        authority_of(&app, &demoted.address.to_string()),
        Some((false, 50_000))
    );
    assert_eq!(
        authority_of(&app, &promoted.address.to_string()),
        Some((true, 50_000))
    );

    // Once in sync, there is nothing left to do.
    assert_eq!(
        epicoin_api::authority::sync_all(&app.state).await.unwrap(),
        0
    );
}

// --- Invitations -----------------------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn invitation_can_be_sent_again(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let created = admin
        .post(
            "/api/admin/users",
            json!({ "email": "ada@epitech.eu", "first_name": "Ada", "last_name": "Lovelace",
                    "role": "student" }),
        )
        .await;
    let id = created.body["id"].as_str().unwrap().to_owned();
    let first = token_in(&app.mail_to("ada@epitech.eu").await);

    let reply = admin
        .post(&format!("/api/admin/users/{id}/invitation"), json!({}))
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);
    let second = token_in(&app.mail_to("ada@epitech.eu").await);
    assert_ne!(first, second);

    // Only the latest link works.
    let stale = app
        .client()
        .post(
            "/api/auth/activate",
            json!({ "token": first, "password": PASSWORD }),
        )
        .await;
    assert_eq!(stale.status, StatusCode::NOT_FOUND);
    let fresh = app
        .client()
        .post(
            "/api/auth/activate",
            json!({ "token": second, "password": PASSWORD }),
        )
        .await;
    assert_eq!(fresh.status, StatusCode::OK);

    // Nothing to resend once the account is active.
    let reply = admin
        .post(&format!("/api/admin/users/{id}/invitation"), json!({}))
        .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.error(), "already_activated");
}

// --- Deletion (GDPR) -------------------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn deleting_a_user_erases_their_personal_data(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, _) = app.login_as(Role::Admin).await;
    let (mut staff, staff_user) = app.login_as(Role::Staff).await;
    let student = app.seed(Role::Student).await;

    // Some history involving both.
    let reply = staff
        .post(
            "/api/staff/distributions",
            json!({ "recipient_ids": [student.id], "amount": 500, "reason_code": "event" }),
        )
        .await;
    assert_eq!(reply.body["status"], "included", "{:?}", reply.body);

    for user in [&student, &staff_user] {
        let reply = admin.delete(&format!("/api/admin/users/{}", user.id)).await;
        assert_eq!(reply.status, StatusCode::NO_CONTENT);
    }

    // No trace of the persons in Postgres: identity, wallet key, sessions.
    assert!(app.user(student.id).await.is_none());
    assert_eq!(count(&app, "SELECT count(*) FROM wallets").await, 1);
    assert_eq!(count(&app, "SELECT count(*) FROM sessions").await, 1);
    assert_eq!(
        staff.get("/api/auth/me").await.status,
        StatusCode::UNAUTHORIZED
    );
    for table in ["users", "audit_log", "chain_txs", "mint_batches"] {
        let dump: String = sqlx::query_scalar(match table {
            "users" => "SELECT COALESCE(string_agg(u::text, ' '), '') FROM users u",
            "audit_log" => "SELECT COALESCE(string_agg(a::text, ' '), '') FROM audit_log a",
            "chain_txs" => "SELECT COALESCE(string_agg(c::text, ' '), '') FROM chain_txs c",
            _ => "SELECT COALESCE(string_agg(b::text, ' '), '') FROM mint_batches b",
        })
        .fetch_one(&app.db)
        .await
        .unwrap();
        for user in [&student, &staff_user] {
            assert!(!dump.contains(&user.email), "{table} still holds an email");
            // The audit trail keeps the opaque id of what was deleted (it is the proof of
            // the deletion); every other table drops even that.
            if table != "audit_log" {
                assert!(
                    !dump.contains(&user.id.to_string()),
                    "{table} still references a deleted user"
                );
            }
        }
        for name in ["Prenom1", "Nom1", "Prenom2", "Nom2"] {
            assert!(!dump.contains(name), "{table} still holds a name");
        }
    }
    // In the audit trail, the deleted users are no longer linked as actors.
    assert_eq!(
        count(
            &app,
            "SELECT count(*) FROM audit_log WHERE actor_user_id IS NULL"
        )
        .await,
        count(
            &app,
            "SELECT count(*) FROM audit_log a WHERE NOT EXISTS \
             (SELECT 1 FROM users u WHERE u.id = a.actor_user_id)"
        )
        .await
    );
    let trail = admin.get("/api/admin/audit").await;
    let mint_entry = trail.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["action"] == "mint.distributed")
        .unwrap();
    assert_eq!(mint_entry["actor_user_id"], Value::Null);
    assert_eq!(mint_entry["actor_name"], Value::Null);

    // The ledger itself is untouched (it never held personal data), and still adds up.
    assert_eq!(app.balance(&student.address), 500);
    assert_eq!(
        count(&app, "SELECT count(*) FROM chain_txs WHERE kind = 'mint'").await,
        1
    );
    // The deleted staff member can no longer mint: authority revoked on chain.
    assert_eq!(
        authority_of(&app, &staff_user.address.to_string()),
        Some((false, 50_000))
    );
}

// --- Audit -----------------------------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn sensitive_actions_are_audited_without_personal_data(db: PgPool) {
    let app = TestApp::start(db).await;
    let (mut admin, admin_user) = app.login_as(Role::Admin).await;
    admin
        .post("/api/admin/users/import", json!({ "csv": CSV }))
        .await;
    let ada = find(&mut admin, "ada@epitech.eu").await;
    let ada_id = ada["id"].as_str().unwrap();
    admin
        .patch(
            &format!("/api/admin/users/{ada_id}"),
            json!({ "role": "staff" }),
        )
        .await;

    let reply = admin.get("/api/admin/audit").await;
    assert_eq!(reply.status, StatusCode::OK);
    let entries = reply.body["items"].as_array().unwrap();
    let actions: Vec<&str> = entries
        .iter()
        .map(|e| e["action"].as_str().unwrap())
        .collect();
    for expected in [
        "auth.login",
        "users.imported",
        "users.updated",
        "authority.granted",
    ] {
        assert!(
            actions.contains(&expected),
            "missing {expected}: {actions:?}"
        );
    }

    let update = entries
        .iter()
        .find(|e| e["action"] == "users.updated")
        .unwrap();
    assert_eq!(update["actor_user_id"], admin_user.id.to_string());
    assert_eq!(update["target_id"], ada_id);
    assert_eq!(update["details"]["role"]["from"], "student");
    assert_eq!(update["details"]["role"]["to"], "staff");
    assert!(update["at"].as_str().unwrap().contains('T'));
    assert!(update["actor_name"].as_str().unwrap().starts_with("Prenom"));

    // The stored details only hold ids, counts and codes.
    let details: String =
        sqlx::query_scalar("SELECT string_agg(details::text, ' ') FROM audit_log")
            .fetch_one(&app.db)
            .await
            .unwrap();
    for personal in ["ada@epitech.eu", "Lovelace", "Turing", "@epitech.eu"] {
        assert!(
            !details.contains(personal),
            "audit details contain {personal}"
        );
    }
}
