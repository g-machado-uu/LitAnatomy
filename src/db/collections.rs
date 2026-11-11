use std::collections::HashMap;

use anyhow::Result;
use rusqlite::{OptionalExtension, params};

use super::Library;
use crate::model::{Collection, Paper, now_string};

impl Library {
    pub fn collections(&self) -> Result<Vec<Collection>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, description, colour, parent_id, archived, bib_export_path, position, screening
             FROM collections ORDER BY position, name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(Collection {
                id: r.get(0)?,
                name: r.get(1)?,
                description: r.get(2)?,
                colour: r.get::<_, i64>(3)?.clamp(0, 255) as u8,
                parent: r.get(4)?,
                archived: r.get(5)?,
                bib_export_path: r.get(6)?,
                position: r.get(7)?,
                screening: r.get(8)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn collection(&self, id: i64) -> Result<Option<Collection>> {
        Ok(self.collections()?.into_iter().find(|c| c.id == id))
    }

    pub fn create_collection(&mut self, name: &str, parent: Option<i64>, colour: u8) -> Result<i64> {
        let position: i64 =
            self.conn.query_row("SELECT coalesce(max(position), -1) + 1 FROM collections", [], |r| r.get(0))?;
        self.conn.execute(
            "INSERT INTO collections (name, parent_id, colour, position, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![name.trim(), parent, colour, position, now_string()],
        )?;
        self.touch();
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_collection(&mut self, c: &Collection) -> Result<()> {
        // A collection can't become its own ancestor.
        let parent = c.parent.filter(|p| *p != c.id && !self.descendants(c.id).unwrap_or_default().contains(p));
        self.conn.execute(
            "UPDATE collections SET name = ?2, description = ?3, colour = ?4, parent_id = ?5, archived = ?6,
                bib_export_path = ?7, position = ?8, screening = ?9 WHERE id = ?1",
            params![
                c.id,
                c.name.trim(),
                c.description,
                c.colour,
                parent,
                c.archived,
                c.bib_export_path.as_deref().filter(|p| !p.trim().is_empty()),
                c.position,
                c.screening
            ],
        )?;
        self.touch();
        Ok(())
    }

    /// Deletes a collection and its sub-collections. Papers stay in the library.
    pub fn delete_collection(&mut self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM collections WHERE id = ?1", [id])?;
        self.touch();
        Ok(())
    }

    /// Ids of every collection below `id`.
    pub fn descendants(&self, id: i64) -> Result<Vec<i64>> {
        let mut stmt = self.conn.prepare(
            "WITH RECURSIVE sub(id) AS (
                SELECT id FROM collections WHERE parent_id = ?1
                UNION SELECT c.id FROM collections c JOIN sub ON c.parent_id = sub.id)
             SELECT id FROM sub",
        )?;
        let ids = stmt.query_map([id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(ids)
    }

    /// Adds papers to a collection; returns how many were not there already.
    pub fn add_to_collection(&mut self, collection: i64, papers: &[i64]) -> Result<usize> {
        let now = now_string();
        let tx = self.conn.transaction()?;
        let mut added = 0;
        for p in papers {
            added += tx.execute(
                "INSERT OR IGNORE INTO collection_papers (collection_id, paper_id, added_at) VALUES (?1, ?2, ?3)",
                params![collection, p, now],
            )?;
        }
        tx.commit()?;
        self.touch();
        Ok(added)
    }

    pub fn remove_from_collection(&mut self, collection: i64, papers: &[i64]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for p in papers {
            tx.execute(
                "DELETE FROM collection_papers WHERE collection_id = ?1 AND paper_id = ?2",
                params![collection, p],
            )?;
        }
        tx.commit()?;
        self.touch();
        Ok(())
    }

    /// The collections a paper is in, with its relevance note for each.
    pub fn memberships(&self, paper: i64) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT cp.collection_id, cp.relevance FROM collection_papers cp
             JOIN collections c ON c.id = cp.collection_id
             WHERE cp.paper_id = ?1 ORDER BY c.position, c.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([paper], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn relevance(&self, collection: i64, paper: i64) -> Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT relevance FROM collection_papers WHERE collection_id = ?1 AND paper_id = ?2",
                params![collection, paper],
                |r| r.get(0),
            )
            .optional()?)
    }

    pub fn set_relevance(&mut self, collection: i64, paper: i64, text: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE collection_papers SET relevance = ?3 WHERE collection_id = ?1 AND paper_id = ?2",
            params![collection, paper, text],
        )?;
        self.conn.execute("UPDATE papers SET modified_at = ?2 WHERE id = ?1", params![paper, now_string()])?;
        self.reindex(paper)?;
        self.touch();
        Ok(())
    }

    /// Per collection: (papers, summarised papers), not counting the Trash.
    pub fn collection_stats(&self) -> Result<HashMap<i64, (usize, usize)>> {
        let mut stmt = self.conn.prepare(
            "SELECT cp.collection_id, count(*), sum(p.status = 2) FROM collection_papers cp
             JOIN papers p ON p.id = cp.paper_id WHERE p.deleted_at IS NULL GROUP BY cp.collection_id",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((r.get::<_, i64>(0)?, (r.get::<_, i64>(1)? as usize, r.get::<_, i64>(2)? as usize)))
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// The papers to export for a collection: its own and its sub-collections'.
    pub fn export_papers(&self, collection: i64) -> Result<Vec<Paper>> {
        let mut ids = vec![collection];
        ids.extend(self.descendants(collection)?);
        let mut paper_ids: Vec<i64> = Vec::new();
        for c in ids {
            let mut stmt = self.conn.prepare(
                "SELECT cp.paper_id FROM collection_papers cp JOIN papers p ON p.id = cp.paper_id
                 WHERE cp.collection_id = ?1 AND p.deleted_at IS NULL",
            )?;
            for id in stmt.query_map([c], |r| r.get::<_, i64>(0))? {
                let id = id?;
                if !paper_ids.contains(&id) {
                    paper_ids.push(id);
                }
            }
        }
        self.papers(&paper_ids)
    }

    /// Swaps a collection with its neighbour among its siblings.
    pub fn move_collection(&mut self, id: i64, up: bool) -> Result<()> {
        let all = self.collections()?;
        let Some(me) = all.iter().find(|c| c.id == id) else { return Ok(()) };
        let siblings: Vec<&_> = all.iter().filter(|c| c.parent == me.parent).collect();
        let Some(pos) = siblings.iter().position(|c| c.id == id) else { return Ok(()) };
        let other = if up { pos.checked_sub(1) } else { Some(pos + 1).filter(|p| *p < siblings.len()) };
        let Some(other) = other else { return Ok(()) };
        let tx = self.conn.transaction()?;
        // Renumber the siblings in their new order so equal positions can't stick.
        let mut order: Vec<i64> = siblings.iter().map(|c| c.id).collect();
        order.swap(pos, other);
        for (i, cid) in order.iter().enumerate() {
            tx.execute("UPDATE collections SET position = ?2 WHERE id = ?1", params![cid, i as i64])?;
        }
        tx.commit()?;
        self.touch();
        Ok(())
    }
}
