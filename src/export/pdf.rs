//! The dissection as a PDF, laid out here and written by hand: A4 pages in
//! the app's own fonts (Inter and DM Serif Display, embedded as subsets), a
//! first page listing the papers, then each paper with its parts. Bookmarks
//! lead to each paper, and DOIs are links.

use std::collections::BTreeMap;
use std::io::Write;

use anyhow::{Result, bail};
use flate2::Compression;
use flate2::write::ZlibEncoder;
use ttf_parser::Face;

use super::blocks::{self, Block, Span};
use super::dissection::{Body, Dissection, PaperNotes, plural, today};

const REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const BOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
const SERIF: &[u8] = include_bytes!("../../assets/fonts/DMSerifDisplay-Regular.ttf");

// A4 in points, and the margins.
const PAGE_W: f32 = 595.28;
const PAGE_H: f32 = 841.89;
const LEFT: f32 = 64.0;
const RIGHT: f32 = 64.0;
const TOP: f32 = 72.0;
const BOTTOM: f32 = 74.0;
const WIDTH: f32 = PAGE_W - LEFT - RIGHT;

type Rgb = [f32; 3];
const INK: Rgb = [0.106, 0.122, 0.165];
const MUTED: Rgb = [0.373, 0.400, 0.459];
/// Utrecht University dark blue.
const ACCENT: Rgb = [0.0, 0.071, 0.251];
const LINK: Rgb = [0.122, 0.310, 0.659];
const RULE: Rgb = [0.851, 0.863, 0.890];
const RAISED: Rgb = [0.949, 0.957, 0.973];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
enum Font {
    Regular = 0,
    Bold = 1,
    Serif = 2,
}

struct FontData {
    face: Face<'static>,
    data: &'static [u8],
    name: &'static str,
}

impl FontData {
    fn load(data: &'static [u8], name: &'static str) -> Result<Self> {
        match Face::parse(data, 0) {
            Ok(face) => Ok(Self { face, data, name }),
            Err(e) => bail!("The {name} font can't be read: {e}"),
        }
    }

    fn glyph(&self, c: char) -> Option<u16> {
        self.face.glyph_index(c).map(|g| g.0)
    }

    /// Advance of glyph `g` in thousandths of the font size.
    fn advance(&self, g: u16) -> f32 {
        let units = self.face.glyph_hor_advance(ttf_parser::GlyphId(g)).unwrap_or(0) as f32;
        units * 1000.0 / self.face.units_per_em() as f32
    }
}

struct Fonts([FontData; 3]);

impl Fonts {
    fn load() -> Result<Self> {
        Ok(Self([
            FontData::load(REGULAR, "Inter-Regular")?,
            FontData::load(BOLD, "Inter-SemiBold")?,
            FontData::load(SERIF, "DMSerifDisplay-Regular")?,
        ]))
    }

    fn get(&self, f: Font) -> &FontData {
        &self.0[f as usize]
    }

    /// The font that draws `c`: `want` if it can, else Inter.
    fn pick(&self, want: Font, c: char) -> Font {
        if want == Font::Regular || self.get(want).glyph(c).is_some() {
            return want;
        }
        // DM Serif lacks Greek and many symbols; SemiBold is Inter's weight.
        if want == Font::Serif && self.get(Font::Regular).glyph(c).is_some() { Font::Regular } else { want }
    }

    fn width(&self, f: Font, text: &str, size: f32) -> f32 {
        let font = self.get(f);
        text.chars().map(|c| font.advance(font.glyph(c).unwrap_or(0))).sum::<f32>() * size / 1000.0
    }
}

#[derive(Clone, Debug)]
enum Op {
    /// `y` is the baseline, measured down from the top of the page.
    Text { font: Font, size: f32, x: f32, y: f32, colour: Rgb, italic: bool, text: String },
    /// `y` is the top edge.
    Rect { x: f32, y: f32, w: f32, h: f32, colour: Rgb },
    Line { x1: f32, y1: f32, x2: f32, y2: f32, width: f32, colour: Rgb },
    Link { x: f32, y: f32, w: f32, h: f32, uri: String },
}

/// A run of text in one font, measured.
#[derive(Clone, Debug)]
struct Piece {
    font: Font,
    italic: bool,
    text: String,
    width: f32,
}

#[derive(Clone, Debug, Default)]
struct Word {
    pieces: Vec<Piece>,
    width: f32,
    space_before: bool,
    break_before: bool,
}

/// One laid-out line: pieces at x offsets from the line's start.
type Line = Vec<(f32, Piece)>;

#[derive(Clone, Copy)]
struct Style {
    size: f32,
    leading: f32,
    colour: Rgb,
    font: Font,
}

impl Style {
    const fn new(font: Font, size: f32, leading: f32, colour: Rgb) -> Self {
        Self { font, size, leading, colour }
    }
}

const BODY: Style = Style::new(Font::Regular, 10.0, 14.6, INK);
const SMALL: Style = Style::new(Font::Regular, 8.8, 12.4, MUTED);

struct Layout<'f> {
    fonts: &'f Fonts,
    pages: Vec<Vec<Op>>,
    y: f32,
    /// (title, page, y) of each paper, for bookmarks.
    outline: Vec<(String, usize, f32)>,
}

impl<'f> Layout<'f> {
    fn new(fonts: &'f Fonts) -> Self {
        Self { fonts, pages: vec![Vec::new()], y: TOP, outline: Vec::new() }
    }

    fn page(&mut self) -> &mut Vec<Op> {
        self.pages.last_mut().expect("a page")
    }

    fn new_page(&mut self) {
        self.pages.push(Vec::new());
        self.y = TOP;
    }

    /// Starts a new page unless `h` more points fit on this one.
    fn ensure(&mut self, h: f32) {
        if self.y + h > PAGE_H - BOTTOM && self.y > TOP {
            self.new_page();
        }
    }

    fn space(&mut self, h: f32) {
        self.y += h;
    }

    fn rule(&mut self, colour: Rgb) {
        let y = self.y;
        self.page().push(Op::Line { x1: LEFT, y1: y, x2: LEFT + WIDTH, y2: y, width: 0.6, colour });
    }

    /// Splits styled spans into measured words.
    fn words(&self, spans: &[(Font, bool, &str)], size: f32) -> Vec<Word> {
        let mut words = Vec::new();
        let mut word = Word::default();
        let (mut space, mut brk) = (false, false);
        for &(want, italic, text) in spans {
            for c in text.chars() {
                if c == '\n' || c.is_whitespace() {
                    if !word.pieces.is_empty() {
                        words.push(std::mem::take(&mut word));
                        space = false;
                        brk = false;
                    }
                    if c == '\n' {
                        brk = true;
                    } else {
                        space = true;
                    }
                    continue;
                }
                if word.pieces.is_empty() {
                    word.space_before = space;
                    word.break_before = brk;
                }
                let font = self.fonts.pick(want, c);
                let w = self.fonts.width(font, c.encode_utf8(&mut [0; 4]), size);
                match word.pieces.last_mut() {
                    Some(p) if p.font == font && p.italic == italic => {
                        p.text.push(c);
                        p.width += w;
                    }
                    _ => word.pieces.push(Piece { font, italic, text: c.to_string(), width: w }),
                }
                word.width += w;
            }
        }
        if !word.pieces.is_empty() {
            words.push(word);
        }
        words
    }

    /// Breaks spans into lines no wider than `max`.
    fn wrap(&self, spans: &[(Font, bool, &str)], size: f32, max: f32) -> Vec<Line> {
        let space = self.fonts.width(Font::Regular, " ", size);
        let mut lines: Vec<Line> = Vec::new();
        let mut line: Line = Vec::new();
        let mut x = 0.0f32;
        let place = |line: &mut Line, x: &mut f32, word: &Word| {
            for p in &word.pieces {
                line.push((*x, p.clone()));
                *x += p.width;
            }
        };
        for mut word in self.words(spans, size) {
            if word.break_before && !line.is_empty() {
                lines.push(std::mem::take(&mut line));
                x = 0.0;
            }
            let gap = if line.is_empty() || !word.space_before { 0.0 } else { space };
            if !line.is_empty() && x + gap + word.width > max {
                lines.push(std::mem::take(&mut line));
                x = 0.0;
            } else {
                x += gap;
            }
            // A word longer than the line (a URL) is cut where it must be.
            while word.width > max {
                let (head, rest) = self.cut(&word, max - x, size);
                place(&mut line, &mut x, &head);
                lines.push(std::mem::take(&mut line));
                x = 0.0;
                word = rest;
            }
            place(&mut line, &mut x, &word);
        }
        if !line.is_empty() {
            lines.push(line);
        }
        lines
    }

    /// The start of `word` that fits in `room` (at least one character), and the rest.
    fn cut(&self, word: &Word, room: f32, size: f32) -> (Word, Word) {
        fn push(into: &mut Word, p: &Piece, c: char, w: f32) {
            match into.pieces.last_mut() {
                Some(last) if last.font == p.font && last.italic == p.italic => {
                    last.text.push(c);
                    last.width += w;
                }
                _ => into.pieces.push(Piece { font: p.font, italic: p.italic, text: c.to_string(), width: w }),
            }
            into.width += w;
        }
        let mut head = Word::default();
        let mut rest = Word::default();
        for p in &word.pieces {
            for c in p.text.chars() {
                let w = self.fonts.width(p.font, c.encode_utf8(&mut [0; 4]), size);
                let fits = rest.pieces.is_empty() && (head.width + w <= room || head.pieces.is_empty());
                push(if fits { &mut head } else { &mut rest }, p, c, w);
            }
        }
        (head, rest)
    }

    /// Draws `lines` at `x`, from the current position, page by page.
    /// `bar` draws a quote's bar beside each line.
    fn flow(&mut self, lines: &[Line], x: f32, style: Style, bar: bool) {
        for line in lines {
            self.ensure(style.leading);
            let base = self.y + style.leading * 0.5 + style.size * 0.36;
            if bar {
                let y = self.y;
                self.page().push(Op::Rect { x: LEFT, y, w: 2.2, h: style.leading, colour: ACCENT });
            }
            for (dx, p) in line {
                let op = Op::Text {
                    font: p.font,
                    size: style.size,
                    x: x + dx,
                    y: base,
                    colour: style.colour,
                    italic: p.italic,
                    text: p.text.clone(),
                };
                self.page().push(op);
            }
            self.y += style.leading;
        }
    }

    fn text(&mut self, text: &str, style: Style, x: f32, width: f32) {
        let lines = self.wrap(&[(style.font, false, text)], style.size, width);
        self.flow(&lines, x, style, false);
    }

    /// A note's Markdown.
    fn notes(&mut self, text: &str) {
        for block in blocks::parse(text) {
            match block {
                Block::Paragraph(spans) => {
                    let lines = self.wrap(&styled(&spans, false), BODY.size, WIDTH);
                    self.flow(&lines, LEFT, BODY, false);
                }
                Block::Heading(spans) => {
                    self.ensure(BODY.leading * 2.0);
                    let lines = self.wrap(&styled(&spans, true), BODY.size, WIDTH);
                    self.flow(&lines, LEFT, BODY, false);
                }
                Block::Item { level, marker, spans } => {
                    let indent = 14.0 * (level as f32 + 1.0);
                    let lines = self.wrap(&styled(&spans, false), BODY.size, WIDTH - indent);
                    self.ensure(BODY.leading);
                    let base = self.y + BODY.leading * 0.5 + BODY.size * 0.36;
                    let mw = self.fonts.width(Font::Regular, &marker, BODY.size);
                    let op = Op::Text {
                        font: Font::Regular,
                        size: BODY.size,
                        x: LEFT + indent - mw - 5.0,
                        y: base,
                        colour: ACCENT,
                        italic: false,
                        text: marker,
                    };
                    self.page().push(op);
                    self.flow(&lines, LEFT + indent, BODY, false);
                    self.y -= 2.0;
                }
                Block::Quote(spans) => {
                    let lines = self.wrap(&styled(&spans, false), BODY.size, WIDTH - 12.0);
                    self.flow(&lines, LEFT + 12.0, Style { colour: MUTED, ..BODY }, true);
                }
                Block::Code(code) => {
                    let lines = self.wrap(&[(Font::Regular, false, &code)], 9.0, WIDTH - 12.0);
                    self.flow(&lines, LEFT + 12.0, Style::new(Font::Regular, 9.0, 13.0, MUTED), false);
                }
            }
            self.y += 5.0;
        }
    }

    fn metrics(&mut self, metrics: &[crate::model::Metric]) {
        let name_w = WIDTH * 0.36;
        let pad = 6.0;
        let style = Style::new(Font::Regular, 9.6, 13.4, INK);
        for (i, m) in metrics.iter().enumerate() {
            let name = self.wrap(&[(Font::Bold, false, m.name.trim())], style.size, name_w - 2.0 * pad);
            let value = self.wrap(&[(Font::Regular, false, m.value_with_unit().trim())], style.size, WIDTH - name_w - 2.0 * pad);
            let h = name.len().max(value.len()).max(1) as f32 * style.leading + 2.0 * 4.0;
            self.ensure(h);
            let top = self.y;
            if i % 2 == 0 {
                self.page().push(Op::Rect { x: LEFT, y: top, w: WIDTH, h, colour: RAISED });
            }
            self.y = top + 4.0;
            self.flow(&name, LEFT + pad, style, false);
            self.y = top + 4.0;
            self.flow(&value, LEFT + name_w + pad, style, false);
            self.y = top + h;
        }
        self.rule(RULE);
        self.y += 6.0;
    }

    fn quotes(&mut self, quotes: &[crate::model::Quote]) {
        let style = Style { colour: INK, ..BODY };
        for q in quotes {
            let text = format!("“{}”", q.text.trim());
            let page = if q.page.trim().is_empty() { String::new() } else { format!("  p. {}", q.page.trim()) };
            let lines = self.wrap(&[(Font::Regular, true, &text), (Font::Bold, false, &page)], style.size, WIDTH - 12.0);
            self.flow(&lines, LEFT + 12.0, style, true);
            self.y += 6.0;
        }
    }

    fn section_label(&mut self, label: &str) {
        // Keep the label with the first lines of what follows.
        self.ensure(18.0 + BODY.leading * 2.0);
        self.y += 8.0;
        self.text(label, Style::new(Font::Bold, 10.5, 15.0, ACCENT), LEFT, WIDTH);
        self.y += 1.0;
    }

    fn paper(&mut self, p: &PaperNotes, d: &Dissection, n: usize) {
        if d.options.page_per_paper {
            if !self.page().is_empty() {
                self.new_page();
            }
        } else if self.y > TOP {
            self.ensure(140.0);
            if self.y > TOP {
                self.space(14.0);
                self.rule(RULE);
                self.space(20.0);
            }
        }
        self.outline.push((p.paper.bib.title.trim().to_string(), self.pages.len() - 1, self.y));
        let label = format!("{n}  ·  {}  ·  {}", p.short_label().to_uppercase(), p.paper.citekey);
        self.text(&label, Style::new(Font::Bold, 8.4, 12.0, ACCENT), LEFT, WIDTH);
        self.space(4.0);
        self.text(p.paper.bib.title.trim(), Style::new(Font::Serif, 19.0, 23.5, INK), LEFT, WIDTH);
        self.space(4.0);
        let authors = p.authors();
        if !authors.is_empty() {
            self.text(&authors, Style::new(Font::Regular, 10.5, 14.5, INK), LEFT, WIDTH);
        }
        let venue = p.venue_line();
        if !venue.is_empty() {
            let lines = self.wrap(&[(Font::Regular, true, &venue)], SMALL.size + 0.6, WIDTH);
            self.flow(&lines, LEFT, Style { size: SMALL.size + 0.6, ..SMALL }, false);
        }
        if let (Some(url), Some(doi)) = (p.doi_url(), p.paper.bib.doi.as_deref()) {
            let text = format!("doi.org/{doi}");
            let style = Style { colour: LINK, ..SMALL };
            let lines = self.wrap(&[(Font::Regular, false, &text)], style.size, WIDTH);
            self.ensure(style.leading * lines.len() as f32);
            let (top, w) = (self.y, lines.iter().map(|l| l.last().map(|(x, p)| x + p.width).unwrap_or(0.0)).fold(0.0, f32::max));
            self.flow(&lines, LEFT, style, false);
            let h = self.y - top;
            self.page().push(Op::Link { x: LEFT, y: top, w, h, uri: url });
        }
        self.text(&p.status_line(), SMALL, LEFT, WIDTH);
        self.space(8.0);
        self.rule(RULE);
        self.space(4.0);
        for s in &p.sections {
            if s.body.is_empty() && !d.options.empty {
                continue;
            }
            self.section_label(&s.label);
            match &s.body {
                b if b.is_empty() => self.text("—", Style { colour: MUTED, ..BODY }, LEFT, WIDTH),
                Body::Text(t) => self.notes(t),
                Body::Metrics(ms) => self.metrics(ms),
                Body::Quotes(qs) => self.quotes(qs),
            }
        }
        if d.options.abstracts && !p.paper.bib.abstract_.trim().is_empty() {
            self.section_label("Abstract");
            self.text(p.paper.bib.abstract_.trim(), Style { colour: MUTED, ..BODY }, LEFT, WIDTH);
        }
    }

    /// The first page: the title and a list of the papers, with the pages
    /// they start on (once known).
    fn cover(&mut self, d: &Dissection, starts: Option<&[usize]>) {
        self.text("LITERATURE DISSECTION", Style::new(Font::Bold, 8.6, 12.0, ACCENT), LEFT, WIDTH);
        self.space(6.0);
        self.text(&d.title, Style::new(Font::Serif, 30.0, 35.0, ACCENT), LEFT, WIDTH);
        self.space(6.0);
        self.text(&format!("{} · {} · LitAnatomy", plural(d.papers.len()), today()), Style { size: 10.0, leading: 14.0, ..SMALL }, LEFT, WIDTH);
        self.space(14.0);
        self.rule(RULE);
        self.space(14.0);
        if d.papers.len() < 2 {
            return;
        }
        let num_w = 22.0;
        let page_w = 30.0;
        let style = Style::new(Font::Regular, 9.6, 13.6, INK);
        for (i, p) in d.papers.iter().enumerate() {
            let label = format!("{}  ", p.short_label());
            let spans = [(Font::Bold, false, label.as_str()), (Font::Regular, false, p.paper.bib.title.trim())];
            let lines = self.wrap(&spans, style.size, WIDTH - num_w - page_w);
            self.ensure(style.leading * lines.len() as f32 + 6.0);
            let top = self.y;
            let base = top + style.leading * 0.5 + style.size * 0.36;
            self.page().push(Op::Text {
                font: Font::Regular,
                size: style.size,
                x: LEFT,
                y: base,
                colour: MUTED,
                italic: false,
                text: format!("{}.", i + 1),
            });
            if let Some(page) = starts.and_then(|s| s.get(i)) {
                let text = (page + 1).to_string();
                let w = self.fonts.width(Font::Regular, &text, style.size);
                self.page().push(Op::Text { font: Font::Regular, size: style.size, x: LEFT + WIDTH - w, y: base, colour: MUTED, italic: false, text });
            }
            self.flow(&lines, LEFT + num_w, style, false);
            self.y += 6.0;
        }
    }

    /// Running heads and page numbers, once the number of pages is known.
    fn furniture(&mut self, title: &str) {
        let total = self.pages.len();
        for (i, page) in self.pages.iter_mut().enumerate() {
            let number = format!("{} / {total}", i + 1);
            let w = self.fonts.width(Font::Regular, &number, 8.0);
            page.push(Op::Text { font: Font::Regular, size: 8.0, x: (PAGE_W - w) / 2.0, y: PAGE_H - 40.0, colour: MUTED, italic: false, text: number });
            if i > 0 {
                let head: String = title.chars().take(90).collect();
                page.push(Op::Text { font: Font::Regular, size: 8.0, x: LEFT, y: 44.0, colour: MUTED, italic: false, text: head });
                page.push(Op::Line { x1: LEFT, y1: 52.0, x2: LEFT + WIDTH, y2: 52.0, width: 0.5, colour: RULE });
            }
        }
    }
}

/// Markdown spans as (font, italic, text).
fn styled(spans: &[Span], all_bold: bool) -> Vec<(Font, bool, &str)> {
    spans.iter().map(|s| (if s.bold || all_bold { Font::Bold } else { Font::Regular }, s.italic, s.text.as_str())).collect()
}

fn lay_out<'f>(fonts: &'f Fonts, d: &Dissection, starts: Option<&[usize]>) -> Layout<'f> {
    let mut layout = Layout::new(fonts);
    layout.cover(d, starts);
    for (i, p) in d.papers.iter().enumerate() {
        layout.paper(p, d, i + 1);
    }
    layout
}

pub fn render(d: &Dissection) -> Result<Vec<u8>> {
    let fonts = Fonts::load()?;
    // Twice: the first page lists where each paper starts, known only after one pass.
    let first = lay_out(&fonts, d, None);
    let starts: Vec<usize> = first.outline.iter().map(|(_, page, _)| *page).collect();
    let mut layout = lay_out(&fonts, d, Some(&starts));
    layout.furniture(&d.title);
    write_pdf(&fonts, &layout, &d.title)
}

// ----- Writing the file -------------------------------------------------------

/// A PDF being assembled: objects by number (from 1).
struct Pdf {
    objects: Vec<Option<Vec<u8>>>,
}

impl Pdf {
    fn alloc(&mut self) -> usize {
        self.objects.push(None);
        self.objects.len()
    }

    fn set(&mut self, id: usize, body: impl Into<Vec<u8>>) {
        self.objects[id - 1] = Some(body.into());
    }

    fn add(&mut self, body: impl Into<Vec<u8>>) -> usize {
        let id = self.alloc();
        self.set(id, body);
        id
    }

    /// A Flate-compressed stream, with `extra` entries in its dictionary.
    fn stream(&mut self, data: &[u8], extra: &str) -> Result<usize> {
        let mut z = ZlibEncoder::new(Vec::new(), Compression::default());
        z.write_all(data)?;
        let packed = z.finish()?;
        let mut body = format!("<< /Length {} /Filter /FlateDecode {extra} >>\nstream\n", packed.len()).into_bytes();
        body.extend_from_slice(&packed);
        body.extend_from_slice(b"\nendstream");
        Ok(self.add(body))
    }

    fn finish(self, root: usize, info: usize) -> Result<Vec<u8>> {
        let mut out = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
        let mut offsets = Vec::new();
        for (i, body) in self.objects.iter().enumerate() {
            let Some(body) = body else { bail!("PDF object {} was never written", i + 1) };
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(
            format!("trailer\n<< /Size {} /Root {root} 0 R /Info {info} 0 R >>\nstartxref\n{xref}\n%%EOF\n", self.objects.len() + 1).as_bytes(),
        );
        Ok(out)
    }
}

/// A number as PDF writes it: "12.5", "0".
fn num(v: f32) -> String {
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    if s == "-0" || s.is_empty() { "0".into() } else { s.to_string() }
}

/// A text string in a dictionary: UTF-16 with a byte-order mark, as hex.
fn text_string(s: &str) -> String {
    let mut out = String::from("<FEFF");
    for unit in s.encode_utf16() {
        out.push_str(&format!("{unit:04X}"));
    }
    out.push('>');
    out
}

/// An ASCII string literal (URIs).
fn literal(s: &str) -> String {
    let mut out = String::from("(");
    for c in s.chars().filter(char::is_ascii) {
        if matches!(c, '(' | ')' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out.push(')');
    out
}

/// The glyphs of one font used in the document, renumbered for its subset.
struct Used {
    remapper: subsetter::GlyphRemapper,
    /// New glyph id → the text it stands for (for copying text out of the PDF).
    unicode: BTreeMap<u16, char>,
    any: bool,
}

fn write_pdf(fonts: &Fonts, layout: &Layout, title: &str) -> Result<Vec<u8>> {
    let mut used: Vec<Used> =
        (0..3).map(|_| Used { remapper: subsetter::GlyphRemapper::new(), unicode: BTreeMap::new(), any: false }).collect();
    // Content streams, with glyphs renumbered as they are first seen.
    let mut contents = Vec::new();
    for page in &layout.pages {
        let mut c = String::new();
        for op in page {
            match op {
                Op::Text { font, size, x, y, colour, italic, text } => {
                    let data = fonts.get(*font);
                    let u = &mut used[*font as usize];
                    u.any = true;
                    let mut hex = String::new();
                    for ch in text.chars() {
                        let old = data.glyph(ch).unwrap_or(0);
                        let new = u.remapper.remap(old);
                        if old != 0 {
                            u.unicode.entry(new).or_insert(ch);
                        }
                        hex.push_str(&format!("{new:04X}"));
                    }
                    let skew = if *italic { "0.2" } else { "0" };
                    c.push_str(&format!(
                        "BT /F{} {} Tf {} {} {} rg 1 0 {skew} 1 {} {} Tm <{hex}> Tj ET\n",
                        *font as usize,
                        num(*size),
                        num(colour[0]),
                        num(colour[1]),
                        num(colour[2]),
                        num(*x),
                        num(PAGE_H - y)
                    ));
                }
                Op::Rect { x, y, w, h, colour } => {
                    c.push_str(&format!(
                        "{} {} {} rg {} {} {} {} re f\n",
                        num(colour[0]),
                        num(colour[1]),
                        num(colour[2]),
                        num(*x),
                        num(PAGE_H - y - h),
                        num(*w),
                        num(*h)
                    ));
                }
                Op::Line { x1, y1, x2, y2, width, colour } => {
                    c.push_str(&format!(
                        "{} {} {} RG {} w {} {} m {} {} l S\n",
                        num(colour[0]),
                        num(colour[1]),
                        num(colour[2]),
                        num(*width),
                        num(*x1),
                        num(PAGE_H - y1),
                        num(*x2),
                        num(PAGE_H - y2)
                    ));
                }
                Op::Link { .. } => {}
            }
        }
        contents.push(c);
    }

    let mut pdf = Pdf { objects: Vec::new() };
    let catalog = pdf.alloc();
    let pages_id = pdf.alloc();
    let mut font_refs = String::new();
    for (i, u) in used.iter().enumerate() {
        if !u.any {
            continue;
        }
        let font = &fonts.0[i];
        let id = embed_font(&mut pdf, font, u, i)?;
        font_refs.push_str(&format!("/F{i} {id} 0 R "));
    }
    let mut page_ids = Vec::new();
    for (page, content) in layout.pages.iter().zip(&contents) {
        let content_id = pdf.stream(content.as_bytes(), "")?;
        let mut annots = Vec::new();
        for op in page {
            if let Op::Link { x, y, w, h, uri } = op {
                let rect = format!("[{} {} {} {}]", num(*x), num(PAGE_H - y - h), num(x + w), num(PAGE_H - y));
                annots.push(pdf.add(format!("<< /Type /Annot /Subtype /Link /Rect {rect} /Border [0 0 0] /A << /S /URI /URI {} >> >>", literal(uri))));
            }
        }
        let annots = if annots.is_empty() {
            String::new()
        } else {
            format!("/Annots [{}]", annots.iter().map(|a| format!("{a} 0 R")).collect::<Vec<_>>().join(" "))
        };
        page_ids.push(pdf.add(format!(
            "<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {} {}] /Resources << /Font << {font_refs}>> >> /Contents {content_id} 0 R {annots} >>",
            num(PAGE_W),
            num(PAGE_H)
        )));
    }
    let kids = page_ids.iter().map(|p| format!("{p} 0 R")).collect::<Vec<_>>().join(" ");
    pdf.set(pages_id, format!("<< /Type /Pages /Kids [{kids}] /Count {} >>", page_ids.len()));

    // Bookmarks: one per paper.
    let mut outlines = String::new();
    if !layout.outline.is_empty() {
        let root = pdf.alloc();
        let ids: Vec<usize> = layout.outline.iter().map(|_| pdf.alloc()).collect();
        for (i, (title, page, y)) in layout.outline.iter().enumerate() {
            let mut entry = format!(
                "<< /Title {} /Parent {root} 0 R /Dest [{} 0 R /XYZ 0 {} 0]",
                text_string(title),
                page_ids[*page],
                num(PAGE_H - y + 20.0)
            );
            if i > 0 {
                entry.push_str(&format!(" /Prev {} 0 R", ids[i - 1]));
            }
            if let Some(next) = ids.get(i + 1) {
                entry.push_str(&format!(" /Next {next} 0 R"));
            }
            entry.push_str(" >>");
            pdf.set(ids[i], entry);
        }
        pdf.set(root, format!("<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {} >>", ids[0], ids[ids.len() - 1], ids.len()));
        outlines = format!("/Outlines {root} 0 R /PageMode /UseOutlines");
    }
    pdf.set(catalog, format!("<< /Type /Catalog /Pages {pages_id} 0 R {outlines} /Lang (en) >>"));
    let date = chrono::Local::now().format("D:%Y%m%d%H%M%S").to_string();
    let info = pdf.add(format!(
        "<< /Title {} /Creator (LitAnatomy) /Producer (LitAnatomy {}) /CreationDate ({date}) >>",
        text_string(title),
        env!("CARGO_PKG_VERSION")
    ));
    pdf.finish(catalog, info)
}

/// Embeds the glyphs `used` of `font` as a Type 0 (CID) font; returns its object.
fn embed_font(pdf: &mut Pdf, font: &FontData, used: &Used, index: usize) -> Result<usize> {
    let face = &font.face;
    let scale = 1000.0 / face.units_per_em() as f32;
    let subset = subsetter::subset(font.data, 0, &used.remapper).map_err(|e| anyhow::anyhow!("Could not embed {}: {e:?}", font.name))?;
    let truetype = face.tables().glyf.is_some();
    let file = if truetype {
        pdf.stream(&subset, &format!("/Length1 {}", subset.len()))?
    } else {
        pdf.stream(&subset, "/Subtype /OpenType")?
    };
    // A subset's name starts with six capital letters and a plus.
    let tag: String = (0..6).map(|i| (b'A' + ((index * 7 + i * 3) % 26) as u8) as char).collect();
    let name = format!("{tag}+{}", font.name);
    let bbox = face.global_bounding_box();
    let descriptor = pdf.add(format!(
        "<< /Type /FontDescriptor /FontName /{name} /Flags 32 /FontBBox [{} {} {} {}] /ItalicAngle 0 \
         /Ascent {} /Descent {} /CapHeight {} /StemV 80 /{} {file} 0 R >>",
        num(bbox.x_min as f32 * scale),
        num(bbox.y_min as f32 * scale),
        num(bbox.x_max as f32 * scale),
        num(bbox.y_max as f32 * scale),
        num(face.ascender() as f32 * scale),
        num(face.descender() as f32 * scale),
        num(face.capital_height().unwrap_or(face.ascender()) as f32 * scale),
        if truetype { "FontFile2" } else { "FontFile3" }
    ));
    let mut widths = String::new();
    for (new, old) in used.remapper.remapped_gids().enumerate() {
        widths.push_str(&format!("{new} [{}] ", num(font.advance(old))));
    }
    let (subtype, map) = if truetype { ("CIDFontType2", "/CIDToGIDMap /Identity") } else { ("CIDFontType0", "") };
    let cid = pdf.add(format!(
        "<< /Type /Font /Subtype /{subtype} /BaseFont /{name} \
         /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> \
         /FontDescriptor {descriptor} 0 R /W [{widths}] {map} >>"
    ));
    let to_unicode = pdf.stream(to_unicode(&used.unicode).as_bytes(), "")?;
    Ok(pdf.add(format!(
        "<< /Type /Font /Subtype /Type0 /BaseFont /{name} /Encoding /Identity-H \
         /DescendantFonts [{cid} 0 R] /ToUnicode {to_unicode} 0 R >>"
    )))
}

/// The CMap that turns glyphs back into text.
fn to_unicode(map: &BTreeMap<u16, char>) -> String {
    let mut out = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n\
         /CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n\
         /CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n\
         1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let entries: Vec<(&u16, &char)> = map.iter().collect();
    for chunk in entries.chunks(100) {
        out.push_str(&format!("{} beginbfchar\n", chunk.len()));
        for (g, c) in chunk {
            let mut buf = [0u16; 2];
            let hex: String = c.encode_utf16(&mut buf).iter().map(|u| format!("{u:04X}")).collect();
            out.push_str(&format!("<{g:04X}> <{hex}>\n"));
        }
        out.push_str("endbfchar\n");
    }
    out.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::export::dissection::{Options, tests::sample};

    #[test]
    fn pdf_reads_back() {
        let (lib, ids, viva) = sample();
        let d = Dissection::load(&lib, &ids, Some(viva), "Viva", Options { abstracts: true, ..Default::default() }).unwrap();
        let bytes = render(&d).unwrap();
        assert!(bytes.starts_with(b"%PDF-1.7"));
        // Subsets keep the file small.
        assert!(bytes.len() < 150_000, "{} bytes", bytes.len());
        let text = pdf_extract::extract_text_from_mem(&bytes).unwrap();
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        for want in ["LITERATURE DISSECTION", "Varactor-tuned reflectarrays", "PIN diodes", "Key Metrics", "30.8 dB", "Bandwidth", "Relevance to This Project", "λ/2"] {
            assert!(flat.contains(want), "{want:?} not in {flat}");
        }
        let doc = pdf_extract::Document::load_mem(&bytes).unwrap();
        // Cover, then a page per paper.
        assert_eq!(doc.get_pages().len(), 3);
    }

    #[test]
    fn long_words_and_breaks_wrap() {
        let fonts = Fonts::load().unwrap();
        let layout = Layout::new(&fonts);
        let url = "https://example.org/".to_string() + &"a".repeat(300);
        let lines = layout.wrap(&[(Font::Regular, false, &url)], 10.0, 200.0);
        assert!(lines.len() > 2);
        for l in &lines {
            let w = l.last().map(|(x, p)| x + p.width).unwrap();
            assert!(w <= 200.5, "line {w} wide");
        }
        let lines = layout.wrap(&[(Font::Regular, false, "one\ntwo three")], 10.0, 500.0);
        assert_eq!(lines.len(), 2);
        assert_eq!(num(12.50), "12.5");
        assert_eq!(num(-0.001), "0");
    }
}
