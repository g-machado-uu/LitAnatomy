use std::collections::{BTreeMap, HashSet};

use anyhow::{Result, bail};
use rusqlite::{OptionalExtension, Row, params, params_from_iter};

use super::{Library, Scope};
use crate::citekey;
use crate::model::{Author, Biblio, NewPaper, Paper, PaperRow, ReadingStatus, authors_short, now_string};

const PAPER_COLS: &str = "id, citekey, key_pinned, entry_type, title, authors, year, month, container, volume, issue,
    pages, publisher, doi, url, isbn, arxiv_id, abstract, keywords, extra_fields, pdf_path, status, rating,
    added_at, modified_at, deleted_at";

fn paper_from_row(r: &Row) -> rusqlite::Result<Paper> {
    let authors: String = r.get(5)?;
    let extra: String = r.get(19)?;
    Ok(Paper {
        id: r.get(0)?,
        citekey: r.get(1)?,
        key_pinned: r.get(2)?,
        bib: Biblio {
            entry_type: r.get(3)?,
            title: r.get(4)?,
            authors: serde_json::from_str(&authors).unwrap_or_default(),
            year: r.get(6)?,
            month: r.get(7)?,
            container: r.get(8)?,
            volume: r.get(9)?,
            issue: r.get(10)?,
            pages: r.get(11)?,
            publisher: r.get(12)?,
            doi: r.get(13)?,
            url: r.get(14)?,
            isbn: r.get(15)?,
            arxiv_id: r.get(16)?,
            abstract_: r.get(17)?,
            keywords: r.get(18)?,
            extra: serde_json::from_str::<BTreeMap<String, String>>(&extra).unwrap_or_default(),
        },
        pdf_path: r.get(20)?,
        status: ReadingStatus::from_i64(r.get(21)?),
        rating: r.get::<_, i64>(22)?.clamp(0, 5) as u8,
        added_at: r.get(23)?,
        modified_at: r.get(24)?,
        deleted: r.get::<_, Option<String>>(25)?.is_some(),
    })
}

/// Lower case letters and digits only, for comparing titles.
pub fn title_fingerprint(title: &str) -> String {
    deunicode::deunicode(title).chars().filter(char::is_ascii_alphanumeric).map(|c| c.to_ascii_lowercase()).collect()
}

impl Library {
    pub fn insert_paper(&mut self, new: &NewPaper, citekey: &str) -> Result<i64> {
        let b = &new.bib;
        let now = now_string();
        self.conn.execute(
            "INSERT INTO papers (citekey, entry_type, title, authors, year, month, container, volume, issue, pages,
                publisher, doi, url, isbn, arxiv_id, abstract, keywords, extra_fields, pdf_path, added_at, modified_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?20)",
            params![
                citekey,
                b.entry_type_or_default(),
                b.title,
                serde_json::to_string(&b.authors)?,
                b.year,
                b.month,
                b.container,
                b.volume,
                b.issue,
                b.pages,
                b.publisher,
                b.doi,
                b.url,
                b.isbn,
                b.arxiv_id,
                b.abstract_,
                b.keywords,
                serde_json::to_string(&b.extra)?,
                new.pdf_path,
                now,
            ],
        )?;
        let id = self.conn.last_insert_rowid();
        self.reindex(id)?;
        self.touch();
        Ok(id)
    }

    /// Adds a paper with a fresh key from `pattern`.
    pub fn add_paper(&mut self, new: &NewPaper, pattern: &str) -> Result<i64> {
        let key = self.unique_citekey(&citekey::generate(pattern, &new.bib), None);
        self.insert_paper(new, &key)
    }

    pub fn paper(&self, id: i64) -> Result<Paper> {
        Ok(self.conn.query_row(&format!("SELECT {PAPER_COLS} FROM papers WHERE id = ?1"), [id], paper_from_row)?)
    }

    pub fn papers(&self, ids: &[i64]) -> Result<Vec<Paper>> {
        let mut out = Vec::with_capacity(ids.len());
        for &id in ids {
            if let Ok(p) = self.paper(id) {
                out.push(p);
            }
        }
        Ok(out)
    }

    pub fn update_biblio(&mut self, id: i64, b: &Biblio) -> Result<()> {
        self.conn.execute(
            "UPDATE papers SET entry_type = ?2, title = ?3, authors = ?4, year = ?5, month = ?6, container = ?7,
                volume = ?8, issue = ?9, pages = ?10, publisher = ?11, doi = ?12, url = ?13, isbn = ?14,
                arxiv_id = ?15, abstract = ?16, keywords = ?17, extra_fields = ?18, modified_at = ?19
             WHERE id = ?1",
            params![
                id,
                b.entry_type_or_default(),
                b.title,
                serde_json::to_string(&b.authors)?,
                b.year,
                b.month,
                b.container,
                b.volume,
                b.issue,
                b.pages,
                b.publisher,
                b.doi,
                b.url,
                b.isbn,
                b.arxiv_id,
                b.abstract_,
                b.keywords,
                serde_json::to_string(&b.extra)?,
                now_string(),
            ],
        )?;
        self.reindex(id)?;
        self.touch();
        Ok(())
    }

    pub fn citekey_taken(&self, key: &str, except: Option<i64>) -> bool {
        self.conn
            .query_row(
                "SELECT 1 FROM papers WHERE citekey = ?1 COLLATE NOCASE AND id IS NOT ?2",
                params![key, except],
                |_| Ok(()),
            )
            .optional()
            .ok()
            .flatten()
            .is_some()
    }

    pub fn unique_citekey(&self, base: &str, except: Option<i64>) -> String {
        citekey::make_unique(base, |k| self.citekey_taken(k, except))
    }

    /// Sets a key by hand; it is pinned so it never changes by itself.
    pub fn set_citekey(&mut self, id: i64, key: &str) -> Result<()> {
        let key = key.trim();
        if !citekey::is_valid_key(key) {
            bail!("Keys may use letters, digits and - _ : . only, with no spaces.");
        }
        if self.citekey_taken(key, Some(id)) {
            bail!("The key \"{key}\" is already used by another paper.");
        }
        self.conn.execute(
            "UPDATE papers SET citekey = ?2, key_pinned = 1, modified_at = ?3 WHERE id = ?1",
            params![id, key, now_string()],
        )?;
        self.touch();
        Ok(())
    }

    /// Generates a new key from `pattern` unless the key is pinned. Returns the key.
    pub fn regenerate_citekey(&mut self, id: i64, pattern: &str, even_if_pinned: bool) -> Result<String> {
        let p = self.paper(id)?;
        if p.key_pinned && !even_if_pinned {
            return Ok(p.citekey);
        }
        let key = self.unique_citekey(&citekey::generate(pattern, &p.bib), Some(id));
        self.conn.execute("UPDATE papers SET citekey = ?2, key_pinned = 0 WHERE id = ?1", params![id, key])?;
        self.touch();
        Ok(key)
    }

    /// Exported keys are pinned: documents now cite them.
    pub fn pin_citekeys(&mut self, ids: &[i64]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for id in ids {
            tx.execute("UPDATE papers SET key_pinned = 1 WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        self.touch();
        Ok(())
    }

    pub fn set_status(&mut self, id: i64, status: ReadingStatus) -> Result<()> {
        self.conn.execute(
            "UPDATE papers SET status = ?2, modified_at = ?3 WHERE id = ?1",
            params![id, status.to_i64(), now_string()],
        )?;
        self.touch();
        Ok(())
    }

    pub fn set_rating(&mut self, id: i64, rating: u8) -> Result<()> {
        self.conn.execute(
            "UPDATE papers SET rating = ?2, modified_at = ?3 WHERE id = ?1",
            params![id, rating.min(5), now_string()],
        )?;
        self.touch();
        Ok(())
    }

    pub fn set_pdf(&mut self, id: i64, path: Option<&str>) -> Result<()> {
        self.conn.execute(
            "UPDATE papers SET pdf_path = ?2, modified_at = ?3 WHERE id = ?1",
            params![id, path, now_string()],
        )?;
        self.touch();
        Ok(())
    }

    pub fn trash(&mut self, ids: &[i64]) -> Result<()> {
        let now = now_string();
        let tx = self.conn.transaction()?;
        for id in ids {
            tx.execute("UPDATE papers SET deleted_at = ?2 WHERE id = ?1", params![id, now])?;
        }
        tx.commit()?;
        self.touch();
        Ok(())
    }

    pub fn restore(&mut self, ids: &[i64]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for id in ids {
            tx.execute("UPDATE papers SET deleted_at = NULL WHERE id = ?1", [id])?;
        }
        tx.commit()?;
        self.touch();
        Ok(())
    }

    /// Deletes papers for good, with their notes, links and memberships.
    pub fn purge(&mut self, ids: &[i64]) -> Result<()> {
        let tx = self.conn.transaction()?;
        for id in ids {
            tx.execute("DELETE FROM papers WHERE id = ?1", [id])?;
            tx.execute("DELETE FROM search WHERE rowid = ?1", [id])?;
        }
        tx.execute("DELETE FROM tags WHERE id NOT IN (SELECT tag_id FROM paper_tags)", [])?;
        tx.commit()?;
        self.touch();
        Ok(())
    }

    pub fn empty_trash(&mut self) -> Result<usize> {
        let ids: Vec<i64> = self
            .conn
            .prepare("SELECT id FROM papers WHERE deleted_at IS NOT NULL")?
            .query_map([], |r| r.get(0))?
            .collect::<rusqlite::Result<_>>()?;
        self.purge(&ids)?;
        Ok(ids.len())
    }

    /// A paper already in the library with the same DOI, or failing that the
    /// same title (ignoring case and punctuation) and year.
    pub fn find_duplicate(&self, b: &Biblio) -> Result<Option<i64>> {
        if let Some(doi) = &b.doi {
            let hit = self
                .conn
                .query_row("SELECT id FROM papers WHERE lower(doi) = lower(?1) ORDER BY deleted_at IS NOT NULL LIMIT 1", [doi], |r| r.get(0))
                .optional()?;
            if hit.is_some() {
                return Ok(hit);
            }
        }
        let fp = title_fingerprint(&b.title);
        if fp.len() < 8 {
            return Ok(None);
        }
        let mut stmt = self.conn.prepare("SELECT id, title, year FROM papers ORDER BY deleted_at IS NOT NULL")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<i32>>(2)?)))?;
        for row in rows {
            let (id, title, year) = row?;
            let same_year = year.is_none() || b.year.is_none() || year == b.year;
            if same_year && title_fingerprint(&title) == fp {
                return Ok(Some(id));
            }
        }
        Ok(None)
    }

    /// Fills the paper's empty fields from `b`. Returns true if it changed.
    pub fn merge_into(&mut self, id: i64, b: &Biblio) -> Result<bool> {
        let mut p = self.paper(id)?;
        if !p.bib.fill_from(b) {
            return Ok(false);
        }
        self.update_biblio(id, &p.bib)?;
        Ok(true)
    }

    pub fn rows(&self, scope: Scope) -> Result<Vec<PaperRow>> {
        let (filter, arg): (&str, Option<i64>) = match scope {
            Scope::All => ("p.deleted_at IS NULL", None),
            Scope::Trash => ("p.deleted_at IS NOT NULL", None),
            Scope::Unfiled => {
                ("p.deleted_at IS NULL AND NOT EXISTS (SELECT 1 FROM collection_papers c WHERE c.paper_id = p.id)", None)
            }
            Scope::Collection(id) => (
                "p.deleted_at IS NULL AND EXISTS (SELECT 1 FROM collection_papers c WHERE c.paper_id = p.id AND c.collection_id = ?1)",
                Some(id),
            ),
        };
        let total = self.library_field_count()?;
        let sql = format!(
            "SELECT p.id, p.citekey, p.title, p.authors, p.year, p.container, p.status, p.rating,
                    p.pdf_path IS NOT NULL, p.doi IS NOT NULL,
                    (SELECT count(*) FROM notes n JOIN fields f ON f.id = n.field_id
                     WHERE n.paper_id = p.id AND f.collection_id IS NULL AND f.hidden = 0
                       AND f.kind <> 'relevance' AND trim(n.content) <> ''),
                    p.modified_at
             FROM papers p WHERE {filter} ORDER BY p.added_at DESC, p.id DESC"
        );
        let mut stmt = self.conn.prepare(&sql)?;
        let map = |r: &Row| {
            let authors: String = r.get(3)?;
            let authors: Vec<Author> = serde_json::from_str(&authors).unwrap_or_default();
            Ok(PaperRow {
                id: r.get(0)?,
                citekey: r.get(1)?,
                title: r.get(2)?,
                authors: authors_short(&authors),
                year: r.get(4)?,
                container: r.get(5)?,
                status: ReadingStatus::from_i64(r.get(6)?),
                rating: r.get::<_, i64>(7)?.clamp(0, 5) as u8,
                has_pdf: r.get(8)?,
                has_doi: r.get(9)?,
                filled: r.get::<_, i64>(10)? as usize,
                field_total: total,
                modified_at: r.get(11)?,
            })
        };
        let rows = match arg {
            Some(a) => stmt.query_map(params_from_iter([a]), map)?.collect::<rusqlite::Result<Vec<_>>>()?,
            None => stmt.query_map([], map)?.collect::<rusqlite::Result<Vec<_>>>()?,
        };
        Ok(rows)
    }

    /// How many library-wide fields count towards "dissected".
    fn library_field_count(&self) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT count(*) FROM fields WHERE collection_id IS NULL AND hidden = 0 AND kind <> 'relevance'",
            [],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    pub fn trash_count(&self) -> Result<usize> {
        let n: i64 = self.conn.query_row("SELECT count(*) FROM papers WHERE deleted_at IS NOT NULL", [], |r| r.get(0))?;
        Ok(n as usize)
    }

    /// All keys, for checking an import against.
    pub fn citekeys(&self) -> Result<HashSet<String>> {
        let mut stmt = self.conn.prepare("SELECT lower(citekey) FROM papers")?;
        let keys = stmt.query_map([], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?;
        Ok(keys)
    }
}
