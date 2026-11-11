use std::collections::HashMap;

use anyhow::{Result, bail};
use rusqlite::{Row, params};

use super::{Library, kind_of};
use crate::model::{Field, FieldKind, now_string};

/// The default "anatomy" of a paper: key, label, prompt and kind.
pub const DEFAULT_FIELDS: &[(&str, &str, &str, FieldKind)] = &[
    ("problem", "Problem & Motivation", "What gap or problem drove this work, and why does it matter?", FieldKind::LongText),
    ("contribution", "Contribution", "What is new here? Say it in two or three sentences.", FieldKind::LongText),
    ("methodology", "Methodology", "How did they do it? Design, model, set-up, simulation or measurement.", FieldKind::LongText),
    ("findings", "Key Findings", "What did they achieve, and what do the results show?", FieldKind::LongText),
    ("metrics", "Key Metrics", "Headline numbers to compare papers by: name, value and unit.", FieldKind::Metrics),
    ("limitations", "Limitations", "Weaknesses the authors admit, and the ones you notice.", FieldKind::LongText),
    ("gaps", "Research Gaps & Future Work", "What is still open? Could it become your contribution?", FieldKind::LongText),
    ("assessment", "Critical Assessment", "How convincing is it? Validation, fair comparisons, reproducibility.", FieldKind::LongText),
    ("quotes", "Key Quotes", "Sentences worth quoting, with their page numbers.", FieldKind::Quotes),
    ("relevance", "Relevance to This Project", "Why does this paper matter for this collection's work?", FieldKind::Relevance),
    ("notes", "Further Notes", "Anything else worth remembering.", FieldKind::LongText),
];

fn field_from_row(r: &Row) -> rusqlite::Result<Field> {
    Ok(Field {
        id: r.get(0)?,
        key: r.get(1)?,
        label: r.get(2)?,
        prompt: r.get(3)?,
        kind: kind_of(&r.get::<_, String>(4)?),
        position: r.get(5)?,
        hidden: r.get(6)?,
        collection: r.get(7)?,
        builtin: r.get(8)?,
    })
}

const FIELD_COLS: &str = "id, key, label, prompt, kind, position, hidden, collection_id, builtin";

impl Library {
    /// Fields shown for a paper: the library's, then `collection`'s own. Hidden included.
    pub fn fields(&self, collection: Option<i64>) -> Result<Vec<Field>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {FIELD_COLS} FROM fields WHERE collection_id IS NULL OR collection_id = ?1
             ORDER BY collection_id IS NOT NULL, position, id"
        ))?;
        let rows = stmt.query_map([collection], field_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Every collection's own fields, for the matrix of "All papers".
    pub fn all_fields(&self) -> Result<Vec<Field>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {FIELD_COLS} FROM fields ORDER BY collection_id IS NOT NULL, collection_id, position, id"
        ))?;
        let rows = stmt.query_map([], field_from_row)?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn add_field(&mut self, label: &str, prompt: &str, kind: FieldKind, collection: Option<i64>) -> Result<i64> {
        if label.trim().is_empty() {
            bail!("A field needs a name.");
        }
        let position: i64 = self.conn.query_row(
            "SELECT coalesce(max(position), -1) + 1 FROM fields WHERE collection_id IS ?1",
            [collection],
            |r| r.get(0),
        )?;
        self.conn.execute(
            "INSERT INTO fields (label, prompt, kind, position, collection_id) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![label.trim(), prompt.trim(), kind.as_str(), position, collection],
        )?;
        self.touch();
        Ok(self.conn.last_insert_rowid())
    }

    /// Renames, re-prompts or hides a field. Built-in fields keep their kind.
    pub fn update_field(&mut self, f: &Field) -> Result<()> {
        if f.label.trim().is_empty() {
            bail!("A field needs a name.");
        }
        self.conn.execute(
            "UPDATE fields SET label = ?2, prompt = ?3, hidden = ?4,
                kind = CASE WHEN builtin = 1 THEN kind ELSE ?5 END WHERE id = ?1",
            params![f.id, f.label.trim(), f.prompt.trim(), f.hidden, f.kind.as_str()],
        )?;
        self.touch();
        Ok(())
    }

    /// Deletes a custom field and everything written in it.
    pub fn delete_field(&mut self, id: i64) -> Result<()> {
        let builtin: bool = self.conn.query_row("SELECT builtin FROM fields WHERE id = ?1", [id], |r| r.get(0))?;
        if builtin {
            bail!("Built-in fields can be hidden but not deleted.");
        }
        self.conn.execute("DELETE FROM fields WHERE id = ?1", [id])?;
        self.touch();
        Ok(())
    }

    /// Moves a field up or down among the fields of the same scope.
    pub fn move_field(&mut self, id: i64, up: bool) -> Result<()> {
        let all = self.all_fields()?;
        let Some(me) = all.iter().find(|f| f.id == id) else { return Ok(()) };
        let mut same: Vec<&Field> = all.iter().filter(|f| f.collection == me.collection).collect();
        same.sort_by_key(|f| (f.position, f.id));
        let Some(pos) = same.iter().position(|f| f.id == id) else { return Ok(()) };
        let other = if up { pos.checked_sub(1) } else { Some(pos + 1).filter(|p| *p < same.len()) };
        let Some(other) = other else { return Ok(()) };
        let mut order: Vec<i64> = same.iter().map(|f| f.id).collect();
        order.swap(pos, other);
        let tx = self.conn.transaction()?;
        for (i, fid) in order.iter().enumerate() {
            tx.execute("UPDATE fields SET position = ?2 WHERE id = ?1", params![fid, i as i64])?;
        }
        tx.commit()?;
        self.touch();
        Ok(())
    }

    /// Field id → content for one paper.
    pub fn notes(&self, paper: i64) -> Result<HashMap<i64, String>> {
        let mut stmt = self.conn.prepare("SELECT field_id, content FROM notes WHERE paper_id = ?1")?;
        let rows = stmt.query_map([paper], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// (paper, field) → content for many papers, for the matrix.
    pub fn notes_for(&self, papers: &[i64]) -> Result<HashMap<(i64, i64), String>> {
        let mut out = HashMap::new();
        let mut stmt = self.conn.prepare("SELECT field_id, content FROM notes WHERE paper_id = ?1")?;
        for &p in papers {
            for row in stmt.query_map([p], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
                let (f, c) = row?;
                out.insert((p, f), c);
            }
        }
        Ok(out)
    }

    pub fn set_note(&mut self, paper: i64, field: i64, content: &str) -> Result<()> {
        let now = now_string();
        if content.trim().is_empty() {
            self.conn.execute("DELETE FROM notes WHERE paper_id = ?1 AND field_id = ?2", params![paper, field])?;
        } else {
            self.conn.execute(
                "INSERT INTO notes (paper_id, field_id, content, updated_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (paper_id, field_id) DO UPDATE SET content = excluded.content, updated_at = excluded.updated_at",
                params![paper, field, content, now],
            )?;
        }
        self.conn.execute("UPDATE papers SET modified_at = ?2 WHERE id = ?1", params![paper, now])?;
        self.reindex(paper)?;
        self.touch();
        if self.auto_status {
            self.advance_status(paper)?;
        }
        Ok(())
    }
}
