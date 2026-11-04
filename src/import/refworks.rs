//! RefWorks tagged format: "RT Journal Article", "A1 Zhang, Wei", "T1 …".

use super::{Parsed, doi, names, normalise_pages, parse_month, parse_year};
use crate::model::{Biblio, NewPaper};

pub fn parse(text: &str) -> Parsed {
    let mut parsed = Parsed::default();
    let mut records: Vec<Vec<(String, String)>> = Vec::new();
    for line in text.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        let is_tag = line.len() >= 2
            && line.as_bytes()[..2].iter().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            && (line.len() == 2 || line.as_bytes()[2] == b' ');
        if !is_tag {
            if let Some(last) = records.last_mut().and_then(|r| r.last_mut()) {
                last.1.push(' ');
                last.1.push_str(line.trim());
            }
            continue;
        }
        let tag = line[..2].to_string();
        let value = line.get(3..).unwrap_or_default().trim().to_string();
        if tag == "RT" {
            records.push(Vec::new());
        }
        if let Some(r) = records.last_mut() {
            r.push((tag, value));
        }
    }
    for (n, r) in records.iter().enumerate() {
        match to_paper(r) {
            Some(p) => parsed.papers.push(p),
            None => parsed.errors.push(format!("Record {}: no title", n + 1)),
        }
    }
    parsed
}

fn entry_type(rt: &str) -> &'static str {
    let rt = rt.to_ascii_lowercase();
    if rt.starts_with("journal") || rt.starts_with("magazine") || rt.starts_with("newspaper") {
        "article"
    } else if rt.starts_with("conference") {
        "inproceedings"
    } else if rt.starts_with("book, section") || rt.starts_with("book section") {
        "incollection"
    } else if rt.starts_with("book") {
        "book"
    } else if rt.starts_with("dissertation") || rt.starts_with("thesis") {
        "phdthesis"
    } else if rt.starts_with("report") {
        "techreport"
    } else if rt.starts_with("web page") {
        "online"
    } else {
        "misc"
    }
}

fn to_paper(fields: &[(String, String)]) -> Option<NewPaper> {
    let first = |tags: &[&str]| {
        tags.iter()
            .find_map(|t| fields.iter().find(|(k, v)| k == t && !v.is_empty()).map(|(_, v)| v.clone()))
            .unwrap_or_default()
    };
    let all = |tag: &str| -> Vec<String> {
        fields.iter().filter(|(k, v)| k == tag && !v.is_empty()).map(|(_, v)| v.clone()).collect()
    };
    let title = first(&["T1", "T2"]);
    if title.is_empty() {
        return None;
    }
    let ty = entry_type(&first(&["RT"]));
    let container = if ty == "article" { first(&["JF", "JO", "T2"]) } else { first(&["T2", "JF", "JO"]) };
    let start = first(&["SP"]);
    let end = first(&["OP"]);
    let pages = if end.is_empty() || start.contains('-') { start } else { format!("{start}-{end}") };
    let url = first(&["UL", "LK"]);
    let date = first(&["YR", "FD"]);
    let bib = Biblio {
        entry_type: ty.to_string(),
        title,
        authors: all("A1").iter().map(|n| names::parse_plain(n)).collect(),
        year: parse_year(&date),
        month: first(&["FD"]).split_whitespace().next().and_then(parse_month),
        container,
        volume: first(&["VO"]),
        issue: first(&["IS"]),
        pages: normalise_pages(&pages),
        publisher: first(&["PB"]),
        doi: doi::normalise(&first(&["DO"])).or_else(|| doi::normalise(&url)),
        url: if url.contains("doi.org/") { String::new() } else { url },
        isbn: if matches!(ty, "book" | "incollection") { first(&["SN"]) } else { String::new() },
        abstract_: first(&["AB"]),
        keywords: all("K1").join("; "),
        ..Default::default()
    };
    Some(NewPaper { bib, source_key: Some(first(&["ID"])).filter(|k| !k.is_empty()), pdf_path: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records() {
        let text = "RT Journal Article
SR Electronic(1)
A1 Zhang, Wei
A1 Li, Na
T1 Multifunctional Metasurface: Simultaneous Beam Steering
JF IEEE Transactions on Antennas and Propagation
YR 2024
FD Mar 2024
VO 72
IS 4
SP 3321
OP 3330
DO 10.1109/TAP.2024.3371697
K1 metasurface
K1 varactor

RT Conference Proceedings
A1 Oliveri, G.
T1 Skins
T2 Proc. AP-S
YR 2022
";
        let p = parse(text);
        assert!(p.errors.is_empty());
        assert_eq!(p.papers.len(), 2);
        let a = &p.papers[0].bib;
        assert_eq!(a.entry_type, "article");
        assert_eq!(a.authors.len(), 2);
        assert_eq!(a.month, Some(3));
        assert_eq!(a.pages, "3321-3330");
        assert_eq!(a.doi.as_deref(), Some("10.1109/tap.2024.3371697"));
        assert_eq!(a.keywords, "metasurface; varactor");
        let b = &p.papers[1].bib;
        assert_eq!(b.entry_type, "inproceedings");
        assert_eq!(b.container, "Proc. AP-S");
    }
}
