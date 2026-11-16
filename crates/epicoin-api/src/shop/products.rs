//! The catalogue: products, their stock and their images.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::json;
use time::OffsetDateTime;
use uuid::Uuid;

use super::MANAGERS;
use crate::audit;
use crate::auth::CurrentUser;
use crate::error::{ApiError, ApiResult};
use crate::state::AppState;

pub const MAX_IMAGE_BYTES: usize = 2 * 1024 * 1024;
const MAX_NAME_CHARS: usize = 100;
const MAX_DESCRIPTION_CHARS: usize = 2_000;
/// 10 million EPC: far beyond any real price, and safely within `i64`.
const MAX_PRICE: i64 = 1_000_000_000;
const MAX_STOCK: i32 = 1_000_000;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Product {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub image_file: Option<String>,
    pub price: i64,
    pub stock: i32,
    pub active: bool,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProductView {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    /// Path of the image, relative to the site root, if the product has one.
    pub image_url: Option<String>,
    /// In units (1 EPC = 100).
    pub price: i64,
    pub stock: i32,
    pub active: bool,
}

impl From<Product> for ProductView {
    fn from(product: Product) -> Self {
        Self {
            id: product.id,
            name: product.name,
            description: product.description,
            image_url: product
                .image_file
                .map(|file| format!("/api/shop/images/{file}")),
            price: product.price,
            stock: product.stock,
            active: product.active,
        }
    }
}

fn views(products: Vec<Product>) -> Json<Vec<ProductView>> {
    Json(products.into_iter().map(Into::into).collect())
}

/// What students see: the products on sale.
pub async fn catalogue(
    State(state): State<AppState>,
    _current: CurrentUser,
) -> ApiResult<Json<Vec<ProductView>>> {
    let products =
        sqlx::query_as::<_, Product>("SELECT * FROM products WHERE active ORDER BY name, id")
            .fetch_all(&state.db)
            .await?;
    Ok(views(products))
}

/// What managers see: every product, including those withdrawn from sale.
pub async fn list_all(
    State(state): State<AppState>,
    current: CurrentUser,
) -> ApiResult<Json<Vec<ProductView>>> {
    current.require(&MANAGERS)?;
    let products = sqlx::query_as::<_, Product>("SELECT * FROM products ORDER BY name, id")
        .fetch_all(&state.db)
        .await?;
    Ok(views(products))
}

fn check_name(name: &str) -> ApiResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(invalid(format!(
            "Le nom est obligatoire ({MAX_NAME_CHARS} caractères maximum)."
        )));
    }
    Ok(name.to_owned())
}

fn check_description(description: &str) -> ApiResult<String> {
    let description = description.trim();
    if description.chars().count() > MAX_DESCRIPTION_CHARS {
        return Err(invalid(format!(
            "La description est limitée à {MAX_DESCRIPTION_CHARS} caractères."
        )));
    }
    Ok(description.to_owned())
}

fn check_price(price: i64) -> ApiResult<i64> {
    if !(1..=MAX_PRICE).contains(&price) {
        return Err(invalid("Le prix doit être supérieur à zéro."));
    }
    Ok(price)
}

fn check_stock(stock: i32) -> ApiResult<i32> {
    if !(0..=MAX_STOCK).contains(&stock) {
        return Err(invalid("Le stock doit être compris entre 0 et 1 000 000."));
    }
    Ok(stock)
}

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::unprocessable("invalid_product", message)
}

#[derive(Deserialize)]
pub struct CreateRequest {
    name: String,
    #[serde(default)]
    description: String,
    price: i64,
    stock: i32,
    #[serde(default = "default_true")]
    active: bool,
}

fn default_true() -> bool {
    true
}

pub async fn create(
    State(state): State<AppState>,
    current: CurrentUser,
    Json(body): Json<CreateRequest>,
) -> ApiResult<(StatusCode, Json<ProductView>)> {
    current.require(&MANAGERS)?;
    let product = sqlx::query_as::<_, Product>(
        "INSERT INTO products (id, name, description, price, stock, active) \
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING *",
    )
    .bind(Uuid::new_v4())
    .bind(check_name(&body.name)?)
    .bind(check_description(&body.description)?)
    .bind(check_price(body.price)?)
    .bind(check_stock(body.stock)?)
    .bind(body.active)
    .fetch_one(&state.db)
    .await?;
    audit::record(
        &state.db,
        Some(current.id()),
        audit::action::PRODUCT_CREATED,
        Some(("product", product.id.to_string())),
        json!({ "price": product.price, "stock": product.stock }),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(product.into())))
}

/// Absent fields are left untouched.
#[derive(Deserialize)]
pub struct UpdateRequest {
    name: Option<String>,
    description: Option<String>,
    price: Option<i64>,
    stock: Option<i32>,
    active: Option<bool>,
}

pub async fn update(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateRequest>,
) -> ApiResult<Json<ProductView>> {
    current.require(&MANAGERS)?;
    let name = body.name.as_deref().map(check_name).transpose()?;
    let description = body
        .description
        .as_deref()
        .map(check_description)
        .transpose()?;
    let price = body.price.map(check_price).transpose()?;
    let stock = body.stock.map(check_stock).transpose()?;

    let product = sqlx::query_as::<_, Product>(
        "UPDATE products SET \
            name = COALESCE($2, name), description = COALESCE($3, description), \
            price = COALESCE($4, price), stock = COALESCE($5, stock), \
            active = COALESCE($6, active), updated_at = now() \
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(name)
    .bind(description)
    .bind(price)
    .bind(stock)
    .bind(body.active)
    .fetch_optional(&state.db)
    .await?
    .ok_or_else(|| ApiError::not_found("Produit"))?;
    audit::record(
        &state.db,
        Some(current.id()),
        audit::action::PRODUCT_UPDATED,
        Some(("product", id.to_string())),
        json!({ "price": body.price, "stock": body.stock, "active": body.active }),
    )
    .await?;
    Ok(Json(product.into()))
}

/// Deletes a product that was never ordered. One that appears in an order must be kept for
/// the order history: withdraw it from sale instead.
pub async fn delete(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
) -> ApiResult<StatusCode> {
    current.require(&MANAGERS)?;
    let ordered: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM order_items WHERE product_id = $1)")
            .bind(id)
            .fetch_one(&state.db)
            .await?;
    if ordered {
        return Err(ApiError::conflict(
            "product_in_orders",
            "Ce produit figure dans des commandes : désactivez-le plutôt que de le supprimer.",
        ));
    }
    let image: Option<Option<String>> =
        sqlx::query_scalar("DELETE FROM products WHERE id = $1 RETURNING image_file")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    let image = image.ok_or_else(|| ApiError::not_found("Produit"))?;
    if let Some(file) = image {
        remove_image(&state, &file).await;
    }
    audit::record(
        &state.db,
        Some(current.id()),
        audit::action::PRODUCT_DELETED,
        Some(("product", id.to_string())),
        json!({}),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

// --- Images ----------------------------------------------------------------------------

/// The format of an image, decided from its first bytes — never from what the client says.
fn image_extension(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

fn content_type(file: &str) -> Option<&'static str> {
    match file.rsplit_once('.')?.1 {
        "png" => Some("image/png"),
        "jpg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// Image files are named by the server (`<uuid>.<ext>`); anything else is not served.
fn is_image_name(file: &str) -> bool {
    match file.rsplit_once('.') {
        Some((stem, _)) => Uuid::try_parse(stem).is_ok() && content_type(file).is_some(),
        None => false,
    }
}

async fn remove_image(state: &AppState, file: &str) {
    if is_image_name(file) {
        let _ = tokio::fs::remove_file(state.settings.upload_dir.join(file)).await;
    }
}

/// Replaces the image of a product. The body is the raw image (PNG, JPEG or WebP, 2 MB max).
pub async fn upload_image(
    State(state): State<AppState>,
    current: CurrentUser,
    Path(id): Path<Uuid>,
    body: Bytes,
) -> ApiResult<Json<ProductView>> {
    current.require(&MANAGERS)?;
    if body.len() > MAX_IMAGE_BYTES {
        return Err(ApiError::new(
            StatusCode::PAYLOAD_TOO_LARGE,
            "image_too_large",
            "L'image ne doit pas dépasser 2 Mo.",
        ));
    }
    let extension = image_extension(&body).ok_or_else(|| {
        ApiError::unprocessable(
            "invalid_image",
            "Format d'image non reconnu : utilisez un fichier PNG, JPEG ou WebP.",
        )
    })?;
    let previous: Option<Option<String>> =
        sqlx::query_scalar("SELECT image_file FROM products WHERE id = $1")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    let previous = previous.ok_or_else(|| ApiError::not_found("Produit"))?;

    let file = format!("{}.{extension}", Uuid::new_v4());
    tokio::fs::create_dir_all(&state.settings.upload_dir)
        .await
        .map_err(ApiError::internal)?;
    tokio::fs::write(state.settings.upload_dir.join(&file), &body)
        .await
        .map_err(ApiError::internal)?;

    let product = sqlx::query_as::<_, Product>(
        "UPDATE products SET image_file = $2, updated_at = now() WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(&file)
    .fetch_one(&state.db)
    .await?;
    if let Some(old) = previous {
        remove_image(&state, &old).await;
    }
    Ok(Json(product.into()))
}

/// Serves a product image. Public: product pictures are not confidential, and the browser
/// fetches them from plain `<img>` tags.
pub async fn image(State(state): State<AppState>, Path(file): Path<String>) -> ApiResult<Response> {
    let not_found = || ApiError::not_found("Image");
    if !is_image_name(&file) {
        return Err(not_found());
    }
    let content_type = content_type(&file).ok_or_else(not_found)?;
    let bytes = tokio::fs::read(state.settings.upload_dir.join(&file))
        .await
        .map_err(|_| not_found())?;
    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            // File names are unique per upload: safe to cache forever.
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
        ],
        bytes,
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_format_is_sniffed_from_the_content() {
        assert_eq!(
            image_extension(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0]),
            Some("png")
        );
        assert_eq!(image_extension(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(
            image_extension(b"RIFF\x00\x00\x00\x00WEBPVP8 "),
            Some("webp")
        );
        assert_eq!(image_extension(b"<svg onload=alert(1)>"), None);
        assert_eq!(image_extension(b"<html>"), None);
        assert_eq!(image_extension(b""), None);
    }

    #[test]
    fn only_server_generated_names_are_served() {
        let good = format!("{}.png", Uuid::new_v4());
        assert!(is_image_name(&good));
        assert!(is_image_name(&good.replace(".png", ".webp")));
        for bad in [
            "../../etc/passwd",
            "..%2F..%2Fsecret.png",
            "image.png",
            "keys.env",
            ".png",
            &format!("{}.svg", Uuid::new_v4()),
            &format!("{}.png/../x", Uuid::new_v4()),
            &format!("{}", Uuid::new_v4()),
        ] {
            assert!(!is_image_name(bad), "{bad}");
        }
    }

    #[test]
    fn product_fields_are_validated() {
        assert_eq!(check_name("  Mug  ").unwrap(), "Mug");
        assert!(check_name("   ").is_err());
        assert!(check_name(&"x".repeat(101)).is_err());
        assert!(check_description(&"x".repeat(2_001)).is_err());
        assert!(check_price(0).is_err());
        assert!(check_price(-5).is_err());
        assert!(check_price(1).is_ok());
        assert!(check_price(MAX_PRICE + 1).is_err());
        assert!(check_stock(-1).is_err());
        assert!(check_stock(0).is_ok());
    }
}
