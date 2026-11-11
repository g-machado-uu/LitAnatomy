//! Writing BibTeX and BibLaTeX.

use serde::{Deserialize, Serialize};

use crate::latex::{escape, protect_title};
use crate::model::{Author, Paper};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dialect {
    #[default]
    Bibtex,
    Biblatex,
}

impl Dialect {
    pub fn label(self) -> &'static str {
        match self {
            Self::Bibtex => "BibTeX (bibtex, natbib)",
            Self::Biblatex => "BibLaTeX (biber)",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub dialect: Dialect,
    pub include_abstract: bool,
}

const BIBTEX_TYPES: &[&str] = &[
    "article", "book", "booklet", "conference", "inbook", "incollection", "inproceedings", "manual",
    "mastersthesis", "misc", "phdthesis", "proceedings", "techreport", "unpublished",
];

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];

/// A whole .bib file, sorted by key so re-exports give small diffs.
pub fn bibliography(papers: &[Paper], opts: Options) -> String {
    let mut sorted: Vec<&Paper> = papers.iter().collect();
    sorted.sort_by(|a, b| a.citekey.to_lowercase().cmp(&b.citekey.to_lowercase()));
    let mut out = format!(
        "% Exported by LitAnatomy {} ({}). Edits here are overwritten by the next export.\n\n",
        env!("CARGO_PKG_VERSION"),
        match opts.dialect {
            Dialect::Bibtex => "BibTeX",
            Dialect::Biblatex => "BibLaTeX",
        }
    );
    for p in sorted {
        out.push_str(&entry(p, opts));
        out.push('\n');
    }
    out
}

pub fn entry(p: &Paper, opts: Options) -> String {
    let b = &p.bib;
    let bib_type = b.entry_type_or_default().to_ascii_lowercase();
    let entry_type = match opts.dialect {
        Dialect::Biblatex => bib_type.clone(),
        Dialect::Bibtex => match bib_type.as_str() {
            "thesis" => "phdthesis".to_string(),
            "report" => "techreport".to_string(),
            t if BIBTEX_TYPES.contains(&t) => t.to_string(),
            _ => "misc".to_string(),
        },
    };
    let mut fields: Vec<(String, String)> = Vec::new();
    let mut add = |name: &str, value: String| {
        if !value.trim().is_empty() {
            fields.push((name.to_string(), value));
        }
    };
    add("author", authors(&b.authors));
    add("title", protect_title(&b.title));
    let container_field = match (entry_type.as_str(), opts.dialect) {
        ("article", Dialect::Biblatex) => "journaltitle",
        ("article", Dialect::Bibtex) => "journal",
        ("inproceedings" | "incollection" | "inbook" | "conference", _) => "booktitle",
        ("techreport" | "report", _) => "institution",
        ("phdthesis" | "mastersthesis" | "thesis", _) => "school",
        ("misc", Dialect::Bibtex) => "howpublished",
        ("book", _) => "series",
        (_, Dialect::Biblatex) => "journaltitle",
        _ => "journal",
    };
    add(container_field, escape(&b.container));
    match opts.dialect {
        Dialect::Bibtex => {
            add("year", b.year.map(|y| y.to_string()).unwrap_or_default());
        }
        Dialect::Biblatex => {
            let date = match (b.year, b.month) {
                (Some(y), Some(m)) => format!("{y}-{m:02}"),
                (Some(y), None) => y.to_string(),
                _ => String::new(),
            };
            add("date", date);
        }
    }
    add("volume", escape(&b.volume));
    add("number", escape(&b.issue));
    add("pages", escape(&b.pages.replace('-', "--")));
    add("publisher", escape(&b.publisher));
    add("isbn", escape(&b.isbn));
    add("doi", b.doi.clone().unwrap_or_default());
    add("url", b.url.clone());
    if !b.arxiv_id.is_empty() {
        add("eprint", b.arxiv_id.clone());
        match opts.dialect {
            Dialect::Bibtex => add("archiveprefix", "arXiv".into()),
            Dialect::Biblatex => add("eprinttype", "arxiv".into()),
        }
    }
    if opts.include_abstract {
        add("abstract", escape(&b.abstract_));
    }
    add("keywords", escape(&b.keywords));
    for (name, value) in &b.extra {
        let clashes = fields.iter().any(|(n, _)| n == name)
            || matches!(name.as_str(), "file" | "date" | "year" | "journal" | "journaltitle" | "booktitle" | "month");
        if !clashes {
            fields.push((name.clone(), value.clone()));
        }
    }
    let mut out = format!("@{entry_type}{{{},\n", p.citekey);
    for (name, value) in &fields {
        out.push_str(&format!("  {name} = {{{value}}},\n"));
    }
    if opts.dialect == Dialect::Bibtex
        && let Some(m) = b.month.filter(|m| (1..=12).contains(m))
    {
        // Month macros, so styles print the month in the document's language.
        out.push_str(&format!("  month = {},\n", MONTHS[m as usize - 1]));
    }
    out.push_str("}\n");
    out
}

fn authors(list: &[Author]) -> String {
    list.iter()
        .map(|a| {
            if a.given.is_empty() {
                // Organisations: braced so the whole name is the family name.
                if a.family.contains(' ') { format!("{{{}}}", escape(&a.family)) } else { escape(&a.family) }
            } else {
                format!("{}, {}", escape(&a.family), escape(&a.given))
            }
        })
        .collect::<Vec<_>>()
        .join(" and ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::import::bibtex as reader;
    use crate::model::{Biblio, ReadingStatus};

    fn paper() -> Paper {
        let mut bib = Biblio {
            entry_type: "article".into(),
            title: "A Wideband RIS for 5G & Beyond".into(),
            authors: vec![Author::new("Zhang", "Wei"), Author::new("Müller", "Jörg"), Author::new("IEEE Antennas Society", "")],
            year: Some(2025),
            month: Some(3),
            container: "IEEE Transactions on Antennas and Propagation".into(),
            volume: "73".into(),
            issue: "9".into(),
            pages: "6123-6135".into(),
            doi: Some("10.1109/tap.2025.3603900".into()),
            abstract_: "Secret".into(),
            ..Default::default()
        };
        bib.extra.insert("note".into(), "Early {A}ccess".into());
        bib.extra.insert("file".into(), ":x.pdf:PDF".into());
        Paper { id: 1, citekey: "zhang2025Wideband".into(), bib, status: ReadingStatus::Reading, ..Default::default() }
    }

    #[test]
    fn bibtex_entry() {
        let e = entry(&paper(), Options::default());
        assert_eq!(
            e,
            "@article{zhang2025Wideband,
  author = {Zhang, Wei and Müller, Jörg and {IEEE Antennas Society}},
  title = {A Wideband {RIS} for {5G} \\& Beyond},
  journal = {IEEE Transactions on Antennas and Propagation},
  year = {2025},
  volume = {73},
  number = {9},
  pages = {6123--6135},
  doi = {10.1109/tap.2025.3603900},
  note = {Early {A}ccess},
  month = mar,
}
"
        );
    }

    #[test]
    fn biblatex_entry() {
        let e = entry(&paper(), Options { dialect: Dialect::Biblatex, include_abstract: true });
        assert!(e.contains("  journaltitle = {IEEE Transactions on Antennas and Propagation},\n"), "{e}");
        assert!(e.contains("  date = {2025-03},\n"));
        assert!(e.contains("  abstract = {Secret},\n"));
        assert!(!e.contains("month"));
        assert!(!e.contains("file"));
    }

    #[test]
    fn round_trip() {
        let original = paper();
        let text = bibliography(&[original.clone()], Options::default());
        let back = reader::parse(&text);
        assert!(back.errors.is_empty(), "{:?}", back.errors);
        let b = &back.papers[0].bib;
        assert_eq!(back.papers[0].source_key.as_deref(), Some("zhang2025Wideband"));
        assert_eq!(b.title, original.bib.title);
        assert_eq!(b.authors, original.bib.authors);
        assert_eq!(b.container, original.bib.container);
        assert_eq!((b.year, b.month), (Some(2025), Some(3)));
        assert_eq!(b.pages, original.bib.pages);
        assert_eq!(b.doi, original.bib.doi);
        assert_eq!(b.extra.get("note"), original.bib.extra.get("note"));
    }

    #[test]
    fn types_per_dialect() {
        let mut p = paper();
        p.bib.entry_type = "online".into();
        assert!(entry(&p, Options::default()).starts_with("@misc{"));
        assert!(entry(&p, Options { dialect: Dialect::Biblatex, ..Default::default() }).starts_with("@online{"));
        p.bib.entry_type = "inproceedings".into();
        assert!(entry(&p, Options::default()).contains("  booktitle = {IEEE"));
    }
}
