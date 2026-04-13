//! HTTP API routes for mesh sync and peer management.
//! - GET  /api/mesh           — mesh status (peers online, sync stats)
//! - GET  /api/mesh/peers     — list peers from heartbeat table
//! - GET  /api/sync/export    — export changes since timestamp (query: table, since)
//! - POST /api/sync/import    — apply SyncChange[] from a peer
//! - GET  /api/sync/status    — sync metadata per peer
//! - POST /api/heartbeat      — receive heartbeat from a peer

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::{get, post};
use axum::Router;
use rusqlite::params;
use serde::Deserialize;
use serde_json::json;

use convergio_db::pool::ConnPool;

use crate::routes_sync_repo::handle_sync_repo;
use crate::sync_apply;
use crate::types::SyncChange;

/// Maximum age (seconds) for HMAC-signed requests to prevent replay attacks.
const HMAC_MAX_AGE_SECS: i64 = 300;

pub struct MeshState {
    pub pool: ConnPool,
    /// Pre-loaded shared secret for HMAC verification (from peers.conf).
    pub shared_secret: Option<Vec<u8>>,
}

pub fn mesh_routes(state: Arc<MeshState>) -> Router {
    Router::new()
        .route("/api/mesh", get(handle_status))
        .route("/api/mesh/peers", get(handle_peers))
        .route("/api/mesh/sync-repo", post(handle_sync_repo))
        .route("/api/node/readiness", get(handle_node_readiness))
        .route("/api/sync/export", get(handle_export))
        .route("/api/sync/import", post(handle_import))
        .route("/api/sync/status", get(handle_sync_status))
        .route("/api/heartbeat", post(handle_heartbeat))
        .with_state(state)
}

async fn handle_status(State(state): State<Arc<MeshState>>) -> Json<serde_json::Value> {
    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(e) => return Json(json!({"error": e.to_string()})),
    };
    let peers_online: u64 = conn
        .query_row(
            "SELECT count(*) FROM peer_heartbeats \
             WHERE last_seen > unixepoch() - 600",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    // +1 for local node (doesn't heartbeat itself)
    let peers_online = peers_online + 1;
    let total_synced: u64 = conn
        .query_row(
            "SELECT COALESCE(SUM(total_applied), 0) FROM mesh_sync_stats",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    Json(json!({
        "peers_online": peers_online,
        "total_synced": total_synced,
    }))
}

async fn handle_peers(State(state): State<Arc<MeshState>>) -> Json<serde_json::Value> {
    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(e) => return Json(json!({"error": e.to_string()})),
    };
    let mut stmt = match conn.prepare(
        "SELECT peer_name, last_seen, version, \
         CASE WHEN last_seen > unixepoch() - 600 THEN 'online' \
         ELSE 'offline' END as status, role \
         FROM peer_heartbeats ORDER BY peer_name",
    ) {
        Ok(s) => s,
        Err(e) => return Json(json!({"error": e.to_string()})),
    };
    let mut peers: Vec<serde_json::Value> = match stmt.query_map([], |row| {
        Ok(json!({
            "peer": row.get::<_, String>(0)?,
            "last_seen": row.get::<_, i64>(1)?,
            "version": row.get::<_, Option<String>>(2)?,
            "status": row.get::<_, String>(3)?,
            "role": row.get::<_, Option<String>>(4)?,
        }))
    }) {
        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
        Err(_) => vec![],
    };

    // Inject local node as always-online (it doesn't heartbeat itself)
    let local_name = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".into());
    let local_role = std::env::var("CONVERGIO_NODE_ROLE").unwrap_or_default();
    let is_local = |name: &str| -> bool {
        name == local_name
            || name.starts_with(&format!("{local_name}."))
            || local_name.starts_with(&format!("{name}."))
    };
    let already_listed = peers.iter().any(|p| {
        p.get("peer")
            .and_then(|v| v.as_str())
            .map(&is_local)
            .unwrap_or(false)
    });
    if !already_listed {
        peers.insert(
            0,
            json!({
                "peer": local_name,
                "last_seen": chrono::Utc::now().timestamp(),
                "version": env!("CARGO_PKG_VERSION"),
                "status": "online",
                "role": local_role,
            }),
        );
    } else {
        // Fix stale self-entry: force it online with current version
        for p in &mut peers {
            let is_self = p
                .get("peer")
                .and_then(|v| v.as_str())
                .map(&is_local)
                .unwrap_or(false);
            if is_self {
                p["status"] = json!("online");
                p["version"] = json!(env!("CARGO_PKG_VERSION"));
                p["last_seen"] = json!(chrono::Utc::now().timestamp());
            }
        }
    }

    Json(json!(peers))
}

#[derive(Debug, Deserialize)]
pub struct ExportQuery {
    pub table: String,
    pub since: Option<String>,
}

async fn handle_export(
    State(state): State<Arc<MeshState>>,
    Query(params): Query<ExportQuery>,
) -> Response {
    // SECURITY: only allow exporting from the SYNC_TABLES allowlist
    if !crate::types::SYNC_TABLES.contains(&params.table.as_str()) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "table not in sync allowlist"})),
        )
            .into_response();
    }
    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(e) => return Json(json!({"error": e.to_string()})).into_response(),
    };
    match sync_apply::export_changes_since(&conn, &params.table, params.since.as_deref()) {
        Ok(changes) => Json(json!({"changes": changes})).into_response(),
        Err(e) => Json(json!({"error": e.to_string()})).into_response(),
    }
}

/// Wrapper for the import payload sent by `send_changes_to_peer`.
#[derive(Debug, Deserialize)]
struct ImportPayload {
    changes: Vec<SyncChange>,
}

async fn handle_import(
    State(state): State<Arc<MeshState>>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    // HMAC verification: if shared secret is configured, require valid signature
    if let Some(secret) = &state.shared_secret {
        let sig_header = headers
            .get("x-mesh-signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let timestamp = headers
            .get("x-mesh-timestamp")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        // Reject stale timestamps to prevent replay attacks
        if let Ok(ts) = timestamp.parse::<i64>() {
            let now = chrono::Utc::now().timestamp();
            if (now - ts).abs() > HMAC_MAX_AGE_SECS {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "HMAC timestamp expired"})),
                )
                    .into_response();
            }
        }
        let body_hash_header = headers
            .get("x-mesh-body-hash")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        // Reconstruct the signed message: timestamp:method:path:bodyhash
        let message = if body_hash_header.is_empty() {
            format!("{timestamp}:POST:/api/sync/import")
        } else {
            format!("{timestamp}:POST:/api/sync/import:{body_hash_header}")
        };
        let sig_bytes = hex::decode(sig_header).unwrap_or_default();
        match crate::auth::verify_hmac(secret, message.as_bytes(), &sig_bytes) {
            Ok(true) => {}
            _ => {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "HMAC verification failed"})),
                )
                    .into_response()
            }
        }
    }
    let payload: ImportPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => return Json(json!({"error": format!("invalid JSON: {e}")})).into_response(),
    };
    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(e) => return Json(json!({"error": e.to_string()})).into_response(),
    };
    match sync_apply::apply_changes(&conn, &payload.changes) {
        Ok(applied) => Json(json!({"applied": applied})).into_response(),
        Err(e) => Json(json!({"error": e.to_string()})).into_response(),
    }
}

async fn handle_sync_status(State(state): State<Arc<MeshState>>) -> Json<serde_json::Value> {
    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(e) => return Json(json!({"error": e.to_string()})),
    };
    let mut stmt = match conn.prepare(
        "SELECT peer_name, total_applied, last_sync_at, \
         last_latency_ms, consecutive_failures \
         FROM mesh_sync_stats ORDER BY peer_name",
    ) {
        Ok(s) => s,
        Err(e) => return Json(json!({"error": e.to_string()})),
    };
    let rows: Vec<serde_json::Value> = match stmt.query_map([], |row| {
        Ok(json!({
            "peer": row.get::<_, String>(0)?,
            "total_applied": row.get::<_, i64>(1)?,
            "last_sync_at": row.get::<_, Option<String>>(2)?,
            "latency_ms": row.get::<_, Option<i64>>(3)?,
            "failures": row.get::<_, i64>(4)?,
        }))
    }) {
        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
        Err(_) => vec![],
    };
    Json(json!(rows))
}

/// Node readiness: aggregates mesh peers, sync stats, and DB health.
async fn handle_node_readiness(State(state): State<Arc<MeshState>>) -> Json<serde_json::Value> {
    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(e) => return Json(json!({"ready": false, "error": e.to_string()})),
    };
    let peers_online: u64 = conn
        .query_row(
            "SELECT count(*) FROM peer_heartbeats WHERE last_seen > unixepoch() - 600",
            [],
            |r| r.get(0),
        )
        .unwrap_or(0);
    let db_ok = conn
        .query_row("SELECT 1", [], |r| r.get::<_, i64>(0))
        .is_ok();
    let sync_healthy: bool = conn
        .query_row(
            "SELECT COALESCE(MAX(consecutive_failures), 0) FROM mesh_sync_stats",
            [],
            |r| r.get::<_, i64>(0),
        )
        .map(|f| f < 5)
        .unwrap_or(true);
    let ready = db_ok && sync_healthy;
    Json(json!({
        "ready": ready,
        "checks": {
            "database": if db_ok { "ok" } else { "down" },
            "mesh_peers": peers_online,
            "sync_healthy": sync_healthy,
        }
    }))
}

#[derive(Debug, Deserialize)]
pub struct HeartbeatRequest {
    pub peer: String,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub capabilities: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
}

async fn handle_heartbeat(
    State(state): State<Arc<MeshState>>,
    headers: axum::http::HeaderMap,
    raw_body: axum::body::Bytes,
) -> Response {
    // HMAC verification: if shared secret is configured, require valid signature
    if let Some(secret) = &state.shared_secret {
        let sig_header = headers
            .get("x-mesh-signature")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let timestamp = headers
            .get("x-mesh-timestamp")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        // Reject stale timestamps to prevent replay attacks
        if let Ok(ts) = timestamp.parse::<i64>() {
            let now = chrono::Utc::now().timestamp();
            if (now - ts).abs() > HMAC_MAX_AGE_SECS {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "HMAC timestamp expired"})),
                )
                    .into_response();
            }
        }
        let body_hash_header = headers
            .get("x-mesh-body-hash")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let message = if body_hash_header.is_empty() {
            format!("{timestamp}:POST:/api/heartbeat")
        } else {
            format!("{timestamp}:POST:/api/heartbeat:{body_hash_header}")
        };
        let sig_bytes = hex::decode(sig_header).unwrap_or_default();
        match crate::auth::verify_hmac(secret, message.as_bytes(), &sig_bytes) {
            Ok(true) => {}
            _ => {
                return (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "HMAC verification failed"})),
                )
                    .into_response()
            }
        }
    }
    let body: HeartbeatRequest = match serde_json::from_slice(&raw_body) {
        Ok(b) => b,
        Err(e) => return Json(json!({"error": format!("invalid JSON: {e}")})).into_response(),
    };
    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(e) => return Json(json!({"error": e.to_string()})).into_response(),
    };
    match conn.execute(
        "INSERT INTO peer_heartbeats (peer_name, last_seen, version, capabilities, role) \
         VALUES (?1, unixepoch(), ?2, ?3, ?4) \
         ON CONFLICT(peer_name) DO UPDATE SET \
         last_seen = unixepoch(), version = excluded.version, \
         capabilities = excluded.capabilities, role = excluded.role",
        params![body.peer, body.version, body.capabilities, body.role],
    ) {
        Ok(_) => Json(json!({"status": "ok"})).into_response(),
        Err(e) => Json(json!({"error": e.to_string()})).into_response(),
    }
}
