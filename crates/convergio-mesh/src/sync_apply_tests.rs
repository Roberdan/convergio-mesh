#![allow(dead_code)]

use super::*;
use crate::types::SyncChange;
use rusqlite::Connection;

#[test]
fn export_nonexistent_table_returns_empty() {
    let conn = Connection::open_in_memory().unwrap();
    let changes = export_changes_since(&conn, "nonexistent", None).unwrap();
    assert!(changes.is_empty());
}

#[test]
fn export_rejects_invalid_table_name() {
    let conn = Connection::open_in_memory().unwrap();
    let result = export_changes_since(&conn, "drop;--", None);
    assert!(result.is_err());
}

#[test]
fn apply_empty_returns_zero() {
    let conn = Connection::open_in_memory().unwrap();
    assert_eq!(apply_changes(&conn, &[]).unwrap(), 0);
}

#[test]
fn export_and_apply_roundtrip() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE plans (
            id INTEGER PRIMARY KEY, name TEXT,
            updated_at TEXT DEFAULT (datetime('now'))
        );
        INSERT INTO plans (id, name) VALUES (1, 'alpha');
        INSERT INTO plans (id, name) VALUES (2, 'beta');",
    )
    .unwrap();
    let changes = export_changes_since(&conn, "plans", None).unwrap();
    assert_eq!(changes.len(), 2);

    let conn2 = Connection::open_in_memory().unwrap();
    conn2
        .execute_batch("CREATE TABLE plans (id INTEGER PRIMARY KEY, name TEXT, updated_at TEXT);")
        .unwrap();
    let applied = apply_changes(&conn2, &changes).unwrap();
    assert_eq!(applied, 2);
}

#[test]
fn apply_rejects_table_not_in_allowlist() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE evil_table (id INTEGER PRIMARY KEY, data TEXT);")
        .unwrap();
    let changes = vec![SyncChange {
        table_name: "evil_table".into(),
        pk: serde_json::json!(1),
        data: serde_json::json!({"id": 1, "data": "hack"}),
    }];
    let applied = apply_changes(&conn, &changes).unwrap();
    assert_eq!(applied, 0);
}

#[test]
fn apply_drops_unsafe_columns_via_schema_filter() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE plans (id INTEGER PRIMARY KEY, name TEXT, updated_at TEXT);")
        .unwrap();
    let mut data = serde_json::Map::new();
    data.insert("id".into(), serde_json::json!(1));
    data.insert("name; DROP TABLE plans--".into(), serde_json::json!("x"));
    let changes = vec![SyncChange {
        table_name: "plans".into(),
        pk: serde_json::json!(1),
        data: serde_json::Value::Object(data),
    }];
    let applied = apply_changes(&conn, &changes).unwrap();
    assert_eq!(applied, 1, "unsafe col filtered out, valid cols applied");
    let count: i64 = conn
        .query_row("SELECT count(*) FROM plans", [], |r| r.get(0))
        .unwrap();
    assert_eq!(count, 1, "row inserted with safe columns only");
}

#[test]
fn lww_skips_older_remote() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE plans (id INTEGER PRIMARY KEY, name TEXT, updated_at TEXT);
         INSERT INTO plans VALUES (1, 'local-newer', '2026-04-21 12:00:00');",
    )
    .unwrap();
    let changes = vec![SyncChange {
        table_name: "plans".into(),
        pk: serde_json::json!(1),
        data: serde_json::json!({"id": 1, "name": "remote-older", "updated_at": "2026-04-21 11:00:00"}),
    }];
    let applied = apply_changes(&conn, &changes).unwrap();
    assert_eq!(applied, 0, "older remote should be skipped");
    let name: String = conn
        .query_row("SELECT name FROM plans WHERE id=1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(name, "local-newer");
}

#[test]
fn lww_applies_newer_remote() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE plans (id INTEGER PRIMARY KEY, name TEXT, updated_at TEXT);
         INSERT INTO plans VALUES (1, 'local-old', '2026-04-21 10:00:00');",
    )
    .unwrap();
    let changes = vec![SyncChange {
        table_name: "plans".into(),
        pk: serde_json::json!(1),
        data: serde_json::json!({"id": 1, "name": "remote-newer", "updated_at": "2026-04-21 12:00:00"}),
    }];
    let applied = apply_changes(&conn, &changes).unwrap();
    assert_eq!(applied, 1);
    let name: String = conn
        .query_row("SELECT name FROM plans WHERE id=1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(name, "remote-newer");
}

#[test]
fn schema_tolerance_ignores_unknown_columns() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch("CREATE TABLE plans (id INTEGER PRIMARY KEY, name TEXT, updated_at TEXT);")
        .unwrap();
    let changes = vec![SyncChange {
        table_name: "plans".into(),
        pk: serde_json::json!(1),
        data: serde_json::json!({
            "id": 1, "name": "alpha",
            "updated_at": "2026-04-21 12:00:00",
            "extra_col": "ignored"
        }),
    }];
    let applied = apply_changes(&conn, &changes).unwrap();
    assert_eq!(applied, 1, "should insert ignoring unknown columns");
    let name: String = conn
        .query_row("SELECT name FROM plans WHERE id=1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(name, "alpha");
}

#[test]
fn bidirectional_lww_roundtrip() {
    let node_a = Connection::open_in_memory().unwrap();
    let node_b = Connection::open_in_memory().unwrap();
    let schema = "CREATE TABLE plans (id INTEGER PRIMARY KEY, name TEXT, updated_at TEXT);";
    node_a.execute_batch(schema).unwrap();
    node_b.execute_batch(schema).unwrap();

    node_a
        .execute_batch("INSERT INTO plans VALUES (1, 'a-version', '2026-04-21 12:00:00');")
        .unwrap();
    node_b
        .execute_batch("INSERT INTO plans VALUES (1, 'b-version', '2026-04-21 11:00:00');")
        .unwrap();

    let a_changes = export_changes_since(&node_a, "plans", None).unwrap();
    let b_changes = export_changes_since(&node_b, "plans", None).unwrap();

    let applied_on_b = apply_changes(&node_b, &a_changes).unwrap();
    let applied_on_a = apply_changes(&node_a, &b_changes).unwrap();

    assert_eq!(applied_on_b, 1, "A (newer) should win on B");
    assert_eq!(applied_on_a, 0, "B (older) should be rejected on A");

    let name_a: String = node_a
        .query_row("SELECT name FROM plans WHERE id=1", [], |r| r.get(0))
        .unwrap();
    let name_b: String = node_b
        .query_row("SELECT name FROM plans WHERE id=1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(name_a, "a-version", "A keeps its newer version");
    assert_eq!(name_b, "a-version", "B converges to A's newer version");
}
