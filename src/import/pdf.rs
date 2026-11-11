//! Importing papers from their PDFs: read the PDF's properties and first
//! pages, find an identifier (DOI, arXiv ID, ISBN), look it up, and check
//! the record found really is this PDF.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::Path;

use anyhow::{Result, anyhow};
use pdf_extract::{Document, Object, PlainTextOutput};

use super::ident::{self, Identifier};
use super::{doi, names};
use crate::db::title_fingerprint;
use crate::latex::collapse_ws;
use crate::model::NewPaper;

/// How many pages are read: a paper's first pages, or for a book (a long
/// PDF) enough to reach the copyright page with its ISBN.
const PAGES: u32 = 4;
const BOOK_PAGES: u32 = 10;
const BOOK_LENGTH: u32 = 50;

/// What a PDF says about itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PdfText {
    /// From the document properties (often empty or junk).
    pub title: String,
    pub author: String,
    /// Every property and the XMP metadata, as text to search for a DOI.
    pub metadata: String,
    /// The text of the first pages.
    pub pages: Vec<String>,
}

pub fn read(path: &Path) -> Result<PdfText> {
    let bytes = std::fs::read(path).map_err(|e| anyhow!("{e}"))?;
    read_mem(&bytes)
}

/// Reads a PDF in memory. The PDF parser can panic on broken files, so a
/// panic becomes an error.
pub fn read_mem(bytes: &[u8]) -> Result<PdfText> {
    let mut doc = catch_unwind(|| Document::load_mem(bytes))
        .map_err(|_| anyhow!("the file could not be parsed"))?
        .map_err(|e| anyhow!("not a readable PDF ({e})"))?;
    if doc.is_encrypted() {
        // Most "encrypted" papers only restrict printing and open with an empty password.
        let _ = doc.decrypt("");
    }
    let mut out = PdfText::default();
    if let Ok(info) = doc.trailer.get_deref(b"Info", &doc).and_then(Object::as_dict) {
        for (key, value) in info.iter() {
            let Ok(raw) = value.as_str() else { continue };
            let text = collapse_ws(&decode_text_string(raw));
            match key.as_slice() {
                b"Title" => out.title = text.clone(),
                b"Author" => out.author = text.clone(),
                _ => {}
            }
            out.metadata.push_str(&text);
            out.metadata.push('\n');
        }
    }
    if let Ok(stream) = doc.catalog().and_then(|c| c.get_deref(b"Metadata", &doc)).and_then(Object::as_stream) {
        let xmp = stream.decompressed_content().unwrap_or_else(|_| stream.content.clone());
        out.metadata.push_str(&String::from_utf8_lossy(&xmp));
    }
    let pages = doc.get_pages().len() as u32;
    let wanted = if pages > BOOK_LENGTH { BOOK_PAGES } else { PAGES };
    for page in 1..=pages.min(wanted) {
        let mut text = String::new();
        let ok = catch_unwind(AssertUnwindSafe(|| {
            let mut output = PlainTextOutput::new(&mut text);
            pdf_extract::output_doc_page(&doc, &mut output, page).is_ok()
        }));
        out.pages.push(if matches!(ok, Ok(true)) { text } else { String::new() });
    }
    Ok(out)
}

/// PDF "text strings": UTF-16 with a byte-order mark, UTF-8 with one, or
/// PDFDocEncoding (close enough to Latin-1 for titles and names).
fn decode_text_string(raw: &[u8]) -> String {
    if let Some(rest) = raw.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = raw.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    raw.iter().map(|&b| b as char).collect()
}

/// Rejoins DOIs broken over two lines, as in
/// "https://doi.org/10.3390/\naerospace12010036".
fn join_broken_dois(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        out.push_str(line.trim_end());
        let last = line.split_whitespace().last().unwrap_or_default();
        let broken = last.contains("10.") && last.contains('/') && last.ends_with(['/', '.', '-', '_'])
            || last.to_ascii_lowercase().ends_with("doi.org/");
        if broken && lines.peek().is_some_and(|next| !next.trim().is_empty()) {
            continue;
        }
        out.push('\n');
    }
    out
}

/// Where in the PDF an identifier was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The document properties or XMP: put there by the publisher, so trusted.
    Metadata,
    FirstPage,
    LaterPage,
    FileName,
}

/// The identifiers to try, best first.
pub fn identifiers(t: &PdfText, file_name: &str) -> Vec<(Identifier, Source)> {
    let mut out: Vec<(Identifier, Source)> = Vec::new();
    let mut add = |id: Identifier, source: Source| {
        if !out.iter().any(|(i, _)| *i == id) {
            out.push((id, source));
        }
    };
    let as_id = |d: String| match d.strip_prefix("10.48550/arxiv.").and_then(ident::arxiv_id) {
        Some(a) => Identifier::Arxiv(a),
        None => Identifier::Doi(d),
    };
    for d in doi::find_all(&t.metadata) {
        add(as_id(d), Source::Metadata);
    }
    let first = t.pages.first().map(|p| join_broken_dois(p)).unwrap_or_default();
    for d in doi::find_all(&first) {
        add(as_id(d), Source::FirstPage);
    }
    for a in ident::find_arxiv(&first) {
        add(Identifier::Arxiv(a), Source::FirstPage);
    }
    let stem = file_name.trim_end_matches(".pdf").trim_end_matches(".PDF");
    if let Some(a) = ident::arxiv_id(stem) {
        add(Identifier::Arxiv(a), Source::FileName);
    }
    if let Some(d) = doi::normalise(&stem.replace('_', "/")).filter(|_| stem.starts_with("10.")) {
        add(as_id(d), Source::FileName);
    }
    for page in t.pages.iter().skip(1) {
        for d in doi::find_all(&join_broken_dois(page)) {
            add(as_id(d), Source::LaterPage);
        }
    }
    for page in &t.pages {
        for i in ident::find_isbns(page) {
            add(Identifier::Isbn(i), Source::LaterPage);
        }
    }
    out
}

/// Online lookups, so tests can answer without the network.
pub trait Online {
    fn lookup(&self, id: &Identifier) -> Result<NewPaper>;
    fn search(&self, query: &str) -> Result<Vec<NewPaper>>;
}

/// A paper made from a PDF, and how sure we are of its details.
#[derive(Clone, Debug, PartialEq)]
pub struct Found {
    pub paper: NewPaper,
    pub note: String,
    /// False when the user should check the details.
    pub sure: bool,
}

/// At most this many identifiers are looked up for one PDF.
const MAX_LOOKUPS: usize = 4;

/// Works out which paper the PDF at `path` is. Never fails: at worst the
/// paper is named after the file, with a note asking to fill it in.
pub fn identify(path: &Path, online: &dyn Online) -> Found {
    let file_name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut found = match read(path) {
        Ok(text) => identify_text(&text, &file_name, online),
        Err(e) => Found {
            paper: from_file_only(&PdfText::default(), &file_name),
            note: format!("Could not read the PDF: {e}. Fill in its details by hand."),
            sure: false,
        },
    };
    found.paper.pdf_path = Some(path.to_string_lossy().into_owned());
    found
}

pub fn identify_text(t: &PdfText, file_name: &str, online: &dyn Online) -> Found {
    let page_fp = title_fingerprint(&format!("{} {}", t.title, t.pages.first().map(String::as_str).unwrap_or_default()));
    let matches_pdf = |p: &NewPaper| {
        let fp = title_fingerprint(&p.bib.title);
        fp.len() >= 8 && page_fp.contains(&fp)
    };
    let mut fallback: Option<Found> = None;
    let mut error: Option<String> = None;
    let ids = identifiers(t, file_name);
    for (id, source) in ids.iter().take(MAX_LOOKUPS).cloned() {
        match online.lookup(&id) {
            Ok(p) if source == Source::Metadata || matches_pdf(&p) => {
                return Found { paper: p, note: format!("Found by its {} ({}).", id.kind(), id.display()), sure: true };
            }
            Ok(p) => {
                fallback.get_or_insert(Found {
                    paper: p,
                    note: format!("Found by {} in the PDF, but its title isn't on the first page: check it's the right paper.", id.display()),
                    sure: false,
                });
            }
            Err(e) => {
                error.get_or_insert(format!("{e:#}"));
            }
        }
    }
    // No identifier worked: search Crossref for the title (or the first page's opening words).
    {
        let query = if plausible_title(&t.title) {
            format!("{} {}", t.title, t.author)
        } else {
            t.pages.first().map(|p| p.split_whitespace().take(60).collect::<Vec<_>>().join(" ")).unwrap_or_default()
        };
        if !query.trim().is_empty() {
            match online.search(&query) {
                Ok(hits) => {
                    if let Some(p) = hits.into_iter().find(|p| matches_pdf(p)) {
                        return Found { paper: p, note: "Found by searching Crossref for its title: check it's the right paper.".into(), sure: false };
                    }
                }
                Err(e) => {
                    error.get_or_insert(format!("{e:#}"));
                }
            }
        }
    }
    if let Some(f) = fallback {
        return f;
    }
    let mut paper = from_file_only(t, file_name);
    // Keep the identifiers the PDF gave, so "Fill in" can use them later (and duplicates are found).
    let trusted = ids.iter().filter(|(_, s)| *s != Source::LaterPage).map(|(i, _)| i);
    for id in trusted.chain(ids.iter().filter(|(i, _)| matches!(i, Identifier::Isbn(_))).map(|(i, _)| i)) {
        match id {
            Identifier::Doi(d) if paper.bib.doi.is_none() => paper.bib.doi = Some(d.clone()),
            Identifier::Arxiv(a) if paper.bib.arxiv_id.is_empty() => paper.bib.arxiv_id = a.clone(),
            Identifier::Isbn(i) if paper.bib.isbn.is_empty() => paper.bib.isbn = i.clone(),
            _ => {}
        }
    }
    let found = paper.bib.doi.is_some() || !paper.bib.arxiv_id.is_empty() || !paper.bib.isbn.is_empty();
    let note = match (error, found) {
        (Some(e), true) => format!("Could not look it up online ({e}). Its identifier is kept: press Fill in on its Metadata tab later."),
        (Some(e), false) => format!("Could not look it up online ({e}). The details come from the PDF itself: check them."),
        (None, _) => "No DOI or arXiv ID found in the PDF. Check its details, or add its DOI on the Metadata tab and fill them in from there.".into(),
    };
    Found { paper, note, sure: false }
}

/// Document titles such as "Microsoft Word - draft3.docx" or "untitled" are useless.
fn plausible_title(title: &str) -> bool {
    let lower = title.trim().to_lowercase();
    lower.chars().filter(|c| c.is_alphabetic()).count() >= 8
        && lower.contains(' ')
        && !lower.starts_with("microsoft ")
        && !lower.starts_with("untitled")
        && ![".doc", ".docx", ".pdf", ".tex", ".dvi", ".indd", ".qxd"].iter().any(|ext| lower.ends_with(ext))
}

/// A paper from what the PDF says, or failing that its file name.
fn from_file_only(t: &PdfText, file_name: &str) -> NewPaper {
    let mut paper = NewPaper::default();
    paper.bib.entry_type = "article".into();
    paper.bib.title = if plausible_title(&t.title) {
        t.title.clone()
    } else {
        let stem = Path::new(file_name).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        collapse_ws(&stem.replace(['_', '+'], " "))
    };
    if paper.bib.title.is_empty() {
        paper.bib.title = "Untitled paper".into();
    }
    paper.bib.authors = split_authors(&t.author).iter().map(|a| names::parse_plain(a)).collect();
    paper
}

/// "A. Zhang, B. Li and C. Wu" or "Zhang, A.; Li, B." → one name each.
fn split_authors(s: &str) -> Vec<String> {
    let s = s.trim();
    if s.is_empty() {
        return Vec::new();
    }
    let parts: Vec<String> = if s.contains(';') {
        s.split(';').map(str::to_string).collect()
    } else {
        names::split_top_level(s, " and ")
            .into_iter()
            .flat_map(|part| {
                // "Zhang, Wei" is one name; "Wei Zhang, Na Li" is two.
                let pieces: Vec<&str> = part.split(',').map(str::trim).collect();
                if pieces.len() == 2 && pieces[0].split_whitespace().count() <= 2 && pieces[1].split_whitespace().count() == 1 {
                    vec![part.to_string()]
                } else {
                    pieces.into_iter().map(str::to_string).collect()
                }
            })
            .collect()
    };
    parts.into_iter().map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()
}

#[cfg(test)]
pub mod tests {
    use std::cell::RefCell;

    use super::*;
    use crate::model::Author;
    use pdf_extract::content::{Content, Operation};
    use pdf_extract::{Dictionary, Stream};

    /// A one-page PDF with `lines` of text and the given document properties.
    pub fn make_pdf(lines: &[&str], title: &str, author: &str) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let mut font = Dictionary::new();
        font.set("Type", Object::Name(b"Font".to_vec()));
        font.set("Subtype", Object::Name(b"Type1".to_vec()));
        font.set("BaseFont", Object::Name(b"Helvetica".to_vec()));
        font.set("Encoding", Object::Name(b"WinAnsiEncoding".to_vec()));
        let font_id = doc.add_object(font);
        let mut fonts = Dictionary::new();
        fonts.set("F1", Object::Reference(font_id));
        let mut resources = Dictionary::new();
        resources.set("Font", Object::Dictionary(fonts));
        let resources_id = doc.add_object(resources);
        let mut ops = vec![Operation::new("BT", vec![]), Operation::new("Tf", vec!["F1".into(), 11.into()])];
        ops.push(Operation::new("Td", vec![50.into(), 780.into()]));
        for (i, line) in lines.iter().enumerate() {
            if i > 0 {
                ops.push(Operation::new("Td", vec![0.into(), (-16).into()]));
            }
            ops.push(Operation::new("Tj", vec![Object::string_literal(*line)]));
        }
        ops.push(Operation::new("ET", vec![]));
        let content = Content { operations: ops };
        let content_id = doc.add_object(Stream::new(Dictionary::new(), content.encode().unwrap()));
        let mut page = Dictionary::new();
        page.set("Type", Object::Name(b"Page".to_vec()));
        page.set("Parent", Object::Reference(pages_id));
        page.set("Contents", Object::Reference(content_id));
        let page_id = doc.add_object(page);
        let mut pages = Dictionary::new();
        pages.set("Type", Object::Name(b"Pages".to_vec()));
        pages.set("Kids", Object::Array(vec![Object::Reference(page_id)]));
        pages.set("Count", Object::Integer(1));
        pages.set("Resources", Object::Reference(resources_id));
        pages.set("MediaBox", Object::Array(vec![0.into(), 0.into(), 595.into(), 842.into()]));
        doc.objects.insert(pages_id, Object::Dictionary(pages));
        let mut catalog = Dictionary::new();
        catalog.set("Type", Object::Name(b"Catalog".to_vec()));
        catalog.set("Pages", Object::Reference(pages_id));
        let catalog_id = doc.add_object(catalog);
        doc.trailer.set("Root", Object::Reference(catalog_id));
        let mut info = Dictionary::new();
        info.set("Title", Object::string_literal(title));
        info.set("Author", Object::string_literal(author));
        let info_id = doc.add_object(info);
        doc.trailer.set("Info", Object::Reference(info_id));
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        bytes
    }

    /// Answers lookups from a fixed list, and records what was asked.
    #[derive(Default)]
    pub struct Canned {
        pub papers: Vec<(Identifier, NewPaper)>,
        pub hits: Vec<NewPaper>,
        pub offline: bool,
        pub asked: RefCell<Vec<String>>,
    }

    impl Online for Canned {
        fn lookup(&self, id: &Identifier) -> Result<NewPaper> {
            self.asked.borrow_mut().push(id.display());
            if self.offline {
                anyhow::bail!("no network");
            }
            self.papers.iter().find(|(i, _)| i == id).map(|(_, p)| p.clone()).ok_or_else(|| anyhow!("not found"))
        }

        fn search(&self, query: &str) -> Result<Vec<NewPaper>> {
            self.asked.borrow_mut().push(format!("search: {query}"));
            if self.offline {
                anyhow::bail!("no network");
            }
            Ok(self.hits.clone())
        }
    }

    pub fn paper(title: &str) -> NewPaper {
        let mut p = NewPaper::default();
        p.bib.title = title.into();
        p
    }

    #[test]
    fn reads_text_and_properties() {
        let bytes = make_pdf(
            &["IEEE Transactions on Testing, vol. 1", "A Study of Testing Things Properly", "https://doi.org/10.1234/test.5678"],
            "A Study of Testing Things Properly",
            "Wei Zhang, Na Li and Kim Wu",
        );
        let t = read_mem(&bytes).unwrap();
        assert_eq!(t.title, "A Study of Testing Things Properly");
        assert!(t.pages[0].contains("10.1234/test.5678"), "{:?}", t.pages);
        let ids = identifiers(&t, "paper.pdf");
        assert_eq!(ids[0], (Identifier::Doi("10.1234/test.5678".into()), Source::FirstPage));
        assert_eq!(split_authors(&t.author), vec!["Wei Zhang", "Na Li", "Kim Wu"]);
        assert!(read_mem(b"not a pdf").is_err());
    }

    #[test]
    fn broken_dois_and_where_ids_come_from() {
        let t = PdfText {
            metadata: "Journal of X, doi:10.1000/meta.1\n".into(),
            pages: vec![
                "Citation: Aerospace 2025, 12, 36. https://doi.org/10.3390/\naerospace12010036\narXiv:2301.12345v2 [cs.LG]".into(),
                "References\n[1] 10.1000/ref.9".into(),
                "ISBN 978-0-470-63155-3".into(),
            ],
            ..Default::default()
        };
        let ids: Vec<(String, Source)> = identifiers(&t, "1706.03762v1.pdf").into_iter().map(|(i, s)| (i.display(), s)).collect();
        assert_eq!(
            ids,
            vec![
                ("10.1000/meta.1".to_string(), Source::Metadata),
                ("10.3390/aerospace12010036".to_string(), Source::FirstPage),
                ("arXiv:2301.12345".to_string(), Source::FirstPage),
                ("arXiv:1706.03762".to_string(), Source::FileName),
                ("10.1000/ref.9".to_string(), Source::LaterPage),
                ("ISBN 9780470631553".to_string(), Source::LaterPage),
            ]
        );
    }

    #[test]
    fn checks_the_record_is_this_pdf() {
        let t = PdfText {
            pages: vec!["Funded by 10.1000/grant.1\nA Study of Testing Things Properly\nWei Zhang\n10.1000/right.2".into()],
            ..Default::default()
        };
        // The first DOI on the page is someone else's paper; the second is this one.
        let online = Canned {
            papers: vec![
                (Identifier::Doi("10.1000/grant.1".into()), paper("Something Else Entirely Different")),
                (Identifier::Doi("10.1000/right.2".into()), paper("A study of testing things properly")),
            ],
            ..Default::default()
        };
        let f = identify_text(&t, "x.pdf", &online);
        assert_eq!(f.paper.bib.title, "A study of testing things properly");
        assert!(f.sure, "{}", f.note);
    }

    #[test]
    fn searches_by_title_then_falls_back_to_the_file() {
        let t = PdfText { title: "A Study of Testing Things Properly".into(), author: "Zhang, Wei".into(), ..Default::default() };
        let online = Canned { hits: vec![paper("Unrelated"), paper("A Study of Testing Things Properly")], ..Default::default() };
        let f = identify_text(&t, "x.pdf", &online);
        assert_eq!(f.paper.bib.title, "A Study of Testing Things Properly");
        assert!(!f.sure);
        assert!(online.asked.borrow()[0].starts_with("search: A Study"));

        // Nothing found online: the PDF's own properties.
        let f = identify_text(&t, "x.pdf", &Canned::default());
        assert_eq!(f.paper.bib.title, "A Study of Testing Things Properly");
        assert_eq!(f.paper.bib.authors, vec![Author::new("Zhang", "Wei")]);
        assert!(f.note.contains("No DOI"), "{}", f.note);

        // Offline, with a useless title: the file name, and why.
        let t = PdfText { title: "Microsoft Word - draft3.docx".into(), pages: vec!["DOI 10.1000/x.1".into()], ..Default::default() };
        let f = identify_text(&t, "zhang_2025_wideband.pdf", &Canned { offline: true, ..Default::default() });
        assert_eq!(f.paper.bib.title, "zhang 2025 wideband");
        assert!(f.note.contains("Could not look it up online"), "{}", f.note);
        assert_eq!(f.paper.bib.doi.as_deref(), Some("10.1000/x.1"), "the DOI is kept for later");
        assert!(f.note.contains("press Fill in"), "{}", f.note);
    }
}
