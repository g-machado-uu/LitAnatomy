//! Review spreadsheets (.xlsx, .xls, .ods or .csv): a row per paper, a
//! column per thing noted about it. Each column is mapped to a detail (title,
//! DOI…), a part of the anatomy, a metric, or a new field. The mapping is
//! guessed from the headings and contents, and can be changed before importing.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{Context, Result, bail};

use crate::db::Library;
use crate::import::{doi, names, parse_year};
use crate::model::{
    Author, Field, FieldKind, Metric, NewPaper, Quote, ReadingStatus, json_or_empty, metrics_from_text, parse_metrics,
    parse_quotes,
};

/// Files read as spreadsheets.
pub const EXTENSIONS: &[&str] = &["xlsx", "xlsm", "xls", "ods", "csv", "tsv"];

pub fn is_sheet(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| EXTENSIONS.iter().any(|x| x.eq_ignore_ascii_case(e)))
}

/// One sheet's cells, as text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Grid {
    pub name: String,
    pub rows: Vec<Vec<String>>,
}

impl Grid {
    pub fn cell(&self, row: usize, col: usize) -> &str {
        self.rows.get(row).and_then(|r| r.get(col)).map(String::as_str).unwrap_or("")
    }

    pub fn width(&self) -> usize {
        self.rows.iter().map(Vec::len).max().unwrap_or(0)
    }

    /// The headings: the first row with at least two cells filled in.
    pub fn header_row(&self) -> usize {
        self.rows.iter().position(|r| r.iter().filter(|c| !c.trim().is_empty()).count() >= 2).unwrap_or(0)
    }
}

/// Every sheet of the file (a CSV file has one).
pub fn read(path: &Path) -> Result<Vec<Grid>> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    if ext == "csv" || ext == "tsv" {
        let bytes = std::fs::read(path).with_context(|| format!("Could not read {}", path.display()))?;
        let text = String::from_utf8_lossy(&bytes);
        let name = path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        return Ok(vec![Grid { name, rows: parse_csv(&text) }]);
    }
    use calamine::Reader;
    let mut book = calamine::open_workbook_auto(path).map_err(|e| anyhow::anyhow!("This file can't be read as a spreadsheet ({e})"))?;
    let mut grids = Vec::new();
    for name in book.sheet_names() {
        let Ok(range) = book.worksheet_range(&name) else { continue };
        // Rows and columns before the used range are kept, so letters match Excel's.
        let (row0, col0) = range.start().map(|(r, c)| (r as usize, c as usize)).unwrap_or((0, 0));
        let mut rows = vec![Vec::new(); row0];
        for r in range.rows() {
            let mut cells = vec![String::new(); col0];
            cells.extend(r.iter().map(cell_text));
            rows.push(cells);
        }
        grids.push(Grid { name, rows });
    }
    if grids.is_empty() {
        bail!("The spreadsheet has no sheets that can be read.");
    }
    Ok(grids)
}

fn cell_text(cell: &calamine::Data) -> String {
    use calamine::Data;
    match cell {
        Data::Empty | Data::Error(_) => String::new(),
        Data::String(s) => s.trim().to_string(),
        // Numbers typed into a cell come back as floats: 2025.0 shows as 2025.
        Data::Float(f) if f.fract() == 0.0 && f.abs() < 1e15 => format!("{}", *f as i64),
        other => other.to_string(),
    }
}

/// CSV (or tab- or semicolon-separated) text, with quoted cells.
pub fn parse_csv(text: &str) -> Vec<Vec<String>> {
    let text = text.trim_start_matches('\u{feff}');
    let first = text.lines().next().unwrap_or("");
    let sep = [',', ';', '\t'].into_iter().max_by_key(|s| first.matches(*s).count()).unwrap_or(',');
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                cell.push('"');
                chars.next();
            }
            '"' if quoted => quoted = false,
            '"' if cell.is_empty() => quoted = true,
            c if c == sep && !quoted => row.push(std::mem::take(&mut cell).trim().to_string()),
            '\r' if !quoted => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut cell).trim().to_string());
                rows.push(std::mem::take(&mut row));
            }
            c => cell.push(c),
        }
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell.trim().to_string());
        rows.push(row);
    }
    rows
}

/// Where a column's cells go.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Ignore,
    Title,
    Authors,
    Year,
    Doi,
    Venue,
    Abstract,
    Keywords,
    Url,
    Key,
    Tags,
    Status,
    Rating,
    /// A part of the anatomy (also Key Metrics, Key Quotes and Relevance).
    Field(i64),
    /// A metric named after the heading, in Key Metrics.
    Metric,
    /// A new field for every paper, named after the heading.
    NewField,
}

impl Target {
    /// The details a column can hold, in the order the menu lists them.
    pub const DETAILS: [Target; 12] = [
        Self::Title,
        Self::Authors,
        Self::Year,
        Self::Doi,
        Self::Venue,
        Self::Abstract,
        Self::Keywords,
        Self::Url,
        Self::Key,
        Self::Tags,
        Self::Status,
        Self::Rating,
    ];

    pub fn label(&self, fields: &[Field], header: &str) -> String {
        match self {
            Self::Ignore => "Leave out".into(),
            Self::Title => "Title".into(),
            Self::Authors => "Authors".into(),
            Self::Year => "Year".into(),
            Self::Doi => "DOI".into(),
            Self::Venue => "Journal or venue".into(),
            Self::Abstract => "Abstract".into(),
            Self::Keywords => "Keywords".into(),
            Self::Url => "URL".into(),
            Self::Key => "Citation key".into(),
            Self::Tags => "Tags".into(),
            Self::Status => "Reading status".into(),
            Self::Rating => "Rating".into(),
            Self::Field(id) => fields.iter().find(|f| f.id == *id).map(|f| f.label.clone()).unwrap_or_else(|| "A field".into()),
            Self::Metric => format!("Metric \"{}\"", clean_header(header)),
            Self::NewField => format!("New field \"{}\"", clean_header(header)),
        }
    }

    /// How a mapping is remembered between imports.
    pub fn code(&self, fields: &[Field]) -> String {
        match self {
            Self::Field(id) => match fields.iter().find(|f| f.id == *id) {
                Some(f) if !f.key.is_empty() => format!("field:{}", f.key),
                Some(f) => format!("label:{}", f.label),
                None => "ignore".into(),
            },
            other => format!("{other:?}").to_lowercase(),
        }
    }

    pub fn from_code(code: &str, fields: &[Field]) -> Option<Self> {
        if let Some(key) = code.strip_prefix("field:") {
            return fields.iter().find(|f| f.key == key && !f.hidden).map(|f| Self::Field(f.id));
        }
        if let Some(label) = code.strip_prefix("label:") {
            return fields.iter().find(|f| f.label == label && !f.hidden).map(|f| Self::Field(f.id));
        }
        [Self::Ignore, Self::Metric, Self::NewField].into_iter().chain(Self::DETAILS).find(|t| t.code(fields) == code)
    }
}

/// A column of the sheet and where it goes.
#[derive(Clone, Debug, PartialEq)]
pub struct Column {
    pub index: usize,
    /// "A", "B"… "AA"
    pub letter: String,
    pub header: String,
    /// The first value, to show.
    pub sample: String,
    /// How many rows have a value.
    pub filled: usize,
    /// Long or multi-line values (a new field for it holds long text).
    pub long: bool,
    pub target: Target,
}

/// The columns under the headings in `header_row`, with their guessed (or
/// remembered) targets. Columns with nothing in them are left out.
pub fn columns(grid: &Grid, header_row: usize, fields: &[Field], remembered: &BTreeMap<String, String>) -> Vec<Column> {
    let mut out = Vec::new();
    for index in 0..grid.width() {
        let header = grid.cell(header_row, index).trim().to_string();
        let cells: Vec<&str> =
            (header_row + 1..grid.rows.len()).map(|r| grid.cell(r, index).trim()).filter(|c| !c.is_empty()).collect();
        if header.is_empty() && cells.is_empty() {
            continue;
        }
        let long = average_len(&cells) > 120 || cells.iter().filter(|c| c.contains('\n')).count() * 3 > cells.len();
        let remembered = (!header.is_empty())
            .then(|| remembered.get(&normalise(&header)))
            .flatten()
            .and_then(|code| Target::from_code(code, fields));
        let target = remembered.unwrap_or_else(|| guess(&header, &cells, fields));
        out.push(Column {
            index,
            letter: letter(index),
            sample: cells.first().map(|s| s.to_string()).unwrap_or_default(),
            filled: cells.len(),
            long,
            header,
            target,
        });
    }
    // Only one column can be the title (or DOI…): later ones are left out.
    let mut seen = Vec::new();
    for c in &mut out {
        if Target::DETAILS.contains(&c.target) && c.target != Target::Tags {
            if seen.contains(&c.target) {
                c.target = Target::Ignore;
            } else {
                seen.push(c.target.clone());
            }
        }
    }
    out
}

/// The remembered form of a heading: lower case, letters and digits.
pub fn normalise(header: &str) -> String {
    header.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

/// A heading as a name: "Limitations:" → "Limitations".
pub fn clean_header(header: &str) -> String {
    let h = header.split_whitespace().collect::<Vec<_>>().join(" ");
    h.trim_end_matches([':', '-', '–', '?', ' ']).to_string()
}

fn letter(mut i: usize) -> String {
    let mut s = String::new();
    loop {
        s.insert(0, (b'A' + (i % 26) as u8) as char);
        if i < 26 {
            break;
        }
        i = i / 26 - 1;
    }
    s
}

fn average_len(cells: &[&str]) -> usize {
    if cells.is_empty() { 0 } else { cells.iter().map(|c| c.chars().count()).sum::<usize>() / cells.len() }
}

/// Where a column most likely goes, from its heading, else from what is in it.
pub fn guess(header: &str, cells: &[&str], fields: &[Field]) -> Target {
    let mostly = |test: &dyn Fn(&str) -> bool| !cells.is_empty() && cells.iter().filter(|c| test(c)).count() * 2 > cells.len();
    let h = normalise(header);
    if h.is_empty() {
        return if mostly(&|c| !doi::find_all(c).is_empty()) {
            Target::Doi
        } else if mostly(&|c| c.starts_with("http")) {
            Target::Url
        } else if mostly(&|c| c.len() <= 10 && parse_year(c).is_some()) {
            Target::Year
        } else {
            Target::Ignore
        };
    }
    let visible: Vec<&Field> = fields.iter().filter(|f| !f.hidden).collect();
    // A field of the same name.
    if let Some(f) = visible.iter().find(|f| normalise(&f.label) == h) {
        return Target::Field(f.id);
    }
    let words: Vec<&str> = h.split(' ').collect();
    // A pattern is a phrase, a word, or (5 letters or more) the start of a word.
    let has = |pats: &[&str]| {
        pats.iter().any(|p| {
            if p.contains(' ') {
                format!(" {h} ").contains(&format!(" {p} "))
            } else if p.len() >= 5 {
                words.iter().any(|w| w.starts_with(p))
            } else {
                words.contains(p)
            }
        })
    };
    let field = |key: &str| visible.iter().find(|f| f.key == key).map(|f| Target::Field(f.id));
    let details: [(&[&str], Target); 13] = [
        (&["doi"], Target::Doi),
        (&["citation key", "cite key", "citekey", "bibtex", "key"], Target::Key),
        (&["abstract"], Target::Abstract),
        (&["keyword"], Target::Keywords),
        (&["title", "paper name", "name of paper", "name of the paper", "paper", "article", "reference", "publication title"], Target::Title),
        (&["author", "authors", "by"], Target::Authors),
        (&["year", "date", "published"], Target::Year),
        (&["journal", "venue", "conference", "proceedings", "source", "publisher", "publication"], Target::Venue),
        (&["url", "link", "website", "web"], Target::Url),
        (&["tag", "tags", "label", "labels", "category", "theme"], Target::Tags),
        (&["status", "read", "progress"], Target::Status),
        (&["rating", "stars", "score"], Target::Rating),
        (&["relevan", "for me", "my work", "my research", "my project", "my thesis", "why it matters", "use for"], Target::Ignore),
    ];
    for (pats, target) in details {
        if has(pats) {
            if target == Target::Ignore {
                // Relevance: why the paper matters (to the one reading).
                if let Some(t) = field("relevance") {
                    return t;
                }
                continue;
            }
            return target;
        }
    }
    let anatomy: [(&[&str], &str); 10] = [
        (&["achiev", "finding", "result", "outcome", "conclusion", "conclud"], "findings"),
        (&["why", "problem", "motivation", "motivat", "aim", "aims", "objective", "goal"], "problem"),
        (&["how", "method", "approach", "design", "setup", "set up", "experiment", "procedure"], "methodology"),
        (&["metric", "performance", "numbers", "specs", "specification"], "metrics"),
        (&["limitation", "weakness", "drawback", "cons", "shortcoming"], "limitations"),
        (&["gap", "gaps", "future", "open question", "open questions", "next step"], "gaps"),
        (&["assessment", "critique", "critical", "opinion", "evaluation", "verdict", "thoughts"], "assessment"),
        (&["quote", "quotes", "quotation"], "quotes"),
        (&["what", "contribution", "contribut", "novel", "novelty", "summary", "summar", "main idea"], "contribution"),
        (&["note", "notes", "additional", "comment", "comments", "other", "remark", "remarks", "misc", "extra"], "notes"),
    ];
    for (pats, key) in anatomy {
        if has(pats)
            && let Some(t) = field(key)
        {
            return t;
        }
    }
    // Anything else: a short value is a metric, a longer one gets its own field.
    let short = average_len(cells) <= 80 && cells.iter().filter(|c| c.contains('\n')).count() * 3 <= cells.len();
    if short && !cells.is_empty() { Target::Metric } else { Target::NewField }
}

/// Something from a row for the paper's notes (not its details).
#[derive(Clone, Debug, PartialEq)]
pub struct SheetNote {
    pub target: Target,
    pub header: String,
    pub text: String,
    /// The column holds long text (a new field for it is long text).
    pub long: bool,
}

/// The papers of a sheet, with their notes, and the rows left out.
#[derive(Clone, Debug, Default)]
pub struct Rows {
    pub papers: Vec<(NewPaper, Vec<SheetNote>)>,
    pub skipped: Vec<String>,
}

/// Reads every row under the headings into a paper. A row needs a title or a
/// DOI; one with only a DOI is titled by it until it is looked up.
pub fn rows(grid: &Grid, header_row: usize, columns: &[Column]) -> Rows {
    let mut out = Rows::default();
    for r in header_row + 1..grid.rows.len() {
        let mut paper = NewPaper::default();
        paper.bib.entry_type = "article".into();
        let mut notes = Vec::new();
        let mut any = false;
        for c in columns {
            let text = grid.cell(r, c.index).trim();
            if text.is_empty() {
                continue;
            }
            any = true;
            let one_line = || text.split_whitespace().collect::<Vec<_>>().join(" ");
            let b = &mut paper.bib;
            match &c.target {
                Target::Ignore => {}
                Target::Title => b.title = one_line(),
                Target::Authors => b.authors = authors(text),
                Target::Year => b.year = parse_year(text),
                Target::Doi => match doi::find_all(text).into_iter().next() {
                    Some(d) => b.doi = Some(d),
                    None if text.starts_with("http") => b.url = one_line(),
                    None => {}
                },
                Target::Venue => b.container = one_line(),
                Target::Abstract => b.abstract_ = text.to_string(),
                Target::Keywords => b.keywords = one_line(),
                Target::Url => b.url = one_line(),
                Target::Key => paper.source_key = Some(one_line()),
                t => notes.push(SheetNote { target: t.clone(), header: clean_header(&c.header), text: text.to_string(), long: c.long }),
            }
        }
        if !any {
            continue;
        }
        if paper.bib.title.is_empty() {
            match &paper.bib.doi {
                Some(d) => paper.bib.title = format!("DOI {d}"),
                None => {
                    out.skipped.push(format!("Row {}: no title or DOI", r + 1));
                    continue;
                }
            }
        }
        out.papers.push((paper, notes));
    }
    out
}

/// "Zhang, W.; Li, N.", "Wei Zhang and Na Li", "Zhang W, Li N, Wu C"…
fn authors(text: &str) -> Vec<Author> {
    let text = text.replace("et al.", "").replace("et al", "");
    let text = text.trim().trim_end_matches([',', ';']);
    let parts: Vec<&str> = if text.contains(';') || text.contains('\n') {
        text.split([';', '\n']).collect()
    } else if text.contains(" and ") || text.contains(" & ") {
        return names::parse_bibtex_list(&text.replace(" & ", " and ").replace(", and ", " and "));
    } else if text.matches(',').count() == 1 {
        vec![text]
    } else {
        text.split(',').collect()
    };
    parts.into_iter().map(str::trim).filter(|p| !p.is_empty()).map(names::parse_plain).collect()
}

/// "Done", "summarised", "reading", "to read"…
fn status(text: &str) -> Option<ReadingStatus> {
    let t = text.to_lowercase();
    if ["to read", "unread", "not read", "todo", "to do", "not started", "no"].iter().any(|p| t.contains(p)) {
        Some(ReadingStatus::ToRead)
    } else if ["summar", "done", "complete", "finish", "read", "yes"].iter().any(|p| t.contains(p)) && !t.contains("reading") {
        Some(ReadingStatus::Summarised)
    } else if ["reading", "progress", "started", "partly", "partial"].iter().any(|p| t.contains(p)) {
        Some(ReadingStatus::Reading)
    } else {
        None
    }
}

/// "4", "4/5", "★★★★"
fn rating(text: &str) -> Option<u8> {
    let stars = text.chars().filter(|c| matches!(c, '★' | '*')).count();
    if stars > 0 {
        return Some(stars.min(5) as u8);
    }
    text.chars().find(char::is_ascii_digit).and_then(|c| c.to_digit(10)).filter(|d| (1..=5).contains(d)).map(|d| d as u8)
}

/// Quotes written in a cell: one per paragraph (or line), with "(p. 12)" pages.
fn quotes(text: &str) -> Vec<Quote> {
    let parts: Vec<&str> = if text.contains("\n\n") { text.split("\n\n").collect() } else { text.lines().collect() };
    parts
        .into_iter()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| {
            let mut text = p.to_string();
            let mut page = String::new();
            if let Some(i) = text.rfind("(p.").or_else(|| text.rfind("(pp."))
                && text.ends_with(')')
            {
                page = text[i..].trim_start_matches("(pp.").trim_start_matches("(p.").trim_end_matches(')').trim().to_string();
                text.truncate(i);
            }
            let text = text.trim().trim_matches(['"', '“', '”', '\'']).trim().to_string();
            Quote { text, page }
        })
        .collect()
}

/// Writes a row's notes into paper `paper`. New fields are made as needed
/// (`created`: heading → field). With `fill_only` (a paper already in the
/// library), only empty parts are written and its status is kept. Relevance
/// goes to `collection`, or to Further Notes without one.
pub fn apply(
    lib: &mut Library,
    paper: i64,
    notes: &[SheetNote],
    collection: Option<i64>,
    created: &mut HashMap<String, i64>,
    fill_only: bool,
) -> Result<()> {
    // Every field: a column can go to a collection's own field too.
    let fields = lib.all_fields()?;
    let kind_of = |id: i64, fields: &[Field]| fields.iter().find(|f| f.id == id).map(|f| f.kind);
    let metrics_field = fields
        .iter()
        .find(|f| f.key == "metrics")
        .or_else(|| fields.iter().find(|f| f.kind == FieldKind::Metrics && f.collection.is_none()))
        .map(|f| f.id);
    let notes_field = fields.iter().find(|f| f.key == "notes").map(|f| f.id);
    // Field → what goes in it: (heading, text).
    let mut texts: BTreeMap<i64, Vec<(String, String)>> = BTreeMap::new();
    let mut metrics: Vec<Metric> = Vec::new();
    let mut tags: Vec<String> = Vec::new();
    let mut new_status = None;
    let mut new_rating = None;
    for n in notes {
        match &n.target {
            Target::Field(id) => match kind_of(*id, &fields) {
                Some(FieldKind::Metrics) => metrics.extend(metrics_from_text(&n.text)),
                Some(FieldKind::Relevance) => match collection {
                    Some(c) => {
                        let current = lib.relevance(c, paper)?.unwrap_or_default();
                        if !fill_only || current.trim().is_empty() {
                            lib.set_relevance(c, paper, &n.text)?;
                        }
                    }
                    None => {
                        if let Some(f) = notes_field {
                            texts.entry(f).or_default().push((n.header.clone(), n.text.clone()));
                        }
                    }
                },
                Some(_) => texts.entry(*id).or_default().push((n.header.clone(), n.text.clone())),
                None => {}
            },
            Target::Metric => {
                let value = n.text.split_whitespace().collect::<Vec<_>>().join(" ");
                metrics.extend(metrics_from_text(&format!("{}: {value}", n.header.replace(':', " "))));
            }
            Target::NewField => {
                let id = match created.get(&n.header) {
                    Some(id) => *id,
                    None => {
                        let kind = if n.long { FieldKind::LongText } else { FieldKind::ShortText };
                        let id = lib.add_field(&n.header, "", kind, None)?;
                        created.insert(n.header.clone(), id);
                        id
                    }
                };
                texts.entry(id).or_default().push((n.header.clone(), n.text.clone()));
            }
            Target::Tags => tags.extend(n.text.split([',', ';', '\n']).map(|t| t.trim().to_string()).filter(|t| !t.is_empty())),
            Target::Status => new_status = status(&n.text),
            Target::Rating => new_rating = rating(&n.text),
            _ => {}
        }
    }
    let fields = lib.all_fields()?;
    let stored = lib.notes(paper)?;
    for (id, parts) in texts {
        let current = stored.get(&id).cloned().unwrap_or_default();
        if fill_only && !current.trim().is_empty() {
            continue;
        }
        let kind = kind_of(id, &fields).unwrap_or(FieldKind::LongText);
        let content = match kind {
            FieldKind::Quotes => {
                let all: Vec<Quote> = parts.iter().flat_map(|(_, t)| quotes(t)).collect();
                json_or_empty(&[parse_quotes(&current), all].concat())
            }
            FieldKind::ShortText => parts.iter().map(|(_, t)| t.split_whitespace().collect::<Vec<_>>().join(" ")).collect::<Vec<_>>().join("; "),
            _ if parts.len() == 1 => parts[0].1.clone(),
            _ => parts.iter().map(|(h, t)| format!("**{h}:** {t}")).collect::<Vec<_>>().join("\n\n"),
        };
        lib.set_note(paper, id, &content)?;
    }
    if let Some(id) = metrics_field.filter(|_| !metrics.is_empty()) {
        let mut all = parse_metrics(stored.get(&id).map(String::as_str).unwrap_or(""));
        for m in metrics {
            if !all.iter().any(|o| o.name.trim().eq_ignore_ascii_case(m.name.trim())) {
                all.push(m);
            }
        }
        lib.set_note(paper, id, &json_or_empty(&all))?;
    }
    if !tags.is_empty() {
        let mut all = lib.tags_of(paper)?;
        for t in tags {
            if !all.iter().any(|o| o.eq_ignore_ascii_case(&t)) {
                all.push(t);
            }
        }
        lib.set_tags(paper, &all)?;
    }
    if !fill_only {
        // Set after the notes, so it wins over the status the notes suggest.
        if let Some(s) = new_status {
            lib.set_status(paper, s)?;
        }
        if let Some(r) = new_rating {
            lib.set_rating(paper, r)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::citekey::DEFAULT_PATTERN;

    /// A sheet like a student's own review table.
    fn student_sheet() -> Grid {
        let rows = vec![
            vec!["", "Paper Name", "What did they do", "Why did they do", "How did they do", "What did they achieve", "Frequency Range", "Limitations:", "Why it is relevant for me", "Additional Points"],
            vec!["DOI: 10.1109/TAP.2025.1234", "A 2-bit reconfigurable\nsurface", "A unit cell with two diodes", "Fewer diodes", "Full-wave simulation", "Four phase states", "26-31 GHz", "Simulated only", "", "Ask about bias lines"],
            vec!["", "Varactor reflectarrays", "Varactor tuning", "", "", "Continuous phase", "", "", "", ""],
            vec!["https://doi.org/10.1000/xyz.9", "", "", "", "", "", "", "", "", ""],
            vec!["", "", "", "", "", "", "", "Orphan note", "", ""],
        ]
        .into_iter()
        .map(|r| r.into_iter().map(String::from).collect())
        .collect();
        Grid { name: "Sheet1".into(), rows }
    }

    #[test]
    fn guesses_a_review_sheet() {
        let lib = Library::open_in_memory().unwrap();
        let fields = lib.fields(None).unwrap();
        let key = |k: &str| Target::Field(fields.iter().find(|f| f.key == k).unwrap().id);
        let grid = student_sheet();
        assert_eq!(grid.header_row(), 0);
        let cols = columns(&grid, 0, &fields, &BTreeMap::new());
        let targets: Vec<Target> = cols.iter().map(|c| c.target.clone()).collect();
        assert_eq!(
            targets,
            [
                Target::Doi,
                Target::Title,
                key("contribution"),
                key("problem"),
                key("methodology"),
                key("findings"),
                Target::Metric,
                key("limitations"),
                key("relevance"),
                key("notes"),
            ]
        );
        assert_eq!(cols[0].letter, "A");
        assert_eq!(cols[1].sample, "A 2-bit reconfigurable\nsurface");
        // Remembered choices win over guesses.
        let mut remembered = BTreeMap::new();
        remembered.insert(normalise("Frequency Range"), "newfield".to_string());
        remembered.insert(normalise("What did they do"), "field:methodology".to_string());
        let cols = columns(&grid, 0, &fields, &remembered);
        assert_eq!(cols[6].target, Target::NewField);
        assert_eq!(cols[2].target, key("methodology"));
        for t in [Target::Title, Target::Metric, key("relevance"), Target::NewField] {
            assert_eq!(Target::from_code(&t.code(&fields), &fields), Some(t));
        }
    }

    #[test]
    fn imports_rows_with_notes() {
        let mut lib = Library::open_in_memory().unwrap();
        let viva = lib.create_collection("Viva", None, 0).unwrap();
        let fields = lib.fields(None).unwrap();
        let grid = student_sheet();
        let mut cols = columns(&grid, 0, &fields, &BTreeMap::new());
        cols[9].target = Target::NewField;
        let rows = rows(&grid, 0, &cols);
        assert_eq!(rows.papers.len(), 3);
        assert_eq!(rows.skipped, ["Row 5: no title or DOI"]);
        let (first, notes) = &rows.papers[0];
        assert_eq!(first.bib.title, "A 2-bit reconfigurable surface");
        assert_eq!(first.bib.doi.as_deref(), Some("10.1109/tap.2025.1234"));
        assert_eq!(rows.papers[2].0.bib.title, "DOI 10.1000/xyz.9");
        let id = lib.add_paper(first, DEFAULT_PATTERN).unwrap();
        lib.add_to_collection(viva, &[id]).unwrap();
        let mut created = HashMap::new();
        apply(&mut lib, id, notes, Some(viva), &mut created, false).unwrap();
        let fields = lib.fields(None).unwrap();
        let note = |lib: &Library, k: &str| {
            let f = fields.iter().find(|f| f.key == k || f.label == k).unwrap();
            lib.notes(id).unwrap().get(&f.id).cloned().unwrap_or_default()
        };
        assert_eq!(note(&lib, "contribution"), "A unit cell with two diodes");
        assert_eq!(note(&lib, "limitations"), "Simulated only");
        assert_eq!(note(&lib, "Additional Points"), "Ask about bias lines");
        assert_eq!(parse_metrics(&note(&lib, "metrics")), [Metric { name: "Frequency Range".into(), value: "26-31".into(), unit: "GHz".into() }]);
        assert!(created.contains_key("Additional Points"));
        // Written notes moved the status on.
        assert_eq!(lib.paper(id).unwrap().status, ReadingStatus::Reading);
        // Again, filling only: nothing written twice, no new field.
        apply(&mut lib, id, notes, Some(viva), &mut created, true).unwrap();
        assert_eq!(lib.fields(None).unwrap().len(), fields.len());
        assert_eq!(parse_metrics(&note(&lib, "metrics")).len(), 1);
    }

    #[test]
    fn cells_read() {
        assert_eq!(parse_csv("a,\"b, c\",\"say \"\"hi\"\"\"\r\n1,\"two\nlines\",3\n"), [["a", "b, c", "say \"hi\""], ["1", "two\nlines", "3"]]);
        assert_eq!(parse_csv("\u{feff}x;y\n1;2"), [["x", "y"], ["1", "2"]]);
        let a = authors("Zhang, W.; Li, N.");
        assert_eq!(a, [Author::new("Zhang", "W."), Author::new("Li", "N.")]);
        assert_eq!(authors("Wei Zhang and Na Li").len(), 2);
        assert_eq!(authors("Zhang W, Li N, Wu C").len(), 3);
        assert_eq!(authors("Zhang et al."), [Author::new("Zhang", "")]);
        assert_eq!(status("Done"), Some(ReadingStatus::Summarised));
        assert_eq!(status("reading"), Some(ReadingStatus::Reading));
        assert_eq!(status("to read"), Some(ReadingStatus::ToRead));
        assert_eq!(rating("★★★★"), Some(4));
        assert_eq!(rating("3/5"), Some(3));
        assert_eq!(quotes("\"First\" (p. 3)\n“Second”"), [Quote { text: "First".into(), page: "3".into() }, Quote { text: "Second".into(), page: String::new() }]);
        assert_eq!(letter(0), "A");
        assert_eq!(letter(27), "AB");
        assert_eq!(clean_header("Limitations:"), "Limitations");
    }

    /// Shows how a real sheet maps, without printing what is in it:
    /// `LITANATOMY_SHEET=<file> cargo test real_sheet -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_sheet() {
        let Ok(path) = std::env::var("LITANATOMY_SHEET") else { return };
        let grids = read(Path::new(&path)).unwrap();
        let lib = Library::open_in_memory().unwrap();
        let fields = lib.fields(None).unwrap();
        for g in &grids {
            let hr = g.header_row();
            let cols = columns(g, hr, &fields, &BTreeMap::new());
            println!("sheet {:?}: headings in row {}", g.name, hr + 1);
            for c in &cols {
                println!("  {} {:<32} → {} ({} filled, long: {})", c.letter, c.header, c.target.label(&fields, &c.header), c.filled, c.long);
            }
            let rows = rows(g, hr, &cols);
            let with_doi = rows.papers.iter().filter(|(p, _)| p.bib.doi.is_some()).count();
            let notes: usize = rows.papers.iter().map(|(_, n)| n.len()).sum();
            println!("  {} papers ({with_doi} with a DOI), {notes} notes, {} rows left out", rows.papers.len(), rows.skipped.len());
        }
    }

    #[test]
    fn exported_table_imports_again() {
        use crate::export::dissection::{Dissection, Options, tests::sample};
        let (lib, ids, viva) = sample();
        let d = Dissection::load(&lib, &ids, Some(viva), "Viva", Options::default()).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("viva.xlsx");
        d.save(crate::export::dissection::Format::Excel, &path).unwrap();
        let grids = read(&path).unwrap();
        let fields = lib.fields(None).unwrap();
        let cols = columns(&grids[0], grids[0].header_row(), &fields, &BTreeMap::new());
        let target = |h: &str| cols.iter().find(|c| c.header == h).map(|c| c.target.clone()).unwrap();
        assert_eq!(target("Key"), Target::Key);
        assert_eq!(target("Title"), Target::Title);
        assert_eq!(target("DOI"), Target::Doi);
        assert_eq!(target("Status"), Target::Status);
        assert_eq!(target("Gain"), Target::Metric);
        assert_eq!(target("Key Quotes"), Target::Field(fields.iter().find(|f| f.key == "quotes").unwrap().id));
        let rows = rows(&grids[0], 0, &cols);
        assert_eq!(rows.papers.len(), 2);
        assert_eq!(rows.papers[0].0.bib.year, Some(2025));
        assert_eq!(rows.papers[0].0.bib.authors.len(), 2);
    }
}
