#![allow(clippy::missing_errors_doc)]

use rusqlite::params;

use crate::storage::{normalize_limit, validate_name};
use crate::Database;
use lilia_core::{FtsHit, Result};

impl Database {
    pub fn fts_index(&self, collection: &str, doc_id: &str, content: &str) -> Result<()> {
        validate_name("collection", collection)?;
        validate_name("doc_id", doc_id)?;
        let mut writer = self.lock_writer()?;
        let tx = writer.transaction()?;
        tx.execute(
            "DELETE FROM _lilia_fts WHERE collection = ?1 AND doc_id = ?2",
            params![collection, doc_id],
        )?;
        tx.execute(
            "INSERT INTO _lilia_fts(collection, doc_id, content) VALUES(?1, ?2, ?3)",
            params![collection, doc_id, content],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn fts_delete(&self, collection: &str, doc_id: &str) -> Result<bool> {
        validate_name("collection", collection)?;
        validate_name("doc_id", doc_id)?;
        let writer = self.lock_writer()?;
        let mut stmt = writer
            .prepare_cached("DELETE FROM _lilia_fts WHERE collection = ?1 AND doc_id = ?2")?;
        let deleted = stmt.execute(params![collection, doc_id])? > 0;
        Ok(deleted)
    }

    pub fn fts_search(&self, collection: &str, query: &str, limit: u32) -> Result<Vec<FtsHit>> {
        validate_name("collection", collection)?;
        let conn = self.read_lock()?;
        let mut stmt = conn.prepare_cached(
            "SELECT doc_id, snippet(_lilia_fts, 2, '<b>', '</b>', '...', 32), rank
             FROM _lilia_fts
             WHERE collection = ?1 AND _lilia_fts MATCH ?2
             ORDER BY rank
             LIMIT ?3",
        )?;
        let match_query = if query.contains(':')
            || query.contains('"')
            || query.contains("AND")
            || query.contains("OR")
            || query.contains('*')
        {
            query.to_string()
        } else {
            let tokens: Vec<String> = query
                .split_whitespace()
                .map(|token| format!("\"{}\"", token.replace('"', "\"\"")))
                .collect();
            if tokens.is_empty() {
                return Ok(Vec::new());
            }
            tokens.join(" ")
        };
        let rows = stmt.query_map(
            params![collection, match_query, normalize_limit(limit)],
            |r| {
                Ok(FtsHit {
                    collection: collection.to_owned(),
                    doc_id: r.get(0)?,
                    snippet: r.get(1)?,
                    rank: r.get(2)?,
                })
            },
        )?;
        let mut hits = Vec::new();
        for r in rows {
            hits.push(r?);
        }
        Ok(hits)
    }
}
