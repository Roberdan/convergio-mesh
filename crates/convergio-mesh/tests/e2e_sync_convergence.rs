//! Integration test for cross-peer sync convergence.
//!
//! Covers plan 2448 T1-02: spin two in-memory peers, insert 10 plans on peer A,
//! run sync cycles, assert peer B converges to identical content. A second test
//! exercises the mid-round insert race documented in
//! docs/sync-drift-root-cause.md — it is #[ignore]'d until T1-03 lands the fix.

use convergio_mesh::sync_apply::{apply_changes, export_changes_since};
use convergio_mesh::types::{SyncChange, SyncMeta};
use rusqlite::Connection;

const PLANS_SCHEMA: &str = "CREATE TABLE plans (\
     id INTEGER PRIMARY KEY, \
     name TEXT, \
     status TEXT DEFAULT 'todo', \
     updated_at TEXT NOT NULL\
 );";

fn setup_peer() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(PLANS_SCHEMA).unwrap();
    // _sync_meta is an orchestrator table in prod; recreate the shape we use here
    conn.execute_batch(
        "CREATE TABLE _sync_meta (\
            peer TEXT NOT NULL, \
            table_name TEXT NOT NULL, \
            last_synced TEXT NOT NULL, \
            PRIMARY KEY (peer, table_name)\
         );",
    )
    .unwrap();
    conn
}

fn insert_plan(conn: &Connection, id: i64, name: &str, updated_at: &str) {
    conn.execute(
        "INSERT INTO plans (id, name, status, updated_at) VALUES (?1, ?2, 'todo', ?3)",
        rusqlite::params![id, name, updated_at],
    )
    .unwrap();
}

fn plan_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT count(*) FROM plans", [], |r| r.get(0))
        .unwrap()
}

fn plan_names(conn: &Connection) -> Vec<String> {
    let mut stmt = conn.prepare("SELECT name FROM plans ORDER BY id").unwrap();
    stmt.query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .filter_map(|r| r.ok())
        .collect()
}

/// One-direction sync round A -> B using the production data plane (export + apply).
/// Returns (exported, applied) and advances `src_cursor_for_dst` using the
/// CURRENT production rule: wall-clock `now`. This matches the behaviour
/// documented as buggy in sync-drift-root-cause.md §2 and §3.
fn sync_round_wallclock(
    src: &Connection,
    dst: &Connection,
    peer_label: &str,
    table: &str,
) -> (usize, usize) {
    let since = src
        .query_row(
            "SELECT last_synced FROM _sync_meta WHERE peer=?1 AND table_name=?2",
            rusqlite::params![peer_label, table],
            |r| r.get::<_, String>(0),
        )
        .ok();
    let changes: Vec<SyncChange> =
        export_changes_since(src, table, since.as_deref()).unwrap_or_default();
    let applied = apply_changes(dst, &changes).unwrap_or(0);
    // Wall-clock cursor advance — mirrors sync.rs::sync_table_with_peer lines 154-159
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    src.execute(
        "INSERT INTO _sync_meta (peer, table_name, last_synced) VALUES (?1, ?2, ?3) \
         ON CONFLICT(peer, table_name) DO UPDATE SET last_synced=excluded.last_synced",
        rusqlite::params![peer_label, table, now],
    )
    .unwrap();
    (changes.len(), applied)
}

#[test]
fn two_peers_converge_after_two_sync_cycles() {
    let a = setup_peer();
    let b = setup_peer();
    let base = "2026-04-21 08:00:00";
    for i in 1..=10_i64 {
        insert_plan(&a, i, &format!("plan-{i}"), base);
    }
    assert_eq!(plan_count(&a), 10);
    assert_eq!(plan_count(&b), 0);

    let (sent1, applied1) = sync_round_wallclock(&a, &b, "peer-b", "plans");
    assert_eq!(sent1, 10, "cycle 1: A exports all 10 plans");
    assert_eq!(applied1, 10, "cycle 1: B applies all 10 plans");

    let (sent2, applied2) = sync_round_wallclock(&a, &b, "peer-b", "plans");
    assert_eq!(sent2, 0, "cycle 2: cursor advanced, nothing new to export");
    assert_eq!(applied2, 0, "cycle 2: nothing to apply");

    assert_eq!(plan_count(&b), 10, "B converged to 10 plans");
    assert_eq!(
        plan_names(&a),
        plan_names(&b),
        "content identical across peers"
    );
}

#[test]
fn bidirectional_convergence_two_cycles() {
    let a = setup_peer();
    let b = setup_peer();
    let base = "2026-04-21 08:00:00";
    for i in 1..=5_i64 {
        insert_plan(&a, i, &format!("a-{i}"), base);
    }
    for i in 6..=10_i64 {
        insert_plan(&b, i, &format!("b-{i}"), base);
    }

    // Cycle 1: both directions
    sync_round_wallclock(&a, &b, "peer-b", "plans");
    sync_round_wallclock(&b, &a, "peer-a", "plans");
    // Cycle 2: both directions (cursors already advanced, nothing new)
    sync_round_wallclock(&a, &b, "peer-b", "plans");
    sync_round_wallclock(&b, &a, "peer-a", "plans");

    assert_eq!(plan_count(&a), 10, "A has all 10 plans");
    assert_eq!(plan_count(&b), 10, "B has all 10 plans");
    assert_eq!(plan_names(&a), plan_names(&b), "identical content");
}

/// Regression test for sync-drift-root-cause.md §3 "insert-during-round race".
/// With the current wall-clock cursor, a row whose `updated_at` predates the
/// previous cursor snapshot is permanently skipped.
/// T1-03 must flip the cursor rule to MAX(exported_updated_at) so this passes.
#[test]
#[ignore = "exposes T1-03 cursor bug; un-ignore after fix"]
fn stale_cursor_never_skips_rows() {
    let a = setup_peer();
    let b = setup_peer();

    // Seed cursor ahead of the inserts — simulates a previous round whose
    // wall-clock advance landed past rows that had not yet been exported.
    a.execute(
        "INSERT INTO _sync_meta (peer, table_name, last_synced) \
         VALUES ('peer-b', 'plans', '2026-04-21 09:00:00')",
        [],
    )
    .unwrap();

    // Rows predate the cursor.
    for i in 1..=10_i64 {
        insert_plan(&a, i, &format!("pre-{i}"), "2026-04-21 08:30:00");
    }

    sync_round_wallclock(&a, &b, "peer-b", "plans");
    sync_round_wallclock(&a, &b, "peer-b", "plans");

    // With the fix, cycle semantics must guarantee convergence despite the
    // stale cursor. With the current buggy code, B stays at 0.
    assert_eq!(
        plan_count(&b),
        10,
        "all plans must reach B even when cursor started ahead of them"
    );
}

/// SyncChange serde roundtrip guard — ensures the wire format used by
/// the mesh transport stays stable.
#[test]
fn sync_change_wire_format_is_stable() {
    let ch = SyncChange {
        table_name: "plans".into(),
        pk: serde_json::json!(42),
        data: serde_json::json!({"id": 42, "name": "x", "updated_at": "2026-04-21 08:00:00"}),
    };
    let wire = serde_json::to_string(&ch).unwrap();
    let back: SyncChange = serde_json::from_str(&wire).unwrap();
    assert_eq!(back.pk, 42);
    assert_eq!(back.table_name, "plans");
}

/// Sanity check that SyncMeta is publicly reachable from the crate API —
/// the prod sync loop depends on this type being stable across releases.
#[test]
fn sync_meta_publicly_constructible() {
    let m = SyncMeta {
        peer: "peer-x".into(),
        table_name: "plans".into(),
        last_synced: "2026-04-21 08:00:00".into(),
    };
    assert_eq!(m.table_name, "plans");
}
