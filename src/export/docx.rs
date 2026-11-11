//! The dissection as a Word document (.docx): a heading per paper, its
//! details, then a heading per part. Written by hand: a .docx is a zip of a
//! few XML files.

use std::io::{Cursor, Write};

use anyhow::Result;
use zip::write::SimpleFileOptions;

use super::blocks::{self, Block, Span};
use super::dissection::{Body, Dissection, PaperNotes, plural, today};

const MUTED: &str = "5F6675";

/// Builds document.xml's body and collects the hyperlinks it needs.
struct Doc {
    body: String,
    links: Vec<String>,
}

impl Doc {
    fn para(&mut self, style: &str, props: &str, runs: &str) {
        self.body.push_str(&format!("<w:p><w:pPr><w:pStyle w:val=\"{style}\"/>{props}</w:pPr>{runs}</w:p>"));
    }

    /// A link to `url` showing `text`.
    fn link(&mut self, url: &str, text: &str) -> String {
        self.links.push(url.to_string());
        let id = format!("rIdLink{}", self.links.len());
        format!("<w:hyperlink r:id=\"{id}\"><w:r><w:rPr><w:rStyle w:val=\"Hyperlink\"/></w:rPr>{}</w:r></w:hyperlink>", text_xml(text))
    }

    fn notes(&mut self, text: &str) {
        for block in blocks::parse(text) {
            match block {
                Block::Paragraph(spans) => self.para("Normal", "", &runs(&spans)),
                Block::Heading(spans) => self.para("Normal", "<w:keepNext/>", &runs(&bold(spans))),
                Block::Quote(spans) => self.para("Quote", "", &runs(&spans)),
                Block::Code(code) => self.para("Code", "", &run(&code, false, false, true)),
                Block::Item { level, marker, spans } => {
                    let left = 360 * (level + 1);
                    let props = format!("<w:ind w:left=\"{left}\" w:hanging=\"300\"/>");
                    let marker = format!("{}<w:r><w:tab/></w:r>", run(&marker, false, false, false));
                    self.para("ListItem", &props, &format!("{marker}{}", runs(&spans)));
                }
            }
        }
    }

    fn paper(&mut self, p: &PaperNotes, d: &Dissection, first: bool) {
        let props = if d.options.page_per_paper && !first { "<w:pageBreakBefore/>" } else { "" };
        self.para("Heading1", props, &run(p.paper.bib.title.trim(), false, false, false));
        let authors = p.authors();
        if !authors.is_empty() {
            self.para("Authors", "", &run(&authors, false, false, false));
        }
        let venue = p.venue_line();
        if !venue.is_empty() {
            self.para("Meta", "", &run(&venue, false, true, false));
        }
        let mut meta = run(&p.paper.citekey, false, false, true);
        if let (Some(url), Some(doi)) = (p.doi_url(), p.paper.bib.doi.as_deref()) {
            meta.push_str(&run("  ·  DOI ", false, false, false));
            meta.push_str(&self.link(&url, doi));
        }
        meta.push_str(&run(&format!("  ·  {}", p.status_line()), false, false, false));
        self.para("Meta", "<w:spacing w:after=\"200\"/>", &meta);
        for s in &p.sections {
            if s.body.is_empty() && !d.options.empty {
                continue;
            }
            self.para("Heading2", "", &run(&s.label, false, false, false));
            match &s.body {
                b if b.is_empty() => self.para("Normal", "", &coloured("—", MUTED)),
                Body::Text(t) => self.notes(t),
                Body::Metrics(ms) => {
                    let mut rows = String::new();
                    for (i, m) in ms.iter().enumerate() {
                        let shade = if i % 2 == 0 { "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"F2F4F8\"/>" } else { "" };
                        let cell = |width: u32, content: &str| {
                            format!(
                                "<w:tc><w:tcPr><w:tcW w:w=\"{width}\" w:type=\"pct\"/>{shade}</w:tcPr><w:p><w:pPr><w:pStyle w:val=\"TableText\"/></w:pPr>{content}</w:p></w:tc>"
                            )
                        };
                        rows.push_str(&format!(
                            "<w:tr><w:trPr><w:cantSplit/></w:trPr>{}{}</w:tr>",
                            cell(2000, &run(&m.name, true, false, false)),
                            cell(3000, &run(&m.value_with_unit(), false, false, false))
                        ));
                    }
                    self.body.push_str(&format!(
                        "<w:tbl><w:tblPr><w:tblStyle w:val=\"Metrics\"/><w:tblW w:w=\"5000\" w:type=\"pct\"/><w:tblLook w:val=\"0000\"/></w:tblPr>\
                         <w:tblGrid><w:gridCol w:w=\"3800\"/><w:gridCol w:w=\"5700\"/></w:tblGrid>{rows}</w:tbl>"
                    ));
                    // Word needs a paragraph between a table and what follows.
                    self.para("Normal", "<w:spacing w:after=\"0\"/>", "");
                }
                Body::Quotes(qs) => {
                    for q in qs {
                        let mut r = run(&format!("“{}”", q.text.trim()), false, false, false);
                        if !q.page.trim().is_empty() {
                            r.push_str(&coloured(&format!("  p. {}", q.page.trim()), MUTED));
                        }
                        self.para("Quote", "", &r);
                    }
                }
            }
        }
        if d.options.abstracts && !p.paper.bib.abstract_.trim().is_empty() {
            self.para("Heading2", "", &run("Abstract", false, false, false));
            self.para("Normal", "", &coloured(p.paper.bib.abstract_.trim(), MUTED));
        }
    }
}

pub fn render(d: &Dissection) -> Result<Vec<u8>> {
    let mut doc = Doc { body: String::new(), links: Vec::new() };
    doc.para("Title", "", &run(&d.title, false, false, false));
    doc.para("Subtitle", "", &run(&format!("Literature dissection · {} · {}", plural(d.papers.len()), today()), false, false, false));
    if d.papers.len() > 1 {
        for (i, p) in d.papers.iter().enumerate() {
            let runs = format!(
                "{}{}",
                run(&format!("{}.  {}  ", i + 1, p.short_label()), true, false, false),
                run(p.paper.bib.title.trim(), false, false, false)
            );
            doc.para("Contents", "", &runs);
        }
        doc.para("Normal", "", "");
    }
    for (i, p) in d.papers.iter().enumerate() {
        doc.paper(p, d, i == 0 && d.papers.len() <= 1);
    }
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
         xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><w:body>{}\
         <w:sectPr><w:footerReference w:type=\"default\" r:id=\"rIdFooter\"/>\
         <w:pgSz w:w=\"11906\" w:h=\"16838\"/>\
         <w:pgMar w:top=\"1247\" w:right=\"1247\" w:bottom=\"1247\" w:left=\"1247\" w:header=\"567\" w:footer=\"567\" w:gutter=\"0\"/>\
         </w:sectPr></w:body></w:document>",
        doc.body
    );
    let mut rels = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rIdStyles\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles\" Target=\"styles.xml\"/>\
         <Relationship Id=\"rIdFooter\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/footer\" Target=\"footer1.xml\"/>",
    );
    for (i, url) in doc.links.iter().enumerate() {
        rels.push_str(&format!(
            "<Relationship Id=\"rIdLink{}\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/hyperlink\" Target=\"{}\" TargetMode=\"External\"/>",
            i + 1,
            esc(url)
        ));
    }
    rels.push_str("</Relationships>");
    let core = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" \
         xmlns:dc=\"http://purl.org/dc/elements/1.1/\" xmlns:dcterms=\"http://purl.org/dc/terms/\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
         <dc:title>{}</dc:title><dc:creator>LitAnatomy</dc:creator>\
         <dcterms:created xsi:type=\"dcterms:W3CDTF\">{}</dcterms:created></cp:coreProperties>",
        esc(&d.title),
        chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ")
    );

    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let files: [(&str, &str); 7] = [
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", ROOT_RELS),
        ("docProps/core.xml", &core),
        ("word/document.xml", &document),
        ("word/_rels/document.xml.rels", &rels),
        ("word/styles.xml", STYLES),
        ("word/footer1.xml", FOOTER),
    ];
    for (name, content) in files {
        zip.start_file(name, opts)?;
        zip.write_all(content.as_bytes())?;
    }
    Ok(zip.finish()?.into_inner())
}

fn bold(spans: Vec<Span>) -> Vec<Span> {
    spans.into_iter().map(|s| Span { bold: true, ..s }).collect()
}

fn runs(spans: &[Span]) -> String {
    spans.iter().map(|s| run(&s.text, s.bold, s.italic, s.code)).collect()
}

fn run(text: &str, bold: bool, italic: bool, code: bool) -> String {
    let mut props = String::new();
    if code {
        props.push_str("<w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\" w:cs=\"Consolas\"/>");
    }
    if bold {
        props.push_str("<w:b/>");
    }
    if italic {
        props.push_str("<w:i/>");
    }
    let props = if props.is_empty() { String::new() } else { format!("<w:rPr>{props}</w:rPr>") };
    format!("<w:r>{props}{}</w:r>", text_xml(text))
}

fn coloured(text: &str, colour: &str) -> String {
    format!("<w:r><w:rPr><w:color w:val=\"{colour}\"/></w:rPr>{}</w:r>", text_xml(text))
}

/// Text for inside a run: line breaks become `<w:br/>`.
fn text_xml(text: &str) -> String {
    text.split('\n').map(|line| format!("<w:t xml:space=\"preserve\">{}</w:t>", esc(line))).collect::<Vec<_>>().join("<w:br/>")
}

/// XML-escapes `s`, leaving out characters XML 1.0 does not allow.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' | '\n' | '\r' => out.push(c),
            c if (c as u32) < 0x20 || c == '\u{FFFE}' || c == '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out
}

const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
<Default Extension=\"xml\" ContentType=\"application/xml\"/>\
<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>\
<Override PartName=\"/word/footer1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml\"/>\
<Override PartName=\"/docProps/core.xml\" ContentType=\"application/vnd.openxmlformats-package.core-properties+xml\"/>\
</Types>";

const ROOT_RELS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"word/document.xml\"/>\
<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties\" Target=\"docProps/core.xml\"/>\
</Relationships>";

const FOOTER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<w:ftr xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
<w:p><w:pPr><w:pStyle w:val=\"Footer\"/><w:jc w:val=\"center\"/></w:pPr>\
<w:r><w:fldChar w:fldCharType=\"begin\"/></w:r><w:r><w:instrText xml:space=\"preserve\"> PAGE </w:instrText></w:r>\
<w:r><w:fldChar w:fldCharType=\"separate\"/></w:r><w:r><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType=\"end\"/></w:r></w:p>\
</w:ftr>";

/// The styles: Calibri text, Georgia headings, Utrecht blue accents.
const STYLES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
<w:styles xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii=\"Calibri\" w:hAnsi=\"Calibri\" w:eastAsia=\"Calibri\" w:cs=\"Calibri\"/>\
<w:color w:val=\"1B1F2A\"/><w:sz w:val=\"21\"/><w:szCs w:val=\"21\"/><w:lang w:val=\"en-GB\"/></w:rPr></w:rPrDefault>\
<w:pPrDefault><w:pPr><w:spacing w:after=\"120\" w:line=\"288\" w:lineRule=\"auto\"/></w:pPr></w:pPrDefault></w:docDefaults>\
<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/><w:qFormat/></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Title\"><w:name w:val=\"Title\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/>\
<w:pPr><w:spacing w:after=\"60\"/></w:pPr><w:rPr><w:rFonts w:ascii=\"Georgia\" w:hAnsi=\"Georgia\" w:cs=\"Georgia\"/><w:color w:val=\"001240\"/><w:sz w:val=\"56\"/><w:szCs w:val=\"56\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Subtitle\"><w:name w:val=\"Subtitle\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/>\
<w:pPr><w:spacing w:after=\"360\"/></w:pPr><w:rPr><w:color w:val=\"5F6675\"/><w:sz w:val=\"22\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Contents\"><w:name w:val=\"Contents\"/><w:basedOn w:val=\"Normal\"/>\
<w:pPr><w:spacing w:after=\"60\"/><w:ind w:left=\"360\" w:hanging=\"360\"/></w:pPr><w:rPr><w:sz w:val=\"20\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Heading1\"><w:name w:val=\"heading 1\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/>\
<w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before=\"480\" w:after=\"80\" w:line=\"264\" w:lineRule=\"auto\"/><w:outlineLvl w:val=\"0\"/></w:pPr>\
<w:rPr><w:rFonts w:ascii=\"Georgia\" w:hAnsi=\"Georgia\" w:cs=\"Georgia\"/><w:color w:val=\"001240\"/><w:sz w:val=\"34\"/><w:szCs w:val=\"34\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Heading2\"><w:name w:val=\"heading 2\"/><w:basedOn w:val=\"Normal\"/><w:next w:val=\"Normal\"/><w:qFormat/>\
<w:pPr><w:keepNext/><w:keepLines/><w:spacing w:before=\"240\" w:after=\"60\"/><w:outlineLvl w:val=\"1\"/></w:pPr>\
<w:rPr><w:b/><w:color w:val=\"001240\"/><w:sz w:val=\"23\"/><w:szCs w:val=\"23\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Authors\"><w:name w:val=\"Authors\"/><w:basedOn w:val=\"Normal\"/>\
<w:pPr><w:spacing w:after=\"40\"/></w:pPr><w:rPr><w:sz w:val=\"22\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Meta\"><w:name w:val=\"Meta\"/><w:basedOn w:val=\"Normal\"/>\
<w:pPr><w:spacing w:after=\"40\"/></w:pPr><w:rPr><w:color w:val=\"5F6675\"/><w:sz w:val=\"19\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Quote\"><w:name w:val=\"Quote\"/><w:basedOn w:val=\"Normal\"/><w:qFormat/>\
<w:pPr><w:pBdr><w:left w:val=\"single\" w:sz=\"18\" w:space=\"8\" w:color=\"001240\"/></w:pBdr><w:ind w:left=\"240\"/></w:pPr><w:rPr><w:i/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"ListItem\"><w:name w:val=\"List Item\"/><w:basedOn w:val=\"Normal\"/>\
<w:pPr><w:spacing w:after=\"40\"/></w:pPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Code\"><w:name w:val=\"Code\"/><w:basedOn w:val=\"Normal\"/>\
<w:pPr><w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"F2F4F8\"/></w:pPr><w:rPr><w:rFonts w:ascii=\"Consolas\" w:hAnsi=\"Consolas\" w:cs=\"Consolas\"/><w:sz w:val=\"19\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"TableText\"><w:name w:val=\"Table Text\"/><w:basedOn w:val=\"Normal\"/>\
<w:pPr><w:spacing w:before=\"40\" w:after=\"40\"/></w:pPr><w:rPr><w:sz w:val=\"20\"/></w:rPr></w:style>\
<w:style w:type=\"paragraph\" w:styleId=\"Footer\"><w:name w:val=\"footer\"/><w:basedOn w:val=\"Normal\"/><w:rPr><w:color w:val=\"5F6675\"/><w:sz w:val=\"18\"/></w:rPr></w:style>\
<w:style w:type=\"character\" w:styleId=\"Hyperlink\"><w:name w:val=\"Hyperlink\"/><w:rPr><w:color w:val=\"1F4FA8\"/><w:u w:val=\"single\"/></w:rPr></w:style>\
<w:style w:type=\"table\" w:styleId=\"Metrics\"><w:name w:val=\"Metrics\"/><w:tblPr><w:tblBorders>\
<w:top w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"D9DCE3\"/><w:bottom w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"D9DCE3\"/>\
<w:insideH w:val=\"single\" w:sz=\"4\" w:space=\"0\" w:color=\"D9DCE3\"/></w:tblBorders>\
<w:tblCellMar><w:left w:w=\"100\" w:type=\"dxa\"/><w:right w:w=\"100\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr></w:style>\
</w:styles>";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::dissection::{Options, tests::sample};
    use std::io::Read;

    #[test]
    fn word_document() {
        let (lib, ids, viva) = sample();
        let d = Dissection::load(&lib, &ids, Some(viva), "Viva & co", Options::default()).unwrap();
        let bytes = render(&d).unwrap();
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut doc = String::new();
        zip.by_name("word/document.xml").unwrap().read_to_string(&mut doc).unwrap();
        assert!(doc.contains("Viva &amp; co"));
        assert!(doc.contains("<w:b/></w:rPr><w:t xml:space=\"preserve\">PIN diodes</w:t>"));
        assert!(doc.contains("Key Metrics") && doc.contains("30.8 dB"));
        assert!(doc.contains("<w:pageBreakBefore/>"));
        assert!(!doc.contains("Methodology"), "parts not written are left out");
        let mut rels = String::new();
        zip.by_name("word/_rels/document.xml.rels").unwrap().read_to_string(&mut rels).unwrap();
        assert!(rels.contains("https://doi.org/10.1109/tap.2025.1"));
        // Every part is well-formed enough to have matching root tags.
        for name in ["word/styles.xml", "[Content_Types].xml", "word/footer1.xml"] {
            let mut s = String::new();
            zip.by_name(name).unwrap().read_to_string(&mut s).unwrap();
            assert!(s.starts_with("<?xml"));
        }
    }
}
