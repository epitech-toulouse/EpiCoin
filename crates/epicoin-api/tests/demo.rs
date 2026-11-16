//! Demo data: the seed fills an empty platform, and only an empty one.

mod common;

use common::*;
use epicoin_api::demo;
use sqlx::PgPool;

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn demo_seed_builds_a_usable_platform(db: PgPool) {
    let app = TestApp::start(db).await;
    let report = demo::seed(&app.state, "demo password").await.unwrap();
    assert_eq!(report.students, 150);
    assert_eq!(report.products, 10);
    assert_eq!(report.mints, 5 * (20 + 10 + 5));

    let count = |sql: &'static str| {
        let db = app.db.clone();
        async move {
            sqlx::query_scalar::<_, i64>(sql)
                .fetch_one(&db)
                .await
                .unwrap()
        }
    };
    assert_eq!(
        count("SELECT count(*) FROM users WHERE role = 'student'").await,
        150
    );
    assert_eq!(
        count("SELECT count(*) FROM users WHERE role = 'staff'").await,
        5
    );
    assert_eq!(count("SELECT count(*) FROM products").await, 10);
    assert_eq!(
        count("SELECT count(*) FROM users WHERE activated_at IS NULL").await,
        0
    );

    // On chain: five active authorities, every mint included, quotas respected.
    let authorities = app.node.authorities();
    assert_eq!(authorities.iter().filter(|a| a.active).count(), 5);
    assert!(authorities.iter().all(|a| a.minted <= a.quota));
    let status = app.node.status();
    assert_eq!(
        status.total_minted,
        5 * (20 * 1_000 + 10 * 1_500 + 5 * 2_000)
    );

    // The demo accounts work through the real login.
    let mut admin = app.client();
    assert_eq!(
        admin.login("admin.demo@epitech.eu", "demo password").await,
        axum::http::StatusCode::OK
    );
    let stats = admin.get("/api/admin/stats").await;
    assert_eq!(stats.body["minted_by_staff"].as_array().unwrap().len(), 5);
    let mut staff = app.client();
    staff
        .login("encadrant1.demo@epitech.eu", "demo password")
        .await;
    assert_eq!(
        staff.get("/api/staff/distributions").await.body["total"],
        35
    );

    // A second run is refused: demo data never lands on a platform in use.
    assert!(demo::seed(&app.state, "demo password").await.is_err());
    assert_eq!(count("SELECT count(*) FROM users").await, 157);
}
