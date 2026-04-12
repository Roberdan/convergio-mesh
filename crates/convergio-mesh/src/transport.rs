//! HTTP transport helpers for background sync.
//!
//! Address resolution with multi-transport fallback (Thunderbolt > LAN > Tailscale),
//! HMAC-signed requests, and sync endpoint communication.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::TcpStream;
use std::time::Duration;
use tracing::{info, warn};

use crate::auth::{compute_hmac, load_shared_secret};
use crate::peers_registry::peers_conf_path_from_env;
use crate::types::SyncChange;

/// Build HMAC auth header: timestamp + method + path + optional body hash.
/// Returns (timestamp, hex-encoded signature) or None if no shared secret.
fn mesh_hmac_header(
    method: &str,
    path_and_query: &str,
    body_hash: Option<&str>,
) -> Option<(String, String)> {
    let conf_path = std::path::PathBuf::from(peers_conf_path_from_env());
    let secret = load_shared_secret(&conf_path)?;
    let timestamp = chrono::Utc::now().timestamp().to_string();
    let message = match body_hash {
        Some(bh) => format!("{timestamp}:{method}:{path_and_query}:{bh}"),
        None => format!("{timestamp}:{method}:{path_and_query}"),
    };
    let sig = compute_hmac(&secret, message.as_bytes()).ok()?;
    Some((timestamp, hex::encode(sig)))
}

/// Build Bearer header value from a raw token string.
fn bearer_from_token(token: &str) -> Option<String> {
    if token.is_empty() {
        None
    } else {
        Some(format!("Bearer {token}"))
    }
}

/// Read CONVERGIO_AUTH_TOKEN from environment (cached per process).
fn auth_bearer_value() -> Option<String> {
    static TOKEN: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    TOKEN
        .get_or_init(|| {
            std::env::var("CONVERGIO_AUTH_TOKEN")
                .ok()
                .and_then(|t| bearer_from_token(&t))
        })
        .clone()
}

/// Apply Bearer + HMAC auth headers to an HTTP request builder.
fn apply_mesh_auth(
    mut req: reqwest::blocking::RequestBuilder,
    method: &str,
    path_and_query: &str,
    body: Option<&[u8]>,
) -> reqwest::blocking::RequestBuilder {
    // Bearer token — required by the peer's auth middleware
    if let Some(bearer) = auth_bearer_value() {
        req = req.header("Authorization", bearer);
    }
    // HMAC signature — optional integrity layer
    let body_hash = body.map(|b| hex::encode(Sha256::digest(b)));
    if let Some((ts, sig)) = mesh_hmac_header(method, path_and_query, body_hash.as_deref()) {
        req = req
            .header("X-Mesh-Timestamp", ts)
            .header("X-Mesh-Signature", sig);
        if let Some(bh) = &body_hash {
            req = req.header("X-Mesh-Body-Hash", bh.as_str());
        }
    }
    req
}

/// Validate peer address is a safe host:port (no path, no scheme, no query).
fn validate_peer_addr(addr: &str) -> Result<(), String> {
    let parts: Vec<&str> = addr.rsplitn(2, ':').collect();
    if parts.len() != 2 {
        return Err(format!("invalid peer address (missing port): {addr}"));
    }
    let host = parts[1];
    let port = parts[0];
    // Port must be numeric
    if port.parse::<u16>().is_err() {
        return Err(format!("invalid port in peer address: {addr}"));
    }
    // Host must be a valid IP or hostname — no slashes, @, queries
    if host.contains('/') || host.contains('@') || host.contains('?') || host.contains('#') {
        return Err(format!("invalid chars in peer address host: {addr}"));
    }
    Ok(())
}

/// POST local changes to peer's /api/sync/import endpoint.
pub fn send_changes_to_peer(peer_addr: &str, changes: &[SyncChange]) -> Result<(), String> {
    validate_peer_addr(peer_addr)?;
    let path = "/api/sync/import";
    let url = format!("http://{peer_addr}{path}");
    let payload = serde_json::json!({ "changes": changes });
    let body_bytes =
        serde_json::to_vec(&payload).map_err(|e| format!("JSON serialize failed: {e}"))?;
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client build failed: {e}"))?;
    let req = client.post(&url).header("content-type", "application/json");
    let req = apply_mesh_auth(req, "POST", path, Some(&body_bytes));
    let req = req.body(body_bytes);
    let resp = req.send().map_err(|e| format!("HTTP POST failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("peer {peer_addr} returned {}", resp.status()));
    }
    Ok(())
}

/// GET remote changes from peer's /api/sync/export endpoint.
pub fn fetch_changes_from_peer(
    peer_addr: &str,
    table: &str,
    since: Option<&str>,
) -> Result<Vec<SyncChange>, String> {
    validate_peer_addr(peer_addr)?;
    // SECURITY: validate table name against SYNC_TABLES allowlist
    if !crate::types::SYNC_TABLES.contains(&table) {
        return Err(format!("table '{table}' not in sync allowlist"));
    }
    let mut path_query = format!("/api/sync/export?table={table}");
    let mut url = format!("http://{peer_addr}{path_query}");
    if let Some(ts) = since {
        let suffix = format!("&since={ts}");
        url.push_str(&suffix);
        path_query.push_str(&suffix);
    }
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| format!("HTTP client build failed: {e}"))?;
    let req = apply_mesh_auth(client.get(&url), "GET", &path_query, None);
    let resp = req.send().map_err(|e| format!("HTTP GET failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("peer returned {}", resp.status()));
    }
    let body: serde_json::Value = resp.json().map_err(|e| format!("JSON parse failed: {e}"))?;
    let changes: Vec<SyncChange> =
        serde_json::from_value(body.get("changes").cloned().unwrap_or_default())
            .map_err(|e| format!("changes parse failed: {e}"))?;
    Ok(changes)
}

/// Resolve best reachable address for a peer.
/// Priority: Thunderbolt (10.0.0.x) > LAN > Tailscale (100.x.x.x).
/// Returns "host:port" without scheme.
pub fn resolve_best_addr(name: &str, fields: &HashMap<String, String>) -> Option<String> {
    let candidates: Vec<(&str, &str)> = [
        (
            "thunderbolt",
            fields.get("thunderbolt_ip").map(|s| s.as_str()),
        ),
        ("lan", fields.get("lan_ip").map(|s| s.as_str())),
        ("tailscale", fields.get("tailscale_ip").map(|s| s.as_str())),
    ]
    .into_iter()
    .filter_map(|(t, ip)| ip.filter(|s| !s.is_empty()).map(|ip| (t, ip)))
    .collect();

    for (transport, ip) in &candidates {
        let addr = format!("{ip}:8420");
        let Ok(sock_addr) = addr.parse() else {
            warn!(peer = name, "bad addr {addr} via {transport}");
            continue;
        };
        if TcpStream::connect_timeout(&sock_addr, Duration::from_secs(2)).is_ok() {
            info!(peer = name, "reachable via {transport} ({addr})");
            return Some(addr);
        }
    }
    None
}

/// Detect local Tailscale IP via `tailscale ip -4` or env override.
pub fn detect_local_tailscale_ip() -> Option<String> {
    if let Ok(ip) = std::env::var("CONVERGIO_LOCAL_TAILSCALE_IP") {
        let ip = ip.trim().to_string();
        if !ip.is_empty() {
            return Some(ip);
        }
    }
    for cmd in &[
        "tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    ] {
        if let Some(ip) = std::process::Command::new(cmd)
            .args(["ip", "-4"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
        {
            return Some(ip);
        }
    }
    None
}

/// Update mesh_sync_stats after a sync round.
pub fn update_mesh_sync_stats(
    conn: &rusqlite::Connection,
    peer_addr: &str,
    sent: usize,
    received: usize,
    applied: usize,
    latency_ms: i64,
) {
    let result = conn.execute(
        "INSERT INTO mesh_sync_stats(peer_name, total_sent, total_received, \
         total_applied, last_sync_at, last_latency_ms, last_error) \
         VALUES(?1, ?2, ?3, ?4, strftime('%s','now'), ?5, NULL) \
         ON CONFLICT(peer_name) DO UPDATE SET \
           total_sent = total_sent + excluded.total_sent, \
           total_received = total_received + excluded.total_received, \
           total_applied = total_applied + excluded.total_applied, \
           last_sync_at = excluded.last_sync_at, \
           last_latency_ms = excluded.last_latency_ms, \
           last_error = NULL",
        rusqlite::params![
            peer_addr,
            sent as i64,
            received as i64,
            applied as i64,
            latency_ms
        ],
    );
    if let Err(e) = result {
        warn!("update mesh_sync_stats failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bearer_from_valid_token() {
        let token = format!("{}-{}", "dev", "local");
        let v = bearer_from_token(&token);
        assert_eq!(v.as_deref(), Some("Bearer dev-local"));
    }

    #[test]
    fn bearer_from_empty_token_is_none() {
        assert!(bearer_from_token("").is_none());
    }

    #[test]
    fn resolve_no_candidates_returns_none() {
        let fields = HashMap::new();
        assert!(resolve_best_addr("ghost", &fields).is_none());
    }

    #[test]
    fn validate_peer_addr_rejects_ssrf() {
        assert!(validate_peer_addr("evil.com/admin@127.0.0.1:8420").is_err());
        assert!(validate_peer_addr("127.0.0.1:8420?redirect=evil").is_err());
        assert!(validate_peer_addr("127.0.0.1:8420#frag").is_err());
        assert!(validate_peer_addr("noport").is_err());
        assert!(validate_peer_addr("host:notaport").is_err());
    }

    #[test]
    fn validate_peer_addr_accepts_valid() {
        assert!(validate_peer_addr("192.168.1.1:8420").is_ok());
        assert!(validate_peer_addr("100.0.0.1:8420").is_ok());
        assert!(validate_peer_addr("myhost.local:8420").is_ok());
    }
}
