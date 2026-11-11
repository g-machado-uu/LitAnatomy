//! DOIs: finding them in messy text, and looking up their metadata.

use anyhow::{Context, Result, anyhow};

use super::csljson;
use crate::model::NewPaper;

/// The first DOI in `text`, normalised to lower case with no prefix or
/// trailing punctuation. Accepts "DOI: 10.…", "Digital Object Identifier
/// 10.…", "https://doi.org/10.…", URL-encoded slashes and so on.
pub fn normalise(text: &str) -> Option<String> {
    find_all(text).into_iter().next()
}

/// Every distinct DOI in `text`, in order.
pub fn find_all(text: &str) -> Vec<String> {
    let text = text.replace("%2F", "/").replace("%2f", "/");
    let bytes = text.as_bytes();
    let mut found: Vec<String> = Vec::new();
    let mut i = 0;
    while let Some(pos) = text[i..].find("10.") {
        let start = i + pos;
        i = start + 3;
        if start > 0 && bytes[start - 1].is_ascii_alphanumeric() {
            continue;
        }
        // Registrant: 4–9 digits, maybe with sub-parts ("10.1000.10/…").
        let mut j = start + 3;
        let reg_start = j;
        while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b'.') {
            j += 1;
        }
        let digits = text[reg_start..j].chars().take_while(char::is_ascii_digit).count();
        if !(4..=9).contains(&digits) || bytes.get(j) != Some(&b'/') {
            continue;
        }
        j += 1;
        let suffix_start = j;
        while j < bytes.len() && !bytes[j].is_ascii_whitespace() && !b"\"'<>{}|\\^`".contains(&bytes[j]) {
            j += 1;
        }
        let mut doi = text[start..j].to_string();
        // Trailing punctuation from the sentence, and unbalanced brackets.
        loop {
            let Some(last) = doi.chars().last() else { break };
            let unbalanced = |open: char, close: char| last == close && doi.matches(open).count() < doi.matches(close).count();
            if ".,;:".contains(last) || unbalanced('(', ')') || unbalanced('[', ']') {
                doi.pop();
            } else {
                break;
            }
        }
        if doi.len() > suffix_start - start {
            let doi = doi.to_lowercase();
            if !found.contains(&doi) {
                found.push(doi);
            }
        }
        i = j.max(i);
    }
    found
}

pub fn url(doi: &str) -> String {
    format!("https://doi.org/{doi}")
}

/// Looks up a DOI: CSL-JSON by content negotiation at doi.org (Crossref,
/// DataCite, mEDRA…), then the Crossref API if that fails.
pub fn fetch(agent: &ureq::Agent, doi: &str) -> Result<NewPaper> {
    let negotiated = (|| -> Result<NewPaper> {
        let body = agent
            .get(&url(doi))
            .header("Accept", "application/vnd.citationstyles.csl+json")
            .call()?
            .body_mut()
            .read_to_string()?;
        let value: serde_json::Value = serde_json::from_str(&body).context("doi.org did not return JSON")?;
        csljson::item_to_paper(&value).ok_or_else(|| anyhow!("doi.org returned no title"))
    })();
    let paper = match negotiated {
        Ok(p) => p,
        Err(first) => {
            let body = agent
                .get(&format!("https://api.crossref.org/works/{doi}"))
                .call()
                .map_err(|e| match e {
                    ureq::Error::StatusCode(404) => anyhow!("DOI {doi} was not found."),
                    e => anyhow!("Could not look up {doi}: {first}; Crossref: {e}"),
                })?
                .body_mut()
                .read_to_string()?;
            let value: serde_json::Value = serde_json::from_str(&body).context("Crossref did not return JSON")?;
            csljson::item_to_paper(&value["message"]).ok_or_else(|| anyhow!("Crossref returned no title for {doi}"))?
        }
    };
    let mut paper = paper;
    paper.bib.doi.get_or_insert_with(|| doi.to_string());
    Ok(paper)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every way the DOIs are written in the student's spreadsheet.
    #[test]
    fn messy_forms() {
        let cases = [
            ("DOI: 10.1109/TAP.2025.3603900", "10.1109/tap.2025.3603900"),
            ("10.1109/TAP.2024.3371697", "10.1109/tap.2024.3371697"),
            ("DOI: 10.1109/AP-S/USNC-URSI47032.2022.9886689", "10.1109/ap-s/usnc-ursi47032.2022.9886689"),
            ("OI: 10.1109/IWAT66946.2026.11501872", "10.1109/iwat66946.2026.11501872"),
            ("https://doi.org/10.1038/s41467-023-40717-9", "10.1038/s41467-023-40717-9"),
            ("DOI:10.1109/OJAP.2025.3637377", "10.1109/ojap.2025.3637377"),
            ("https://doi.org/10.1109/TAP.2024.3424952.", "10.1109/tap.2024.3424952"),
            ("Digital Object Identifier 10.1109/TAP.2025.3571069", "10.1109/tap.2025.3571069"),
            ("https://dx.doi.org/10.1002%2Fmop.123", "10.1002/mop.123"),
            ("(see doi:10.1000/xyz(1)2).", "10.1000/xyz(1)2"),
            ("10.1016/S0140-6736(20)30183-5", "10.1016/s0140-6736(20)30183-5"),
        ];
        for (input, want) in cases {
            assert_eq!(normalise(input).as_deref(), Some(want), "{input}");
        }
    }

    #[test]
    fn not_dois() {
        for s in ["", "https://ieeexplore.ieee.org/document/9886689", "version 10.2 of x", "110.1109/abc", "10.12/x"] {
            assert_eq!(normalise(s), None, "{s}");
        }
    }

    #[test]
    fn several_in_a_paste() {
        let text = "10.1109/TAP.2025.3603900\nDOI: 10.1109/TAP.2024.3371697, and 10.1109/tap.2025.3603900 again";
        assert_eq!(find_all(text), vec!["10.1109/tap.2025.3603900", "10.1109/tap.2024.3371697"]);
    }

    /// Needs the network: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn live_lookup() {
        let p = fetch(&crate::import::http_agent(""), "10.1038/s41467-023-40717-9").expect("lookup");
        assert!(p.bib.title.to_lowercase().contains("metasurface"), "{}", p.bib.title);
        assert_eq!(p.bib.year, Some(2023));
        assert!(!p.bib.authors.is_empty());
    }
}
