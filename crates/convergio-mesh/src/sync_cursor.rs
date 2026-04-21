//! Sync cursor advance rule.
//!
//! Extracted from sync.rs to stay under the 300-line budget and to make the
//! cursor rule — described in docs/sync-drift-root-cause.md — cheap to test
//! without a full HTTP round-trip.

/// Compute the next `last_synced` value for a peer+table pair.
///
/// Rules:
/// 1. Advance to `MAX(exported_max, applied_max)` — never wall-clock.
/// 2. Cap at `round_start_at` so a future-skewed row can't poison the cursor.
/// 3. Never move the cursor backwards vs `prev_since`.
/// 4. If nothing exchanged, stay at `prev_since` (and keep the previous
///    cursor row — caller writes only when `Some(_)` is returned).
pub fn compute_new_cursor(
    prev_since: Option<&str>,
    exported_max: Option<&str>,
    applied_max: Option<&str>,
    round_start_at: &str,
) -> Option<String> {
    let candidate = match (exported_max, applied_max) {
        (Some(a), Some(b)) => Some(if a >= b { a } else { b }),
        (Some(a), None) | (None, Some(a)) => Some(a),
        (None, None) => None,
    };
    let capped = candidate.map(|c| {
        if c < round_start_at {
            c.to_string()
        } else {
            round_start_at.to_string()
        }
    });
    let advanced = capped.or_else(|| prev_since.map(String::from));
    match (advanced, prev_since) {
        (Some(a), Some(p)) if a.as_str() < p => Some(p.to_string()),
        (a, _) => a,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_advances_to_exported_max_not_wallclock() {
        let c = compute_new_cursor(
            None,
            Some("2026-04-21 08:00:00"),
            None,
            "2026-04-21 12:00:00",
        );
        assert_eq!(c.as_deref(), Some("2026-04-21 08:00:00"));
    }

    #[test]
    fn cursor_takes_max_of_exported_and_applied() {
        let c = compute_new_cursor(
            None,
            Some("2026-04-21 08:00:00"),
            Some("2026-04-21 09:00:00"),
            "2026-04-21 12:00:00",
        );
        assert_eq!(c.as_deref(), Some("2026-04-21 09:00:00"));
    }

    #[test]
    fn cursor_capped_at_round_start_for_future_skew() {
        let c = compute_new_cursor(
            None,
            Some("2099-01-01 00:00:00"),
            None,
            "2026-04-21 12:00:00",
        );
        assert_eq!(c.as_deref(), Some("2026-04-21 12:00:00"));
    }

    #[test]
    fn cursor_stays_put_when_nothing_exchanged() {
        let c = compute_new_cursor(
            Some("2026-04-21 10:00:00"),
            None,
            None,
            "2026-04-21 12:00:00",
        );
        assert_eq!(c.as_deref(), Some("2026-04-21 10:00:00"));
    }

    #[test]
    fn cursor_never_moves_backwards() {
        let c = compute_new_cursor(
            Some("2026-04-21 11:00:00"),
            Some("2026-04-21 08:00:00"),
            None,
            "2026-04-21 12:00:00",
        );
        assert_eq!(c.as_deref(), Some("2026-04-21 11:00:00"));
    }

    #[test]
    fn cursor_no_previous_no_exchange_returns_none() {
        let c = compute_new_cursor(None, None, None, "2026-04-21 12:00:00");
        assert!(c.is_none());
    }
}
