---
version: "1.0"
last_updated: "2026-04-07"
author: "convergio-team"
tags: ["adr"]
---

# ADR-023: Mesh Multi-Transport Auth

**Status:** Accepted
**Date:** 2026-04-05
**Deciders:** Roberto D'Angelo

## Context

Convergio mesh connects heterogeneous Apple Silicon nodes (M1 Pro, M4 Max, etc.)
across multiple network topologies. Nodes may be on the same desk (Thunderbolt),
same LAN (Wi-Fi/Ethernet), or remote (Tailscale VPN). The mesh must:

- Authenticate all cross-node API calls
- Failover transparently between transports
- Avoid self-sync loops (node syncing to itself)
- Maintain liveness via heartbeat

Session work exposed three bugs:

1. **URL without scheme** — `peers.conf` had `10.0.0.2:3001` instead of
   `http://10.0.0.2:3001`, causing `reqwest` to interpret the host as a scheme
2. **Self-sync** — a node would discover its own Tailscale IP as a peer and
   attempt to sync with itself, creating duplicate events
3. **401 cross-node** — `CONVERGIO_AUTH_TOKEN` was per-machine; nodes rejected
   each other's Bearer tokens

## Decision

### Authentication — dual layer

| Layer | Mechanism | Scope |
|-------|-----------|-------|
| Bearer token | `CONVERGIO_AUTH_TOKEN` env var | CLI-to-daemon (local) |
| HMAC signature | `shared_secret` in `peers.conf` | Peer-to-peer (cross-node) |

Cross-node requests include both:
- `Authorization: Bearer <local_token>` (validated by receiving daemon)
- `X-Mesh-HMAC: <hmac_sha256(body, shared_secret)>` (validates message integrity)

The `shared_secret` is identical across all nodes in the cluster, set once during
`cvg setup` and distributed via secure copy. Bearer tokens remain per-machine.

### Multi-transport failover

Each peer in `peers.conf` declares ordered transport addresses:

```toml
[[peer]]
name = "kernel"
role = "kernel"
addresses = [
    "10.0.0.2:3001",    # Thunderbolt (fastest, ~40Gbps)
    "192.168.1.50:3001", # LAN (fallback)
    "100.64.0.2:3001",   # Tailscale (last resort)
]
shared_secret = "abc123..."
```

The mesh client tries addresses in order. On connection failure or timeout (2s),
it falls through to the next address. Successful address is cached for 60s.

Priority logic: **Thunderbolt** (`10.0.0.x`) > **LAN** (`192.168.x.x`) >
**Tailscale** (`100.x.x.x`).

### Self-skip

On startup, the daemon collects all local IPs (including Tailscale `100.x.x.x`).
When iterating peers, any peer whose addresses all match local IPs is skipped.
This eliminates self-sync without relying on hostname comparison.

### Heartbeat

Bidirectional heartbeat every **30 seconds**:

```text
Node A ──heartbeat──> Node B    (includes local timestamp + load)
Node A <──heartbeat── Node B    (response includes B's timestamp + load)
```

If 3 consecutive heartbeats fail (90s), the peer is marked `unreachable`.
The mesh UI (`cvg mesh status`) shows per-peer transport, latency, and state.

## Bugs Fixed

| Bug | Root cause | Fix |
|-----|-----------|-----|
| URL without `http://` | `peers.conf` stored bare `host:port` | Prepend `http://` if no scheme detected |
| Self-sync loop | Tailscale IP not in local IP set | Collect all interface IPs including `utun` |
| 401 cross-node | Bearer token was machine-specific | Add HMAC layer for peer auth, keep Bearer for local |

## Consequences

- New nodes must receive `shared_secret` during setup (security boundary)
- `peers.conf` is the single source of truth for mesh topology
- Transport failover adds up to 4s latency on first call if Thunderbolt+LAN are down
- Heartbeat interval (30s) balances liveness detection vs. network overhead
- Self-skip is IP-based, not name-based — works across hostname changes
