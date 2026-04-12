//! POST /api/mesh/sync-repo — git pull + rebuild + CLI update on THIS node.
//! SECURITY: This endpoint executes system commands and MUST require auth.

use axum::response::Json;
use serde_json::json;

/// Pull latest code, rebuild daemon and CLI binary.
/// Requires CONVERGIO_SYNC_REPO_ENABLED=1 env var to be active.
pub async fn handle_sync_repo(Json(body): Json<serde_json::Value>) -> Json<serde_json::Value> {
    // SECURITY: gate behind explicit opt-in env var
    let enabled = std::env::var("CONVERGIO_SYNC_REPO_ENABLED")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    if !enabled {
        return Json(json!({
            "ok": false,
            "error": "sync-repo endpoint disabled (set CONVERGIO_SYNC_REPO_ENABLED=1)"
        }));
    }
    let peer = body["peer"].as_str().unwrap_or("self");
    tracing::info!("mesh: sync-repo requested for peer={peer}");

    let repo_root = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| ".".into());

    // Reset any dirty files (e.g. Cargo.lock) before pulling
    let _ = std::process::Command::new("git")
        .args(["checkout", "--", "."])
        .current_dir(&repo_root)
        .output();

    let pull = std::process::Command::new("git")
        .args(["pull", "origin", "main"])
        .current_dir(&repo_root)
        .output();

    match pull {
        Ok(out) if out.status.success() => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            tracing::info!("mesh: git pull succeeded: {}", stdout.trim());

            let daemon_dir = format!("{repo_root}/daemon");
            let build = std::process::Command::new("cargo")
                .args(["build", "--release"])
                .current_dir(&daemon_dir)
                .output();

            let build_ok = build.map(|o| o.status.success()).unwrap_or(false);
            if build_ok {
                tracing::info!("mesh: rebuild OK — restart daemon to apply");
            }

            let cli_ok = if build_ok {
                let ok = std::process::Command::new("cargo")
                    .args(["install", "--path", "crates/convergio-cli", "--force"])
                    .current_dir(&daemon_dir)
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                if ok {
                    tracing::info!("mesh: CLI auto-update OK");
                } else {
                    tracing::warn!("mesh: CLI auto-update failed");
                }
                ok
            } else {
                false
            };

            Json(json!({
                "ok": true,
                "git_pull": stdout.trim(),
                "build": if build_ok { "success" } else { "failed" },
                "cli_update": if cli_ok { "success" } else { "skipped_or_failed" },
                "note": "restart daemon to use new binary",
            }))
        }
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            tracing::error!("mesh: git pull failed: {stderr}");
            Json(json!({"ok": false, "error": format!("git pull failed: {stderr}")}))
        }
        Err(e) => Json(json!({"ok": false, "error": format!("git command failed: {e}")})),
    }
}
