//! Papers' dissections (their notes, field by field), gathered for export to
//! Word, PDF, Excel, CSV or Markdown.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};

use crate::db::Library;
use crate::import::doi;
use crate::model::{Field, FieldKind, Metric, Paper, Quote, authors_long, authors_short, parse_metrics, parse_quotes};

/// The formats a dissection can be saved in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Format {
    #[default]
    Pdf,
    Word,
    Excel,
    Csv,
    Markdown,
}

impl Format {
    pub const ALL: [Format; 5] = [Self::Pdf, Self::Word, Self::Excel, Self::Csv, Self::Markdown];

    pub fn label(self) -> &'static str {
        match self {
            Self::Pdf => "PDF",
            Self::Word => "Word",
            Self::Excel => "Excel",
            Self::Csv => "CSV",
            Self::Markdown => "Markdown",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Word => "docx",
            Self::Excel => "xlsx",
            Self::Csv => "csv",
            Self::Markdown => "md",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Self::Pdf => "A formatted document to read or print, one paper after another.",
            Self::Word => "A document you can edit, with a heading per paper and per part.",
            Self::Excel => "A table: a row per paper, a column per part and per metric. It can be imported again.",
            Self::Csv => "The same table as plain text, for any spreadsheet or script.",
            Self::Markdown => "Plain text with headings, for notes apps and version control.",
        }
    }

    /// Documents (not tables) can start each paper on a new page.
    pub fn paged(self) -> bool {
        matches!(self, Self::Pdf | Self::Word)
    }
}

/// What goes into an export besides the notes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub abstracts: bool,
    /// Show parts not written yet (as "—") in documents; tables always have every column.
    pub empty: bool,
    /// Documents: each paper starts on a new page.
    pub page_per_paper: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { abstracts: false, empty: false, page_per_paper: true }
    }
}

/// One part of a paper's dissection.
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    /// Light Markdown, as typed.
    Text(String),
    Metrics(Vec<Metric>),
    Quotes(Vec<Quote>),
}

impl Body {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Text(t) => t.trim().is_empty(),
            Self::Metrics(m) => m.is_empty(),
            Self::Quotes(q) => q.is_empty(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Section {
    pub label: String,
    pub body: Body,
}

/// A paper and its notes, in field order.
#[derive(Clone, Debug, PartialEq)]
pub struct PaperNotes {
    pub paper: Paper,
    pub tags: Vec<String>,
    pub sections: Vec<Section>,
}

impl PaperNotes {
    /// "Zhang et al. (2025)"
    pub fn short_label(&self) -> String {
        let who = authors_short(&self.paper.bib.authors);
        match (who.is_empty(), self.paper.bib.year) {
            (false, Some(y)) => format!("{who} ({y})"),
            (false, None) => who,
            (true, Some(y)) => format!("{} ({y})", self.paper.citekey),
            (true, None) => self.paper.citekey.clone(),
        }
    }

    pub fn authors(&self) -> String {
        authors_long(&self.paper.bib.authors)
    }

    /// "IEEE Trans. Antennas Propag. · 2025 · vol. 73(4) · pp. 12–19"
    pub fn venue_line(&self) -> String {
        let b = &self.paper.bib;
        let mut parts = Vec::new();
        if !b.container.trim().is_empty() {
            parts.push(b.container.trim().to_string());
        }
        if let Some(y) = b.year {
            parts.push(y.to_string());
        }
        if !b.volume.is_empty() {
            let issue = if b.issue.is_empty() { String::new() } else { format!("({})", b.issue) };
            parts.push(format!("vol. {}{issue}", b.volume));
        }
        if !b.pages.is_empty() {
            parts.push(format!("pp. {}", b.pages.replace('-', "–")));
        }
        parts.join(" · ")
    }

    /// "Summarised · rated 4/5 · tags: x, y"
    pub fn status_line(&self) -> String {
        let mut parts = vec![self.paper.status.label().to_string()];
        if self.paper.rating > 0 {
            parts.push(format!("rated {}/5", self.paper.rating));
        }
        if !self.tags.is_empty() {
            parts.push(format!("tags: {}", self.tags.join(", ")));
        }
        parts.join(" · ")
    }

    pub fn doi_url(&self) -> Option<String> {
        self.paper.bib.doi.as_deref().map(doi::url)
    }
}

/// Everything an export needs.
#[derive(Clone, Debug, PartialEq)]
pub struct Dissection {
    /// The collection's name, or "Selected papers".
    pub title: String,
    pub papers: Vec<PaperNotes>,
    /// The parts, as columns of a table: (label, kind).
    pub parts: Vec<(String, FieldKind)>,
    pub options: Options,
}

impl Dissection {
    /// The papers `ids` (in that order) with the fields they have: the
    /// library's, and those of the collections they are in (`context`'s first).
    /// In `context`, Relevance is that collection's note; elsewhere it lists
    /// each collection's.
    pub fn load(lib: &Library, ids: &[i64], context: Option<i64>, title: &str, options: Options) -> Result<Self> {
        let collections: HashMap<i64, String> = lib.collections()?.into_iter().map(|c| (c.id, c.name)).collect();
        let mut fields: Vec<Field> = lib.fields(None)?.into_iter().filter(|f| !f.hidden).collect();
        let mut memberships = HashMap::new();
        let mut order: Vec<i64> = context.into_iter().collect();
        for &id in ids {
            let m = lib.memberships(id)?;
            for (c, _) in &m {
                if !order.contains(c) {
                    order.push(*c);
                }
            }
            memberships.insert(id, m);
        }
        for c in order {
            fields.extend(lib.fields(Some(c))?.into_iter().filter(|f| f.collection == Some(c) && !f.hidden));
        }
        let mut papers = Vec::new();
        for &id in ids {
            let paper = lib.paper(id).with_context(|| format!("Paper {id} is not in the library"))?;
            let notes = lib.notes(id)?;
            let mine = memberships.remove(&id).unwrap_or_default();
            let mut sections = Vec::new();
            for f in &fields {
                if f.collection.is_some_and(|c| !mine.iter().any(|(m, _)| *m == c)) {
                    continue;
                }
                let stored = notes.get(&f.id).cloned().unwrap_or_default();
                let body = match f.kind {
                    FieldKind::Metrics => Body::Metrics(parse_metrics(&stored)),
                    FieldKind::Quotes => Body::Quotes(parse_quotes(&stored)),
                    FieldKind::Relevance => Body::Text(relevance(&mine, context, &collections)),
                    _ => Body::Text(stored),
                };
                sections.push(Section { label: f.label.clone(), body });
            }
            papers.push(PaperNotes { tags: lib.tags_of(id)?, paper, sections });
        }
        let parts = fields.iter().map(|f| (f.label.clone(), f.kind)).collect();
        Ok(Self { title: title.to_string(), papers, parts, options })
    }

    /// Writes the dissection to `path` in `format`.
    pub fn save(&self, format: Format, path: &Path) -> Result<()> {
        let bytes = match format {
            Format::Pdf => super::pdf::render(self)?,
            Format::Word => super::docx::render(self)?,
            Format::Excel => super::xlsx::render(self)?,
            Format::Csv => to_csv(&self.table()).into_bytes(),
            Format::Markdown => self.to_markdown().into_bytes(),
        };
        std::fs::write(path, bytes).with_context(|| format!("Could not write {}", path.display()))
    }

    /// The names of every metric written, in first-seen order (case-insensitive).
    pub fn metric_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for p in &self.papers {
            for s in &p.sections {
                if let Body::Metrics(ms) = &s.body {
                    for m in ms {
                        let name = m.name.trim();
                        if !name.is_empty() && !names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                            names.push(name.to_string());
                        }
                    }
                }
            }
        }
        names
    }

    /// A row per paper: its details, then a column per part (metrics get a
    /// column each, named after the metric, so the table imports again).
    pub fn table(&self) -> Table {
        let mut headers: Vec<String> =
            ["Key", "Title", "Authors", "Year", "Venue", "DOI", "Status", "Rating", "Tags"].map(String::from).to_vec();
        if self.options.abstracts {
            headers.push("Abstract".into());
        }
        let text_parts: Vec<&String> = self.parts.iter().filter(|(_, k)| *k != FieldKind::Metrics).map(|(l, _)| l).collect();
        let metric_names = self.metric_names();
        headers.extend(text_parts.iter().map(|l| l.to_string()));
        headers.extend(metric_names.iter().cloned());
        let rows = self
            .papers
            .iter()
            .map(|p| {
                let b = &p.paper.bib;
                let mut row = vec![
                    p.paper.citekey.clone(),
                    b.title.clone(),
                    p.authors(),
                    b.year.map(|y| y.to_string()).unwrap_or_default(),
                    b.container.clone(),
                    b.doi.clone().unwrap_or_default(),
                    p.paper.status.label().to_string(),
                    if p.paper.rating > 0 { p.paper.rating.to_string() } else { String::new() },
                    p.tags.join(", "),
                ];
                if self.options.abstracts {
                    row.push(b.abstract_.clone());
                }
                for label in &text_parts {
                    let cell = p.sections.iter().find(|s| &&s.label == label).map(|s| cell_text(&s.body)).unwrap_or_default();
                    row.push(cell);
                }
                let metrics: Vec<&Metric> = p
                    .sections
                    .iter()
                    .filter_map(|s| match &s.body {
                        Body::Metrics(m) => Some(m),
                        _ => None,
                    })
                    .flatten()
                    .collect();
                for name in &metric_names {
                    let value = metrics.iter().find(|m| m.name.trim().eq_ignore_ascii_case(name)).map(|m| m.value_with_unit());
                    row.push(value.unwrap_or_default());
                }
                row
            })
            .collect();
        Table { headers, rows }
    }

    pub fn to_markdown(&self) -> String {
        let mut out = format!("# {}\n\n", self.title);
        out.push_str(&format!("{} · exported from LitAnatomy on {}\n\n", plural(self.papers.len()), today()));
        for p in &self.papers {
            out.push_str(&format!("## {}\n\n", p.paper.bib.title.trim()));
            let authors = p.authors();
            if !authors.is_empty() {
                out.push_str(&format!("{authors}  \n"));
            }
            let venue = p.venue_line();
            if !venue.is_empty() {
                out.push_str(&format!("*{venue}*  \n"));
            }
            if let Some(url) = p.doi_url() {
                out.push_str(&format!("<{url}>  \n"));
            }
            out.push_str(&format!("`{}` · {}\n\n", p.paper.citekey, p.status_line()));
            for s in &p.sections {
                if s.body.is_empty() && !self.options.empty {
                    continue;
                }
                out.push_str(&format!("### {}\n\n", s.label));
                match &s.body {
                    Body::Text(t) if t.trim().is_empty() => out.push_str("—\n\n"),
                    Body::Text(t) => out.push_str(&format!("{}\n\n", t.trim())),
                    Body::Metrics(ms) if ms.is_empty() => out.push_str("—\n\n"),
                    Body::Metrics(ms) => {
                        out.push_str("| Metric | Value |\n|---|---|\n");
                        for m in ms {
                            out.push_str(&format!("| {} | {} |\n", md_cell(&m.name), md_cell(&m.value_with_unit())));
                        }
                        out.push('\n');
                    }
                    Body::Quotes(qs) if qs.is_empty() => out.push_str("—\n\n"),
                    Body::Quotes(qs) => {
                        for q in qs {
                            let page = if q.page.trim().is_empty() { String::new() } else { format!(" (p. {})", q.page.trim()) };
                            let text = q.text.trim().replace('\n', "\n> ");
                            out.push_str(&format!("> “{text}”{page}\n\n"));
                        }
                    }
                }
            }
            if self.options.abstracts && !p.paper.bib.abstract_.trim().is_empty() {
                out.push_str(&format!("### Abstract\n\n{}\n\n", p.paper.bib.abstract_.trim()));
            }
        }
        out
    }
}

/// A plain table of the dissection (CSV and Excel).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

pub fn to_csv(t: &Table) -> String {
    let quote = |s: &str| {
        if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
    };
    // A byte-order mark, so Excel reads the file as UTF-8.
    let mut out = String::from("\u{feff}");
    for row in std::iter::once(&t.headers).chain(&t.rows) {
        out.push_str(&row.iter().map(|c| quote(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    out
}

/// A part as one cell of a table.
pub fn cell_text(body: &Body) -> String {
    match body {
        Body::Text(t) => t.trim().to_string(),
        Body::Metrics(ms) => ms.iter().map(|m| format!("{}: {}", m.name, m.value_with_unit())).collect::<Vec<_>>().join("\n"),
        Body::Quotes(qs) => qs
            .iter()
            .map(|q| {
                let page = if q.page.trim().is_empty() { String::new() } else { format!(" (p. {})", q.page.trim()) };
                format!("“{}”{page}", q.text.trim())
            })
            .collect::<Vec<_>>()
            .join("\n"),
    }
}

fn relevance(mine: &[(i64, String)], context: Option<i64>, names: &HashMap<i64, String>) -> String {
    if let Some(c) = context
        && let Some((_, text)) = mine.iter().find(|(m, _)| *m == c)
    {
        return text.clone();
    }
    let written: Vec<&(i64, String)> = mine.iter().filter(|(_, t)| !t.trim().is_empty()).collect();
    match written.as_slice() {
        [] => String::new(),
        [(_, one)] => one.clone(),
        many => many
            .iter()
            .map(|(c, t)| format!("**{}:** {}", names.get(c).map(String::as_str).unwrap_or("Collection"), t.trim()))
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}

fn md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

pub fn plural(n: usize) -> String {
    if n == 1 { "1 paper".into() } else { format!("{n} papers") }
}

/// "11 November 2026"
pub fn today() -> String {
    chrono::Local::now().format("%-d %B %Y").to_string()
}

/// A file name for `title`: "Viva – dissection.pdf"
pub fn file_name(title: &str, format: Format) -> String {
    let clean: String = title.chars().map(|c| if c.is_alphanumeric() || " -_().,&".contains(c) { c } else { '_' }).collect();
    let clean = clean.trim();
    let clean = if clean.is_empty() { "Papers" } else { clean };
    format!("{clean} – dissection.{}", format.extension())
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use crate::citekey::DEFAULT_PATTERN;
    use crate::model::{Author, Biblio, NewPaper, ReadingStatus, json_or_empty};

    /// A library with two dissected papers in a collection: (library, papers, collection).
    pub fn sample() -> (Library, Vec<i64>, i64) {
        let mut lib = Library::open_in_memory().unwrap();
        let viva = lib.create_collection("Viva", None, 0).unwrap();
        let add = |lib: &mut Library, family: &str, year: i32, title: &str, doi: Option<&str>| {
            let p = NewPaper {
                bib: Biblio {
                    entry_type: "article".into(),
                    title: title.into(),
                    authors: vec![Author::new(family, "Wei"), Author::new("Li", "Na")],
                    year: Some(year),
                    container: "IEEE Transactions on Antennas and Propagation".into(),
                    volume: "73".into(),
                    pages: "12-19".into(),
                    doi: doi.map(String::from),
                    abstract_: "We study surfaces.".into(),
                    ..Default::default()
                },
                ..Default::default()
            };
            lib.add_paper(&p, DEFAULT_PATTERN).unwrap()
        };
        let a = add(&mut lib, "Zhang", 2025, "A 2-bit wideband reconfigurable surface — λ/2 cells", Some("10.1109/tap.2025.1"));
        let b = add(&mut lib, "Müller", 2024, "Varactor-tuned reflectarrays", None);
        lib.add_to_collection(viva, &[a, b]).unwrap();
        let field = |lib: &Library, key: &str| lib.fields(None).unwrap().into_iter().find(|f| f.key == key).unwrap().id;
        lib.set_note(a, field(&lib, "contribution"), "Two **PIN diodes** give four phase states.\n\n- low loss\n- *wide* band").unwrap();
        lib.set_note(a, field(&lib, "limitations"), "Only simulated above 30 GHz.").unwrap();
        let metrics = vec![
            Metric { name: "Gain".into(), value: "30.8".into(), unit: "dB".into() },
            Metric { name: "Bandwidth".into(), value: "26–31".into(), unit: "GHz".into() },
        ];
        lib.set_note(a, field(&lib, "metrics"), &json_or_empty(&metrics)).unwrap();
        let quotes = vec![Quote { text: "The first 2-bit surface with two diodes.".into(), page: "3".into() }];
        lib.set_note(a, field(&lib, "quotes"), &json_or_empty(&quotes)).unwrap();
        lib.set_relevance(viva, a, "The examiner asked about it.").unwrap();
        lib.set_note(b, field(&lib, "contribution"), "Varactors instead of diodes.").unwrap();
        lib.set_note(b, field(&lib, "metrics"), &json_or_empty(&[Metric { name: "gain".into(), value: "21".into(), unit: "dBi".into() }])).unwrap();
        lib.set_status(b, ReadingStatus::Summarised).unwrap();
        lib.set_tags(a, &["RIS".to_string()]).unwrap();
        (lib, vec![a, b], viva)
    }

    #[test]
    fn table_and_markdown() {
        let (lib, ids, viva) = sample();
        let d = Dissection::load(&lib, &ids, Some(viva), "Viva", Options { abstracts: true, ..Default::default() }).unwrap();
        assert_eq!(d.papers.len(), 2);
        assert_eq!(d.metric_names(), ["Gain", "Bandwidth"]);
        let t = d.table();
        let col = |h: &str| t.headers.iter().position(|x| x == h).unwrap_or_else(|| panic!("no column {h}"));
        assert!(!t.headers.iter().any(|h| h == "Key Metrics"), "metrics get a column each");
        assert_eq!(t.rows[0][col("Gain")], "30.8 dB");
        assert_eq!(t.rows[1][col("Gain")], "21 dBi");
        assert_eq!(t.rows[0][col("Relevance to This Project")], "The examiner asked about it.");
        assert_eq!(t.rows[0][col("Key Quotes")], "“The first 2-bit surface with two diodes.” (p. 3)");
        assert_eq!(t.rows[0][col("Authors")], "Wei Zhang and Na Li");
        assert_eq!(t.rows[1][col("Status")], "Summarised");
        assert_eq!(t.rows[0][col("Abstract")], "We study surfaces.");
        let csv = to_csv(&t);
        assert!(csv.starts_with('\u{feff}'));
        assert!(csv.contains("\"Two **PIN diodes** give four phase states.\n\n- low loss\n- *wide* band\""));
        let md = d.to_markdown();
        assert!(md.contains("## Varactor-tuned reflectarrays"));
        assert!(md.contains("| Gain | 30.8 dB |"));
        assert!(md.contains("> “The first 2-bit surface with two diodes.” (p. 3)"));
        assert!(!md.contains("### Methodology"), "empty parts left out");
        let with_empty = Dissection { options: Options { empty: true, ..d.options }, ..d };
        assert!(with_empty.to_markdown().contains("### Methodology\n\n—"));
    }

    #[test]
    fn every_format_saves() {
        let (lib, ids, viva) = sample();
        let d = Dissection::load(&lib, &ids, Some(viva), "Viva", Options::default()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        // LITANATOMY_SHOTS=<dir> keeps the files, to look at.
        let keep = std::env::var("LITANATOMY_SHOTS").ok().map(std::path::PathBuf::from);
        for f in Format::ALL {
            let path = keep.as_deref().unwrap_or(dir.path()).join(file_name("Viva", f));
            d.save(f, &path).unwrap();
            assert!(std::fs::metadata(&path).unwrap().len() > 100, "{} written", f.label());
        }
        assert_eq!(file_name("A/B: c?", Format::Pdf), "A_B_ c_ – dissection.pdf");
    }
}
