//! RIS (EndNote, Mendeley, Zotero, Web of Science, Scopus, IEEE Xplore…).

use super::{Parsed, doi, names, normalise_pages, parse_month, parse_year};
use crate::model::{Biblio, NewPaper};

pub fn parse(text: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let mut current: Option<Vec<(String, String)>> = None;
    let mut last_tag = String::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim_end();
        let tagged = line.len() >= 5
            && line.as_bytes()[..2].iter().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            && line[2..].starts_with("  -");
        if !tagged {
            // A continuation of the previous field (some exports wrap abstracts).
            if let Some(fields) = current.as_mut()
                && !line.trim().is_empty()
                && let Some(last) = fields.last_mut()
                && last.0 == last_tag
            {
                last.1.push(' ');
                last.1.push_str(line.trim());
            }
            continue;
        }
        let tag = line[..2].to_string();
        let value = line[5.min(line.len())..].trim().to_string();
        match tag.as_str() {
            "TY" => {
                if current.is_some() {
                    parsed.errors.push(format!("Line {}: a record starts before the previous one ended", n + 1));
                }
                current = Some(vec![(tag.clone(), value)]);
            }
            "ER" => {
                if let Some(fields) = current.take() {
                    match to_paper(&fields) {
                        Some(p) => parsed.papers.push(p),
                        None => parsed.errors.push(format!("Line {}: a record has no title", n + 1)),
                    }
                }
            }
            _ => {
                if let Some(fields) = current.as_mut() {
                    fields.push((tag.clone(), value));
                }
            }
        }
        last_tag = tag;
    }
    if let Some(fields) = current
        && let Some(p) = to_paper(&fields)
    {
        // Missing final "ER" — still worth importing.
        parsed.papers.push(p);
    }
    parsed
}

fn entry_type(ty: &str) -> &'static str {
    match ty {
        "JOUR" | "JFULL" | "MGZN" | "NEWS" | "EJOUR" => "article",
        "CONF" | "CPAPER" => "inproceedings",
        "BOOK" | "EBOOK" | "EDBOOK" => "book",
        "CHAP" | "ECHAP" => "incollection",
        "THES" => "phdthesis",
        "RPRT" => "techreport",
        "ELEC" | "WEB" | "BLOG" => "online",
        "PAT" => "patent",
        "DATA" => "dataset",
        _ => "misc",
    }
}

fn to_paper(fields: &[(String, String)]) -> Option<NewPaper> {
    let first = |tags: &[&str]| {
        tags.iter()
            .find_map(|t| fields.iter().find(|(k, v)| k == t && !v.is_empty()).map(|(_, v)| v.clone()))
            .unwrap_or_default()
    };
    let all = |tags: &[&str]| -> Vec<String> {
        fields.iter().filter(|(k, v)| tags.contains(&k.as_str()) && !v.is_empty()).map(|(_, v)| v.clone()).collect()
    };
    let title = first(&["TI", "T1", "CT"]);
    if title.is_empty() {
        return None;
    }
    let ty = first(&["TY"]);
    let container = if matches!(ty.as_str(), "CONF" | "CPAPER" | "CHAP") {
        first(&["T2", "BT", "JF", "JO", "JA", "J2"])
    } else {
        first(&["JF", "JO", "T2", "JA", "J2", "BT"])
    };
    let date = first(&["PY", "Y1", "DA"]);
    let start = first(&["SP"]);
    let end = first(&["EP"]);
    let pages = if end.is_empty() || start.contains('-') { start } else { format!("{start}-{end}") };
    let doi_field = first(&["DO", "DI", "M3"]);
    let url = first(&["UR", "L2"]);
    let sn = first(&["SN"]);
    let mut bib = Biblio {
        entry_type: entry_type(&ty).to_string(),
        title,
        authors: all(&["AU", "A1"]).iter().map(|n| names::parse_plain(n)).collect(),
        year: parse_year(&date),
        month: date.split('/').nth(1).and_then(parse_month),
        container,
        volume: first(&["VL"]),
        issue: first(&["IS", "CP"]),
        pages: normalise_pages(&pages),
        publisher: first(&["PB"]),
        doi: doi::normalise(&doi_field).or_else(|| doi::normalise(&url)),
        url: if url.contains("doi.org/") { String::new() } else { url },
        abstract_: first(&["AB", "N2"]),
        keywords: all(&["KW"]).join("; "),
        ..Default::default()
    };
    // SN holds an ISBN for books and an ISSN for journals.
    if matches!(bib.entry_type.as_str(), "book" | "incollection") {
        bib.isbn = sn;
    } else if !sn.is_empty() {
        bib.extra.insert("issn".into(), sn);
    }
    let editors = all(&["A2", "ED"]);
    if !editors.is_empty() && bib.entry_type != "article" {
        bib.extra.insert("editor".into(), editors.join(" and "));
    }
    let pdf = all(&["L1", "LK"]).into_iter().find(|l| l.to_ascii_lowercase().ends_with(".pdf"));
    let pdf = pdf.map(|p| p.strip_prefix("file://").map(str::to_string).unwrap_or(p));
    let source_key = Some(first(&["ID"])).filter(|k| !k.is_empty());
    Some(NewPaper { bib, source_key, pdf_path: pdf })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Author;

    const SAMPLE: &str = "TY  - JOUR
AU  - Zhang, Wei
AU  - Li, Na
TI  - A Wideband Reconfigurable Intelligent Surface for 5G Millimeter-Wave Applications
T2  - IEEE Transactions on Antennas and Propagation
PY  - 2024/03/01/
VL  - 72
IS  - 3
SP  - 2399
EP  - 2410
SN  - 0018-926X
DO  - 10.1109/TAP.2024.0000001
AB  - We propose a 1-bit RIS
  with wide bandwidth.
KW  - RIS
KW  - mmWave
ER  -

TY  - CONF
AU  - Oliveri, G.
TI  - Synthesis of Modular Electromagnetic Skins
T2  - 2022 IEEE AP-S
PY  - 2022
L1  - file:///home/me/skins.pdf
ER  -
";

    #[test]
    fn records() {
        let p = parse(SAMPLE);
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        assert_eq!(p.papers.len(), 2);
        let a = &p.papers[0].bib;
        assert_eq!(a.entry_type, "article");
        assert_eq!(a.authors, vec![Author::new("Zhang", "Wei"), Author::new("Li", "Na")]);
        assert_eq!(a.year, Some(2024));
        assert_eq!(a.month, Some(3));
        assert_eq!(a.pages, "2399-2410");
        assert_eq!(a.container, "IEEE Transactions on Antennas and Propagation");
        assert_eq!(a.doi.as_deref(), Some("10.1109/tap.2024.0000001"));
        assert_eq!(a.abstract_, "We propose a 1-bit RIS with wide bandwidth.");
        assert_eq!(a.keywords, "RIS; mmWave");
        assert_eq!(a.extra.get("issn").map(String::as_str), Some("0018-926X"));
        let b = &p.papers[1];
        assert_eq!(b.bib.entry_type, "inproceedings");
        assert_eq!(b.bib.container, "2022 IEEE AP-S");
        assert_eq!(b.pdf_path.as_deref(), Some("/home/me/skins.pdf"));
    }
}
