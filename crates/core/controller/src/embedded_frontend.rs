//! Serves the SvelteKit frontend embedded at compile time via `rust-embed`.
//!
//! Only compiled when the `embed-frontend` Cargo feature is enabled.
//! The entire `frontend/build/` directory is baked into the binary's
//! read-only data section, producing a single self-contained executable.

use axum::Router;
use axum::body::Body;
use axum::http::{HeaderValue, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

/// Embedded SvelteKit build output.
#[derive(RustEmbed)]
#[folder = "../../../frontend/build"]
struct Assets;

/// Build an axum [`Router`] that serves embedded frontend assets.
///
/// - `/api` and `/api/{*path}` return 404 (same as the filesystem path).
/// - Known static files are served with the correct `Content-Type`.
/// - Immutable assets (`_app/immutable/*`) get a one-year cache header.
/// - `index.html` is served with `Cache-Control: no-cache`.
/// - Unmatched non-API paths fall back to `index.html` (SPA routing).
pub fn router() -> Router {
    let api_not_found = Router::new()
        .route(
            "/api/{*path}",
            axum::routing::any(uptrakit_web_api::api_not_found),
        )
        .route("/api", axum::routing::any(uptrakit_web_api::api_not_found));

    api_not_found.fallback(serve_embedded)
}

/// Serve an embedded asset or fall back to `index.html` for SPA routing.
async fn serve_embedded(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');

    // Try the exact path first.
    if let Some(response) = serve_asset(path) {
        return response;
    }

    // SPA fallback: serve index.html for unknown paths.
    serve_asset("index.html").unwrap_or_else(|| StatusCode::NOT_FOUND.into_response())
}

/// Look up an embedded file by path and build a response with appropriate
/// headers. Returns `None` if the file is not embedded.
fn serve_asset(path: &str) -> Option<Response> {
    let asset = Assets::get(path)?;

    let content_type = asset.metadata.mimetype();

    let mut response = Response::builder().header(header::CONTENT_TYPE, content_type);

    // SvelteKit puts fingerprinted assets under `_app/immutable/`.
    // These can be cached indefinitely.
    if path.starts_with("_app/immutable/") {
        response = response.header(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=31536000, immutable"),
        );
    } else if path == "index.html" {
        response = response.header(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    }

    // `rust-embed` returns `Cow<'static, [u8]>`: borrowed (`&'static [u8]`)
    // in release builds, owned (`Vec<u8>`) in debug builds (filesystem reads).
    // Use `Bytes::from_static` for the borrowed case to avoid heap allocation.
    let body = match asset.data {
        std::borrow::Cow::Borrowed(data) => Body::from(axum::body::Bytes::from_static(data)),
        std::borrow::Cow::Owned(data) => Body::from(data),
    };

    // The builder only fails on invalid header names/values, which we control.
    // Using `ok()` to avoid `unwrap()`.
    response.body(body).ok()
}
