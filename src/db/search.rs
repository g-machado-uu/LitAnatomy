use std::collections::HashSet;

use anyhow::Result;
use rusqlite::params;

use super::Library;
use crate::model::{Author, authors_long};

impl Library {
    /// Rebuilds a paper's entry in the full-text index.
    pub(super) fn reindex(&self, paper: i64) -> Result<()> {
        let (title, authors, details): (String, String, String) = self.conn.query_row(
            "SELECT title, authors, container || ' ' || keywords || ' ' || abstract || ' ' || citekey || ' ' || coalesce(doi, '')
             FROM papers WHERE id = ?1",
            [paper],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )?;
        let authors: Vec<Author> = serde_json::from_str(&authors).unwrap_or_default();
        let notes: String = self.conn.query_row(
            "SELECT coalesce(group_concat(content, ' '), '') || ' ' ||
                    coalesce((SELECT group_concat(relevance, ' ') FROM collection_papers WHERE paper_id = ?1), '')
             FROM notes WHERE paper_id = ?1",
            [paper],
            |r| r.get(0),
        )?;
        self.conn.execute("DELETE FROM search WHERE rowid = ?1", [paper])?;
        self.conn.execute(
            "INSERT INTO search (rowid, title, authors, details, notes) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![paper, title, authors_long(&authors), details, notes],
        )?;
        Ok(())
    }

    /// Papers matching every word of `query` (as a prefix) in their title,
    /// authors, venue, keywords, abstract, key, DOI or notes.
    pub fn search(&self, query: &str) -> Result<HashSet<i64>> {
        let terms: Vec<String> = query
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(|w| format!("\"{w}\"*"))
            .collect();
        if terms.is_empty() {
            return Ok(HashSet::new());
        }
        let mut stmt = self.conn.prepare("SELECT rowid FROM search WHERE search MATCH ?1")?;
        let rows = stmt.query_map([terms.join(" ")], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}
