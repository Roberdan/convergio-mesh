//! Cross-poll routes: query local or remote plan summary via mesh proxy.
//!
//! GET /api/mesh/plans/summary         — local plan count + latest plan id
//! GET /api/mesh/plans/summary?via=X   — proxy to peer X (no loop: strips via)

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use axum::Router;
use serde::Deserialize;
use serde_json::json;

use crate::peers_registry::peers_conf_path_from_env;
use crate::peers_types::PeersRegistry;
use crate::routes::MeshState;
use crate::transport::resolve_best_addr;

#[derive(Debug, Deserialize)]
struct PlanSummaryQuery {
    via: Option<String>,
}

pub fn cross_poll_routes(state: Arc<MeshState>) -> Router {
    Router::new()
        .route("/api/mesh/plans/summary", get(handle_plans_summary))
        .with_state(state)
}

async fn handle_plans_summary(
    State(state): State<Arc<MeshState>>,
    Query(params): Query<PlanSummaryQuery>,
) -> Response {
    match params.via {
        Some(ref peer_name) => proxy_to_peer(peer_name).await,
        None => local_plans_summary(&state),
    }
}

fn local_plans_summary(state: &MeshState) -> Response {
    let conn = match state.pool.get() {
        Ok(c) => c,
        Err(e) => return Json(json!({"error": e.to_string()})).into_response(),
    };
    let plan_count: i64 = conn
        .query_row("SELECT count(*) FROM plans", [], |r| r.get(0))
        .unwrap_or(0);
    let latest_plan_id: Option<i64> = conn
        .query_row("SELECT id FROM plans ORDER BY id DESC LIMIT 1", [], |r| {
            r.get(0)
        })
        .ok();
    Json(json!({
        "plan_count": plan_count,
        "latest_plan_id": latest_plan_id,
        "source": "local",
    }))
    .into_response()
}

async fn proxy_to_peer(peer_name: &str) -> Response {
    let peer_name = peer_name.to_owned();
    let resolved = tokio::task::spawn_blocking({
        let peer_name = peer_name.clone();
        move || resolve_peer_addr(&peer_name)
    })
    .await;
    let (canonical, addr) = match resolved {
        Ok(Ok(pair)) => pair,
        Ok(Err(msg)) => return bad_gateway(&msg),
        Err(e) => return Json(json!({"error": format!("task join: {e}")})).into_response(),
    };
    let url = format!("http://{addr}/api/mesh/plans/summary");
    let client = match reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => return Json(json!({"error": format!("http client: {e}")})).into_response(),
    };
    let mut req = client.get(&url);
    if let Ok(token) = std::env::var("CONVERGIO_AUTH_TOKEN") {
        if !token.is_empty() {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
    }
    match req.send().await {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(mut body) => {
                body["source"] = json!(canonical);
                Json(body).into_response()
            }
            Err(e) => bad_gateway(&format!("parse peer response: {e}")),
        },
        Ok(resp) => bad_gateway(&format!("peer returned {}", resp.status())),
        Err(e) => bad_gateway(&format!("peer request failed: {e}")),
    }
}

fn resolve_peer_addr(peer_name: &str) -> Result<(String, String), String> {
    let conf_path = std::path::PathBuf::from(peers_conf_path_from_env());
    let registry = PeersRegistry::load(&conf_path).map_err(|e| format!("load peers.conf: {e}"))?;
    let (canonical, cfg) = registry
        .get_peer(peer_name)
        .ok_or_else(|| format!("peer '{peer_name}' not found"))?;
    let canonical = canonical.to_owned();
    let fields = peer_config_to_fields(cfg);
    let addr = resolve_best_addr(&canonical, &fields)
        .ok_or_else(|| format!("peer '{canonical}' unreachable"))?;
    Ok((canonical, addr))
}

fn peer_config_to_fields(cfg: &crate::peers_types::PeerConfig) -> HashMap<String, String> {
    let mut f = HashMap::new();
    f.insert("tailscale_ip".into(), cfg.tailscale_ip.clone());
    if let Some(ref ip) = cfg.lan_ip {
        f.insert("lan_ip".into(), ip.clone());
    }
    if let Some(ref ip) = cfg.thunderbolt_ip {
        f.insert("thunderbolt_ip".into(), ip.clone());
    }
    f
}

fn bad_gateway(msg: &str) -> Response {
    (StatusCode::BAD_GATEWAY, Json(json!({"error": msg}))).into_response()
}
