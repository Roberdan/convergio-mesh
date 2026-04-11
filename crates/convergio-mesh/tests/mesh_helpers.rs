//! Shared test helpers for mesh E2E tests.

use axum::body::Body;
use axum::http::Request;
use convergio_db::pool::ConnPool;
use convergio_mesh::routes_role_config::role_config_routes;
use convergio_mesh::schema;

pub fn setup() -> (axum::Router, ConnPool) {
    let pool = convergio_db::pool::create_memory_pool().unwrap();
    let conn = pool.get().unwrap();
    convergio_db::migration::ensure_registry(&conn).unwrap();
    convergio_db::migration::apply_migrations(&conn, "mesh", &schema::migrations()).unwrap();
    drop(conn);
    let app = role_config_routes(pool.clone());
    (app, pool)
}

pub fn rebuild(pool: &ConnPool) -> axum::Router {
    role_config_routes(pool.clone())
}

pub async fn body_json(resp: axum::http::Response<Body>) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

pub fn post_json(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .body(Body::from(body.to_owned()))
        .unwrap()
}

pub fn get_req(uri: &str) -> Request<Body> {
    Request::builder().uri(uri).body(Body::empty()).unwrap()
}
