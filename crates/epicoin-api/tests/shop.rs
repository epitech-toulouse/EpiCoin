//! The shop: catalogue, checkout (idempotent, stock-safe), order lifecycle, refunds.

mod common;

use std::time::Duration;

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use common::*;
use epicoin_api::chain::{record_tx, TxMeta};
use epicoin_api::models::Role;
use epicoin_api::shop::orders::order_ref;
use epicoin_core::{Transaction, TxPayload};
use serde_json::{json, Value};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

/// A shop with a manager, a product, and a student who owns `balance` units.
struct Shop {
    app: TestApp,
    manager: Client,
    student: Client,
    student_user: Seeded,
    staff: Client,
    product: Value,
}

impl Shop {
    async fn open(db: PgPool, balance: u64, price: i64, stock: i32) -> Self {
        let app = TestApp::start(db).await;
        let (mut manager, _) = app.login_as(Role::ShopManager).await;
        let (student, student_user) = app.login_as(Role::Student).await;
        let (staff, _) = app.login_as(Role::Staff).await;
        let reply = manager
            .post(
                "/api/shop/manage/products",
                json!({ "name": "Mug EpiCoin", "description": "Un mug.", "price": price,
                        "stock": stock }),
            )
            .await;
        assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.body);
        let mut shop = Self {
            app,
            manager,
            student,
            student_user,
            staff,
            product: reply.body,
        };
        if balance > 0 {
            let id = shop.student_user.id;
            shop.fund(id, balance).await;
        }
        shop
    }

    async fn fund(&mut self, user_id: Uuid, amount: u64) {
        let reply = self
            .staff
            .post(
                "/api/staff/distributions",
                json!({ "recipient_ids": [user_id], "amount": amount, "reason_code": "event" }),
            )
            .await;
        assert_eq!(reply.body["status"], "included", "{:?}", reply.body);
    }

    fn product_id(&self) -> &str {
        self.product["id"].as_str().unwrap()
    }

    async fn stock(&self) -> i32 {
        sqlx::query_scalar("SELECT stock FROM products WHERE id = $1")
            .bind(Uuid::parse_str(self.product_id()).unwrap())
            .fetch_one(&self.app.db)
            .await
            .unwrap()
    }

    fn cart(&self, quantity: i32) -> Value {
        json!({ "items": [{ "product_id": self.product_id(), "quantity": quantity }] })
    }

    /// The student buys `quantity` of the product.
    async fn buy(&mut self, quantity: i32, key: &str) -> Reply {
        let cart = self.cart(quantity);
        buy(&mut self.student, cart, key).await
    }

    fn treasury_balance(&self) -> u64 {
        self.app.balance(&self.app.treasury_key.address())
    }

    fn student_balance(&self) -> u64 {
        self.app.balance(&self.student_user.address)
    }

    async fn count(&self, sql: &'static str) -> i64 {
        sqlx::query_scalar(sql)
            .fetch_one(&self.app.db)
            .await
            .unwrap()
    }
}

async fn buy(client: &mut Client, cart: Value, key: &str) -> Reply {
    client
        .send(
            Method::POST,
            "/api/shop/orders",
            Some(cart),
            &[("idempotency-key", key)],
        )
        .await
}

// --- Catalogue -------------------------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn manager_manages_the_catalogue(db: PgPool) {
    let mut shop = Shop::open(db, 0, 1_500, 10).await;
    assert_eq!(shop.product["name"], "Mug EpiCoin");
    assert_eq!(shop.product["price"], 1_500);
    assert_eq!(shop.product["stock"], 10);
    assert_eq!(shop.product["active"], true);
    assert_eq!(shop.product["image_url"], Value::Null);
    let path = format!("/api/shop/manage/products/{}", shop.product_id());

    let updated = shop
        .manager
        .patch(&path, json!({ "price": 1_200, "stock": 7 }))
        .await;
    assert_eq!(updated.status, StatusCode::OK);
    assert_eq!(updated.body["price"], 1_200);
    assert_eq!(updated.body["stock"], 7);
    assert_eq!(
        updated.body["name"], "Mug EpiCoin",
        "untouched fields are kept"
    );

    // Students see what is on sale…
    let catalogue = shop.student.get("/api/shop/products").await;
    assert_eq!(catalogue.body.as_array().unwrap().len(), 1);
    // …and not what was withdrawn.
    shop.manager.patch(&path, json!({ "active": false })).await;
    let catalogue = shop.student.get("/api/shop/products").await;
    assert!(catalogue.body.as_array().unwrap().is_empty());
    let all = shop.manager.get("/api/shop/manage/products").await;
    assert_eq!(all.body.as_array().unwrap().len(), 1);
    assert_eq!(all.body[0]["active"], false);

    // Never ordered: can be deleted.
    assert_eq!(
        shop.manager.delete(&path).await.status,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        shop.manager.delete(&path).await.status,
        StatusCode::NOT_FOUND
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn invalid_products_are_refused(db: PgPool) {
    let mut shop = Shop::open(db, 0, 100, 1).await;
    for body in [
        json!({ "name": "", "price": 100, "stock": 1 }),
        json!({ "name": "Gratuit", "price": 0, "stock": 1 }),
        json!({ "name": "Négatif", "price": -100, "stock": 1 }),
        json!({ "name": "Stock négatif", "price": 100, "stock": -1 }),
        json!({ "name": "x".repeat(101), "price": 100, "stock": 1 }),
    ] {
        let reply = shop.manager.post("/api/shop/manage/products", body).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(reply.error(), "invalid_product");
    }
    let path = format!("/api/shop/manage/products/{}", shop.product_id());
    let reply = shop.manager.patch(&path, json!({ "price": 0 })).await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    let reply = shop
        .manager
        .patch(
            &format!("/api/shop/manage/products/{}", Uuid::new_v4()),
            json!({ "price": 5 }),
        )
        .await;
    assert_eq!(reply.status, StatusCode::NOT_FOUND);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn only_managers_and_admins_manage_the_shop(db: PgPool) {
    let mut shop = Shop::open(db, 0, 100, 5).await;
    let id = shop.product_id().to_owned();
    let order = Uuid::new_v4();
    for client in [&mut shop.student, &mut shop.staff] {
        let attempts = [
            client.get("/api/shop/manage/products").await.status,
            client
                .post(
                    "/api/shop/manage/products",
                    json!({ "name": "X", "price": 1, "stock": 1 }),
                )
                .await
                .status,
            client
                .patch(
                    &format!("/api/shop/manage/products/{id}"),
                    json!({ "price": 1 }),
                )
                .await
                .status,
            client
                .delete(&format!("/api/shop/manage/products/{id}"))
                .await
                .status,
            client.get("/api/shop/manage/orders").await.status,
            client
                .post(
                    &format!("/api/shop/manage/orders/{order}/status"),
                    json!({ "status": "ready" }),
                )
                .await
                .status,
            client
                .post(
                    &format!("/api/shop/manage/orders/{order}/cancel"),
                    json!({}),
                )
                .await
                .status,
        ];
        for status in attempts {
            assert_eq!(status, StatusCode::FORBIDDEN);
        }
    }
    assert_eq!(shop.stock().await, 5);

    // An administrator can stand in for the shop manager.
    let (mut admin, _) = shop.app.login_as(Role::Admin).await;
    assert_eq!(
        admin.get("/api/shop/manage/products").await.status,
        StatusCode::OK
    );
    // Staff cannot buy (they hold no EPC), and anonymous visitors see nothing.
    let reply = buy(&mut shop.staff, json!({ "items": [] }), "k").await;
    assert_eq!(reply.status, StatusCode::FORBIDDEN);
    assert_eq!(
        shop.app.client().get("/api/shop/products").await.status,
        StatusCode::UNAUTHORIZED
    );
}

// --- Images ----------------------------------------------------------------------------

const PNG: &[u8] = &[
    0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R',
];

/// Sends a raw body as a logged-in client (the JSON helper cannot).
async fn upload(app: &TestApp, client: &Client, path: &str, bytes: Vec<u8>) -> (StatusCode, Value) {
    let request = Request::builder()
        .method(Method::POST)
        .uri(path)
        .header(
            header::COOKIE,
            format!("epicoin_session={}", client.cookie.as_ref().unwrap()),
        )
        .header("x-csrf-token", client.csrf.as_ref().unwrap())
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(bytes))
        .unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

async fn fetch(app: &TestApp, path: &str) -> (StatusCode, Option<String>, Vec<u8>) {
    let request = Request::builder().uri(path).body(Body::empty()).unwrap();
    let response = app.router.clone().oneshot(request).await.unwrap();
    let status = response.status();
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, content_type, bytes.to_vec())
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn product_images_are_uploaded_checked_and_served(db: PgPool) {
    let shop = Shop::open(db, 0, 100, 5).await;
    let path = format!("/api/shop/manage/products/{}/image", shop.product_id());

    let (status, body) = upload(&shop.app, &shop.manager, &path, PNG.to_vec()).await;
    assert_eq!(status, StatusCode::OK, "{body:?}");
    let url = body["image_url"].as_str().unwrap().to_owned();
    assert!(url.starts_with("/api/shop/images/") && url.ends_with(".png"));

    let (status, content_type, bytes) = fetch(&shop.app, &url).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type.as_deref(), Some("image/png"));
    assert_eq!(bytes, PNG);

    // What is not an image is refused, whatever it claims to be.
    for bad in [
        b"<svg xmlns='http://www.w3.org/2000/svg' onload='alert(1)'/>".to_vec(),
        b"<html><script>alert(1)</script></html>".to_vec(),
        Vec::new(),
    ] {
        let (status, body) = upload(&shop.app, &shop.manager, &path, bad).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error"], "invalid_image");
    }
    let mut huge = PNG.to_vec();
    huge.resize(3 * 1024 * 1024, 0);
    let (status, _) = upload(&shop.app, &shop.manager, &path, huge).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

    // Replacing the image removes the old file.
    let mut jpeg = vec![0xFF, 0xD8, 0xFF, 0xE0];
    jpeg.extend_from_slice(b"fake jpeg body");
    let (status, body) = upload(&shop.app, &shop.manager, &path, jpeg).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body["image_url"].as_str().unwrap().ends_with(".jpg"));
    assert_eq!(fetch(&shop.app, &url).await.0, StatusCode::NOT_FOUND);

    // Only server-generated names are ever read from disk.
    for sneaky in [
        "/api/shop/images/..%2F..%2Fkeys.env",
        "/api/shop/images/chain.redb",
        "/api/shop/images/notauuid.png",
    ] {
        assert_eq!(
            fetch(&shop.app, sneaky).await.0,
            StatusCode::NOT_FOUND,
            "{sneaky}"
        );
    }

    // Students cannot upload.
    let (status, _) = upload(&shop.app, &shop.student, &path, PNG.to_vec()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

// --- Checkout --------------------------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn checkout_pays_on_chain_and_takes_the_stock(db: PgPool) {
    let mut shop = Shop::open(db, 5_000, 1_500, 10).await;

    let reply = shop.buy(2, "order-1").await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.body);
    assert_eq!(reply.body["status"], "pending");
    assert_eq!(reply.body["total"], 3_000);
    assert_eq!(reply.body["items"][0]["name"], "Mug EpiCoin");
    assert_eq!(reply.body["items"][0]["quantity"], 2);
    assert_eq!(reply.body["items"][0]["unit_price"], 1_500);
    assert!(reply.body.get("buyer").is_none());
    let order_id = reply.body["id"].as_str().unwrap().to_owned();

    // The money moved on chain, the stock moved in the database.
    assert_eq!(shop.student_balance(), 2_000);
    assert_eq!(shop.treasury_balance(), 3_000);
    assert_eq!(shop.stock().await, 8);
    assert_eq!(
        shop.student.get("/api/me/wallet").await.body["balance"],
        2_000
    );

    // The student sees the order and a readable history line.
    let orders = shop.student.get("/api/shop/orders").await;
    assert_eq!(orders.body["total"], 1);
    assert_eq!(orders.body["items"][0]["id"], order_id);
    let one = shop
        .student
        .get(&format!("/api/shop/orders/{order_id}"))
        .await;
    assert_eq!(one.body["status"], "pending");
    let history = shop.student.get("/api/me/transactions").await;
    let line = &history.body["items"][0];
    assert_eq!(line["kind"], "purchase");
    assert_eq!(line["direction"], "debit");
    assert_eq!(line["amount"], 3_000);
    assert_eq!(line["order_id"], order_id);

    // The manager sees it in the queue, with the buyer.
    let queue = shop
        .manager
        .get("/api/shop/manage/orders?status=pending")
        .await;
    assert_eq!(queue.body["total"], 1);
    assert_eq!(
        queue.body["items"][0]["buyer"]["id"],
        shop.student_user.id.to_string()
    );
    assert_eq!(
        queue.body["items"][0]["buyer"]["email"],
        shop.student_user.email
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn checkout_requires_an_idempotency_key(db: PgPool) {
    let mut shop = Shop::open(db, 5_000, 1_500, 10).await;
    let cart = shop.cart(1);
    let reply = shop.student.post("/api/shop/orders", cart).await;
    assert_eq!(reply.status, StatusCode::BAD_REQUEST);
    assert_eq!(shop.student_balance(), 5_000);
    assert_eq!(shop.stock().await, 10);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn a_double_click_debits_once(db: PgPool) {
    let mut shop = Shop::open(db, 5_000, 1_500, 10).await;

    let first = shop.buy(1, "click").await;
    let second = shop.buy(1, "click").await;
    assert_eq!(first.status, StatusCode::CREATED);
    assert_eq!(
        second.status,
        StatusCode::OK,
        "the retry returns the existing order"
    );
    assert_eq!(first.body["id"], second.body["id"]);
    assert_eq!(second.body["status"], "pending");

    assert_eq!(shop.student_balance(), 3_500, "debited once");
    assert_eq!(shop.treasury_balance(), 1_500);
    assert_eq!(shop.stock().await, 9, "one item taken");
    assert_eq!(shop.count("SELECT count(*) FROM orders").await, 1);
    assert_eq!(
        shop.count("SELECT count(*) FROM chain_txs WHERE kind = 'purchase'")
            .await,
        1
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn simultaneous_identical_requests_debit_once(db: PgPool) {
    let mut shop = Shop::open(db, 5_000, 1_500, 10).await;
    let cart = shop.cart(1);
    // Two tabs, or a retry racing the original request: same session, same key.
    let mut twin = shop.app.client();
    twin.cookie = shop.student.cookie.clone();
    twin.csrf = shop.student.csrf.clone();

    let (a, b) = tokio::join!(
        buy(&mut shop.student, cart.clone(), "same-key"),
        buy(&mut twin, cart.clone(), "same-key"),
    );
    assert!(a.status.is_success(), "{:?}", a.body);
    assert!(b.status.is_success(), "{:?}", b.body);
    assert_eq!(a.body["id"], b.body["id"]);

    assert_eq!(shop.student_balance(), 3_500);
    assert_eq!(shop.stock().await, 9);
    assert_eq!(shop.count("SELECT count(*) FROM orders").await, 1);
    assert_eq!(
        shop.count("SELECT count(*) FROM chain_txs WHERE kind = 'purchase'")
            .await,
        1
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn out_of_stock_blocks_the_purchase_without_debit(db: PgPool) {
    let mut shop = Shop::open(db, 50_000, 1_500, 2).await;

    let reply = shop.buy(3, "too-many").await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.error(), "out_of_stock");
    assert_eq!(reply.body["details"]["products"][0]["available"], 2);

    // Nothing happened anywhere: no debit, no order, no transaction, stock intact.
    assert_eq!(shop.student_balance(), 50_000);
    assert_eq!(shop.treasury_balance(), 0);
    assert_eq!(shop.stock().await, 2);
    assert_eq!(shop.count("SELECT count(*) FROM orders").await, 0);
    assert_eq!(
        shop.count("SELECT count(*) FROM chain_txs WHERE kind = 'purchase'")
            .await,
        0
    );

    // Buying what is left works, then the shelf is empty.
    let reply = shop.buy(2, "the-last-two").await;
    assert_eq!(reply.status, StatusCode::CREATED);
    assert_eq!(shop.stock().await, 0);
    let reply = shop.buy(1, "one-more").await;
    assert_eq!(reply.error(), "out_of_stock");
    assert_eq!(shop.student_balance(), 47_000);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn two_students_racing_for_the_last_item(db: PgPool) {
    let mut shop = Shop::open(db, 5_000, 1_500, 1).await;
    let (mut rival, rival_user) = shop.app.login_as(Role::Student).await;
    shop.fund(rival_user.id, 5_000).await;
    let cart = shop.cart(1);

    let (a, b) = tokio::join!(
        buy(&mut shop.student, cart.clone(), "mine"),
        buy(&mut rival, cart.clone(), "no-mine"),
    );
    let statuses = [a.status, b.status];
    assert!(statuses.contains(&StatusCode::CREATED), "{statuses:?}");
    assert!(statuses.contains(&StatusCode::CONFLICT), "{statuses:?}");

    // Exactly one item sold, exactly one student debited.
    assert_eq!(shop.stock().await, 0);
    assert_eq!(shop.treasury_balance(), 1_500);
    let debited = [
        shop.student_balance(),
        shop.app.balance(&rival_user.address),
    ];
    assert!(
        debited == [3_500, 5_000] || debited == [5_000, 3_500],
        "{debited:?}"
    );
    assert_eq!(shop.count("SELECT count(*) FROM orders").await, 1);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn insufficient_balance_blocks_the_purchase(db: PgPool) {
    let mut shop = Shop::open(db, 1_000, 1_500, 10).await;
    let reply = shop.buy(1, "broke").await;
    assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(reply.error(), "payment_refused");
    assert_eq!(reply.body["details"]["reason"], "insufficient_balance");
    assert_eq!(reply.body["details"]["balance"], 1_000);
    assert_eq!(reply.body["details"]["needed"], 1_500);

    assert_eq!(shop.student_balance(), 1_000);
    assert_eq!(shop.stock().await, 10);
    assert_eq!(shop.count("SELECT count(*) FROM orders").await, 0);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn overspending_with_parallel_orders_is_stopped_by_the_chain(db: PgPool) {
    // 2 000 units, two simultaneous orders of 1 500: each one alone is affordable, so the
    // API's own balance check lets both through. The chain refuses the second.
    let mut shop = Shop::open(db, 2_000, 1_500, 10).await;
    let cart = shop.cart(1);
    let mut twin = shop.app.client();
    twin.cookie = shop.student.cookie.clone();
    twin.csrf = shop.student.csrf.clone();

    let (a, b) = tokio::join!(
        buy(&mut shop.student, cart.clone(), "first"),
        buy(&mut twin, cart.clone(), "second"),
    );
    let mut statuses = [a.status, b.status];
    statuses.sort();
    assert_eq!(
        statuses,
        [StatusCode::CREATED, StatusCode::UNPROCESSABLE_ENTITY],
        "{:?} / {:?}",
        a.body,
        b.body
    );
    let refused = if a.status == StatusCode::CREATED {
        &b
    } else {
        &a
    };
    assert_eq!(refused.error(), "payment_refused");
    assert_eq!(refused.body["details"]["reason"], "insufficient_balance");

    // One debit, one item gone; the refused order gave its stock back.
    assert_eq!(shop.student_balance(), 500);
    assert_eq!(shop.treasury_balance(), 1_500);
    assert_eq!(shop.stock().await, 9);
    assert_eq!(
        shop.count("SELECT count(*) FROM orders WHERE status = 'pending'")
            .await,
        1
    );
    // If the refused order was recorded at all, it is marked failed, with its reason.
    assert_eq!(
        shop.count("SELECT count(*) FROM orders WHERE status NOT IN ('pending', 'failed')")
            .await,
        0
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn invalid_carts_are_refused(db: PgPool) {
    let mut shop = Shop::open(db, 50_000, 100, 10).await;
    let id = shop.product_id().to_owned();
    let cases = [
        (json!({ "items": [] }), "empty_cart"),
        (
            json!({ "items": [{ "product_id": id, "quantity": 0 }] }),
            "invalid_quantity",
        ),
        (
            json!({ "items": [{ "product_id": id, "quantity": -3 }] }),
            "invalid_quantity",
        ),
        (
            json!({ "items": [{ "product_id": id, "quantity": 101 }] }),
            "invalid_quantity",
        ),
        (
            json!({ "items": [{ "product_id": Uuid::new_v4(), "quantity": 1 }] }),
            "product_unavailable",
        ),
    ];
    for (i, (cart, error)) in cases.into_iter().enumerate() {
        let reply = buy(&mut shop.student, cart, &format!("bad-{i}")).await;
        assert_eq!(reply.status, StatusCode::UNPROCESSABLE_ENTITY, "{error}");
        assert_eq!(reply.error(), error);
    }

    // A product withdrawn from sale cannot be bought, even with a stale catalogue.
    shop.manager
        .patch(
            &format!("/api/shop/manage/products/{id}"),
            json!({ "active": false }),
        )
        .await;
    let reply = shop.buy(1, "withdrawn").await;
    assert_eq!(reply.error(), "product_unavailable");

    assert_eq!(shop.student_balance(), 50_000);
    assert_eq!(shop.stock().await, 10);
    assert_eq!(shop.count("SELECT count(*) FROM orders").await, 0);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn cart_with_several_products_is_one_payment(db: PgPool) {
    let mut shop = Shop::open(db, 10_000, 1_500, 10).await;
    let sticker = shop
        .manager
        .post(
            "/api/shop/manage/products",
            json!({ "name": "Sticker", "price": 200, "stock": 50 }),
        )
        .await;
    let sticker_id = sticker.body["id"].as_str().unwrap().to_owned();
    let cart = json!({ "items": [
        { "product_id": shop.product_id(), "quantity": 1 },
        { "product_id": sticker_id, "quantity": 2 },
        // The same product twice is merged.
        { "product_id": sticker_id, "quantity": 3 },
    ]});

    let reply = buy(&mut shop.student, cart, "basket").await;
    assert_eq!(reply.status, StatusCode::CREATED, "{:?}", reply.body);
    assert_eq!(reply.body["total"], 1_500 + 5 * 200);
    assert_eq!(reply.body["items"].as_array().unwrap().len(), 2);
    assert_eq!(shop.student_balance(), 10_000 - 2_500);
    assert_eq!(
        shop.count("SELECT count(*) FROM chain_txs WHERE kind = 'purchase'")
            .await,
        1
    );
    let stock: i32 = sqlx::query_scalar("SELECT stock FROM products WHERE name = 'Sticker'")
        .fetch_one(&shop.app.db)
        .await
        .unwrap();
    assert_eq!(stock, 45);

    // A later price change does not rewrite the order.
    shop.manager
        .patch(
            &format!("/api/shop/manage/products/{sticker_id}"),
            json!({ "price": 999, "name": "Sticker collector" }),
        )
        .await;
    let order = shop
        .student
        .get(&format!(
            "/api/shop/orders/{}",
            reply.body["id"].as_str().unwrap()
        ))
        .await;
    assert_eq!(order.body["total"], 2_500);
    let line = order.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["product_id"] == sticker_id)
        .unwrap();
    assert_eq!(line["unit_price"], 200);
    assert_eq!(line["name"], "Sticker");

    // A product that appears in an order cannot be deleted, only withdrawn.
    let reply = shop
        .manager
        .delete(&format!("/api/shop/manage/products/{sticker_id}"))
        .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.error(), "product_in_orders");
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn orders_are_private_to_their_buyer(db: PgPool) {
    let mut shop = Shop::open(db, 5_000, 1_500, 10).await;
    let reply = shop.buy(1, "mine").await;
    let order_id = reply.body["id"].as_str().unwrap().to_owned();

    let (mut other, _) = shop.app.login_as(Role::Student).await;
    assert_eq!(
        other
            .get(&format!("/api/shop/orders/{order_id}"))
            .await
            .status,
        StatusCode::NOT_FOUND
    );
    assert_eq!(other.get("/api/shop/orders").await.body["total"], 0);
    // The same idempotency key used by someone else is unrelated.
    shop.fund(
        Uuid::parse_str(
            other.get("/api/auth/me").await.body["user"]["id"]
                .as_str()
                .unwrap(),
        )
        .unwrap(),
        5_000,
    )
    .await;
    let theirs = buy(&mut other, shop.cart(1), "mine").await;
    assert_eq!(theirs.status, StatusCode::CREATED);
    assert_ne!(theirs.body["id"], order_id);
}

// --- Lifecycle -------------------------------------------------------------------------

async fn set_status(manager: &mut Client, order_id: &str, status: &str) -> Reply {
    manager
        .post(
            &format!("/api/shop/manage/orders/{order_id}/status"),
            json!({ "status": status }),
        )
        .await
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn order_goes_from_pending_to_ready_to_delivered(db: PgPool) {
    let mut shop = Shop::open(db, 5_000, 1_500, 10).await;
    let order = shop.buy(1, "o").await;
    let id = order.body["id"].as_str().unwrap().to_owned();

    // Steps cannot be skipped or invented.
    for bad in ["delivered", "cancelled", "pending", "failed", "nonsense"] {
        let reply = set_status(&mut shop.manager, &id, bad).await;
        assert_eq!(reply.status, StatusCode::CONFLICT, "{bad}");
        assert_eq!(reply.error(), "invalid_transition");
    }

    let ready = set_status(&mut shop.manager, &id, "ready").await;
    assert_eq!(ready.status, StatusCode::OK);
    assert_eq!(ready.body["status"], "ready");
    assert_eq!(
        shop.student
            .get(&format!("/api/shop/orders/{id}"))
            .await
            .body["status"],
        "ready"
    );

    let delivered = set_status(&mut shop.manager, &id, "delivered").await;
    assert_eq!(delivered.body["status"], "delivered");

    // Delivered is final: no going back, no refund.
    assert_eq!(
        set_status(&mut shop.manager, &id, "ready").await.status,
        StatusCode::CONFLICT
    );
    let cancel = shop
        .manager
        .post(&format!("/api/shop/manage/orders/{id}/cancel"), json!({}))
        .await;
    assert_eq!(cancel.status, StatusCode::CONFLICT);
    assert_eq!(shop.student_balance(), 3_500);

    let missing = set_status(&mut shop.manager, &Uuid::new_v4().to_string(), "ready").await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);

    assert_eq!(
        shop.count("SELECT count(*) FROM audit_log WHERE action = 'shop.order_status_changed'")
            .await,
        2
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn cancelling_an_order_refunds_on_chain(db: PgPool) {
    let mut shop = Shop::open(db, 5_000, 1_500, 10).await;
    let order = shop.buy(2, "o").await;
    let id = order.body["id"].as_str().unwrap().to_owned();
    assert_eq!(shop.student_balance(), 2_000);
    assert_eq!(shop.stock().await, 8);
    let supply = shop.app.node.status().circulating_supply;

    let reply = shop
        .manager
        .post(&format!("/api/shop/manage/orders/{id}/cancel"), json!({}))
        .await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(reply.body["status"], "cancelled");

    // The student has their EPC back, the treasury gave them up, the items are back on
    // the shelf, and no EPC was created or destroyed in the process.
    assert_eq!(shop.student_balance(), 5_000);
    assert_eq!(shop.treasury_balance(), 0);
    assert_eq!(shop.stock().await, 10);
    assert_eq!(shop.app.node.status().circulating_supply, supply);

    // On chain: a Refund that points at the original Purchase.
    let purchase: String = sqlx::query_scalar("SELECT hash FROM chain_txs WHERE kind = 'purchase'")
        .fetch_one(&shop.app.db)
        .await
        .unwrap();
    let refund = shop.app.node.recent_txs(1).unwrap().remove(0).tx.unwrap();
    assert_eq!(refund.from, shop.app.treasury_key.address());
    assert!(matches!(
        refund.payload,
        TxPayload::Refund { purchase: hash } if hash.to_string() == purchase
    ));

    // The student sees both movements.
    let history = shop.student.get("/api/me/transactions").await;
    let kinds: Vec<(&str, &str)> = history.body["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["kind"].as_str().unwrap(),
                l["direction"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        kinds,
        vec![
            ("refund", "credit"),
            ("purchase", "debit"),
            ("mint", "credit")
        ]
    );

    // Refunding twice is impossible: by the API…
    let again = shop
        .manager
        .post(&format!("/api/shop/manage/orders/{id}/cancel"), json!({}))
        .await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(shop.student_balance(), 5_000);
    // …and audited.
    assert_eq!(
        shop.count("SELECT count(*) FROM audit_log WHERE action = 'shop.order_refunded'")
            .await,
        1
    );
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn a_ready_order_can_still_be_cancelled(db: PgPool) {
    let mut shop = Shop::open(db, 5_000, 1_500, 10).await;
    let order = shop.buy(1, "o").await;
    let id = order.body["id"].as_str().unwrap().to_owned();
    set_status(&mut shop.manager, &id, "ready").await;

    let reply = shop
        .manager
        .post(&format!("/api/shop/manage/orders/{id}/cancel"), json!({}))
        .await;
    assert_eq!(reply.body["status"], "cancelled");
    assert_eq!(shop.student_balance(), 5_000);
    assert_eq!(shop.stock().await, 10);
    // A cancelled order cannot be revived.
    assert_eq!(
        set_status(&mut shop.manager, &id, "ready").await.status,
        StatusCode::CONFLICT
    );
}

// --- Crash safety ----------------------------------------------------------------------

/// Reproduces exactly what `checkout` commits in step 1, as if the API had stopped right
/// after: order awaiting payment, stock taken, signed Purchase recorded, nothing sent.
async fn interrupted_checkout(shop: &Shop, quantity: i32, buyer_nonce: u64) -> (Uuid, Transaction) {
    let order_id = Uuid::new_v4();
    let product_id = Uuid::parse_str(shop.product_id()).unwrap();
    let price = shop.product["price"].as_i64().unwrap();
    let total = price * i64::from(quantity);
    let keypair = epicoin_api::wallets::load_keypair(
        &shop.app.db,
        &shop.app.state.vault,
        shop.student_user.id,
    )
    .await
    .unwrap();
    let payment = Transaction::sign(
        &shop.app.genesis.chain_id,
        &keypair,
        buyer_nonce,
        TxPayload::Purchase {
            amount: total as u64,
            order_ref: order_ref(order_id),
        },
    );

    let mut tx = shop.app.db.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO orders (id, user_id, idempotency_key, status, total) \
         VALUES ($1, $2, $3, 'awaiting_payment', $4)",
    )
    .bind(order_id)
    .bind(shop.student_user.id)
    .bind(order_id.to_string())
    .bind(total)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query("UPDATE products SET stock = stock - $2 WHERE id = $1")
        .bind(product_id)
        .bind(quantity)
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO order_items (order_id, product_id, product_name, unit_price, quantity) \
         VALUES ($1, $2, 'Mug EpiCoin', $3, $4)",
    )
    .bind(order_id)
    .bind(product_id)
    .bind(price)
    .bind(quantity)
    .execute(&mut *tx)
    .await
    .unwrap();
    record_tx(
        &mut tx,
        &payment,
        &TxMeta {
            actor_user_id: Some(shop.student_user.id),
            subject_user_id: Some(shop.student_user.id),
            amount: Some(total as u64),
            order_id: Some(order_id),
            ..TxMeta::default()
        },
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    (order_id, payment)
}

async fn order_status(shop: &Shop, id: Uuid) -> (String, Option<String>) {
    sqlx::query_as("SELECT status, failure FROM orders WHERE id = $1")
        .bind(id)
        .fetch_one(&shop.app.db)
        .await
        .unwrap()
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn a_checkout_interrupted_before_submission_completes_later(db: PgPool) {
    let shop = Shop::open(db, 5_000, 1_500, 10).await;
    let (order_id, _) = interrupted_checkout(&shop, 2, 0).await;
    assert_eq!(order_status(&shop, order_id).await.0, "awaiting_payment");
    assert_eq!(shop.student_balance(), 5_000, "not debited yet");
    assert_eq!(shop.stock().await, 8, "stock is held meanwhile");

    // The reconciler delivers the stored payment…
    shop.app
        .state
        .chain
        .reconcile(Duration::ZERO)
        .await
        .unwrap();
    shop.app
        .eventually("the payment on chain", || shop.student_balance() == 2_000)
        .await;
    // …and settles the order.
    shop.app
        .state
        .chain
        .reconcile(Duration::ZERO)
        .await
        .unwrap();
    assert_eq!(order_status(&shop, order_id).await.0, "pending");
    assert_eq!(shop.treasury_balance(), 3_000);
    assert_eq!(shop.stock().await, 8);

    // Running it again and again never pays twice.
    for _ in 0..3 {
        shop.app
            .state
            .chain
            .reconcile(Duration::ZERO)
            .await
            .unwrap();
    }
    assert_eq!(shop.student_balance(), 2_000);
    assert_eq!(shop.treasury_balance(), 3_000);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn an_interrupted_checkout_that_cannot_be_paid_releases_its_stock(db: PgPool) {
    // The student can afford one mug, not four.
    let shop = Shop::open(db, 2_000, 1_500, 10).await;
    let (order_id, _) = interrupted_checkout(&shop, 4, 0).await;
    assert_eq!(shop.stock().await, 6);

    shop.app
        .state
        .chain
        .reconcile(Duration::ZERO)
        .await
        .unwrap();

    let (status, failure) = order_status(&shop, order_id).await;
    assert_eq!(status, "failed");
    assert_eq!(failure.as_deref(), Some("insufficient_balance"));
    assert_eq!(shop.stock().await, 10, "the stock is given back");
    assert_eq!(shop.student_balance(), 2_000, "nothing was debited");
    assert_eq!(shop.treasury_balance(), 0);

    // Settling is idempotent: the stock is not given back twice.
    shop.app
        .state
        .chain
        .reconcile(Duration::ZERO)
        .await
        .unwrap();
    assert_eq!(shop.stock().await, 10);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn a_retry_of_an_interrupted_checkout_does_not_pay_again(db: PgPool) {
    let mut shop = Shop::open(db, 5_000, 1_500, 10).await;
    let (order_id, payment) = interrupted_checkout(&shop, 1, 0).await;
    // The payment did reach the node, but the API never saw the answer.
    shop.app.node.submit(payment).unwrap();

    // The user, seeing no confirmation, tries again with the same key.
    let reply = shop.buy(1, &order_id.to_string()).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.body);
    assert_eq!(reply.body["id"], order_id.to_string());
    assert_eq!(reply.body["status"], "pending");

    assert_eq!(shop.student_balance(), 3_500, "debited exactly once");
    assert_eq!(shop.stock().await, 9);
    assert_eq!(shop.count("SELECT count(*) FROM orders").await, 1);
}

// --- Around the shop -------------------------------------------------------------------

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn the_chain_learns_nothing_about_orders(db: PgPool) {
    let mut shop = Shop::open(db, 5_000, 1_500, 10).await;
    let order = shop.buy(1, "o").await;
    let order_id = Uuid::parse_str(order.body["id"].as_str().unwrap()).unwrap();

    let mut chain_dump = String::new();
    for block in shop.app.node.blocks(1, 1_000).unwrap() {
        chain_dump.push_str(&serde_json::to_string(&block).unwrap());
        chain_dump.push_str(&hex::encode(block.to_bytes()));
        chain_dump.push_str(&String::from_utf8_lossy(&block.to_bytes()));
    }
    // The purchase is there, under an opaque reference…
    assert!(chain_dump.contains(&order_ref(order_id).to_string()));
    // …but neither the order id, the product, nor the buyer's identity.
    for private in [
        order_id.to_string(),
        order_id.simple().to_string(),
        hex::encode(order_id.as_bytes()),
        "Mug".to_owned(),
        shop.product_id().to_owned(),
        shop.student_user.email.clone(),
        shop.student_user.id.to_string(),
    ] {
        assert!(
            !chain_dump.contains(&private),
            "the chain contains {private:?}"
        );
    }
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn dashboard_reports_shop_sales(db: PgPool) {
    let mut shop = Shop::open(db, 10_000, 1_500, 10).await;
    shop.buy(2, "kept").await;
    let cancelled = shop.buy(1, "cancelled").await;
    shop.manager
        .post(
            &format!(
                "/api/shop/manage/orders/{}/cancel",
                cancelled.body["id"].as_str().unwrap()
            ),
            json!({}),
        )
        .await;

    let (mut admin, _) = shop.app.login_as(Role::Admin).await;
    let stats = admin.get("/api/admin/stats").await;
    assert_eq!(stats.status, StatusCode::OK, "{:?}", stats.body);
    let totals = &stats.body["shop"]["totals"];
    assert_eq!(totals["orders"], 1);
    assert_eq!(totals["revenue"], 3_000);
    assert_eq!(totals["refunded_orders"], 1);
    assert_eq!(totals["refunded"], 1_500);
    assert_eq!(stats.body["shop"]["by_product"][0]["name"], "Mug EpiCoin");
    assert_eq!(stats.body["shop"]["by_product"][0]["quantity"], 2);
    assert_eq!(stats.body["shop"]["by_product"][0]["revenue"], 3_000);
    // The treasury holds exactly the revenue.
    assert_eq!(stats.body["supply"]["treasury_balance"], 3_000);
    assert_eq!(stats.body["supply"]["held_by_users"], 7_000);
}

#[sqlx::test(migrator = "epicoin_api::MIGRATOR")]
async fn deleting_a_buyer_keeps_the_order_without_their_identity(db: PgPool) {
    let mut shop = Shop::open(db, 5_000, 1_500, 10).await;
    shop.buy(1, "o").await;
    let (mut admin, _) = shop.app.login_as(Role::Admin).await;
    let reply = admin
        .delete(&format!("/api/admin/users/{}", shop.student_user.id))
        .await;
    assert_eq!(reply.status, StatusCode::NO_CONTENT);

    let queue = shop.manager.get("/api/shop/manage/orders").await;
    assert_eq!(queue.body["total"], 1);
    assert!(queue.body["items"][0].get("buyer").is_none());
    assert_eq!(queue.body["items"][0]["total"], 1_500);
    let dump: String = sqlx::query_scalar("SELECT string_agg(o::text, ' ') FROM orders o")
        .fetch_one(&shop.app.db)
        .await
        .unwrap();
    assert!(!dump.contains(&shop.student_user.id.to_string()));
}
