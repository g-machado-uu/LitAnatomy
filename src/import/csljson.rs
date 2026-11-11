//! CSL-JSON (Zotero, Mendeley, doi.org) and Crossref's very similar format.

use serde_json::Value;

use super::{Parsed, doi, normalise_pages, parse_year};
use crate::latex::collapse_ws;
use crate::model::{Author, Biblio, NewPaper};

pub fn parse(text: &str) -> Parsed {
    let value: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => return Parsed { papers: Vec::new(), errors: vec![format!("Not valid JSON: {e}")] },
    };
    let items: Vec<&Value> = match &value {
        Value::Array(items) => items.iter().collect(),
        Value::Object(o) => match o.get("items").or_else(|| o.get("message").and_then(|m| m.get("items"))) {
            Some(Value::Array(items)) => items.iter().collect(),
            _ => vec![&value],
        },
        _ => Vec::new(),
    };
    let mut parsed = Parsed::default();
    for (n, item) in items.into_iter().enumerate() {
        match item_to_paper(item) {
            Some(p) => parsed.papers.push(p),
            None => parsed.errors.push(format!("Item {}: no title", n + 1)),
        }
    }
    parsed
}

/// A string, or the first string of an array (Crossref uses arrays).
fn text(v: &Value, key: &str) -> String {
    let s = match v.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a.iter().find_map(Value::as_str).unwrap_or_default().to_string(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    };
    collapse_ws(&strip_tags(&s))
}

/// Crossref abstracts and titles may contain JATS/HTML tags.
fn strip_tags(s: &str) -> String {
    if !s.contains('<') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"")
}

fn date_parts(v: &Value) -> (Option<i32>, Option<u8>) {
    for key in ["issued", "published-print", "published-online", "published", "created"] {
        let Some(d) = v.get(key) else { continue };
        if let Some(parts) = d.get("date-parts").and_then(|p| p.get(0)).and_then(Value::as_array) {
            let num = |i: usize| {
                parts.get(i).and_then(|x| x.as_i64().or_else(|| x.as_str().and_then(|s| s.parse().ok())))
            };
            if let Some(y) = num(0) {
                return (Some(y as i32), num(1).and_then(|m| u8::try_from(m).ok()).filter(|m| (1..=12).contains(m)));
            }
        }
        if let Some(raw) = d.get("raw").or_else(|| d.get("literal")).and_then(Value::as_str)
            && let Some(y) = parse_year(raw)
        {
            return (Some(y), None);
        }
    }
    (None, None)
}

fn entry_type(csl: &str) -> &'static str {
    match csl {
        "article-journal" | "journal-article" | "article-magazine" | "article-newspaper" | "review" => "article",
        "paper-conference" | "proceedings-article" => "inproceedings",
        "book" | "monograph" | "edited-book" | "reference-book" => "book",
        "chapter" | "book-chapter" | "book-section" | "entry-encyclopedia" | "entry-dictionary" => "incollection",
        "thesis" | "dissertation" => "phdthesis",
        "report" => "techreport",
        "webpage" | "post-weblog" | "post" => "online",
        "patent" => "patent",
        "dataset" => "dataset",
        _ => "misc",
    }
}

pub fn item_to_paper(v: &Value) -> Option<NewPaper> {
    let title = text(v, "title");
    if title.is_empty() {
        return None;
    }
    let authors = v
        .get("author")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|p| {
                    let family = text(p, "family");
                    let given = text(p, "given");
                    let literal = text(p, "literal");
                    let name = text(p, "name");
                    if !family.is_empty() {
                        Some(Author::new(family, given))
                    } else if !literal.is_empty() || !name.is_empty() {
                        Some(Author::new(if literal.is_empty() { name } else { literal }, ""))
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    let (year, month) = date_parts(v);
    let mut container = text(v, "container-title");
    if container.is_empty() {
        // Crossref puts proceedings titles under "event" sometimes.
        container = v.get("event").map(|e| text(e, "name")).unwrap_or_default();
    }
    let keywords = match v.get("keyword").or_else(|| v.get("subject")) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(a)) => a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join("; "),
        _ => String::new(),
    };
    let doi_text = text(v, "DOI");
    let url = text(v, "URL");
    let bib = Biblio {
        entry_type: entry_type(&text(v, "type")).to_string(),
        title,
        authors,
        year,
        month,
        container,
        volume: text(v, "volume"),
        issue: text(v, "issue"),
        pages: normalise_pages(&text(v, "page")),
        publisher: text(v, "publisher"),
        doi: doi::normalise(&doi_text),
        url: if url.contains("doi.org/") { String::new() } else { url },
        isbn: text(v, "ISBN"),
        abstract_: text(v, "abstract"),
        keywords,
        ..Default::default()
    };
    let source_key = match v.get("citation-key").or_else(|| v.get("id")) {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        _ => None,
    };
    Some(NewPaper { bib, source_key, pdf_path: None })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doi_org_csl() {
        let json = r#"{
            "type": "article-journal",
            "id": "https://doi.org/10.1038/s41467-023-40717-9",
            "title": "A universal metasurface antenna to manipulate all fundamental characteristics of electromagnetic waves",
            "author": [{"family": "Wang", "given": "Si Ran"}, {"family": "Cui", "given": "Tie Jun"}, {"literal": "Metamaterials Lab"}],
            "container-title": "Nature Communications",
            "volume": "14", "issue": "1", "page": "4783",
            "publisher": "Springer Science and Business Media LLC",
            "issued": {"date-parts": [[2023, 8, 8]]},
            "DOI": "10.1038/s41467-023-40717-9",
            "URL": "https://doi.org/10.1038/s41467-023-40717-9",
            "abstract": "<jats:p>Metasurfaces &amp; more.</jats:p>"
        }"#;
        let p = parse(json);
        assert!(p.errors.is_empty());
        let b = &p.papers[0].bib;
        assert_eq!(b.entry_type, "article");
        assert_eq!(b.authors.len(), 3);
        assert_eq!(b.authors[0], Author::new("Wang", "Si Ran"));
        assert_eq!(b.authors[2], Author::new("Metamaterials Lab", ""));
        assert_eq!(b.year, Some(2023));
        assert_eq!(b.month, Some(8));
        assert_eq!(b.container, "Nature Communications");
        assert_eq!(b.pages, "4783");
        assert_eq!(b.doi.as_deref(), Some("10.1038/s41467-023-40717-9"));
        assert_eq!(b.url, "");
        assert_eq!(b.abstract_, "Metasurfaces & more.");
    }

    #[test]
    fn crossref_message_with_arrays() {
        let json = r#"{"message": {
            "type": "proceedings-article",
            "title": ["Synthesis of Modular\n Electromagnetic Skins"],
            "container-title": ["2022 IEEE AP-S"],
            "author": [{"given": "G.", "family": "Oliveri"}],
            "published-print": {"date-parts": [[2022, 7]]},
            "page": "1-2",
            "DOI": "10.1109/AP-S/USNC-URSI47032.2022.9886689",
            "subject": ["Antennas", "Skins"]
        }}"#;
        let v: Value = serde_json::from_str(json).unwrap();
        let p = item_to_paper(&v["message"]).unwrap();
        assert_eq!(p.bib.entry_type, "inproceedings");
        assert_eq!(p.bib.title, "Synthesis of Modular Electromagnetic Skins");
        assert_eq!(p.bib.container, "2022 IEEE AP-S");
        assert_eq!(p.bib.year, Some(2022));
        assert_eq!(p.bib.keywords, "Antennas; Skins");
    }

    #[test]
    fn zotero_export_array() {
        let json = r#"[{"id": "zhang2025Wideband", "type": "paper-conference", "title": "X", "issued": {"date-parts": [["2025"]]}},
                       {"id": "notitle"}]"#;
        let p = parse(json);
        assert_eq!(p.papers.len(), 1);
        assert_eq!(p.papers[0].source_key.as_deref(), Some("zhang2025Wideband"));
        assert_eq!(p.papers[0].bib.year, Some(2025));
        assert_eq!(p.errors.len(), 1);
    }
}
