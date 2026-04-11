//! HTTP API routes for convergio-mesh.

use axum::Router;

/// Returns the router for this crate's API endpoints.
pub fn routes() -> Router {
    Router::new()
    // .route("/api/mesh/health", get(health))
}
