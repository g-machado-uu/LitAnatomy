//! The library: one SQLite file holding papers, collections, fields, notes,
//! tags and links. Every change bumps [`Library::revision`], which the UI
//! uses to know when its cached lists are stale.

mod collections;
mod fields;
mod links;
mod papers;
mod screening;
mod search;
mod synthesis;

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, params};

pub use fields::DEFAULT_FIELDS;
pub use links::LinkView;
pub use papers::title_fingerprint;
pub use screening::ScreeningSummary;

use crate::model::FieldKind;

/// Which papers a list shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    All,
    Unfiled,
    Trash,
    Collection(i64),
}

pub struct Library {
    conn: Connection,
    path: Option<PathBuf>,
    revision: u64,
}

const SCHEMA_VERSION: i64 = 2;

const SCHEMA_V1: &str = "
CREATE TABLE papers (
    id INTEGER PRIMARY KEY,
    citekey TEXT NOT NULL UNIQUE,
    key_pinned INTEGER NOT NULL DEFAULT 0,
    entry_type TEXT NOT NULL DEFAULT 'article',
    title TEXT NOT NULL DEFAULT '',
    authors TEXT NOT NULL DEFAULT '[]',
    year INTEGER,
    month INTEGER,
    container TEXT NOT NULL DEFAULT '',
    volume TEXT NOT NULL DEFAULT '',
    issue TEXT NOT NULL DEFAULT '',
    pages TEXT NOT NULL DEFAULT '',
    publisher TEXT NOT NULL DEFAULT '',
    doi TEXT,
    url TEXT NOT NULL DEFAULT '',
    isbn TEXT NOT NULL DEFAULT '',
    arxiv_id TEXT NOT NULL DEFAULT '',
    abstract TEXT NOT NULL DEFAULT '',
    keywords TEXT NOT NULL DEFAULT '',
    extra_fields TEXT NOT NULL DEFAULT '{}',
    pdf_path TEXT,
    status INTEGER NOT NULL DEFAULT 0,
    rating INTEGER NOT NULL DEFAULT 0,
    added_at TEXT NOT NULL,
    modified_at TEXT NOT NULL,
    deleted_at TEXT
);
CREATE INDEX papers_doi ON papers(doi);

CREATE TABLE collections (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    colour INTEGER NOT NULL DEFAULT 0,
    parent_id INTEGER REFERENCES collections(id) ON DELETE CASCADE,
    archived INTEGER NOT NULL DEFAULT 0,
    bib_export_path TEXT,
    position INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);

CREATE TABLE collection_papers (
    collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    paper_id INTEGER NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
    added_at TEXT NOT NULL,
    relevance TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (collection_id, paper_id)
);
CREATE INDEX collection_papers_paper ON collection_papers(paper_id);

CREATE TABLE fields (
    id INTEGER PRIMARY KEY,
    key TEXT NOT NULL DEFAULT '',
    label TEXT NOT NULL,
    prompt TEXT NOT NULL DEFAULT '',
    kind TEXT NOT NULL,
    position INTEGER NOT NULL,
    hidden INTEGER NOT NULL DEFAULT 0,
    collection_id INTEGER REFERENCES collections(id) ON DELETE CASCADE,
    builtin INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE notes (
    paper_id INTEGER NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
    field_id INTEGER NOT NULL REFERENCES fields(id) ON DELETE CASCADE,
    content TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (paper_id, field_id)
);

CREATE TABLE tags (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE
);

CREATE TABLE paper_tags (
    paper_id INTEGER NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
    tag_id INTEGER NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    PRIMARY KEY (paper_id, tag_id)
);

CREATE TABLE paper_links (
    id INTEGER PRIMARY KEY,
    from_id INTEGER NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
    to_id INTEGER NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
    relation TEXT NOT NULL,
    note TEXT NOT NULL DEFAULT '',
    UNIQUE (from_id, to_id, relation)
);

CREATE VIRTUAL TABLE search USING fts5(
    title, authors, details, notes,
    tokenize = 'unicode61 remove_diacritics 2'
);
";

/// v0.2: systematic-review screening and collection synthesis.
const SCHEMA_V2: &str = "
ALTER TABLE collections ADD COLUMN screening INTEGER NOT NULL DEFAULT 0;
ALTER TABLE collections ADD COLUMN synthesis TEXT NOT NULL DEFAULT '';
ALTER TABLE collections ADD COLUMN outline TEXT NOT NULL DEFAULT '';

ALTER TABLE collection_papers ADD COLUMN decision INTEGER NOT NULL DEFAULT 0;
ALTER TABLE collection_papers ADD COLUMN decision_criteria TEXT NOT NULL DEFAULT '[]';
ALTER TABLE collection_papers ADD COLUMN decision_note TEXT NOT NULL DEFAULT '';
ALTER TABLE collection_papers ADD COLUMN decided_at TEXT;

CREATE TABLE criteria (
    id INTEGER PRIMARY KEY,
    collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    label TEXT NOT NULL,
    position INTEGER NOT NULL
);

CREATE TABLE themes (
    id INTEGER PRIMARY KEY,
    collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    notes TEXT NOT NULL DEFAULT '',
    position INTEGER NOT NULL
);

CREATE TABLE theme_papers (
    theme_id INTEGER NOT NULL REFERENCES themes(id) ON DELETE CASCADE,
    paper_id INTEGER NOT NULL REFERENCES papers(id) ON DELETE CASCADE,
    note TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (theme_id, paper_id)
);
CREATE INDEX theme_papers_paper ON theme_papers(paper_id);
";

impl Library {
    /// Opens (or creates) the library file at `path`.
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).with_context(|| format!("Could not create {}", dir.display()))?;
        }
        let conn = Connection::open(path).with_context(|| format!("Could not open {}", path.display()))?;
        Self::init(conn, Some(path.to_path_buf()))
    }

    #[cfg(test)]
    pub fn open_in_memory() -> Result<Self> {
        Self::init(Connection::open_in_memory()?, None)
    }

    fn init(conn: Connection, path: Option<PathBuf>) -> Result<Self> {
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA foreign_keys = ON;")?;
        let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version > SCHEMA_VERSION {
            bail!(
                "This library was made by a newer version of LitAnatomy (format {version}). Please update the app."
            );
        }
        // A file that isn't ours (e.g. some other SQLite database) is refused.
        if version == 0 {
            let tables: i64 = conn.query_row("SELECT count(*) FROM sqlite_master", [], |r| r.get(0))?;
            if tables > 0 {
                bail!("This file is not a LitAnatomy library.");
            }
        }
        let mut lib = Library { conn, path, revision: 1 };
        lib.migrate(version)?;
        Ok(lib)
    }

    /// Brings the file up to [`SCHEMA_VERSION`], one version at a time, each
    /// step in its own transaction.
    fn migrate(&mut self, from: i64) -> Result<()> {
        if from < 1 {
            let tx = self.conn.transaction()?;
            tx.execute_batch(SCHEMA_V1)?;
            for (i, (key, label, prompt, kind)) in DEFAULT_FIELDS.iter().enumerate() {
                tx.execute(
                    "INSERT INTO fields (key, label, prompt, kind, position, builtin) VALUES (?1, ?2, ?3, ?4, ?5, 1)",
                    params![key, label, prompt, kind.as_str(), i as i64],
                )?;
            }
            tx.execute_batch("PRAGMA user_version = 1;")?;
            tx.commit()?;
        }
        if from < 2 {
            let tx = self.conn.transaction()?;
            tx.execute_batch(SCHEMA_V2)?;
            tx.execute_batch("PRAGMA user_version = 2;")?;
            tx.commit()?;
        }
        Ok(())
    }

    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// Bumped on every change.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    fn touch(&mut self) {
        self.revision += 1;
    }

    /// Writes a consistent copy of the library to `dest` (for backups).
    pub fn backup_to(&self, dest: &Path) -> Result<()> {
        if dest.exists() {
            std::fs::remove_file(dest)?;
        }
        self.conn.execute("VACUUM INTO ?1", params![dest.to_string_lossy()])?;
        Ok(())
    }
}

fn kind_of(s: &str) -> FieldKind {
    FieldKind::parse(s)
}

#[cfg(test)]
mod tests;
