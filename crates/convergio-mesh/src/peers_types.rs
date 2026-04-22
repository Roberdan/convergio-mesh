//! Peer configuration types for peers.conf INI format.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PeersError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("missing required field '{field}' in peer '{peer}'")]
    MissingField { peer: String, field: String },
    #[error("peer '{0}' not found")]
    NotFound(String),
    #[error("parse error at line {line}: {msg}")]
    Parse { line: usize, msg: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PeerConfig {
    pub ssh_alias: String,
    pub user: String,
    pub os: String,
    pub tailscale_ip: String,
    pub dns_name: String,
    pub capabilities: Vec<String>,
    pub role: String,
    pub status: String,
    pub thunderbolt_ip: Option<String>,
    pub lan_ip: Option<String>,
    pub mac_address: Option<String>,
    pub gh_account: Option<String>,
    pub runners: Option<u32>,
    pub runner_paths: Option<String>,
    /// Path to convergio repo on this node (for sync-repo / SSH deploy).
    pub repo_path: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PeersRegistry {
    pub shared_secret: String,
    pub peers: BTreeMap<String, PeerConfig>,
}

/// Canonicalize a peer/host name so duplicates collapse to a single identity.
///
/// Rules:
/// - Trim surrounding whitespace.
/// - Strip trailing `.local` (mDNS suffix), case-insensitive.
/// - Lowercase the result.
///
/// Example: `M5Max.local`, `M5Max`, `m5max.LOCAL` → `m5max`.
pub fn canonical_peer_name(raw: &str) -> String {
    let trimmed = raw.trim();
    let stripped = if trimmed.len() >= 6 {
        let (head, tail) = trimmed.split_at(trimmed.len() - 6);
        if tail.eq_ignore_ascii_case(".local") {
            head
        } else {
            trimmed
        }
    } else {
        trimmed
    };
    stripped.to_ascii_lowercase()
}

#[cfg(test)]
mod canonical_tests {
    use super::canonical_peer_name;

    #[test]
    fn collapses_dot_local_and_case() {
        assert_eq!(canonical_peer_name("M5Max.local"), "m5max");
        assert_eq!(canonical_peer_name("M5Max"), "m5max");
        assert_eq!(canonical_peer_name("m5max.LOCAL"), "m5max");
        assert_eq!(canonical_peer_name("M5max.Local"), "m5max");
    }

    #[test]
    fn trims_whitespace() {
        assert_eq!(canonical_peer_name("  M5Max.local  "), "m5max");
    }

    #[test]
    fn leaves_non_local_suffix_intact() {
        assert_eq!(canonical_peer_name("node1.ts.net"), "node1.ts.net");
        assert_eq!(canonical_peer_name("Worker-7"), "worker-7");
    }

    #[test]
    fn bare_local_suffix_yields_empty() {
        // `.local` alone (6 chars) collapses to `""`; defined behaviour for
        // edge case — callers must never pass a bare suffix.
        assert_eq!(canonical_peer_name(".local"), "");
    }
}
