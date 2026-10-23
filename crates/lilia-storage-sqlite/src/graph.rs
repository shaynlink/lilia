#![allow(clippy::missing_errors_doc)]

use rusqlite::params;

use crate::storage::{normalize_limit, validate_name};
use crate::Database;
use lilia_core::{Direction, ErrorCode, GraphEdge, GraphNode, LiliaError, Result};

impl Database {
    pub fn graph_set_node(
        &self,
        graph: &str,
        id: &str,
        kind: &str,
        metadata: &serde_json::Value,
    ) -> Result<()> {
        validate_name("graph", graph)?;
        validate_name("id", id)?;
        let meta_str = serde_json::to_string(metadata)
            .map_err(|e| LiliaError::new(ErrorCode::InvalidInput, e.to_string(), false))?;
        let writer = self.lock_writer()?;
        let mut stmt = writer.prepare_cached(
            "INSERT INTO _lilia_graph_nodes(graph, id, kind, metadata) VALUES(?1, ?2, ?3, ?4)
             ON CONFLICT(graph, id) DO UPDATE SET kind=excluded.kind, metadata=excluded.metadata",
        )?;
        stmt.execute(params![graph, id, kind, meta_str])?;
        Ok(())
    }

    pub fn graph_get_node(&self, graph: &str, id: &str) -> Result<Option<GraphNode>> {
        validate_name("graph", graph)?;
        validate_name("id", id)?;
        let conn = self.read_lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT kind, metadata FROM _lilia_graph_nodes WHERE graph = ?1 AND id = ?2",
        )?;
        let mut rows = stmt.query(params![graph, id])?;
        if let Some(row) = rows.next()? {
            let kind: String = row.get(0)?;
            let meta_str: String = row.get(1)?;
            let metadata = serde_json::from_str(&meta_str)
                .map_err(|e| LiliaError::new(ErrorCode::Storage, e.to_string(), false))?;
            Ok(Some(GraphNode {
                graph: graph.to_owned(),
                id: id.to_owned(),
                kind,
                metadata,
            }))
        } else {
            Ok(None)
        }
    }

    pub fn graph_delete_node(&self, graph: &str, id: &str) -> Result<bool> {
        validate_name("graph", graph)?;
        validate_name("id", id)?;
        let mut writer = self.lock_writer()?;
        let tx = writer.transaction()?;
        tx.execute(
            "DELETE FROM _lilia_graph_edges WHERE graph = ?1 AND (source = ?2 OR target = ?2)",
            params![graph, id],
        )?;
        let deleted = tx.execute(
            "DELETE FROM _lilia_graph_nodes WHERE graph = ?1 AND id = ?2",
            params![graph, id],
        )? > 0;
        tx.commit()?;
        Ok(deleted)
    }

    pub fn graph_list_nodes(
        &self,
        graph: &str,
        kind_filter: Option<&str>,
        limit: u32,
    ) -> Result<Vec<GraphNode>> {
        validate_name("graph", graph)?;
        let conn = self.read_lock()?;
        let query = if kind_filter.is_some() {
            "SELECT id, kind, metadata FROM _lilia_graph_nodes WHERE graph = ?1 AND kind = ?2 ORDER BY id LIMIT ?3"
        } else {
            "SELECT id, kind, metadata FROM _lilia_graph_nodes WHERE graph = ?1 ORDER BY id LIMIT ?2"
        };
        let mut stmt = conn.prepare_cached(query)?;
        let mut rows = if let Some(kind) = kind_filter {
            stmt.query(params![graph, kind, normalize_limit(limit)])?
        } else {
            stmt.query(params![graph, normalize_limit(limit)])?
        };
        let mut nodes = Vec::new();
        while let Some(r) = rows.next()? {
            let id: String = r.get(0)?;
            let kind: String = r.get(1)?;
            let meta_str: String = r.get(2)?;
            let metadata = serde_json::from_str(&meta_str)
                .map_err(|e| LiliaError::new(ErrorCode::Storage, e.to_string(), false))?;
            nodes.push(GraphNode {
                graph: graph.to_owned(),
                id,
                kind,
                metadata,
            });
        }
        Ok(nodes)
    }

    pub fn graph_add_edge(
        &self,
        graph: &str,
        source: &str,
        target: &str,
        kind: &str,
    ) -> Result<()> {
        validate_name("graph", graph)?;
        validate_name("source", source)?;
        validate_name("target", target)?;
        let writer = self.lock_writer()?;
        let mut stmt = writer.prepare_cached(
            "INSERT OR IGNORE INTO _lilia_graph_edges(graph, source, target, kind) VALUES(?1, ?2, ?3, ?4)",
        )?;
        stmt.execute(params![graph, source, target, kind])?;
        Ok(())
    }

    pub fn graph_remove_edge(
        &self,
        graph: &str,
        source: &str,
        target: &str,
        kind: &str,
    ) -> Result<bool> {
        validate_name("graph", graph)?;
        validate_name("source", source)?;
        validate_name("target", target)?;
        let writer = self.lock_writer()?;
        let mut stmt = writer.prepare_cached(
            "DELETE FROM _lilia_graph_edges WHERE graph = ?1 AND source = ?2 AND target = ?3 AND kind = ?4",
        )?;
        let deleted = stmt.execute(params![graph, source, target, kind])? > 0;
        Ok(deleted)
    }

    pub fn graph_list_edges(
        &self,
        graph: &str,
        source: Option<&str>,
        target: Option<&str>,
        kind: Option<&str>,
        limit: u32,
    ) -> Result<Vec<GraphEdge>> {
        validate_name("graph", graph)?;
        let conn = self.read_lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT source, target, kind FROM _lilia_graph_edges
             WHERE graph = ?1
               AND (?2 IS NULL OR source = ?2)
               AND (?3 IS NULL OR target = ?3)
               AND (?4 IS NULL OR kind = ?4)
             ORDER BY source, target, kind
             LIMIT ?5",
        )?;
        let rows = stmt.query_map(
            params![graph, source, target, kind, normalize_limit(limit)],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let mut edges = Vec::new();
        for r in rows {
            let (s, t, k) = r?;
            edges.push(GraphEdge {
                graph: graph.to_owned(),
                source: s,
                target: t,
                kind: k,
            });
        }
        Ok(edges)
    }

    pub fn graph_reachability(
        &self,
        graph: &str,
        start_nodes: &[String],
        direction: Direction,
    ) -> Result<Vec<String>> {
        validate_name("graph", graph)?;
        if start_nodes.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.read_lock()?;
        let starts_json = serde_json::to_string(start_nodes)
            .map_err(|e| LiliaError::new(ErrorCode::InvalidInput, e.to_string(), false))?;

        let sql = match direction {
            Direction::Incoming => {
                "WITH RECURSIVE reached(node) AS (
                    SELECT value FROM json_each(?1)
                    UNION
                    SELECT e.source FROM _lilia_graph_edges e
                    JOIN reached r ON e.target = r.node
                    WHERE e.graph = ?2
                )
                SELECT DISTINCT node FROM reached ORDER BY node;"
            }
            Direction::Outgoing => {
                "WITH RECURSIVE reached(node) AS (
                    SELECT value FROM json_each(?1)
                    UNION
                    SELECT e.target FROM _lilia_graph_edges e
                    JOIN reached r ON e.source = r.node
                    WHERE e.graph = ?2
                )
                SELECT DISTINCT node FROM reached ORDER BY node;"
            }
            Direction::Both => {
                "WITH RECURSIVE reached(node) AS (
                    SELECT value FROM json_each(?1)
                    UNION
                    SELECT CASE WHEN e.source = r.node THEN e.target ELSE e.source END
                    FROM _lilia_graph_edges e
                    JOIN reached r ON e.source = r.node OR e.target = r.node
                    WHERE e.graph = ?2
                )
                SELECT DISTINCT node FROM reached ORDER BY node;"
            }
        };

        let mut stmt = conn.prepare_cached(sql)?;
        let rows = stmt.query_map(params![starts_json, graph], |r| r.get::<_, String>(0))?;
        let mut nodes = Vec::new();
        for r in rows {
            nodes.push(r?);
        }
        Ok(nodes)
    }
}
