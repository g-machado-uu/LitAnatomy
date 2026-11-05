//! Paper identifiers in pasted text: DOIs, arXiv IDs, ISBNs and PubMed IDs.

use super::doi;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Identifier {
    /// Lower case, no prefix.
    Doi(String),
    /// "2301.12345" or "hep-th/9901001", without a version.
    Arxiv(String),
    /// Digits only (and a final X for some ISBN-10s).
    Isbn(String),
    Pmid(String),
}

impl Identifier {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Doi(_) => "DOI",
            Self::Arxiv(_) => "arXiv ID",
            Self::Isbn(_) => "ISBN",
            Self::Pmid(_) => "PubMed ID",
        }
    }

    /// As people write it: "10.1109/…", "arXiv:2301.12345", "ISBN 978…", "PMID 123".
    pub fn display(&self) -> String {
        match self {
            Self::Doi(d) => d.clone(),
            Self::Arxiv(a) => format!("arXiv:{a}"),
            Self::Isbn(i) => format!("ISBN {i}"),
            Self::Pmid(p) => format!("PMID {p}"),
        }
    }
}

/// Every distinct identifier in `text`, line by line. arXiv DOIs
/// (10.48550/arXiv.…) become arXiv IDs, whose records are richer.
pub fn find_all(text: &str) -> Vec<Identifier> {
    let mut found: Vec<Identifier> = Vec::new();
    let push = |id: Identifier, found: &mut Vec<Identifier>| {
        if !found.contains(&id) {
            found.push(id);
        }
    };
    for line in text.lines() {
        for d in doi::find_all(line) {
            match d.strip_prefix("10.48550/arxiv.").and_then(arxiv_id) {
                Some(a) => push(Identifier::Arxiv(a), &mut found),
                None => push(Identifier::Doi(d), &mut found),
            }
        }
        let tokens: Vec<&str> = line
            .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
            .map(|t| t.trim_matches(|c: char| "()[]<>\"'.".contains(c)))
            .filter(|t| !t.is_empty())
            .collect();
        let lone = tokens.len() == 1;
        let mut prefix: Option<&'static str> = None;
        for token in tokens {
            let lower = token.to_ascii_lowercase();
            let current = prefix.take();
            // A DOI (already found above) is never anything else.
            if doi::normalise(token).is_some() || lower.starts_with("doi") {
                continue;
            }
            if let Some(kind) = keyword(&lower) {
                prefix = Some(kind);
                continue;
            }
            let (kind, value) = match split_prefixed(token) {
                Some((k, v)) => (Some(k), v),
                None => (current, token),
            };
            if let Some(id) = classify(kind, value, lone) {
                push(id, &mut found);
            }
        }
    }
    found
}

/// A word that announces the next token: "ISBN", "PMID:", "arXiv".
fn keyword(lower: &str) -> Option<&'static str> {
    match lower.trim_end_matches(':') {
        "isbn" | "isbn-13" | "isbn-10" | "isbn13" | "isbn10" | "e-isbn" | "eisbn" => Some("isbn"),
        "pmid" | "pubmed" | "pubmed id" => Some("pmid"),
        "arxiv" => Some("arxiv"),
        _ => None,
    }
}

/// "arXiv:2301.12345", "PMID:123", "ISBN:978…" and URLs of arXiv and PubMed.
fn split_prefixed(token: &str) -> Option<(&'static str, &str)> {
    let lower = token.to_ascii_lowercase();
    for (prefix, kind) in [("arxiv:", "arxiv"), ("pmid:", "pmid"), ("isbn:", "isbn"), ("isbn-13:", "isbn"), ("isbn-10:", "isbn")] {
        if lower.starts_with(prefix) {
            return Some((kind, &token[prefix.len()..]));
        }
    }
    for (marker, kind) in [
        ("arxiv.org/abs/", "arxiv"),
        ("arxiv.org/pdf/", "arxiv"),
        ("pubmed.ncbi.nlm.nih.gov/", "pmid"),
        ("ncbi.nlm.nih.gov/pubmed/", "pmid"),
    ] {
        if let Some(i) = lower.find(marker) {
            let rest = &token[i + marker.len()..];
            let rest = rest.split(['?', '#']).next().unwrap_or_default().trim_end_matches('/');
            let rest = rest.strip_suffix(".pdf").unwrap_or(rest);
            return Some((kind, rest));
        }
    }
    None
}

fn classify(kind: Option<&str>, value: &str, lone: bool) -> Option<Identifier> {
    match kind {
        Some("arxiv") => arxiv_id(value).map(Identifier::Arxiv),
        Some("isbn") => isbn(value).map(Identifier::Isbn),
        Some("pmid") => pmid(value).map(Identifier::Pmid),
        _ => {
            if let Some(a) = arxiv_id(value) {
                Some(Identifier::Arxiv(a))
            } else if let Some(i) = isbn(value) {
                Some(Identifier::Isbn(i))
            } else if lone {
                // A number on its own line is a PubMed ID; elsewhere it could be anything.
                pmid(value).map(Identifier::Pmid)
            } else {
                None
            }
        }
    }
}

/// arXiv IDs in any text, such as the stamp in a preprint's margin
/// ("arXiv:2301.12345v2 [cs.LG]") or a file name ("2301.12345v2.pdf").
pub fn find_arxiv(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let lower = text.to_ascii_lowercase();
    for (i, _) in lower.match_indices("arxiv") {
        let rest = text[i + 5..].trim_start_matches([':', ' ', '.', '/']);
        let token = rest.split(|c: char| c.is_whitespace() || c == '[' || c == ',').next().unwrap_or_default();
        if let Some(a) = arxiv_id(token.trim_end_matches(['.', ')'])) {
            if !found.contains(&a) {
                found.push(a);
            }
        }
    }
    found
}

/// A new-style ("2301.12345v2") or old-style ("hep-th/9901001") arXiv ID,
/// without its version.
pub fn arxiv_id(s: &str) -> Option<String> {
    let s = s.trim();
    let base = match s.rfind('v') {
        Some(i) if i > 0 && s[i + 1..].chars().all(|c| c.is_ascii_digit()) && i + 1 < s.len() => &s[..i],
        _ => s,
    };
    let b = base.as_bytes();
    // New style: YYMM.NNNN (to 2014) or YYMM.NNNNN.
    if (b.len() == 9 || b.len() == 10)
        && b[4] == b'.'
        && base[..4].chars().all(|c| c.is_ascii_digit())
        && base[5..].chars().all(|c| c.is_ascii_digit())
    {
        let month: u8 = base[2..4].parse().ok()?;
        return (1..=12).contains(&month).then(|| base.to_string());
    }
    // Old style: archive(.SUBJ)/YYMMNNN.
    let (archive, number) = base.split_once('/')?;
    let archive_ok = !archive.is_empty()
        && archive.split('.').next().is_some_and(|a| !a.is_empty() && a.chars().all(|c| c.is_ascii_lowercase() || c == '-'));
    (archive_ok && number.len() == 7 && number.chars().all(|c| c.is_ascii_digit())).then(|| base.to_string())
}

/// An ISBN-10 or ISBN-13 with a valid check digit, as digits only.
pub fn isbn(s: &str) -> Option<String> {
    let cleaned: String = s.chars().filter(|c| !matches!(c, '-' | ' ' | '\u{2010}' | '\u{2011}' | '\u{2013}')).collect();
    let cleaned = cleaned.to_ascii_uppercase();
    let digits: Vec<u32> = cleaned.chars().map(|c| if c == 'X' { 10 } else { c.to_digit(10).unwrap_or(99) }).collect();
    if digits.iter().any(|d| *d == 99) {
        return None;
    }
    match digits.len() {
        13 if (cleaned.starts_with("978") || cleaned.starts_with("979")) && !cleaned.contains('X') => {
            let sum: u32 = digits.iter().enumerate().map(|(i, d)| if i % 2 == 0 { *d } else { d * 3 }).sum();
            (sum % 10 == 0).then_some(cleaned)
        }
        10 if !cleaned[..9].contains('X') => {
            let sum: u32 = digits.iter().enumerate().map(|(i, d)| d * (10 - i as u32)).sum();
            (sum % 11 == 0).then_some(cleaned)
        }
        _ => None,
    }
}

/// ISBNs in running text, e.g. on a book's copyright page: only those
/// announced by "ISBN", so stray numbers are never taken for one.
pub fn find_isbns(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let upper = text.to_ascii_uppercase();
    for (i, _) in upper.match_indices("ISBN") {
        let mut rest = &text[i + 4..];
        // Skip an "-13"/"-10"/"13:" label (but not the "10" an ISBN-10 may start with).
        for label in ["-13", "-10", "13:", "10:"] {
            if let Some(r) = rest.strip_prefix(label) {
                rest = r;
                break;
            }
        }
        let rest = rest.trim_start_matches(|c: char| c == ':' || c.is_whitespace());
        let candidate: String = rest.chars().take_while(|c| c.is_ascii_digit() || matches!(c, '-' | ' ' | 'X' | 'x')).collect();
        // Try the longest run first, then shorter ones (a run may swallow the next word's digits).
        let words: Vec<&str> = candidate.split(' ').filter(|w| !w.is_empty()).collect();
        for n in (1..=words.len()).rev() {
            if let Some(isbn) = isbn(&words[..n].join("")) {
                if !found.contains(&isbn) {
                    found.push(isbn);
                }
                break;
            }
        }
    }
    found
}

fn pmid(s: &str) -> Option<String> {
    (!s.is_empty() && s.len() <= 9 && s.chars().all(|c| c.is_ascii_digit()) && !s.starts_with('0')).then(|| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(text: &str) -> Vec<String> {
        find_all(text).iter().map(Identifier::display).collect()
    }

    #[test]
    fn mixed_paste() {
        let text = "DOI: 10.1109/TAP.2025.3603900\n\
                    arXiv:2301.12345v2\n\
                    https://arxiv.org/abs/hep-th/9901001\n\
                    ISBN 978-0-470-63155-3\n\
                    PMID: 31452104\n\
                    https://pubmed.ncbi.nlm.nih.gov/12345678/\n\
                    https://doi.org/10.48550/arXiv.2106.01234\n\
                    23456789";
        assert_eq!(
            ids(text),
            vec![
                "10.1109/tap.2025.3603900",
                "arXiv:2301.12345",
                "arXiv:hep-th/9901001",
                "ISBN 9780470631553",
                "PMID 31452104",
                "PMID 12345678",
                "arXiv:2106.01234",
                "PMID 23456789",
            ]
        );
    }

    #[test]
    fn not_identifiers() {
        // Numbers inside a sentence, a DOI's digits and impossible arXiv months.
        assert!(find_all("Vol. 12, no. 3, pp. 45-67, 2025").is_empty());
        assert_eq!(ids("10.1109/TAP.2024.3371697"), vec!["10.1109/tap.2024.3371697"]);
        assert!(arxiv_id("2313.12345").is_none());
        assert!(isbn("978-0-470-63155-4").is_none(), "bad check digit");
        assert_eq!(isbn("0-306-40615-2").as_deref(), Some("0306406152"));
        assert_eq!(isbn("0-8044-2957-X").as_deref(), Some("080442957X"));
    }

    #[test]
    fn arxiv_in_running_text() {
        assert_eq!(find_arxiv("arXiv:2301.12345v2  [cs.LG]  3 Mar 2023"), vec!["2301.12345"]);
        assert_eq!(find_arxiv("Preprint arXiv 1706.03762."), vec!["1706.03762"]);
        assert!(find_arxiv("see arxiv.org for more").is_empty());
    }

    #[test]
    fn isbns_on_a_copyright_page() {
        let page = "Library of Congress Cataloging-in-Publication Data\nISBN 978-0-470-63155-3 (hardback)\nISBN-10: 0-306-40615-2\nPrinted in 2012";
        assert_eq!(find_isbns(page), vec!["9780470631553", "0306406152"]);
        assert!(find_isbns("ISBN unknown 12345").is_empty());
    }
}
