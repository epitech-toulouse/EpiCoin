//! The shop: a catalogue of products, orders paid with an on-chain `Purchase`, and
//! cancellations refunded with an on-chain `Refund`.
//!
//! - [`products`]: catalogue and its management (shop managers).
//! - [`orders`]: checkout, order lifecycle, refunds.

pub mod orders;
pub mod products;

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, post};
use axum::Router;

use crate::models::Role;
use crate::state::AppState;

/// Who can manage the shop. Administrators can stand in for the shop manager.
pub const MANAGERS: [Role; 2] = [Role::ShopManager, Role::Admin];

pub fn router() -> Router<AppState> {
    Router::new()
        // Everyone logged in.
        .route("/products", get(products::catalogue))
        .route("/images/{file}", get(products::image))
        // Students.
        .route("/orders", get(orders::mine).post(orders::checkout))
        .route("/orders/{id}", get(orders::mine_one))
        // Shop managers.
        .route(
            "/manage/products",
            get(products::list_all).post(products::create),
        )
        .route(
            "/manage/products/{id}",
            axum::routing::patch(products::update).delete(products::delete),
        )
        .route(
            "/manage/products/{id}/image",
            post(products::upload_image)
                .layer(DefaultBodyLimit::max(products::MAX_IMAGE_BYTES + 1024)),
        )
        .route("/manage/orders", get(orders::queue))
        .route("/manage/orders/{id}/status", post(orders::set_status))
        .route("/manage/orders/{id}/cancel", post(orders::cancel))
}
