//! E2E tests for heartbeat role verification via topology API.

mod mesh_helpers;

use mesh_helpers::*;
use tower::ServiceExt;

#[tokio::test]
async fn heartbeat_role_matches_assignment() {
    let (app, pool) = setup();
    app.oneshot(post_json(
        "/api/mesh/config/role",
        r#"{"peer_name":"studio","role":"kernel"}"#,
    ))
    .await
    .unwrap();

    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO peer_heartbeats (peer_name, last_seen, role) \
         VALUES ('studio', unixepoch(), 'kernel')",
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
    assert_eq!(node["assigned_role"], "kernel");
    assert_eq!(node["heartbeat_role"], "kernel");
}

#[tokio::test]
async fn heartbeat_role_mismatch_detected() {
    let (app, pool) = setup();
    app.oneshot(post_json(
        "/api/mesh/config/role",
        r#"{"peer_name":"node1","role":"orchestrator"}"#,
    ))
    .await
    .unwrap();

    let conn = pool.get().unwrap();
    conn.execute(
        "INSERT INTO peer_heartbeats (peer_name, last_seen, role) \
         VALUES ('node1', unixepoch(), 'worker')",
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
    assert_eq!(node["assigned_role"], "orchestrator");
    assert_eq!(node["heartbeat_role"], "worker");
}

#[tokio::test]
async fn heartbeat_offline_node_in_topology() {
    let (app, pool) = setup();
    app.oneshot(post_json(
        "/api/mesh/config/role",
        r#"{"peer_name":"offline-node","role":"worker"}"#,
    ))
    .await
    .unwrap();

    // No heartbeat inserted — node is offline
    let app2 = rebuild(&pool);
    let resp = app2
        .oneshot(get_req("/api/mesh/config/topology"))
        .await
        .unwrap();
    let json = body_json(resp).await;
    let node = &json["nodes"].as_array().unwrap()[0];
    assert_eq!(node["assigned_role"], "worker");
    assert!(node["last_seen"].is_null());
    assert!(node["heartbeat_role"].is_null());
}
