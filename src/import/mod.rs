//! Reading references from files and DOI lookups into [`NewPaper`]s.

pub mod bibtex;
pub mod csljson;
pub mod doi;
pub mod names;
pub mod refworks;
pub mod ris;

use std::path::Path;

use anyhow::{Context, Result};

use crate::model::NewPaper;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Bibtex,
    Ris,
    RefWorks,
    CslJson,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Self::Bibtex => "BibTeX / BibLaTeX",
            Self::Ris => "RIS",
            Self::RefWorks => "RefWorks",
            Self::CslJson => "CSL-JSON",
        }
    }
}

/// File extensions offered in the import dialog.
pub const EXTENSIONS: &[&str] = &["bib", "bibtex", "biblatex", "ris", "txt", "json", "csl"];

#[derive(Debug, Default)]
pub struct Parsed {
    pub papers: Vec<NewPaper>,
    /// One message per entry that could not be read.
    pub errors: Vec<String>,
}

/// Works out the format from the content, using the file name as a tie-breaker.
pub fn detect(file_name: &str, text: &str) -> Option<Format> {
    let lower = file_name.to_ascii_lowercase();
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        return Some(Format::CslJson);
    }
    if text.lines().any(|l| l.starts_with("TY  -")) {
        return Some(Format::Ris);
    }
    if text.lines().any(|l| l.starts_with("RT ")) {
        return Some(Format::RefWorks);
    }
    if text.contains('@') && (lower.ends_with(".bib") || text.lines().any(|l| l.trim_start().starts_with('@'))) {
        return Some(Format::Bibtex);
    }
    if lower.ends_with(".ris") {
        Some(Format::Ris)
    } else if lower.ends_with(".json") {
        Some(Format::CslJson)
    } else {
        None
    }
}

pub fn parse(format: Format, text: &str) -> Parsed {
    let text = text.trim_start_matches('\u{feff}');
    match format {
        Format::Bibtex => bibtex::parse(text),
        Format::Ris => ris::parse(text),
        Format::RefWorks => refworks::parse(text),
        Format::CslJson => csljson::parse(text),
    }
}

pub fn read_file(path: &Path) -> Result<(Format, Parsed)> {
    let bytes = std::fs::read(path).with_context(|| format!("Could not read {}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let format = detect(&name, &text).with_context(|| {
        format!("{name} doesn't look like a BibTeX, BibLaTeX, RIS, RefWorks or CSL-JSON file.")
    })?;
    Ok((format, parse(format, &text)))
}

/// "12--19", "12–19", "12 - 19" → "12-19".
pub fn normalise_pages(p: &str) -> String {
    let p = p.trim().replace(['–', '—'], "-");
    let parts: Vec<&str> = p.split('-').map(str::trim).filter(|s| !s.is_empty()).collect();
    match parts.as_slice() {
        [] => String::new(),
        [one] => one.to_string(),
        [a, b] => format!("{a}-{b}"),
        _ => p,
    }
}

/// Months as numbers from "3", "mar", "March" or "Mar.".
pub fn parse_month(m: &str) -> Option<u8> {
    let m = m.trim().trim_end_matches('.').to_ascii_lowercase();
    if let Ok(n) = m.parse::<u8>() {
        return (1..=12).contains(&n).then_some(n);
    }
    const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
    MONTHS.iter().position(|p| m.starts_with(p)).map(|i| i as u8 + 1)
}

/// The first four-digit number that looks like a year.
pub fn parse_year(s: &str) -> Option<i32> {
    let digits: Vec<char> = s.chars().collect();
    for w in 0..digits.len() {
        if digits.len() - w < 4 {
            break;
        }
        let slice = &digits[w..w + 4];
        let before_ok = w == 0 || !digits[w - 1].is_ascii_digit();
        let after_ok = digits.get(w + 4).is_none_or(|c| !c.is_ascii_digit());
        if before_ok && after_ok && slice.iter().all(char::is_ascii_digit) {
            let y: i32 = slice.iter().collect::<String>().parse().ok()?;
            if (1000..=2999).contains(&y) {
                return Some(y);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detection() {
        assert_eq!(detect("x.bib", "@article{a, title={x}}"), Some(Format::Bibtex));
        assert_eq!(detect("x.txt", "% comment\n@ARTICLE{1,}"), Some(Format::Bibtex));
        assert_eq!(detect("x.txt", "TY  - JOUR\nER  - "), Some(Format::Ris));
        assert_eq!(detect("x.txt", "RT Journal Article\nT1 x"), Some(Format::RefWorks));
        assert_eq!(detect("x.json", "[{\"title\":\"x\"}]"), Some(Format::CslJson));
        assert_eq!(detect("x.txt", "hello"), None);
    }

    #[test]
    fn pages_months_years() {
        assert_eq!(normalise_pages("12--19"), "12-19");
        assert_eq!(normalise_pages("12 – 19"), "12-19");
        assert_eq!(normalise_pages("e1234"), "e1234");
        assert_eq!(parse_month("mar"), Some(3));
        assert_eq!(parse_month("September"), Some(9));
        assert_eq!(parse_month("12"), Some(12));
        assert_eq!(parse_month("13"), None);
        assert_eq!(parse_year("2025/03/01/"), Some(2025));
        assert_eq!(parse_year("Published 2019"), Some(2019));
        assert_eq!(parse_year("123456"), None);
    }
}
