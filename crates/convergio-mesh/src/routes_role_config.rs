//! HTTP API routes for mesh node role configuration.
//!
//! - GET  /api/mesh/config/roles    — list available roles
//! - GET  /api/mesh/config/role     — get this node's current role
//! - POST /api/mesh/config/role     — set a node's role assignment
//! - GET  /api/mesh/config/topology — view all role assignments

use std::sync::Arc;

use axum::extract::State;
use axum::response::Json;
use axum::routing::get;
use axum::Router;
use rusqlite::params;
use serde::Deserialize;
use serde_json::json;

use convergio_db::pool::ConnPool;

pub struct RoleConfigState {
    pub pool: ConnPool,
}

pub fn role_config_routes(pool: ConnPool) -> Router {
    let state = Arc::new(RoleConfigState { pool });
    Router::new()
        .route("/api/mesh/config/roles", get(handle_available_roles))
        .route(
            "/api/mesh/config/role",
            get(handle_get_role).post(handle_set_role),
        )
        .route("/api/mesh/config/topology", get(handle_topology))
        .with_state(state)
}

/// GET /api/mesh/config/roles — list valid node roles.
async fn handle_available_roles() -> Json<serde_json::Value> {
    Json(json!({
        "roles": [
            {"id": "all", "description": "All extensions loaded (single-node default)"},
            {"id": "orchestrator", "description": "Plans DB, platform services, worker coordination"},
            {"id": "kernel", "description": "Local AI kernel, Telegram, voice"},
            {"id": "voice", "description": "Voice I/O only"},
            {"id": "worker", "description": "Receives delegated tasks, runs agents"},
            {"id": "nightagent", "description": "Night agent workloads (knowledge sync, nightly jobs)"},
        ]
    }))
}

#[derive(Debug, Deserialize)]
pub struct SetRoleRequest {
    pub peer_name: String,
    pub role: String,
}

const VALID_ROLES: &[&str] = &[
    "all",
    "orchestrator",
    "kernel",
    "voice",
    "worker",
    "nightagent",
];

/// POST /api/mesh/config/role — assign a role to a node.
async fn handle_set_role(
    State(state): State<Arc<RoleConfigState>>,
    Json(body): Json<SetRoleRequest>,
) -> Json<serde_json::Value> {
    if !VALID_ROLES.contains(&body.role.as_str()) {
        return Json(json!({
            "error": format!("invalid role '{}'; valid: {}", body.role,
                             VALID_ROLES.join(", "))
        }));
    }
    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(e) => return Json(json!({"error": e.to_string()})),
    };
    // Upsert into role_assignments table
    match conn.execute(
        "INSERT INTO mesh_role_assignments (peer_name, role, updated_at) \
         VALUES (?1, ?2, datetime('now')) \
         ON CONFLICT(peer_name) DO UPDATE SET \
         role = excluded.role, updated_at = datetime('now')",
        params![body.peer_name, body.role],
    ) {
        Ok(_) => Json(json!({
            "status": "assigned",
            "peer_name": body.peer_name,
            "role": body.role,
        })),
        Err(e) => Json(json!({"error": e.to_string()})),
    }
}

/// GET /api/mesh/config/role — get this node's assigned role.
async fn handle_get_role(State(state): State<Arc<RoleConfigState>>) -> Json<serde_json::Value> {
    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(e) => return Json(json!({"error": e.to_string()})),
    };
    let hostname = hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".into());
    let role = conn
        .query_row(
            "SELECT role FROM mesh_role_assignments WHERE peer_name = ?1",
            params![hostname],
            |r| r.get::<_, String>(0),
        )
        .unwrap_or_else(|_| "all".into());
    Json(json!({
        "peer_name": hostname,
        "role": role,
    }))
}

/// GET /api/mesh/config/topology — all node role assignments +
/// last heartbeat status.
async fn handle_topology(State(state): State<Arc<RoleConfigState>>) -> Json<serde_json::Value> {
    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(e) => return Json(json!({"error": e.to_string()})),
    };
    let mut stmt = match conn.prepare(
        "SELECT ra.peer_name, ra.role, ra.updated_at, \
         ph.last_seen, \
         CASE WHEN ph.last_seen > unixepoch() - 600 \
              THEN 'online' ELSE 'offline' END as status, \
         ph.role as heartbeat_role \
         FROM mesh_role_assignments ra \
         LEFT JOIN peer_heartbeats ph ON ph.peer_name = ra.peer_name \
         ORDER BY ra.peer_name",
    ) {
        Ok(s) => s,
        Err(e) => return Json(json!({"error": e.to_string()})),
    };
    let nodes: Vec<serde_json::Value> = stmt
        .query_map([], |row| {
            Ok(json!({
                "peer_name": row.get::<_, String>(0)?,
                "assigned_role": row.get::<_, String>(1)?,
                "assigned_at": row.get::<_, String>(2)?,
                "last_seen": row.get::<_, Option<i64>>(3)?,
                "status": row.get::<_, Option<String>>(4)?,
                "heartbeat_role": row.get::<_, Option<String>>(5)?,
            }))
        })
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default();
    Json(json!({"nodes": nodes, "count": nodes.len()}))
}
