#![allow(clippy::missing_errors_doc)]

use lilia_core::{Direction, GraphEdge, GraphNode, Result};
use lilia_storage_sqlite::Database;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImpactAnalysis {
    pub direct: Vec<String>,
    pub dependants: Vec<String>,
    pub total_affected: usize,
}

#[derive(Debug)]
pub struct GraphStore<'a> {
    db: &'a Database,
    graph_id: String,
}

impl<'a> GraphStore<'a> {
    #[must_use]
    pub fn new(db: &'a Database, graph_id: impl Into<String>) -> Self {
        Self {
            db,
            graph_id: graph_id.into(),
        }
    }

    #[must_use]
    pub fn graph_id(&self) -> &str {
        &self.graph_id
    }

    pub fn set_node(&self, id: &str, kind: &str, metadata: &serde_json::Value) -> Result<()> {
        self.db.graph_set_node(&self.graph_id, id, kind, metadata)
    }

    pub fn get_node(&self, id: &str) -> Result<Option<GraphNode>> {
        self.db.graph_get_node(&self.graph_id, id)
    }

    pub fn delete_node(&self, id: &str) -> Result<bool> {
        self.db.graph_delete_node(&self.graph_id, id)
    }

    pub fn list_nodes(&self, kind: Option<&str>, limit: u32) -> Result<Vec<GraphNode>> {
        self.db.graph_list_nodes(&self.graph_id, kind, limit)
    }

    pub fn add_edge(&self, source: &str, target: &str, kind: &str) -> Result<()> {
        self.db.graph_add_edge(&self.graph_id, source, target, kind)
    }

    pub fn remove_edge(&self, source: &str, target: &str, kind: &str) -> Result<bool> {
        self.db
            .graph_remove_edge(&self.graph_id, source, target, kind)
    }

    pub fn list_edges(
        &self,
        source: Option<&str>,
        target: Option<&str>,
        kind: Option<&str>,
        limit: u32,
    ) -> Result<Vec<GraphEdge>> {
        self.db
            .graph_list_edges(&self.graph_id, source, target, kind, limit)
    }

    pub fn reachability(
        &self,
        start_nodes: &[String],
        direction: Direction,
    ) -> Result<Vec<String>> {
        self.db
            .graph_reachability(&self.graph_id, start_nodes, direction)
    }

    pub fn impact_analysis(&self, changed_nodes: &[String]) -> Result<ImpactAnalysis> {
        if changed_nodes.is_empty() {
            return Ok(ImpactAnalysis {
                direct: Vec::new(),
                dependants: Vec::new(),
                total_affected: 0,
            });
        }
        let direct_set: BTreeSet<String> = changed_nodes.iter().cloned().collect();
        // Traverse incoming edges (all nodes that depend on changed_nodes)
        let reached = self.reachability(changed_nodes, Direction::Incoming)?;
        let mut dependants = Vec::new();
        for node in reached {
            if !direct_set.contains(&node) {
                dependants.push(node);
            }
        }
        let total_affected = direct_set.len() + dependants.len();
        Ok(ImpactAnalysis {
            direct: direct_set.into_iter().collect(),
            dependants,
            total_affected,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lilia_storage_sqlite::DatabaseOptions;
    use serde_json::json;

    #[test]
    fn test_graph_store_and_impact() {
        let dir = tempfile::tempdir().unwrap();
        let db = Database::open(DatabaseOptions::durable(
            dir.path().join("store_test.lilia"),
        ))
        .unwrap();
        let graph = GraphStore::new(&db, "workspace_deps");

        graph
            .set_node("core", "library", &json!({ "pkg": "core" }))
            .unwrap();
        graph
            .set_node("auth", "library", &json!({ "pkg": "auth" }))
            .unwrap();
        graph
            .set_node("api", "service", &json!({ "pkg": "api" }))
            .unwrap();
        graph
            .set_node("web", "frontend", &json!({ "pkg": "web" }))
            .unwrap();

        // api -> auth -> core
        // web -> api
        graph.add_edge("auth", "core", "depends_on").unwrap();
        graph.add_edge("api", "auth", "depends_on").unwrap();
        graph.add_edge("web", "api", "depends_on").unwrap();

        let impact = graph.impact_analysis(&["core".to_string()]).unwrap();
        assert_eq!(impact.direct, vec!["core"]);
        assert_eq!(impact.dependants, vec!["api", "auth", "web"]);
        assert_eq!(impact.total_affected, 4);

        let impact_auth = graph.impact_analysis(&["auth".to_string()]).unwrap();
        assert_eq!(impact_auth.direct, vec!["auth"]);
        assert_eq!(impact_auth.dependants, vec!["api", "web"]);
        assert_eq!(impact_auth.total_affected, 3);
    }
}
