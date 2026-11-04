//! Zotero libraries: the papers whose PDFs Zotero has downloaded, with their
//! details, read from Zotero's own database. Zotero locks `zotero.sqlite`
//! while it runs, so a copy (with its write-ahead log) is read instead.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, params};

use super::{Parsed, doi, normalise_pages, parse_month, parse_year};
use crate::latex::{collapse_ws, escape};
use crate::model::{Author, Biblio, NewPaper};

/// Zotero's data folder in its default place (`~/Zotero` on every system),
/// if it has a library in it.
pub fn default_dir() -> Option<PathBuf> {
    let dir = directories::UserDirs::new()?.home_dir().join("Zotero");
    dir.join("zotero.sqlite").is_file().then_some(dir)
}

#[derive(Debug, Default)]
pub struct ZoteroImport {
    /// One paper per Zotero item with a PDF on this computer.
    pub parsed: Parsed,
    /// Items left out because none of their PDFs has been downloaded.
    pub without_pdf: usize,
}

/// Reads the papers with downloaded PDFs from the Zotero data folder `dir`.
pub fn read(dir: &Path) -> Result<ZoteroImport> {
    let db = dir.join("zotero.sqlite");
    if !db.is_file() {
        bail!("There is no Zotero library (zotero.sqlite) in {}. Choose Zotero's data folder: the one with the \"storage\" folder in it.", dir.display());
    }
    let snapshot = Snapshot::take(&db)?;
    let conn = Connection::open(&snapshot.db).context("Could not open the copy of the Zotero library")?;
    read_db(&conn, dir).context("Could not read the Zotero library")
}

/// A temporary copy of the database, removed when dropped.
struct Snapshot {
    dir: PathBuf,
    db: PathBuf,
}

impl Snapshot {
    fn take(db: &Path) -> Result<Self> {
        let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        let dir = std::env::temp_dir().join(format!("litanatomy-zotero-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).with_context(|| format!("Could not create {}", dir.display()))?;
        let snapshot = Self { db: dir.join("zotero.sqlite"), dir };
        std::fs::copy(db, &snapshot.db).with_context(|| format!("Could not copy {}", db.display()))?;
        // Recent changes may still be in the write-ahead log.
        let wal = db.with_file_name("zotero.sqlite-wal");
        if wal.is_file() {
            std::fs::copy(&wal, snapshot.dir.join("zotero.sqlite-wal")).with_context(|| format!("Could not copy {}", wal.display()))?;
        }
        Ok(snapshot)
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Item types that are not papers.
const NOT_PAPERS: &str = "('attachment', 'note', 'annotation')";

pub fn read_db(conn: &Connection, dir: &Path) -> Result<ZoteroImport> {
    let tables: i64 = conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type = 'table' AND name IN
         ('items', 'itemAttachments', 'itemData', 'itemDataValues', 'fieldsCombined', 'itemTypesCombined', 'deletedItems')",
        [],
        |r| r.get(0),
    )?;
    if tables < 7 {
        bail!("This doesn't look like a Zotero library, or it is from a Zotero version older than 5.");
    }

    // The first PDF on disk of each item (or of a PDF stored on its own).
    let mut stmt = conn.prepare(
        "SELECT a.itemID, a.parentItemID, i.key, a.linkMode, a.path
         FROM itemAttachments a JOIN items i ON i.itemID = a.itemID
         WHERE a.contentType = 'application/pdf' AND a.linkMode IN (0, 1, 2)
           AND a.itemID NOT IN (SELECT itemID FROM deletedItems)
           AND (a.parentItemID IS NULL OR a.parentItemID NOT IN (SELECT itemID FROM deletedItems))
         ORDER BY coalesce(a.parentItemID, a.itemID), a.itemID",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, Option<String>>(4)?.unwrap_or_default(),
        ))
    })?;
    let mut pdfs: Vec<(i64, bool, PathBuf)> = Vec::new();
    let mut seen = HashSet::new();
    for row in rows {
        let (attachment, parent, key, link_mode, path) = row?;
        let item = parent.unwrap_or(attachment);
        if seen.contains(&item) {
            continue;
        }
        let file = match (link_mode, path.strip_prefix("storage:")) {
            // Imported: the file is in storage/<attachment key>/.
            (0 | 1, Some(name)) => dir.join("storage").join(&key).join(name),
            // Linked: an absolute path. Paths relative to Zotero's
            // "linked attachment base directory" can't be resolved here.
            (2, None) if Path::new(&path).is_absolute() => PathBuf::from(&path),
            _ => continue,
        };
        if file.is_file() {
            seen.insert(item);
            pdfs.push((item, parent.is_none(), file));
        }
    }

    let papers_in_zotero: i64 = conn.query_row(
        &format!(
            "SELECT count(*) FROM items i JOIN itemTypesCombined t ON t.itemTypeID = i.itemTypeID
             WHERE t.typeName NOT IN {NOT_PAPERS} AND i.itemID NOT IN (SELECT itemID FROM deletedItems)"
        ),
        [],
        |r| r.get(0),
    )?;
    let with_parent = pdfs.iter().filter(|(_, standalone, _)| !standalone).count();

    let mut out = ZoteroImport { without_pdf: (papers_in_zotero as usize).saturating_sub(with_parent), ..Default::default() };
    for (item, standalone, file) in pdfs {
        let record = Record::load(conn, item)?;
        let pdf = file.to_string_lossy().into_owned();
        match record.to_paper(standalone, &file, pdf) {
            Some(p) => out.parsed.papers.push(p),
            None => out.parsed.errors.push(format!("{}: no title", file.display())),
        }
    }
    Ok(out)
}

/// One Zotero item, as stored.
#[derive(Debug, Default)]
pub struct Record {
    pub item_type: String,
    pub fields: HashMap<String, String>,
    /// (creator type, author), in Zotero's order.
    pub creators: Vec<(String, Author)>,
    pub tags: Vec<String>,
}

impl Record {
    fn load(conn: &Connection, item: i64) -> Result<Self> {
        let item_type = conn.query_row(
            "SELECT t.typeName FROM items i JOIN itemTypesCombined t ON t.itemTypeID = i.itemTypeID WHERE i.itemID = ?1",
            params![item],
            |r| r.get(0),
        )?;
        let mut stmt = conn.prepare_cached(
            "SELECT f.fieldName, v.value FROM itemData d
             JOIN fieldsCombined f ON f.fieldID = d.fieldID
             JOIN itemDataValues v ON v.valueID = d.valueID
             WHERE d.itemID = ?1",
        )?;
        let fields = stmt
            .query_map(params![item], |r| Ok((r.get::<_, String>(0)?, value_text(r.get_ref(1)?))))?
            .collect::<rusqlite::Result<HashMap<_, _>>>()?;
        let mut stmt = conn.prepare_cached(
            "SELECT ct.creatorType, c.firstName, c.lastName, c.fieldMode FROM itemCreators ic
             JOIN creators c ON c.creatorID = ic.creatorID
             JOIN creatorTypes ct ON ct.creatorTypeID = ic.creatorTypeID
             WHERE ic.itemID = ?1 ORDER BY ic.orderIndex",
        )?;
        let creators = stmt
            .query_map(params![item], |r| {
                let first: String = r.get::<_, Option<String>>(1)?.unwrap_or_default();
                let last: String = r.get::<_, Option<String>>(2)?.unwrap_or_default();
                // fieldMode 1: one name (an organisation).
                let single = r.get::<_, Option<i64>>(3)?.unwrap_or(0) == 1;
                let author = if single { Author::new(last.trim(), "") } else { Author::new(last.trim(), first.trim()) };
                Ok((r.get::<_, String>(0)?, author))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        // Tags you added come before the publisher's keywords.
        let mut stmt = conn.prepare_cached(
            "SELECT t.name FROM itemTags it JOIN tags t ON t.tagID = it.tagID WHERE it.itemID = ?1 ORDER BY it.type, t.name",
        )?;
        let tags = stmt.query_map(params![item], |r| r.get(0))?.collect::<rusqlite::Result<Vec<String>>>()?;
        Ok(Self { item_type, fields, creators, tags })
    }

    fn get(&self, name: &str) -> String {
        self.fields.get(name).map(|v| collapse_ws(v)).unwrap_or_default()
    }

    /// The first non-empty field of `names`.
    fn first(&self, names: &[&str]) -> String {
        names.iter().map(|n| self.get(n)).find(|v| !v.is_empty()).unwrap_or_default()
    }

    /// "Key: value" lines in the free-text Extra field.
    fn extra_line(&self, key: &str) -> String {
        let extra = self.fields.get("extra").map(String::as_str).unwrap_or_default();
        extra
            .lines()
            .filter_map(|l| l.split_once(':'))
            .find(|(k, _)| k.trim().eq_ignore_ascii_case(key))
            .map(|(_, v)| v.trim().to_string())
            .unwrap_or_default()
    }

    /// The paper for this item. A PDF stored on its own has only a title
    /// (or just its file name); its details can be filled in from a DOI later.
    pub fn to_paper(&self, standalone: bool, file: &Path, pdf: String) -> Option<NewPaper> {
        let mut title = self.get("title");
        if standalone {
            let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            if title.is_empty() || title.eq_ignore_ascii_case(&file.file_name()?.to_string_lossy()) {
                title = stem;
            }
            let bib = Biblio { entry_type: "misc".into(), title, ..Default::default() };
            return (!bib.title.is_empty()).then(|| NewPaper { bib, source_key: None, pdf_path: Some(pdf) });
        }
        if title.is_empty() {
            return None;
        }
        let entry_type = entry_type(&self.item_type, &self.get("thesisType"));
        let (year, month) = date(&self.get("date"));

        let by_type = |t: &str| self.creators.iter().filter(|(ct, _)| ct == t).map(|(_, a)| a.clone()).collect::<Vec<_>>();
        let mut authors = by_type("author");
        let mut editors = by_type("editor");
        if authors.is_empty() {
            // The main creator of a patent, a talk, software…
            const SECONDARY: [&str; 6] = ["editor", "seriesEditor", "translator", "bookAuthor", "reviewedAuthor", "contributor"];
            authors = self.creators.iter().filter(|(ct, _)| !SECONDARY.contains(&ct.as_str())).map(|(_, a)| a.clone()).collect();
        }
        if authors.is_empty() {
            // An edited book.
            authors = std::mem::take(&mut editors);
        }

        let mut container = self.first(&[
            "publicationTitle",
            "proceedingsTitle",
            "bookTitle",
            "websiteTitle",
            "blogTitle",
            "forumTitle",
            "encyclopediaTitle",
            "dictionaryTitle",
            "programTitle",
        ]);
        if container.is_empty() && self.item_type == "conferencePaper" {
            container = self.get("conferenceName");
        }
        let url = self.get("url");
        let doi = [self.get("DOI"), self.extra_line("DOI"), url.clone()].iter().find_map(|s| doi::normalise(s));
        let arxiv = self.get("archiveID");
        let arxiv = arxiv.strip_prefix("arXiv:").map(str::to_string).unwrap_or_else(|| self.extra_line("arXiv"));

        let mut bib = Biblio {
            entry_type: entry_type.into(),
            title,
            authors,
            year,
            month,
            container,
            volume: self.get("volume"),
            issue: self.first(&["issue", "reportNumber", "patentNumber"]),
            pages: normalise_pages(&self.get("pages")),
            publisher: self.first(&["publisher", "university", "institution", "company", "label"]),
            doi,
            url: if url.contains("doi.org/") { String::new() } else { url },
            isbn: self.get("ISBN"),
            arxiv_id: arxiv,
            abstract_: self.fields.get("abstractNote").map(|s| s.trim().to_string()).unwrap_or_default(),
            keywords: self.tags.join("; "),
            ..Default::default()
        };
        let issn = Some(self.get("ISSN")).filter(|s| !s.is_empty()).unwrap_or_else(|| self.extra_line("ISSN"));
        // Extra fields are kept as raw LaTeX.
        let mut keep = |name: &str, value: String| {
            if !value.is_empty() {
                bib.extra.insert(name.into(), escape(&value));
            }
        };
        keep("issn", issn);
        keep("series", self.get("series"));
        keep("edition", self.get("edition"));
        keep("address", self.get("place"));
        if !editors.is_empty() && entry_type != "article" {
            let names: Vec<String> =
                editors.iter().map(|a| if a.given.is_empty() { a.family.clone() } else { format!("{}, {}", a.family, a.given) }).collect();
            keep("editor", names.join(" and "));
        }

        let key = [self.get("citationKey"), self.extra_line("Citation Key")].into_iter().find(|k| !k.is_empty());
        Some(NewPaper { bib, source_key: key, pdf_path: Some(pdf) })
    }
}

/// Zotero stores values as text, but numbers may come back as integers.
fn value_text(v: rusqlite::types::ValueRef<'_>) -> String {
    use rusqlite::types::ValueRef;
    match v {
        ValueRef::Text(t) | ValueRef::Blob(t) => String::from_utf8_lossy(t).into_owned(),
        ValueRef::Integer(i) => i.to_string(),
        ValueRef::Real(f) => f.to_string(),
        ValueRef::Null => String::new(),
    }
}

/// Zotero dates look like "2025-03-00 2025-03": a sortable "YYYY-MM-DD"
/// (zeros where unknown), then the date as typed.
fn date(raw: &str) -> (Option<i32>, Option<u8>) {
    let sortable = raw.split_whitespace().next().unwrap_or_default();
    let parts: Vec<&str> = sortable.split('-').collect();
    if let [y, m, _] = parts.as_slice()
        && y.len() == 4
        && let Ok(year) = y.parse::<i32>()
    {
        let year = (year > 0).then_some(year);
        return (year.or_else(|| parse_year(raw)), parse_month(m));
    }
    (parse_year(raw), None)
}

fn entry_type(item_type: &str, thesis_type: &str) -> &'static str {
    match item_type {
        "journalArticle" | "magazineArticle" | "newspaperArticle" => "article",
        "conferencePaper" => "inproceedings",
        "book" => "book",
        "bookSection" | "encyclopediaArticle" | "dictionaryEntry" => "incollection",
        "thesis" if thesis_type.to_lowercase().contains("master") => "mastersthesis",
        "thesis" => "phdthesis",
        "report" => "techreport",
        "webpage" | "blogPost" | "forumPost" => "online",
        "patent" => "patent",
        "dataset" => "dataset",
        "manuscript" => "unpublished",
        _ => "misc",
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A small library with Zotero's tables (only the columns read here).
    pub(crate) fn library(dir: &Path) -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE itemTypesCombined (itemTypeID INT, typeName TEXT);
             CREATE TABLE fieldsCombined (fieldID INT, fieldName TEXT);
             CREATE TABLE creatorTypes (creatorTypeID INTEGER PRIMARY KEY, creatorType TEXT);
             CREATE TABLE items (itemID INTEGER PRIMARY KEY, itemTypeID INT, libraryID INT, key TEXT);
             CREATE TABLE itemData (itemID INT, fieldID INT, valueID);
             CREATE TABLE itemDataValues (valueID INTEGER PRIMARY KEY, value UNIQUE);
             CREATE TABLE creators (creatorID INTEGER PRIMARY KEY, firstName TEXT, lastName TEXT, fieldMode INT);
             CREATE TABLE itemCreators (itemID INT, creatorID INT, creatorTypeID INT, orderIndex INT);
             CREATE TABLE itemAttachments (itemID INTEGER PRIMARY KEY, parentItemID INT, linkMode INT, contentType TEXT, path TEXT);
             CREATE TABLE deletedItems (itemID INTEGER PRIMARY KEY);
             CREATE TABLE tags (tagID INTEGER PRIMARY KEY, name TEXT);
             CREATE TABLE itemTags (itemID INT, tagID INT, type INT);
             INSERT INTO itemTypesCombined VALUES (1, 'journalArticle'), (2, 'attachment'), (3, 'bookSection'), (4, 'note');
             INSERT INTO fieldsCombined VALUES (1, 'title'), (2, 'date'), (3, 'publicationTitle'), (4, 'DOI'), (5, 'pages'),
               (6, 'extra'), (7, 'bookTitle'), (8, 'citationKey'), (9, 'volume'), (10, 'ISSN'), (11, 'url');
             INSERT INTO creatorTypes VALUES (1, 'author'), (2, 'editor');

             -- 1: an article with its PDF downloaded.
             INSERT INTO items VALUES (1, 1, 1, 'PARENT01'), (2, 2, 1, 'PDFKEY01');
             INSERT INTO itemDataValues VALUES (1, 'Reconfigurable Intelligent   Surfaces'), (2, '2025-03-00 March 2025'),
               (3, 'IEEE Transactions on Antennas and Propagation'), (4, '10.1109/TAP.2025.1234567'), (5, '12–19'),
               (6, 'ISSN: 0018-926X'), (7, 'zhangReconfigurable2025'), (8, 73);
             INSERT INTO itemData VALUES (1, 1, 1), (1, 2, 2), (1, 3, 3), (1, 4, 4), (1, 5, 5), (1, 6, 6), (1, 8, 7), (1, 9, 8);
             INSERT INTO creators VALUES (1, 'Wei', 'Zhang', 0), (2, '', 'IEEE Antennas Society', 1), (3, 'Ann', 'Editor', 0);
             INSERT INTO itemCreators VALUES (1, 1, 1, 0), (1, 2, 1, 1);
             INSERT INTO tags VALUES (1, 'RIS'), (2, 'metasurfaces'), (3, 'to cite');
             INSERT INTO itemTags VALUES (1, 1, 1), (1, 2, 1), (1, 3, 0);
             INSERT INTO itemAttachments VALUES (2, 1, 1, 'application/pdf', 'storage:Zhang - 2025.pdf');

             -- 3: a chapter whose PDF was never downloaded.
             INSERT INTO items VALUES (3, 3, 1, 'PARENT03'), (4, 2, 1, 'PDFKEY04');
             INSERT INTO itemDataValues VALUES (9, 'A chapter');
             INSERT INTO itemData VALUES (3, 1, 9);
             INSERT INTO itemAttachments VALUES (4, 3, 0, 'application/pdf', 'storage:chapter.pdf');

             -- 5: in the bin, with a PDF.
             INSERT INTO items VALUES (5, 1, 1, 'PARENT05'), (6, 2, 1, 'PDFKEY06');
             INSERT INTO itemDataValues VALUES (10, 'Deleted paper');
             INSERT INTO itemData VALUES (5, 1, 10);
             INSERT INTO itemAttachments VALUES (6, 5, 0, 'application/pdf', 'storage:deleted.pdf');
             INSERT INTO deletedItems VALUES (5);

             -- 7: a PDF on its own, without an item.
             INSERT INTO items VALUES (7, 2, 1, 'LONEPDF7');
             INSERT INTO itemDataValues VALUES (11, 'viruses_as_power_generators.pdf');
             INSERT INTO itemData VALUES (7, 1, 11);
             INSERT INTO itemAttachments VALUES (7, NULL, 0, 'application/pdf', 'storage:viruses_as_power_generators.pdf');

             -- 8: a note, not a paper.
             INSERT INTO items VALUES (8, 4, 1, 'NOTE0008');",
        )
        .unwrap();
        for (key, name) in [("PDFKEY01", "Zhang - 2025.pdf"), ("PDFKEY06", "deleted.pdf"), ("LONEPDF7", "viruses_as_power_generators.pdf")] {
            std::fs::create_dir_all(dir.join("storage").join(key)).unwrap();
            std::fs::write(dir.join("storage").join(key).join(name), b"%PDF-1.4").unwrap();
        }
        // An empty storage folder, like those of items Zotero hasn't downloaded.
        std::fs::create_dir_all(dir.join("storage").join("PDFKEY04")).unwrap();
        conn
    }

    #[test]
    fn papers_with_downloaded_pdfs() {
        let dir = tempfile::tempdir().unwrap();
        let conn = library(dir.path());
        let got = read_db(&conn, dir.path()).unwrap();
        assert!(got.parsed.errors.is_empty(), "{:?}", got.parsed.errors);
        assert_eq!(got.without_pdf, 1, "the chapter without a PDF");
        let [article, lone] = got.parsed.papers.as_slice() else { panic!("{:?}", got.parsed.papers) };

        let b = &article.bib;
        assert_eq!(b.entry_type, "article");
        assert_eq!(b.title, "Reconfigurable Intelligent Surfaces");
        assert_eq!(b.authors, vec![Author::new("Zhang", "Wei"), Author::new("IEEE Antennas Society", "")]);
        assert_eq!((b.year, b.month), (Some(2025), Some(3)));
        assert_eq!(b.container, "IEEE Transactions on Antennas and Propagation");
        assert_eq!(b.volume, "73");
        assert_eq!(b.pages, "12-19");
        assert_eq!(b.doi.as_deref(), Some("10.1109/tap.2025.1234567"));
        assert_eq!(b.keywords, "to cite; RIS; metasurfaces");
        assert_eq!(b.extra.get("issn").map(String::as_str), Some("0018-926X"));
        assert_eq!(article.source_key.as_deref(), Some("zhangReconfigurable2025"));
        let pdf = dir.path().join("storage").join("PDFKEY01").join("Zhang - 2025.pdf");
        assert_eq!(article.pdf_path.as_deref(), Some(pdf.to_string_lossy().as_ref()));

        assert_eq!(lone.bib.title, "viruses_as_power_generators");
        assert_eq!(lone.bib.entry_type, "misc");
        assert!(lone.pdf_path.as_deref().is_some_and(|p| p.ends_with("viruses_as_power_generators.pdf")));
    }

    #[test]
    fn copies_a_locked_library() {
        let dir = tempfile::tempdir().unwrap();
        let mem = library(dir.path());
        mem.execute("VACUUM INTO ?1", params![dir.path().join("zotero.sqlite").to_string_lossy()]).unwrap();
        // Zotero keeps the library open and exclusively locked.
        let zotero = Connection::open(dir.path().join("zotero.sqlite")).unwrap();
        zotero.execute_batch("PRAGMA locking_mode = EXCLUSIVE; BEGIN EXCLUSIVE;").unwrap();
        let got = read(dir.path()).unwrap();
        assert_eq!(got.parsed.papers.len(), 2);
        drop(zotero);

        let err = read(&dir.path().join("storage")).unwrap_err();
        assert!(err.to_string().contains("no Zotero library"));
    }

    #[test]
    fn chapters_dates_and_types() {
        let mut r = Record { item_type: "bookSection".into(), ..Default::default() };
        r.fields.insert("title".into(), "A chapter".into());
        r.fields.insert("bookTitle".into(), "Handbook of Antennas".into());
        r.fields.insert("extra".into(), "DOI: 10.1007/978-3-030-1\nCitation Key: smith2020chapter".into());
        r.fields.insert("place".into(), "Cham & Berlin".into());
        r.creators = vec![("author".into(), Author::new("Smith", "Jo")), ("editor".into(), Author::new("Lee", "Kim"))];
        let p = r.to_paper(false, Path::new("x.pdf"), "x.pdf".into()).unwrap();
        assert_eq!(p.bib.entry_type, "incollection");
        assert_eq!(p.bib.container, "Handbook of Antennas");
        assert_eq!(p.bib.doi.as_deref(), Some("10.1007/978-3-030-1"));
        assert_eq!(p.bib.extra.get("editor").map(String::as_str), Some("Lee, Kim"));
        assert_eq!(p.bib.extra.get("address").map(String::as_str), Some(r"Cham \& Berlin"));
        assert_eq!(p.source_key.as_deref(), Some("smith2020chapter"));

        // An edited book: the editors are its authors.
        let mut book = Record { item_type: "book".into(), ..Default::default() };
        book.fields.insert("title".into(), "Edited".into());
        book.creators = vec![("editor".into(), Author::new("Lee", "Kim"))];
        let p = book.to_paper(false, Path::new("x.pdf"), "x.pdf".into()).unwrap();
        assert_eq!(p.bib.authors, vec![Author::new("Lee", "Kim")]);
        assert!(!p.bib.extra.contains_key("editor"));

        assert_eq!(date("2024-00-00 2024"), (Some(2024), None));
        assert_eq!(date("2017-01-09 2017/01/09"), (Some(2017), Some(1)));
        assert_eq!(date("0000-00-00 Spring 1998"), (Some(1998), None));
        assert_eq!(date(""), (None, None));
        assert_eq!(entry_type("thesis", "Master's thesis"), "mastersthesis");
        assert_eq!(entry_type("thesis", "PhD Thesis"), "phdthesis");
        assert_eq!(entry_type("preprint", ""), "misc");
    }
}
