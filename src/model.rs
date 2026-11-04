//! The library's data types, shared by the database, importers, exporters and UI.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    #[serde(default)]
    pub family: String,
    #[serde(default)]
    pub given: String,
}

impl Author {
    pub fn new(family: impl Into<String>, given: impl Into<String>) -> Self {
        Self { family: family.into(), given: given.into() }
    }

    /// "Given Family", or just the family name for organisations.
    pub fn display(&self) -> String {
        if self.given.is_empty() {
            self.family.clone()
        } else {
            format!("{} {}", self.given, self.family)
        }
    }
}

/// "Zhang", "Zhang & Li" or "Zhang et al."
pub fn authors_short(authors: &[Author]) -> String {
    match authors {
        [] => String::new(),
        [a] => a.family.clone(),
        [a, b] => format!("{} & {}", a.family, b.family),
        [a, ..] => format!("{} et al.", a.family),
    }
}

/// "A. Zhang, B. Li and C. Wu"
pub fn authors_long(authors: &[Author]) -> String {
    let names: Vec<String> = authors.iter().map(Author::display).collect();
    match names.as_slice() {
        [] => String::new(),
        [one] => one.clone(),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ReadingStatus {
    #[default]
    ToRead,
    Reading,
    Summarised,
}

impl ReadingStatus {
    pub const ALL: [ReadingStatus; 3] = [Self::ToRead, Self::Reading, Self::Summarised];

    pub fn label(self) -> &'static str {
        match self {
            Self::ToRead => "To read",
            Self::Reading => "Reading",
            Self::Summarised => "Summarised",
        }
    }

    pub fn to_i64(self) -> i64 {
        match self {
            Self::ToRead => 0,
            Self::Reading => 1,
            Self::Summarised => 2,
        }
    }

    pub fn from_i64(v: i64) -> Self {
        match v {
            1 => Self::Reading,
            2 => Self::Summarised,
            _ => Self::ToRead,
        }
    }
}

/// Bibliographic data, as imported or edited. Text is plain Unicode (LaTeX
/// markup is decoded on import), except maths, which keeps its `$…$`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Biblio {
    /// BibTeX/BibLaTeX entry type, lower case: article, inproceedings, book…
    pub entry_type: String,
    pub title: String,
    pub authors: Vec<Author>,
    pub year: Option<i32>,
    pub month: Option<u8>,
    /// Journal, proceedings or book title.
    pub container: String,
    pub volume: String,
    pub issue: String,
    /// Page range with a plain hyphen: "12-19".
    pub pages: String,
    pub publisher: String,
    /// Normalised, lower case, without any "https://doi.org/" prefix.
    pub doi: Option<String>,
    pub url: String,
    pub isbn: String,
    pub arxiv_id: String,
    pub abstract_: String,
    pub keywords: String,
    /// Fields we don't model, kept verbatim (raw LaTeX) so export doesn't lose them.
    pub extra: BTreeMap<String, String>,
}

impl Biblio {
    pub fn entry_type_or_default(&self) -> &str {
        if self.entry_type.is_empty() { "article" } else { &self.entry_type }
    }

    /// Fills this record's empty fields from `other`. Returns true if anything changed.
    pub fn fill_from(&mut self, other: &Biblio) -> bool {
        let mut changed = false;
        let mut fill = |mine: &mut String, theirs: &String| {
            if mine.trim().is_empty() && !theirs.trim().is_empty() {
                *mine = theirs.clone();
                changed = true;
            }
        };
        fill(&mut self.title, &other.title);
        fill(&mut self.container, &other.container);
        fill(&mut self.volume, &other.volume);
        fill(&mut self.issue, &other.issue);
        fill(&mut self.pages, &other.pages);
        fill(&mut self.publisher, &other.publisher);
        fill(&mut self.url, &other.url);
        fill(&mut self.isbn, &other.isbn);
        fill(&mut self.arxiv_id, &other.arxiv_id);
        fill(&mut self.abstract_, &other.abstract_);
        fill(&mut self.keywords, &other.keywords);
        fill(&mut self.entry_type, &other.entry_type);
        if self.authors.is_empty() && !other.authors.is_empty() {
            self.authors = other.authors.clone();
            changed = true;
        }
        if self.year.is_none() && other.year.is_some() {
            self.year = other.year;
            changed = true;
        }
        if self.month.is_none() && other.month.is_some() {
            self.month = other.month;
            changed = true;
        }
        if self.doi.is_none() && other.doi.is_some() {
            self.doi = other.doi.clone();
            changed = true;
        }
        for (k, v) in &other.extra {
            if !self.extra.contains_key(k) {
                self.extra.insert(k.clone(), v.clone());
                changed = true;
            }
        }
        changed
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Paper {
    pub id: i64,
    pub citekey: String,
    /// Pinned keys are never regenerated (set on first export or a manual edit).
    pub key_pinned: bool,
    pub bib: Biblio,
    /// As stored: relative to the PDF folder in Settings, or absolute.
    pub pdf_path: Option<String>,
    pub status: ReadingStatus,
    /// 0 = not rated, else 1–5.
    pub rating: u8,
    pub added_at: String,
    pub modified_at: String,
    pub deleted: bool,
}

/// A paper read from a file or found by DOI, not yet in the library.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NewPaper {
    pub bib: Biblio,
    /// The key it had in the source file, if any.
    pub source_key: Option<String>,
    pub pdf_path: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Collection {
    pub id: i64,
    pub name: String,
    pub description: String,
    /// Index into the theme's label colours.
    pub colour: u8,
    pub parent: Option<i64>,
    pub archived: bool,
    /// Where auto-export writes this collection's .bib (if turned on in Settings).
    pub bib_export_path: Option<String>,
    pub position: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FieldKind {
    /// Free text with light Markdown.
    LongText,
    /// One line; good for matrix columns such as "Frequency range".
    ShortText,
    /// Verbatim quotes with page numbers.
    Quotes,
    /// Name / value / unit rows.
    Metrics,
    /// Why the paper matters, stored per collection.
    Relevance,
}

impl FieldKind {
    pub const USER_KINDS: [FieldKind; 4] = [Self::LongText, Self::ShortText, Self::Metrics, Self::Quotes];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::LongText => "long_text",
            Self::ShortText => "short_text",
            Self::Quotes => "quotes",
            Self::Metrics => "metrics",
            Self::Relevance => "relevance",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "short_text" => Self::ShortText,
            "quotes" => Self::Quotes,
            "metrics" => Self::Metrics,
            "relevance" => Self::Relevance,
            _ => Self::LongText,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::LongText => "Long text",
            Self::ShortText => "Short text",
            Self::Quotes => "Quotes",
            Self::Metrics => "Metrics",
            Self::Relevance => "Relevance (per collection)",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    pub id: i64,
    /// Stable identifier for built-in fields ("contribution"…); empty for custom ones.
    pub key: String,
    pub label: String,
    pub prompt: String,
    pub kind: FieldKind,
    pub position: i64,
    pub hidden: bool,
    /// None: every paper has it. Some: only papers viewed in that collection.
    pub collection: Option<i64>,
    pub builtin: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quote {
    pub text: String,
    #[serde(default)]
    pub page: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metric {
    pub name: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub unit: String,
}

impl Metric {
    pub fn value_with_unit(&self) -> String {
        if self.unit.trim().is_empty() {
            self.value.clone()
        } else {
            format!("{} {}", self.value, self.unit)
        }
    }
}

pub fn parse_quotes(s: &str) -> Vec<Quote> {
    if s.trim().is_empty() {
        return Vec::new();
    }
    serde_json::from_str(s).unwrap_or_else(|_| vec![Quote { text: s.to_string(), page: String::new() }])
}

pub fn parse_metrics(s: &str) -> Vec<Metric> {
    if s.trim().is_empty() {
        return Vec::new();
    }
    serde_json::from_str(s).unwrap_or_default()
}

pub fn json_or_empty<T: Serialize>(items: &[T]) -> String {
    if items.is_empty() {
        String::new()
    } else {
        serde_json::to_string(items).unwrap_or_default()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Relation {
    BuildsOn,
    Extends,
    Contradicts,
    ComparesAgainst,
    SameGroup,
    Related,
}

impl Relation {
    pub const ALL: [Relation; 6] = [
        Self::BuildsOn,
        Self::Extends,
        Self::Contradicts,
        Self::ComparesAgainst,
        Self::SameGroup,
        Self::Related,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::BuildsOn => "builds_on",
            Self::Extends => "extends",
            Self::Contradicts => "contradicts",
            Self::ComparesAgainst => "compares_against",
            Self::SameGroup => "same_group",
            Self::Related => "related",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "builds_on" => Self::BuildsOn,
            "extends" => Self::Extends,
            "contradicts" => Self::Contradicts,
            "compares_against" => Self::ComparesAgainst,
            "same_group" => Self::SameGroup,
            _ => Self::Related,
        }
    }

    /// Read from the paper that has the link: "this paper builds on …".
    pub fn label(self) -> &'static str {
        match self {
            Self::BuildsOn => "Builds on",
            Self::Extends => "Extends",
            Self::Contradicts => "Contradicts",
            Self::ComparesAgainst => "Compares against",
            Self::SameGroup => "Same research group as",
            Self::Related => "Related to",
        }
    }

    /// Read from the other end: "this paper is built on by …".
    pub fn inverse_label(self) -> &'static str {
        match self {
            Self::BuildsOn => "Built on by",
            Self::Extends => "Extended by",
            Self::Contradicts => "Contradicted by",
            Self::ComparesAgainst => "Compared against by",
            Self::SameGroup => "Same research group as",
            Self::Related => "Related to",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Link {
    pub id: i64,
    pub from: i64,
    pub to: i64,
    pub relation: Relation,
    pub note: String,
}

/// One row of a paper list.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PaperRow {
    pub id: i64,
    pub citekey: String,
    pub title: String,
    pub authors: String,
    pub year: Option<i32>,
    pub container: String,
    pub status: ReadingStatus,
    pub rating: u8,
    pub has_pdf: bool,
    pub has_doi: bool,
    /// Library-wide text fields filled in, out of how many.
    pub filled: usize,
    pub field_total: usize,
    pub modified_at: String,
}

pub fn now_string() -> String {
    chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn author_lists() {
        let a = vec![Author::new("Zhang", "Wei"), Author::new("Li", "Na"), Author::new("Wu", "")];
        assert_eq!(authors_short(&a[..1]), "Zhang");
        assert_eq!(authors_short(&a[..2]), "Zhang & Li");
        assert_eq!(authors_short(&a), "Zhang et al.");
        assert_eq!(authors_long(&a), "Wei Zhang, Na Li and Wu");
    }

    #[test]
    fn fill_only_empty() {
        let mut a = Biblio { title: "Kept".into(), ..Default::default() };
        let b = Biblio { title: "Other".into(), volume: "3".into(), year: Some(2020), ..Default::default() };
        assert!(a.fill_from(&b));
        assert_eq!(a.title, "Kept");
        assert_eq!(a.volume, "3");
        assert_eq!(a.year, Some(2020));
        assert!(!a.fill_from(&b));
    }

    #[test]
    fn quotes_and_metrics_round_trip() {
        let q = vec![Quote { text: "x".into(), page: "4".into() }];
        assert_eq!(parse_quotes(&json_or_empty(&q)), q);
        assert_eq!(parse_quotes("plain text"), vec![Quote { text: "plain text".into(), page: String::new() }]);
        let m = vec![Metric { name: "Gain".into(), value: "30.8".into(), unit: "dB".into() }];
        assert_eq!(parse_metrics(&json_or_empty(&m)), m);
        assert_eq!(m[0].value_with_unit(), "30.8 dB");
        assert!(parse_metrics("").is_empty());
    }
}
