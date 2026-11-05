//! A collection's synthesis: notes across its papers, themes that group
//! them, and a draft outline of the related-work section.

use anyhow::{Result, bail};
use rusqlite::params;

use super::Library;
use crate::model::Theme;

impl Library {
    /// The collection's overview notes and outline.
    pub fn synthesis(&self, collection: i64) -> Result<(String, String)> {
        Ok(self.conn.query_row(
            "SELECT synthesis, outline FROM collections WHERE id = ?1",
            [collection],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?)
    }

    pub fn set_synthesis_notes(&mut self, collection: i64, text: &str) -> Result<()> {
        self.conn.execute("UPDATE collections SET synthesis = ?2 WHERE id = ?1", params![collection, text])?;
        self.touch();
        Ok(())
    }

    pub fn set_outline(&mut self, collection: i64, text: &str) -> Result<()> {
        self.conn.execute("UPDATE collections SET outline = ?2 WHERE id = ?1", params![collection, text])?;
        self.touch();
        Ok(())
    }

    /// The collection's themes in order, each with its papers (not those in the Trash).
    pub fn themes(&self, collection: i64) -> Result<Vec<Theme>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, collection_id, name, notes, position FROM themes WHERE collection_id = ?1 ORDER BY position, id",
        )?;
        let mut themes: Vec<Theme> = stmt
            .query_map([collection], |r| {
                Ok(Theme {
                    id: r.get(0)?,
                    collection: r.get(1)?,
                    name: r.get(2)?,
                    notes: r.get(3)?,
                    position: r.get(4)?,
                    papers: Vec::new(),
                })
            })?
            .collect::<rusqlite::Result<_>>()?;
        let mut stmt = self.conn.prepare(
            "SELECT tp.paper_id, tp.note FROM theme_papers tp JOIN papers p ON p.id = tp.paper_id
             WHERE tp.theme_id = ?1 AND p.deleted_at IS NULL ORDER BY p.year, p.title COLLATE NOCASE",
        )?;
        for t in &mut themes {
            t.papers = stmt.query_map([t.id], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        }
        Ok(themes)
    }

    pub fn add_theme(&mut self, collection: i64, name: &str) -> Result<i64> {
        if name.trim().is_empty() {
            bail!("Give the theme a name.");
        }
        let position: i64 = self.conn.query_row(
            "SELECT coalesce(max(position), -1) + 1 FROM themes WHERE collection_id = ?1",
            [collection],
            |r| r.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO themes (collection_id, name, position) VALUES (?1, ?2, ?3)",
            params![collection, name.trim(), position],
        )?;
        self.touch();
        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_theme(&mut self, id: i64, name: &str, notes: &str) -> Result<()> {
        if name.trim().is_empty() {
            bail!("A theme needs a name.");
        }
        self.conn.execute("UPDATE themes SET name = ?2, notes = ?3 WHERE id = ?1", params![id, name.trim(), notes])?;
        self.touch();
        Ok(())
    }

    pub fn delete_theme(&mut self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM themes WHERE id = ?1", [id])?;
        self.touch();
        Ok(())
    }

    /// Swaps a theme with its neighbour.
    pub fn move_theme(&mut self, id: i64, up: bool) -> Result<()> {
        let collection: i64 = self.conn.query_row("SELECT collection_id FROM themes WHERE id = ?1", [id], |r| r.get(0))?;
        let mut order: Vec<i64> = self.themes(collection)?.iter().map(|t| t.id).collect();
        let Some(pos) = order.iter().position(|t| *t == id) else { return Ok(()) };
        let other = if up { pos.checked_sub(1) } else { Some(pos + 1).filter(|p| *p < order.len()) };
        let Some(other) = other else { return Ok(()) };
        order.swap(pos, other);
        let tx = self.conn.transaction()?;
        for (i, tid) in order.iter().enumerate() {
            tx.execute("UPDATE themes SET position = ?2 WHERE id = ?1", params![tid, i as i64])?;
        }
        tx.commit()?;
        self.touch();
        Ok(())
    }

    /// Adds papers to a theme; returns how many were not there already.
    pub fn add_to_theme(&mut self, theme: i64, papers: &[i64]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut added = 0;
        for p in papers {
            added += tx.execute("INSERT OR IGNORE INTO theme_papers (theme_id, paper_id) VALUES (?1, ?2)", params![theme, p])?;
        }
        tx.commit()?;
        self.touch();
        Ok(added)
    }

    pub fn remove_from_theme(&mut self, theme: i64, paper: i64) -> Result<()> {
        self.conn.execute("DELETE FROM theme_papers WHERE theme_id = ?1 AND paper_id = ?2", params![theme, paper])?;
        self.touch();
        Ok(())
    }

    /// How a paper fits a theme.
    pub fn set_theme_note(&mut self, theme: i64, paper: i64, note: &str) -> Result<()> {
        self.conn.execute(
            "UPDATE theme_papers SET note = ?3 WHERE theme_id = ?1 AND paper_id = ?2",
            params![theme, paper, note],
        )?;
        self.touch();
        Ok(())
    }

    /// The themes a paper is in: (collection, theme name).
    pub fn themes_of(&self, paper: i64) -> Result<Vec<(i64, String)>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.collection_id, t.name FROM theme_papers tp JOIN themes t ON t.id = tp.theme_id
             WHERE tp.paper_id = ?1 ORDER BY t.collection_id, t.position",
        )?;
        let rows = stmt.query_map([paper], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}
