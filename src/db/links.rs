use std::collections::HashSet;

use anyhow::{Result, bail};
use rusqlite::params;

use super::Library;
use crate::model::{Link, Relation};

/// A link as seen from one paper.
#[derive(Clone, Debug, PartialEq)]
pub struct LinkView {
    pub link: Link,
    /// True if this paper is the link's `from` end.
    pub outgoing: bool,
    pub other_id: i64,
    pub other_title: String,
    pub other_key: String,
}

impl LinkView {
    pub fn label(&self) -> &'static str {
        if self.outgoing { self.link.relation.label() } else { self.link.relation.inverse_label() }
    }
}

impl Library {
    pub fn links_of(&self, paper: i64) -> Result<Vec<LinkView>> {
        let mut stmt = self.conn.prepare(
            "SELECT l.id, l.from_id, l.to_id, l.relation, l.note, o.id, o.title, o.citekey
             FROM paper_links l JOIN papers o ON o.id = CASE WHEN l.from_id = ?1 THEN l.to_id ELSE l.from_id END
             WHERE (l.from_id = ?1 OR l.to_id = ?1) AND o.deleted_at IS NULL
             ORDER BY l.relation, o.title",
        )?;
        let rows = stmt.query_map([paper], |r| {
            let from: i64 = r.get(1)?;
            Ok(LinkView {
                link: Link {
                    id: r.get(0)?,
                    from,
                    to: r.get(2)?,
                    relation: Relation::parse(&r.get::<_, String>(3)?),
                    note: r.get(4)?,
                },
                outgoing: from == paper,
                other_id: r.get(5)?,
                other_title: r.get(6)?,
                other_key: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn add_link(&mut self, from: i64, to: i64, relation: Relation, note: &str) -> Result<()> {
        if from == to {
            bail!("A paper can't be linked to itself.");
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO paper_links (from_id, to_id, relation, note) VALUES (?1, ?2, ?3, ?4)",
            params![from, to, relation.as_str(), note.trim()],
        )?;
        self.touch();
        Ok(())
    }

    pub fn delete_link(&mut self, id: i64) -> Result<()> {
        self.conn.execute("DELETE FROM paper_links WHERE id = ?1", [id])?;
        self.touch();
        Ok(())
    }

    /// Every tag with how many papers (outside the Trash) have it.
    pub fn tags(&self) -> Result<Vec<(i64, String, usize)>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.id, t.name, count(p.id) FROM tags t
             LEFT JOIN paper_tags pt ON pt.tag_id = t.id
             LEFT JOIN papers p ON p.id = pt.paper_id AND p.deleted_at IS NULL
             GROUP BY t.id ORDER BY t.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? as usize)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    pub fn tags_of(&self, paper: i64) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare(
            "SELECT t.name FROM tags t JOIN paper_tags pt ON pt.tag_id = t.id
             WHERE pt.paper_id = ?1 ORDER BY t.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([paper], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }

    /// Replaces a paper's tags. Tags nobody uses any more are removed.
    pub fn set_tags(&mut self, paper: i64, names: &[String]) -> Result<()> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM paper_tags WHERE paper_id = ?1", [paper])?;
        let mut seen = HashSet::new();
        for name in names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
            if !seen.insert(name.to_lowercase()) {
                continue;
            }
            tx.execute("INSERT OR IGNORE INTO tags (name) VALUES (?1)", [name])?;
            tx.execute(
                "INSERT OR IGNORE INTO paper_tags (paper_id, tag_id) SELECT ?1, id FROM tags WHERE name = ?2",
                params![paper, name],
            )?;
        }
        tx.execute("DELETE FROM tags WHERE id NOT IN (SELECT tag_id FROM paper_tags)", [])?;
        tx.commit()?;
        self.touch();
        Ok(())
    }

    pub fn papers_with_tag(&self, tag: i64) -> Result<HashSet<i64>> {
        let mut stmt = self.conn.prepare("SELECT paper_id FROM paper_tags WHERE tag_id = ?1")?;
        let rows = stmt.query_map([tag], |r| r.get(0))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    }
}
