//! MCP tool definitions for the mesh extension.

use convergio_types::extension::McpToolDef;
use serde_json::json;

pub fn mesh_tools() -> Vec<McpToolDef> {
    vec![
        McpToolDef {
            name: "cvg_mesh_status".into(),
            description: "Get peer topology and active connections.".into(),
            method: "GET".into(),
            path: "/api/mesh".into(),
            input_schema: json!({"type": "object", "properties": {}}),
            min_ring: "community".into(),
            path_params: vec![],
        },
        McpToolDef {
            name: "cvg_node_readiness".into(),
            description: "Run node health checks and return readiness report.".into(),
            method: "GET".into(),
            path: "/api/node/readiness".into(),
            input_schema: json!({"type": "object", "properties": {}}),
            min_ring: "community".into(),
            path_params: vec![],
        },
    ]
}
