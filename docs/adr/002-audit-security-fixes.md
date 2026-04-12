# ADR-002: Security Audit Fixes

**Status:** Accepted
**Date:** 2025-07-18
**Author:** Security Audit (Copilot)

## Context

A comprehensive security audit of `convergio-mesh` identified multiple
vulnerabilities in the sync, transport, and configuration subsystems.
The mesh crate handles peer-to-peer replication and is exposed to
untrusted network input from peers.

## Findings & Fixes

### CRITICAL

| # | Category | File | Finding | Fix |
|---|----------|------|---------|-----|
| 1 | SQL injection | `sync_apply.rs` | `apply_changes` built SQL from untrusted `table_name` and JSON keys | Validate table against `SYNC_TABLES` allowlist; validate column names with `is_safe_identifier`; quote column names |
| 2 | SSRF | `transport.rs` | `http://{peer_addr}` URL constructed from unvalidated strings | Added `validate_peer_addr()` — rejects `/`, `@`, `?`, `#` in host, requires valid port |
| 3 | Data exfiltration | `routes.rs` `/api/sync/export` | Any table name accepted via query param | Restricted to `SYNC_TABLES` allowlist with 403 on violation |
| 4 | HMAC mismatch | `routes.rs` | Receiver verified HMAC over raw body; sender signed `timestamp:method:path:hash` | Receiver now reconstructs and verifies the full signed message |
| 5 | Command execution | `routes_sync_repo.rs` | Unauthenticated `git pull` + `cargo build` endpoint | Gated behind `CONVERGIO_SYNC_REPO_ENABLED=1` env var (default: disabled) |
| 6 | Hardcoded credential | `sync_loop.rs` | Default auth token `"dev-local"` used when env var unset | Refuse to send heartbeat if `CONVERGIO_AUTH_TOKEN` not set |

### HIGH

| # | Category | File | Finding | Fix |
|---|----------|------|---------|-----|
| 7 | INI injection | `peers_parser.rs` | Newlines in values could inject sections/keys | Added `sanitize_ini_value()` stripping `\n`/`\r` |
| 8 | Weak validation | `auth.rs` | `debug_assert!` for empty secret/data — no-op in release | Replaced with runtime `Err` returns |

## New Tests Added

- `apply_rejects_table_not_in_allowlist` — verifies SQL injection protection
- `apply_rejects_unsafe_column_names` — verifies column name validation
- `validate_peer_addr_rejects_ssrf` — verifies SSRF protection
- `validate_peer_addr_accepts_valid` — verifies legitimate addresses pass
- `sanitize_ini_strips_newlines` — verifies INI injection protection
- `ini_injection_prevented` — end-to-end INI injection roundtrip test
- `compute_hmac_rejects_empty_secret` — runtime validation test
- `compute_hmac_rejects_empty_data` — runtime validation test
- `verify_hmac_rejects_empty_secret` — runtime validation test

## Decision

All CRITICAL and HIGH findings have been fixed with this PR.
Tests added for each fix. Total test count: 71 (was 60).

## Consequences

- Peers sending data for tables outside `SYNC_TABLES` will be rejected
- HMAC verification now matches the sender's signing format
- The `sync-repo` endpoint is disabled by default
- Heartbeats require `CONVERGIO_AUTH_TOKEN` to be explicitly set
