//! Looking papers up online by identifier: DOIs (doi.org, Crossref), arXiv,
//! ISBNs (Open Library, then Google Books) and PubMed (NCBI), and searching
//! Crossref by title for PDFs that carry no identifier.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;

use super::ident::Identifier;
use super::{csljson, doi, names, normalise_pages, parse_month, parse_year};
use crate::latex::collapse_ws;
use crate::model::{Author, Biblio, NewPaper};

/// The paper an identifier points to.
pub fn lookup(agent: &ureq::Agent, id: &Identifier) -> Result<NewPaper> {
    match id {
        Identifier::Doi(d) => doi::fetch(agent, d),
        Identifier::Arxiv(a) => arxiv(agent, a),
        Identifier::Isbn(i) => isbn(agent, i),
        Identifier::Pmid(p) => pubmed(agent, p),
    }
}

fn get_text(agent: &ureq::Agent, url: &str) -> Result<String> {
    Ok(agent.get(url).call()?.body_mut().read_to_string()?)
}

// ----- arXiv -----------------------------------------------------------------

/// An arXiv preprint. If it has been published, the journal's record (by its
/// DOI) is used, keeping the arXiv ID and abstract.
fn arxiv(agent: &ureq::Agent, id: &str) -> Result<NewPaper> {
    let xml = get_text(agent, &format!("https://export.arxiv.org/api/query?id_list={id}&max_results=1"))
        .with_context(|| format!("Could not reach arXiv for {id}"))?;
    let preprint = parse_arxiv(&xml).ok_or_else(|| anyhow!("arXiv has no paper {id}."))?;
    // arXiv's own DOIs (10.48550/…) are the preprint, not a published version.
    if let Some(d) = preprint.bib.doi.clone().filter(|d| !d.starts_with("10.48550/"))
        && let Ok(mut published) = doi::fetch(agent, &d)
    {
        published.bib.arxiv_id = preprint.bib.arxiv_id.clone();
        published.bib.fill_from(&preprint.bib);
        return Ok(published);
    }
    Ok(preprint)
}

/// The first entry of an arXiv API (Atom) response.
pub fn parse_arxiv(xml: &str) -> Option<NewPaper> {
    let entry = element(xml, "entry")?;
    let id_url = element(entry, "id").unwrap_or_default();
    if id_url.contains("api/errors") {
        return None;
    }
    let title = collapse_ws(&unescape_xml(element(entry, "title")?));
    if title.is_empty() {
        return None;
    }
    let id = id_url.rsplit_once("/abs/").map(|(_, id)| id).unwrap_or_default();
    let id = super::ident::arxiv_id(id).unwrap_or_else(|| id.to_string());
    let authors = elements(entry, "author")
        .into_iter()
        .filter_map(|a| element(a, "name"))
        .map(|n| names::parse_plain(&collapse_ws(&unescape_xml(n))))
        .collect();
    let published = element(entry, "published").unwrap_or_default();
    let doi = element(entry, "arxiv:doi").and_then(doi::normalise);
    let mut bib = Biblio {
        entry_type: "misc".into(),
        title,
        authors,
        year: parse_year(published),
        month: published.get(5..7).and_then(parse_month),
        doi,
        url: format!("https://arxiv.org/abs/{id}"),
        arxiv_id: id,
        abstract_: collapse_ws(&unescape_xml(element(entry, "summary").unwrap_or_default())),
        ..Default::default()
    };
    if let Some(class) = attribute(entry, "arxiv:primary_category", "term") {
        bib.extra.insert("primaryclass".into(), class);
    }
    Some(NewPaper { bib, source_key: None, pdf_path: None })
}

/// The text inside the first `<name …>…</name>`.
fn element<'a>(xml: &'a str, name: &str) -> Option<&'a str> {
    elements(xml, name).into_iter().next()
}

fn elements<'a>(xml: &'a str, name: &str) -> Vec<&'a str> {
    let open = format!("<{name}");
    let close = format!("</{name}>");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(&open) {
        let after = &rest[start + open.len()..];
        // "<name>" or "<name attr…>", not "<namespace…>".
        if !after.starts_with('>') && !after.starts_with(' ') && !after.starts_with('/') {
            rest = after;
            continue;
        }
        let Some(gt) = after.find('>') else { break };
        if after[..gt].ends_with('/') {
            rest = &after[gt + 1..];
            continue;
        }
        let body = &after[gt + 1..];
        let Some(end) = body.find(&close) else { break };
        out.push(body[..end].trim());
        rest = &body[end + close.len()..];
    }
    out
}

/// The value of `attr` on the first `<name …>` tag.
fn attribute(xml: &str, name: &str, attr: &str) -> Option<String> {
    let start = xml.find(&format!("<{name} "))?;
    let tag = &xml[start..start + xml[start..].find('>')?];
    let key = format!("{attr}=\"");
    let i = tag.find(&key)? + key.len();
    Some(unescape_xml(&tag[i..i + tag[i..].find('"')?]))
}

fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

// ----- ISBN ------------------------------------------------------------------

fn isbn(agent: &ureq::Agent, isbn: &str) -> Result<NewPaper> {
    let open_library = get_text(agent, &format!("https://openlibrary.org/api/books?bibkeys=ISBN:{isbn}&format=json&jscmd=data"))
        .ok()
        .and_then(|body| serde_json::from_str::<Value>(&body).ok())
        .and_then(|v| parse_open_library(&v, isbn));
    if let Some(p) = open_library {
        return Ok(p);
    }
    let body = get_text(agent, &format!("https://www.googleapis.com/books/v1/volumes?q=isbn:{isbn}"))
        .with_context(|| format!("Could not look up ISBN {isbn}"))?;
    let v: Value = serde_json::from_str(&body).context("Google Books did not return JSON")?;
    parse_google_books(&v, isbn).ok_or_else(|| anyhow!("No book found with ISBN {isbn}."))
}

fn str_of<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn with_subtitle(title: &str, subtitle: &str) -> String {
    let title = collapse_ws(title);
    if subtitle.trim().is_empty() { title } else { format!("{title}: {}", collapse_ws(subtitle)) }
}

pub fn parse_open_library(v: &Value, isbn: &str) -> Option<NewPaper> {
    let book = v.get(format!("ISBN:{isbn}"))?;
    let title = with_subtitle(str_of(book, "title"), str_of(book, "subtitle"));
    if title.is_empty() {
        return None;
    }
    let names = |key: &str| -> Vec<String> {
        book.get(key)
            .and_then(Value::as_array)
            .map(|a| a.iter().map(|x| str_of(x, "name").to_string()).filter(|n| !n.is_empty()).collect())
            .unwrap_or_default()
    };
    let bib = Biblio {
        entry_type: "book".into(),
        title,
        authors: names("authors").iter().map(|n| names::parse_plain(n)).collect(),
        year: parse_year(str_of(book, "publish_date")),
        publisher: names("publishers").join("; "),
        isbn: isbn.to_string(),
        ..Default::default()
    };
    Some(NewPaper { bib, source_key: None, pdf_path: None })
}

pub fn parse_google_books(v: &Value, isbn: &str) -> Option<NewPaper> {
    let info = v.get("items")?.get(0)?.get("volumeInfo")?;
    let title = with_subtitle(str_of(info, "title"), str_of(info, "subtitle"));
    if title.is_empty() {
        return None;
    }
    let authors = info
        .get("authors")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(names::parse_plain).collect())
        .unwrap_or_default();
    let bib = Biblio {
        entry_type: "book".into(),
        title,
        authors,
        year: parse_year(str_of(info, "publishedDate")),
        publisher: str_of(info, "publisher").to_string(),
        isbn: isbn.to_string(),
        abstract_: collapse_ws(str_of(info, "description")),
        ..Default::default()
    };
    Some(NewPaper { bib, source_key: None, pdf_path: None })
}

// ----- PubMed ----------------------------------------------------------------

/// A PubMed record from NCBI's E-utilities. With a DOI, the publisher's
/// record is used, filled in from PubMed.
fn pubmed(agent: &ureq::Agent, pmid: &str) -> Result<NewPaper> {
    let body = get_text(
        agent,
        &format!("https://eutils.ncbi.nlm.nih.gov/entrez/eutils/esummary.fcgi?db=pubmed&retmode=json&id={pmid}"),
    )
    .with_context(|| format!("Could not reach PubMed for {pmid}"))?;
    let v: Value = serde_json::from_str(&body).context("PubMed did not return JSON")?;
    let record = parse_pubmed(&v, pmid).ok_or_else(|| anyhow!("PubMed has no paper {pmid}."))?;
    if let Some(d) = record.bib.doi.clone()
        && let Ok(mut published) = doi::fetch(agent, &d)
    {
        published.bib.fill_from(&record.bib);
        return Ok(published);
    }
    Ok(record)
}

pub fn parse_pubmed(v: &Value, pmid: &str) -> Option<NewPaper> {
    let r = v.get("result")?.get(pmid)?;
    if r.get("error").is_some() {
        return None;
    }
    let title = collapse_ws(str_of(r, "title").trim_end_matches('.'));
    if title.is_empty() {
        return None;
    }
    let authors = r
        .get("authors")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter(|x| str_of(x, "authtype").is_empty() || str_of(x, "authtype") == "Author")
                .map(|x| medline_name(str_of(x, "name")))
                .filter(|a| !a.family.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let doi = r
        .get("articleids")
        .and_then(Value::as_array)
        .and_then(|ids| ids.iter().find(|i| str_of(i, "idtype") == "doi"))
        .and_then(|i| doi::normalise(str_of(i, "value")));
    let date = str_of(r, "pubdate");
    let container = [str_of(r, "fulljournalname"), str_of(r, "source")].into_iter().find(|s| !s.is_empty()).unwrap_or_default();
    let mut bib = Biblio {
        entry_type: "article".into(),
        title,
        authors,
        year: parse_year(date),
        month: date.split_whitespace().nth(1).and_then(parse_month),
        container: collapse_ws(container),
        volume: str_of(r, "volume").to_string(),
        issue: str_of(r, "issue").to_string(),
        pages: normalise_pages(str_of(r, "pages")),
        doi,
        ..Default::default()
    };
    bib.extra.insert("pmid".into(), pmid.to_string());
    Some(NewPaper { bib, source_key: None, pdf_path: None })
}

/// MEDLINE names: "Smith JA" → Smith, J. A.; "van der Berg K" → van der Berg, K.
fn medline_name(name: &str) -> Author {
    let words: Vec<&str> = name.split_whitespace().collect();
    match words.split_last() {
        Some((initials, family)) if !family.is_empty() && initials.len() <= 3 && initials.chars().all(|c| c.is_ascii_uppercase()) => {
            let given = initials.chars().map(|c| format!("{c}.")).collect::<Vec<_>>().join(" ");
            Author::new(family.join(" "), given)
        }
        _ => Author::new(name.trim(), ""),
    }
}

// ----- Crossref search -------------------------------------------------------

/// Crossref's best matches for a title or citation text.
pub fn search_crossref(agent: &ureq::Agent, query: &str) -> Result<Vec<NewPaper>> {
    let query: String = query.chars().take(400).collect();
    if query.trim().is_empty() {
        bail!("Nothing to search for.");
    }
    let body = agent
        .get("https://api.crossref.org/works")
        .query("query.bibliographic", &query)
        .query("rows", "5")
        .call()
        .context("Could not search Crossref")?
        .body_mut()
        .read_to_string()?;
    let v: Value = serde_json::from_str(&body).context("Crossref did not return JSON")?;
    Ok(v["message"]["items"].as_array().map(|items| items.iter().filter_map(csljson::item_to_paper).collect()).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arxiv_atom() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <id>http://arxiv.org/api/x</id>
  <title type="html">ArXiv Query</title>
  <entry>
    <id>http://arxiv.org/abs/1706.03762v7</id>
    <published>2017-06-12T17:57:34Z</published>
    <title>Attention Is All
      You Need</title>
    <summary>  The dominant sequence transduction models &amp; more.
    </summary>
    <author><name>Ashish Vaswani</name></author>
    <author><name>Noam Shazeer</name><arxiv:affiliation>Google</arxiv:affiliation></author>
    <arxiv:doi xmlns:arxiv="http://arxiv.org/schemas/atom">10.48550/arXiv.1706.03762</arxiv:doi>
    <arxiv:primary_category xmlns:arxiv="http://arxiv.org/schemas/atom" term="cs.CL" scheme="http://arxiv.org/schemas/atom"/>
  </entry>
</feed>"#;
        let p = parse_arxiv(xml).unwrap();
        assert_eq!(p.bib.title, "Attention Is All You Need");
        assert_eq!(p.bib.arxiv_id, "1706.03762");
        assert_eq!(p.bib.authors, vec![Author::new("Vaswani", "Ashish"), Author::new("Shazeer", "Noam")]);
        assert_eq!((p.bib.year, p.bib.month), (Some(2017), Some(6)));
        assert_eq!(p.bib.abstract_, "The dominant sequence transduction models & more.");
        assert_eq!(p.bib.extra.get("primaryclass").map(String::as_str), Some("cs.CL"));
        assert_eq!(p.bib.doi.as_deref(), Some("10.48550/arxiv.1706.03762"));
        let error = r#"<feed><entry><id>http://arxiv.org/api/errors#incorrect_id_format</id><title>Error</title></entry></feed>"#;
        assert!(parse_arxiv(error).is_none());
        assert!(parse_arxiv("<feed></feed>").is_none());
    }

    #[test]
    fn books() {
        let ol: Value = serde_json::from_str(
            r#"{"ISBN:9780470631553": {"title": "Microwave Engineering", "subtitle": "",
                "authors": [{"name": "David M. Pozar"}], "publishers": [{"name": "Wiley"}], "publish_date": "2012"}}"#,
        )
        .unwrap();
        let p = parse_open_library(&ol, "9780470631553").unwrap();
        assert_eq!(p.bib.title, "Microwave Engineering");
        assert_eq!(p.bib.authors, vec![Author::new("Pozar", "David M.")]);
        assert_eq!((p.bib.year, p.bib.publisher.as_str(), p.bib.entry_type.as_str()), (Some(2012), "Wiley", "book"));
        assert!(parse_open_library(&serde_json::json!({}), "9780470631553").is_none());
        let gb: Value = serde_json::from_str(
            r#"{"items": [{"volumeInfo": {"title": "Antenna Theory", "subtitle": "Analysis and Design",
                "authors": ["Constantine A. Balanis"], "publisher": "Wiley", "publishedDate": "2016-02-01"}}]}"#,
        )
        .unwrap();
        let p = parse_google_books(&gb, "9781118642061").unwrap();
        assert_eq!(p.bib.title, "Antenna Theory: Analysis and Design");
        assert_eq!(p.bib.year, Some(2016));
        assert!(parse_google_books(&serde_json::json!({"totalItems": 0}), "1").is_none());
    }

    #[test]
    fn pubmed_summary() {
        let v: Value = serde_json::from_str(
            r#"{"result": {"uids": ["31452104"], "31452104": {
                "title": "Deep learning for antenna design.", "pubdate": "2019 Aug 26",
                "source": "Nat Commun", "fulljournalname": "Nature communications",
                "volume": "10", "issue": "1", "pages": "3841-3850",
                "authors": [{"name": "Smith JA", "authtype": "Author"}, {"name": "van der Berg K", "authtype": "Author"},
                            {"name": "Antenna Group", "authtype": "CollectiveName"}],
                "articleids": [{"idtype": "pubmed", "value": "31452104"}, {"idtype": "doi", "value": "10.1038/s41467-019-11111-1"}]}}}"#,
        )
        .unwrap();
        let p = parse_pubmed(&v, "31452104").unwrap();
        assert_eq!(p.bib.title, "Deep learning for antenna design");
        assert_eq!(p.bib.authors, vec![Author::new("Smith", "J. A."), Author::new("van der Berg", "K.")]);
        assert_eq!((p.bib.year, p.bib.month), (Some(2019), Some(8)));
        assert_eq!(p.bib.container, "Nature communications");
        assert_eq!(p.bib.pages, "3841-3850");
        assert_eq!(p.bib.doi.as_deref(), Some("10.1038/s41467-019-11111-1"));
        assert_eq!(p.bib.extra.get("pmid").map(String::as_str), Some("31452104"));
        let missing: Value = serde_json::json!({"result": {"1": {"uid": "1", "error": "cannot get document summary"}}});
        assert!(parse_pubmed(&missing, "1").is_none());
    }

    /// Needs the network: `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn live_lookups() {
        let agent = crate::import::http_agent("");
        let p = lookup(&agent, &Identifier::Arxiv("1706.03762".into())).expect("arXiv");
        assert!(p.bib.title.contains("Attention"), "{}", p.bib.title);
        let p = lookup(&agent, &Identifier::Isbn("9780470631553".into())).expect("ISBN");
        assert!(p.bib.title.to_lowercase().contains("microwave"), "{}", p.bib.title);
        let p = lookup(&agent, &Identifier::Pmid("31452104".into())).expect("PubMed");
        assert!(!p.bib.title.is_empty());
        let hits = search_crossref(&agent, "A universal metasurface antenna to manipulate all fundamental characteristics of electromagnetic waves").expect("search");
        assert!(hits.iter().any(|h| h.bib.doi.as_deref() == Some("10.1038/s41467-023-40717-9")));
    }
}
