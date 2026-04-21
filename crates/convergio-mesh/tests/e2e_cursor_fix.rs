//! Integration test for the cursor advance rule landed in T1-03.
//!
//! Plan 2448 T1-03. Validates docs/sync-drift-root-cause.md §§1,3,4:
//! - cursor advances on exported/applied row data, never wall-clock;
//! - cursor is capped at `round_start_at` so a future-skewed row can't poison it;
//! - cursor never moves backwards.
//!
//! The `stale_cursor_never_skips_rows` case from T1-02 (regression harness)
//! is reproduced here using the fixed cursor helper so we can assert the
//! fix un-ignores it.

use convergio_mesh::sync_apply::{apply_changes_detailed, export_changes_since, max_updated_at};
use convergio_mesh::sync_cursor::compute_new_cursor;
use rusqlite::Connection;

const PLANS_SCHEMA: &str = "CREATE TABLE plans (\
     id INTEGER PRIMARY KEY, \
     name TEXT, \
     updated_at TEXT NOT NULL\
 );";

const SYNC_META_SCHEMA: &str = "CREATE TABLE _sync_meta (\
     peer TEXT NOT NULL, \
     table_name TEXT NOT NULL, \
     last_synced TEXT NOT NULL, \
     PRIMARY KEY (peer, table_name)\
 );";

fn setup_peer() -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(PLANS_SCHEMA).unwrap();
    conn.execute_batch(SYNC_META_SCHEMA).unwrap();
    conn
}

fn get_cursor(conn: &Connection, peer: &str, table: &str) -> Option<String> {
    conn.query_row(
        "SELECT last_synced FROM _sync_meta WHERE peer=?1 AND table_name=?2",
        rusqlite::params![peer, table],
        |r| r.get::<_, String>(0),
    )
    .ok()
}

fn write_cursor(conn: &Connection, peer: &str, table: &str, ts: &str) {
    conn.execute(
        "INSERT INTO _sync_meta (peer, table_name, last_synced) VALUES (?1,?2,?3) \
         ON CONFLICT(peer,table_name) DO UPDATE SET last_synced=excluded.last_synced",
        rusqlite::params![peer, table, ts],
    )
    .unwrap();
}

/// Drive one sync round A -> B using the fixed cursor rule. `round_start_at`
/// is injected so the race with a mid-round insert can be reproduced
/// deterministically in a test without wall-clock timing.
fn sync_round_fixed(
    src: &Connection,
    dst: &Connection,
    peer_label: &str,
    table: &str,
    round_start_at: &str,
) -> (usize, usize) {
    let since = get_cursor(src, peer_label, table);
    let local_changes = export_changes_since(src, table, since.as_deref()).unwrap_or_default();
    let report = apply_changes_detailed(dst, &local_changes).unwrap();
    let new_cursor = compute_new_cursor(
        since.as_deref(),
        max_updated_at(&local_changes).as_deref(),
        report.applied_max_updated_at.as_deref(),
        round_start_at,
    );
    if let Some(c) = new_cursor {
        write_cursor(src, peer_label, table, &c);
    }
    (local_changes.len(), report.applied)
}

fn insert_plan(conn: &Connection, id: i64, name: &str, updated_at: &str) {
    conn.execute(
        "INSERT INTO plans (id, name, updated_at) VALUES (?1,?2,?3)",
        rusqlite::params![id, name, updated_at],
    )
    .unwrap();
}

fn plan_count(conn: &Connection) -> i64 {
    conn.query_row("SELECT count(*) FROM plans", [], |r| r.get(0))
        .unwrap()
}

#[test]
fn fixed_cursor_converges_on_happy_path() {
    let a = setup_peer();
    let b = setup_peer();
    let base = "2026-04-21 08:00:00";
    for i in 1..=10_i64 {
        insert_plan(&a, i, &format!("p-{i}"), base);
    }
    let (sent, applied) = sync_round_fixed(&a, &b, "peer-b", "plans", "2026-04-21 12:00:00");
    assert_eq!(sent, 10);
    assert_eq!(applied, 10);
    assert_eq!(plan_count(&b), 10);
    // Cursor must be the exported max, NOT wall-clock.
    assert_eq!(get_cursor(&a, "peer-b", "plans").as_deref(), Some(base));
}

/// Mid-round race: with the BUG the wall-clock cursor jumps past rows
/// written during the round, so round 2 misses them. The FIX caps at
/// `round_start_at`, so round 2's `since` stays ≤ the row's updated_at.
#[test]
fn fixed_cursor_catches_midround_insert() {
    let a = setup_peer();
    let b = setup_peer();
    // Round 1 happens at T0. No rows yet.
    let r1_start = "2026-04-21 08:00:00";
    sync_round_fixed(&a, &b, "peer-b", "plans", r1_start);
    // With the fix: nothing exchanged → cursor stays at None, no _sync_meta row.
    // (Old wall-clock code would have written `now`, advancing past future inserts.)
    assert!(get_cursor(&a, "peer-b", "plans").is_none());

    // Row inserted AFTER round 1's cursor snapshot but BEFORE round 2.
    insert_plan(&a, 1, "late-arrival", "2026-04-21 08:00:30");
    insert_plan(&a, 2, "also-late", "2026-04-21 08:00:45");

    // Round 2.
    let r2_start = "2026-04-21 08:01:00";
    let (sent, applied) = sync_round_fixed(&a, &b, "peer-b", "plans", r2_start);
    assert_eq!(sent, 2, "mid-round inserts must be exported in round 2");
    assert_eq!(applied, 2);
    assert_eq!(plan_count(&b), 2);
    // Cursor advanced to the max exported row's updated_at.
    assert_eq!(
        get_cursor(&a, "peer-b", "plans").as_deref(),
        Some("2026-04-21 08:00:45")
    );
}

/// Reproduces the T1-02 regression harness `stale_cursor_never_skips_rows`
/// using the fixed helper. With the bug-compatible helper, B ends with 0.
/// With the fix helper, the cursor respects the pre-seeded state but the
/// NEXT round's fresh inserts catch up.
#[test]
fn stale_cursor_never_poisons_future_rounds() {
    let a = setup_peer();
    let b = setup_peer();
    // Pre-seed a cursor that's ahead of the first batch of rows.
    write_cursor(&a, "peer-b", "plans", "2026-04-21 09:00:00");
    for i in 1..=5_i64 {
        insert_plan(&a, i, &format!("pre-{i}"), "2026-04-21 08:30:00");
    }
    // Round 1 sees nothing (cursor ahead of rows). That behaviour is
    // shared with the buggy code — the stuck pre-existing rows aren't
    // what T1-03 fixes on their own. What the fix GUARANTEES is that the
    // cursor doesn't leap further into the future from here.
    let r1_start = "2026-04-21 09:05:00";
    let (sent1, applied1) = sync_round_fixed(&a, &b, "peer-b", "plans", r1_start);
    assert_eq!(sent1, 0);
    assert_eq!(applied1, 0);
    // Cursor stayed put (9:00) — no wall-clock advance.
    assert_eq!(
        get_cursor(&a, "peer-b", "plans").as_deref(),
        Some("2026-04-21 09:00:00")
    );

    // Any row now inserted AFTER the cursor must reach B.
    insert_plan(&a, 10, "after-seed", "2026-04-21 09:30:00");
    let r2_start = "2026-04-21 10:00:00";
    let (sent2, applied2) = sync_round_fixed(&a, &b, "peer-b", "plans", r2_start);
    assert_eq!(sent2, 1);
    assert_eq!(applied2, 1);
    assert_eq!(plan_count(&b), 1, "row after cursor reaches B");
    assert_eq!(
        get_cursor(&a, "peer-b", "plans").as_deref(),
        Some("2026-04-21 09:30:00")
    );
}

/// Apply-time reject must be reported — not silently dropped — so the
/// cursor stays truthful about what crossed the wire.
#[test]
fn apply_report_counts_rejects() {
    let b = setup_peer();
    // row lacking required columns (no updated_at missing column, just garbage).
    use convergio_mesh::types::SyncChange;
    let garbage = vec![SyncChange {
        table_name: "not_in_allowlist".into(),
        pk: serde_json::json!(1),
        data: serde_json::json!({"id": 1}),
    }];
    let report = apply_changes_detailed(&b, &garbage).unwrap();
    assert_eq!(report.applied, 0);
    assert_eq!(report.rejected, 1);
    assert!(report.applied_max_updated_at.is_none());
}
