//! E2E tests for mesh node role configuration API.

mod mesh_helpers;

use mesh_helpers::*;
use tower::ServiceExt;

// ── AVAILABLE ROLES ──────────────────────────────────────────

#[tokio::test]
async fn list_available_roles() {
    let (app, _pool) = setup();
    let resp = app
        .oneshot(get_req("/api/mesh/config/roles"))
        .await
        .unwrap();
    let json = body_json(resp).await;
    let roles = json["roles"].as_array().unwrap();
    assert!(roles.len() >= 4, "must have at least the base roles");
    let ids: Vec<&str> = roles.iter().map(|r| r["id"].as_str().unwrap()).collect();
    assert!(ids.contains(&"all"));
    assert!(ids.contains(&"orchestrator"));
    assert!(ids.contains(&"kernel"));
    assert!(ids.contains(&"worker"));
    assert!(ids.contains(&"nightagent"));
    assert!(ids.contains(&"voice"));
}

// ── SET ROLE ─────────────────────────────────────────────────

#[tokio::test]
async fn set_role_valid() {
    let (app, _pool) = setup();
    let body = r#"{"peer_name":"mac-studio","role":"orchestrator"}"#;
    let resp = app
        .oneshot(post_json("/api/mesh/config/role", body))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["status"], "assigned");
    assert_eq!(json["peer_name"], "mac-studio");
    assert_eq!(json["role"], "orchestrator");
}

#[tokio::test]
async fn set_role_invalid_rejected() {
    let (app, _pool) = setup();
    let body = r#"{"peer_name":"node1","role":"invalid-role"}"#;
    let resp = app
        .oneshot(post_json("/api/mesh/config/role", body))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert!(json["error"].as_str().unwrap().contains("invalid role"));
}

#[tokio::test]
async fn set_role_all_valid_roles() {
    let (_app, pool) = setup();
    let valid = [
        "all",
        "orchestrator",
        "kernel",
        "voice",
        "worker",
        "nightagent",
    ];
    for (i, role) in valid.iter().enumerate() {
        let app = rebuild(&pool);
        let body = format!(r#"{{"peer_name":"node-{i}","role":"{role}"}}"#);
        let resp = app
            .oneshot(post_json("/api/mesh/config/role", &body))
            .await
            .unwrap();
        let json = body_json(resp).await;
        assert_eq!(json["status"], "assigned", "role: {role}");
    }
}

#[tokio::test]
async fn set_role_updates_existing() {
    let (app, pool) = setup();
    app.oneshot(post_json(
        "/api/mesh/config/role",
        r#"{"peer_name":"node1","role":"worker"}"#,
    ))
    .await
    .unwrap();

    let app2 = rebuild(&pool);
    let resp = app2
        .oneshot(post_json(
            "/api/mesh/config/role",
            r#"{"peer_name":"node1","role":"orchestrator"}"#,
        ))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["status"], "assigned");
    assert_eq!(json["role"], "orchestrator");
}

// ── TOPOLOGY ─────────────────────────────────────────────────

#[tokio::test]
async fn topology_empty() {
    let (app, _pool) = setup();
    let resp = app
        .oneshot(get_req("/api/mesh/config/topology"))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["count"], 0);
    assert!(json["nodes"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn topology_shows_assigned_nodes() {
    let (app, pool) = setup();
    app.oneshot(post_json(
        "/api/mesh/config/role",
        r#"{"peer_name":"mac-studio","role":"orchestrator"}"#,
    ))
    .await
    .unwrap();

    let app2 = rebuild(&pool);
    app2.oneshot(post_json(
        "/api/mesh/config/role",
        r#"{"peer_name":"macbook","role":"worker"}"#,
    ))
    .await
    .unwrap();

    let app3 = rebuild(&pool);
    let resp = app3
        .oneshot(get_req("/api/mesh/config/topology"))
        .await
        .unwrap();
    let json = body_json(resp).await;
    assert_eq!(json["count"], 2);
    let nodes = json["nodes"].as_array().unwrap();
    assert_eq!(nodes[0]["peer_name"], "mac-studio");
    assert_eq!(nodes[0]["assigned_role"], "orchestrator");
    assert_eq!(nodes[1]["peer_name"], "macbook");
    assert_eq!(nodes[1]["assigned_role"], "worker");
}

#[tokio::test]
async fn topology_shows_heartbeat_status() {
    let (app, pool) = setup();
    app.oneshot(post_json(
        "/api/mesh/config/role",
        r#"{"peer_name":"mac-studio","role":"orchestrator"}"#,
    ))
    .await
    .unwrap();

    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO peer_heartbeats (peer_name, last_seen, role) \
         VALUES ('mac-studio', unixepoch(), 'orchestrator')",
        [],
    )
    .unwrap();
    drop(conn);

    let app2 = rebuild(&pool);
    let resp = app2
        .oneshot(get_req("/api/mesh/config/topology"))
        .await
        .unwrap();
    let json = body_json(resp).await;
    let node = &json["nodes"].as_array().unwrap()[0];
    assert_eq!(node["status"], "online");
    assert_eq!(node["heartbeat_role"], "orchestrator");
}
