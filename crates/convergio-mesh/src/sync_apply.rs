//! Sync data exchange: export local changes, apply remote changes.
//!
//! Extracted from sync.rs to stay under 250-line limit.

use rusqlite::Connection;
use tracing;

use crate::types::SyncChange;

/// Export rows with updated_at > since. Returns SyncChange vec.
/// Tables that don't exist are silently skipped.
pub fn export_changes_since(
    conn: &Connection,
    table: &str,
    since: Option<&str>,
) -> Result<Vec<SyncChange>, rusqlite::Error> {
    if !table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(rusqlite::Error::InvalidParameterName(
            "invalid table name".to_string(),
        ));
    }
    let exists: bool = conn
        .prepare("SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1")?
        .exists(rusqlite::params![table])?;
    if !exists {
        return Ok(vec![]);
    }

    // Detect timestamp column: prefer updated_at, fall back to created_at
    let ts_col = detect_timestamp_column(conn, table);

    let sql = match (&since, &ts_col) {
        (Some(_), Some(col)) => format!(
            "SELECT id, * FROM \"{table}\" \
             WHERE REPLACE(\"{col}\",'T',' ') > ?1 ORDER BY id"
        ),
        _ => format!("SELECT id, * FROM \"{table}\" ORDER BY id"),
    };
    let mut stmt = conn.prepare(&sql)?;
    let col_count = stmt.column_count();
    let col_names: Vec<String> = (0..col_count)
        .map(|i| stmt.column_name(i).unwrap_or("").to_string())
        .collect();

    let params: Vec<Box<dyn rusqlite::types::ToSql>> = match since {
        Some(ts) => vec![Box::new(ts.replace('T', " "))],
        None => vec![],
    };
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let rows = stmt.query_map(param_refs.as_slice(), |row| {
        // Support both INTEGER and TEXT primary keys
        let pk: serde_json::Value = match row.get::<_, i64>(0) {
            Ok(i) => serde_json::json!(i),
            Err(_) => match row.get::<_, String>(0) {
                Ok(s) => serde_json::json!(s),
                Err(_) => serde_json::json!(0),
            },
        };
        let mut data = serde_json::Map::new();
        for (i, name) in col_names.iter().enumerate().skip(1) {
            let val: rusqlite::types::Value = row.get(i)?;
            data.insert(name.clone(), sqlite_to_json(val));
        }
        Ok(SyncChange {
            table_name: table.to_string(),
            pk,
            data: serde_json::Value::Object(data),
        })
    })?;
    rows.collect()
}

/// Detect which timestamp column a table has: updated_at, created_at, or none.
fn detect_timestamp_column(conn: &Connection, table: &str) -> Option<String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info(\"{table}\")"))
        .ok()?;
    let cols: Vec<String> = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .ok()?
        .filter_map(|r| r.ok())
        .collect();
    if cols.iter().any(|c| c == "updated_at") {
        Some("updated_at".into())
    } else if cols.iter().any(|c| c == "created_at") {
        Some("created_at".into())
    } else {
        None
    }
}

fn sqlite_to_json(val: rusqlite::types::Value) -> serde_json::Value {
    match val {
        rusqlite::types::Value::Null => serde_json::Value::Null,
        rusqlite::types::Value::Integer(i) => serde_json::json!(i),
        rusqlite::types::Value::Real(f) => serde_json::json!(f),
        rusqlite::types::Value::Text(s) => serde_json::json!(s),
        rusqlite::types::Value::Blob(b) => {
            use base64::{engine::general_purpose::STANDARD, Engine};
            serde_json::json!(STANDARD.encode(&b))
        }
    }
}

/// Validate that a name (table or column) contains only safe identifier chars.
fn is_safe_identifier(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Get column names for a local table via PRAGMA table_info.
fn local_table_columns(conn: &Connection, table: &str) -> Vec<String> {
    let Ok(mut stmt) = conn.prepare(&format!("PRAGMA table_info(\"{table}\")")) else {
        return vec![];
    };
    stmt.query_map([], |row| row.get::<_, String>(1))
        .map(|rows| rows.filter_map(|r| r.ok()).collect())
        .unwrap_or_default()
}

/// LWW check: return local row's updated_at for conflict resolution.
fn local_updated_at(conn: &Connection, table: &str, pk: &serde_json::Value) -> Option<String> {
    let sql = format!("SELECT updated_at FROM \"{table}\" WHERE id = ?1");
    match pk {
        serde_json::Value::Number(n) => {
            let id = n.as_i64()?;
            conn.query_row(&sql, [id], |r| r.get::<_, Option<String>>(0))
                .ok()
                .flatten()
        }
        serde_json::Value::String(s) => conn
            .query_row(&sql, [s.as_str()], |r| r.get::<_, Option<String>>(0))
            .ok()
            .flatten(),
        _ => None,
    }
}

/// Apply remote changes with LWW conflict resolution.
/// - Validates table names against SYNC_TABLES allowlist
/// - Validates column names against safe identifier rules
/// - Filters out columns the local schema doesn't have (schema tolerance)
/// - Only overwrites when remote `updated_at` is strictly newer (LWW)
pub fn apply_changes(conn: &Connection, changes: &[SyncChange]) -> Result<usize, rusqlite::Error> {
    if changes.is_empty() {
        return Ok(0);
    }
    let mut applied = 0;
    conn.execute("PRAGMA foreign_keys = OFF", [])?;
    let result = apply_changes_inner(conn, changes, &mut applied);
    conn.execute("PRAGMA foreign_keys = ON", [])?;
    result.map(|_| applied)
}

fn apply_changes_inner(
    conn: &Connection,
    changes: &[SyncChange],
    applied: &mut usize,
) -> Result<(), rusqlite::Error> {
    for change in changes {
        if !crate::types::SYNC_TABLES.contains(&change.table_name.as_str()) {
            tracing::warn!(table = %change.table_name, "rejected: not in SYNC_TABLES");
            continue;
        }
        let Some(obj) = change.data.as_object() else {
            continue;
        };
        let local_cols = local_table_columns(conn, &change.table_name);
        if local_cols.is_empty() {
            continue;
        }
        // Filter to columns that exist locally (schema tolerance)
        let cols: Vec<&str> = obj
            .keys()
            .map(|k| k.as_str())
            .filter(|c| local_cols.iter().any(|lc| lc == c))
            .collect();
        if cols.is_empty() {
            continue;
        }
        if cols.iter().any(|c| !is_safe_identifier(c)) {
            tracing::warn!(table = %change.table_name, "rejected: unsafe column names");
            continue;
        }
        // LWW: skip if local row has newer or equal updated_at
        if let Some(remote_ts) = obj.get("updated_at").and_then(|v| v.as_str()) {
            if let Some(local_ts) = local_updated_at(conn, &change.table_name, &change.pk) {
                let r = remote_ts.replace('T', " ");
                let l = local_ts.replace('T', " ");
                if r <= l {
                    continue;
                }
            }
        }
        let quoted: Vec<String> = cols.iter().map(|c| format!("\"{c}\"")).collect();
        let ph: Vec<String> = (1..=cols.len()).map(|i| format!("?{i}")).collect();
        let sql = format!(
            "INSERT OR REPLACE INTO \"{}\" ({}) VALUES ({})",
            change.table_name,
            quoted.join(", "),
            ph.join(", ")
        );
        let vals: Vec<String> = cols
            .iter()
            .filter_map(|c| obj.get(*c).map(json_to_sql_string))
            .collect();
        let params: Vec<&dyn rusqlite::types::ToSql> = vals
            .iter()
            .map(|v| v as &dyn rusqlite::types::ToSql)
            .collect();
        if conn.execute(&sql, params.as_slice()).is_ok() {
            *applied += 1;
        }
    }
    Ok(())
}

fn json_to_sql_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    }
}

#[cfg(test)]
#[path = "sync_apply_tests.rs"]
mod tests;
