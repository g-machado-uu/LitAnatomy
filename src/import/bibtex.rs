//! A forgiving BibTeX / BibLaTeX reader: one bad entry is reported and
//! skipped, the rest still import. Handles `@string` macros, `#`
//! concatenation, month macros, `@comment` and `@preamble`.

use std::collections::HashMap;

use super::{Parsed, doi, names, normalise_pages, parse_month, parse_year};
use crate::latex;
use crate::model::{Biblio, NewPaper};

/// An entry as written in the file: field values are raw LaTeX.
#[derive(Debug, Default, PartialEq)]
pub struct RawEntry {
    pub entry_type: String,
    pub key: String,
    /// Field names in lower case, in file order.
    pub fields: Vec<(String, String)>,
}

pub fn parse(text: &str) -> Parsed {
    let (entries, errors) = parse_raw(text);
    Parsed { papers: entries.iter().map(to_new_paper).collect(), errors }
}

pub fn parse_raw(text: &str) -> (Vec<RawEntry>, Vec<String>) {
    let mut p = Parser { s: text.as_bytes(), text, i: 0, macros: HashMap::new() };
    let mut entries = Vec::new();
    let mut errors = Vec::new();
    while p.seek_at() {
        let at = p.i;
        match p.item() {
            Ok(Some(e)) => entries.push(e),
            Ok(None) => {}
            Err(msg) => {
                let line = text[..at].matches('\n').count() + 1;
                errors.push(format!("Line {line}: {msg}"));
                // Carry on from the next '@' at the start of a line.
                p.i = at + 1;
                while p.i < p.s.len() && !(p.s[p.i] == b'@' && (p.i == 0 || p.s[p.i - 1] == b'\n')) {
                    p.i += 1;
                }
            }
        }
    }
    (entries, errors)
}

struct Parser<'a> {
    s: &'a [u8],
    text: &'a str,
    i: usize,
    macros: HashMap<String, String>,
}

impl Parser<'_> {
    fn seek_at(&mut self) -> bool {
        while self.i < self.s.len() && self.s[self.i] != b'@' {
            self.i += 1;
        }
        self.i < self.s.len()
    }

    fn ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn ident(&mut self) -> String {
        let start = self.i;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            if c.is_ascii_alphanumeric() || b"_-:./+'!?*&|<>;$^`~[]".contains(&c) || c >= 0x80 {
                self.i += 1;
            } else {
                break;
            }
        }
        self.text[start..self.i].to_string()
    }

    fn item(&mut self) -> Result<Option<RawEntry>, String> {
        self.i += 1; // '@'
        self.ws();
        let kind = self.ident().to_ascii_lowercase();
        if kind.is_empty() {
            return Err("an '@' is not followed by an entry type".into());
        }
        self.ws();
        let close = match self.s.get(self.i) {
            Some(b'{') => b'}',
            Some(b'(') => b')',
            _ if kind == "comment" => return Ok(None), // "@comment rest of line"
            _ => return Err(format!("@{kind} is not followed by '{{' or '('")),
        };
        self.i += 1;
        match kind.as_str() {
            "comment" | "preamble" => {
                self.i -= 1;
                self.balanced_skip()?;
                Ok(None)
            }
            "string" => {
                self.ws();
                let name = self.ident().to_ascii_lowercase();
                self.ws();
                self.expect(b'=')?;
                let value = self.value()?;
                self.ws();
                self.expect(close)?;
                self.macros.insert(name, value);
                Ok(None)
            }
            _ => self.entry(kind, close).map(Some),
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        if self.s.get(self.i) == Some(&c) {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected '{}'", c as char))
        }
    }

    fn balanced_skip(&mut self) -> Result<(), String> {
        let open = self.s[self.i];
        let close = if open == b'(' { b')' } else { b'}' };
        let mut depth = 0;
        while self.i < self.s.len() {
            let c = self.s[self.i];
            self.i += 1;
            if c == open {
                depth += 1;
            } else if c == close {
                depth -= 1;
                if depth == 0 {
                    return Ok(());
                }
            }
        }
        Err("unbalanced braces".into())
    }

    fn entry(&mut self, entry_type: String, close: u8) -> Result<RawEntry, String> {
        self.ws();
        let start = self.i;
        while self.i < self.s.len() && !matches!(self.s[self.i], b',' | b'}' | b')') && !self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
        let key = self.text[start..self.i].to_string();
        self.ws();
        let mut entry = RawEntry { entry_type, key, fields: Vec::new() };
        loop {
            self.ws();
            match self.s.get(self.i) {
                Some(b',') => {
                    self.i += 1;
                    continue;
                }
                Some(&c) if c == close => {
                    self.i += 1;
                    return Ok(entry);
                }
                None => return Err(format!("entry '{}' is not closed", entry.key)),
                _ => {}
            }
            let name = self.ident().to_ascii_lowercase();
            if name.is_empty() {
                return Err(format!("unexpected character in entry '{}'", entry.key));
            }
            self.ws();
            self.expect(b'=').map_err(|_| format!("field '{name}' in '{}' has no '='", entry.key))?;
            let value = self.value().map_err(|e| format!("{e} in field '{name}' of '{}'", entry.key))?;
            entry.fields.push((name, value));
        }
    }

    /// A value: pieces joined by '#'. Braced and quoted pieces are kept raw
    /// (without the outer delimiters); macros are expanded.
    fn value(&mut self) -> Result<String, String> {
        let mut out = String::new();
        loop {
            self.ws();
            match self.s.get(self.i) {
                Some(b'{') => {
                    let start = self.i + 1;
                    self.balanced_skip()?;
                    out.push_str(&self.text[start..self.i - 1]);
                }
                Some(b'"') => {
                    self.i += 1;
                    let start = self.i;
                    let mut depth = 0;
                    loop {
                        match self.s.get(self.i) {
                            None => return Err("unclosed quote".into()),
                            Some(b'{') => depth += 1,
                            Some(b'}') => depth -= 1,
                            Some(b'"') if depth == 0 && self.s[self.i - 1] != b'\\' => break,
                            _ => {}
                        }
                        self.i += 1;
                    }
                    out.push_str(&self.text[start..self.i]);
                    self.i += 1;
                }
                Some(c) if c.is_ascii_digit() => {
                    let start = self.i;
                    while self.i < self.s.len() && self.s[self.i].is_ascii_digit() {
                        self.i += 1;
                    }
                    out.push_str(&self.text[start..self.i]);
                }
                Some(c) if c.is_ascii_alphabetic() => {
                    let name = self.ident().to_ascii_lowercase();
                    match self.macros.get(&name) {
                        Some(v) => out.push_str(v),
                        None => match parse_month(&name) {
                            Some(m) if name.len() == 3 => out.push_str(&m.to_string()),
                            _ => out.push_str(&name),
                        },
                    }
                }
                _ => return Err("missing value".into()),
            }
            self.ws();
            if self.s.get(self.i) == Some(&b'#') {
                self.i += 1;
            } else {
                return Ok(out);
            }
        }
    }
}

/// Fields that become part of [`Biblio`]; the rest go to `extra`.
const MODELLED: &[&str] = &[
    "title", "author", "year", "date", "month", "journal", "journaltitle", "booktitle", "volume", "number",
    "issue", "pages", "publisher", "doi", "url", "isbn", "eprint", "eprinttype", "archiveprefix", "abstract",
    "keywords", "file", "fjournal",
];

pub fn to_new_paper(e: &RawEntry) -> NewPaper {
    let get = |name: &str| e.fields.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str());
    let text = |name: &str| get(name).map(latex::to_unicode).unwrap_or_default();
    let mut bib = Biblio {
        entry_type: e.entry_type.clone(),
        title: text("title"),
        authors: get("author").map(names::parse_bibtex_list).unwrap_or_default(),
        container: [text("journaltitle"), text("journal"), text("booktitle")]
            .into_iter()
            .find(|s| !s.is_empty())
            .unwrap_or_default(),
        volume: text("volume"),
        issue: if get("number").is_some() { text("number") } else { text("issue") },
        pages: get("pages").map(normalise_pages).unwrap_or_default(),
        publisher: text("publisher"),
        url: get("url").map(|u| u.trim().replace("\\_", "_").replace("\\%", "%").replace("\\&", "&")).unwrap_or_default(),
        isbn: text("isbn"),
        abstract_: text("abstract"),
        keywords: text("keywords"),
        ..Default::default()
    };
    bib.year = get("year").and_then(parse_year).or_else(|| get("date").and_then(parse_year));
    bib.month = get("month").and_then(parse_month).or_else(|| {
        // BibLaTeX date "2025-03-01"
        get("date").and_then(|d| d.split('-').nth(1)).and_then(parse_month)
    });
    bib.doi = get("doi").and_then(doi::normalise).or_else(|| get("url").and_then(doi::normalise));
    let is_arxiv = |n| get(n).is_some_and(|v| v.eq_ignore_ascii_case("arxiv"));
    if is_arxiv("archiveprefix") || is_arxiv("eprinttype") {
        bib.arxiv_id = text("eprint");
    }
    for (name, value) in &e.fields {
        let keep_eprint = name == "eprint" && bib.arxiv_id.is_empty();
        if !MODELLED.contains(&name.as_str()) || keep_eprint {
            bib.extra.entry(name.clone()).or_insert_with(|| value.clone());
        }
    }
    bib.extra.retain(|_, v| !v.trim().is_empty());
    let source_key = (!e.key.is_empty()).then(|| e.key.clone());
    NewPaper { bib, source_key, pdf_path: get("file").and_then(file_field_path) }
}

/// The first PDF in a JabRef or Zotero `file` field:
/// `:papers/x.pdf:PDF` or `Full Text PDF:/home/me/x.pdf:application/pdf`.
fn file_field_path(v: &str) -> Option<String> {
    for item in v.split(';') {
        // Windows paths are written "C\:\\dir\\x.pdf".
        let unescaped = item.replace("\\:", "\u{0}").replace("\\\\", "\\");
        let parts: Vec<&str> = unescaped.split(':').collect();
        let path = match parts.as_slice() {
            [_, path, ..] => path,
            [path] => path,
            [] => continue,
        };
        let path = path.replace('\u{0}', ":");
        if path.to_ascii_lowercase().ends_with(".pdf") {
            return Some(path);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Author;

    const IEEE: &str = r#"
@ARTICLE{10234847,
  author={Zhang, Wei and Li, Na and Oliveri, Giacomo},
  journal={IEEE Transactions on Antennas and Propagation},
  title={A 2-Bit Wideband 5G mm-Wave {RIS} With Low Sidelobe Levels and No Quantization Lobe},
  year={2025},
  volume={73},
  number={9},
  pages={6123-6135},
  keywords={Reconfigurable intelligent surfaces;PIN photodiodes},
  doi={10.1109/TAP.2025.3603900}}

@INPROCEEDINGS{9886689,
  author={Oliveri, G. and Massa, A.},
  booktitle={2022 IEEE International Symposium on Antennas and Propagation (AP-S/URSI)},
  title={Synthesis of Modular Electromagnetic Skins for Multiple Blind Spot Coverage in Urban Scenarios},
  year={2022},
  pages={1--2},
  doi={10.1109/AP-S/USNC-URSI47032.2022.9886689}}
"#;

    #[test]
    fn ieee_export() {
        let p = parse(IEEE);
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        assert_eq!(p.papers.len(), 2);
        let a = &p.papers[0];
        assert_eq!(a.source_key.as_deref(), Some("10234847"));
        assert_eq!(a.bib.entry_type, "article");
        assert_eq!(a.bib.title, "A 2-Bit Wideband 5G mm-Wave RIS With Low Sidelobe Levels and No Quantization Lobe");
        assert_eq!(a.bib.authors[0], Author::new("Zhang", "Wei"));
        assert_eq!(a.bib.authors.len(), 3);
        assert_eq!(a.bib.container, "IEEE Transactions on Antennas and Propagation");
        assert_eq!(a.bib.year, Some(2025));
        assert_eq!(a.bib.volume, "73");
        assert_eq!(a.bib.issue, "9");
        assert_eq!(a.bib.pages, "6123-6135");
        assert_eq!(a.bib.doi.as_deref(), Some("10.1109/tap.2025.3603900"));
        assert_eq!(a.bib.keywords, "Reconfigurable intelligent surfaces;PIN photodiodes");
        let b = &p.papers[1];
        assert_eq!(b.bib.entry_type, "inproceedings");
        assert_eq!(b.bib.pages, "1-2");
        assert_eq!(b.bib.container, "2022 IEEE International Symposium on Antennas and Propagation (AP-S/URSI)");
    }

    #[test]
    fn strings_concatenation_months_and_quotes() {
        let text = r#"
@string{tap = "IEEE Trans. Antennas Propag."}
@comment{ignored @article{nope, title={x}} }
@preamble{"\newcommand{\noop}[1]{}"}
@article(smith2020,
  title = "Quoted {T}itle with {\"o}",
  journal = tap # " (Special issue)",
  month = mar,
  year = 2020,
  note = {Kept {as} raw},
)
"#;
        let p = parse(text);
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        assert_eq!(p.papers.len(), 1);
        let b = &p.papers[0].bib;
        assert_eq!(b.title, "Quoted Title with ö");
        assert_eq!(b.container, "IEEE Trans. Antennas Propag. (Special issue)");
        assert_eq!(b.month, Some(3));
        assert_eq!(b.year, Some(2020));
        assert_eq!(b.extra.get("note").map(String::as_str), Some("Kept {as} raw"));
    }

    #[test]
    fn biblatex_fields() {
        let text = r#"@online{wu2021,
  author = {Wu, Tao},
  title = {Preprint},
  date = {2021-06-15},
  journaltitle = {arXiv},
  eprint = {2106.01234},
  eprinttype = {arxiv},
  file = {Full Text PDF:/home/me/papers/wu.pdf:application/pdf},
}"#;
        let p = parse(text);
        let n = &p.papers[0];
        assert_eq!(n.bib.year, Some(2021));
        assert_eq!(n.bib.month, Some(6));
        assert_eq!(n.bib.arxiv_id, "2106.01234");
        assert_eq!(n.pdf_path.as_deref(), Some("/home/me/papers/wu.pdf"));
        assert!(n.bib.extra.is_empty(), "{:?}", n.bib.extra);
    }

    #[test]
    fn bad_entry_is_skipped() {
        let text = "@article{bad, title = }\n@article{good, title={Fine}}\n";
        let p = parse(text);
        assert_eq!(p.papers.len(), 1);
        assert_eq!(p.papers[0].bib.title, "Fine");
        assert_eq!(p.errors.len(), 1);
        assert!(p.errors[0].starts_with("Line 1"), "{}", p.errors[0]);
    }

    #[test]
    fn jabref_and_windows_file_fields() {
        assert_eq!(file_field_path(":papers/x.pdf:PDF").as_deref(), Some("papers/x.pdf"));
        assert_eq!(file_field_path(r":C\:\\Papers\\x.pdf:PDF").as_deref(), Some(r"C:\Papers\x.pdf"));
        assert_eq!(file_field_path("Snapshot:/x.html:text/html"), None);
    }
}
