//! Notes' light Markdown as simple blocks of styled text, for the Word and
//! PDF exports.

use pulldown_cmark::{Event, Parser, Tag, TagEnd};

/// A run of text in one style. A `\n` inside is a line break.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Span {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    Paragraph(Vec<Span>),
    /// A list item: its nesting (from 0) and its marker ("•" or "3.").
    Item { level: usize, marker: String, spans: Vec<Span> },
    Heading(Vec<Span>),
    Quote(Vec<Span>),
    Code(String),
}

/// Parses `text` (CommonMark, as the app shows it) into blocks.
pub fn parse(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut spans: Vec<Span> = Vec::new();
    let (mut bold, mut italic) = (0usize, 0usize);
    // Each open list: the next number, or None for bullets.
    let mut lists: Vec<Option<u64>> = Vec::new();
    let mut item: Option<(usize, String)> = None;
    let mut quote = 0usize;
    let mut heading = false;
    let mut code: Option<String> = None;

    fn push(spans: &mut Vec<Span>, text: &str, bold: bool, italic: bool, code: bool) {
        if let Some(last) = spans.last_mut()
            && last.bold == bold
            && last.italic == italic
            && last.code == code
        {
            last.text.push_str(text);
            return;
        }
        spans.push(Span { text: text.to_string(), bold, italic, code });
    }

    let flush = |spans: &mut Vec<Span>, blocks: &mut Vec<Block>, item: &mut Option<(usize, String)>, quote: usize, heading: bool| {
        // Trim the ends of the block, keeping inner spaces.
        if let Some(first) = spans.first_mut() {
            first.text = first.text.trim_start().to_string();
        }
        if let Some(last) = spans.last_mut() {
            last.text = last.text.trim_end().to_string();
        }
        spans.retain(|s| !s.text.is_empty());
        if spans.is_empty() {
            return;
        }
        let taken = std::mem::take(spans);
        let block = if let Some((level, marker)) = item.take() {
            Block::Item { level, marker, spans: taken }
        } else if heading {
            Block::Heading(taken)
        } else if quote > 0 {
            Block::Quote(taken)
        } else {
            Block::Paragraph(taken)
        };
        blocks.push(block);
    };

    for event in Parser::new(text) {
        match event {
            Event::Start(Tag::Strong) => bold += 1,
            Event::End(TagEnd::Strong) => bold = bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => italic += 1,
            Event::End(TagEnd::Emphasis) => italic = italic.saturating_sub(1),
            Event::Start(Tag::Heading { .. }) => {
                flush(&mut spans, &mut blocks, &mut item, quote, heading);
                heading = true;
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(&mut spans, &mut blocks, &mut item, quote, heading);
                heading = false;
            }
            Event::Start(Tag::BlockQuote(_)) => {
                flush(&mut spans, &mut blocks, &mut item, quote, heading);
                quote += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                flush(&mut spans, &mut blocks, &mut item, quote, heading);
                quote = quote.saturating_sub(1);
            }
            Event::Start(Tag::List(start)) => {
                flush(&mut spans, &mut blocks, &mut item, quote, heading);
                lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                flush(&mut spans, &mut blocks, &mut item, quote, heading);
                lists.pop();
            }
            Event::Start(Tag::Item) => {
                flush(&mut spans, &mut blocks, &mut item, quote, heading);
                let level = lists.len().saturating_sub(1);
                let marker = match lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}.");
                        *n += 1;
                        m
                    }
                    _ => "•".to_string(),
                };
                item = Some((level, marker));
            }
            Event::End(TagEnd::Item) | Event::End(TagEnd::Paragraph) => {
                flush(&mut spans, &mut blocks, &mut item, quote, heading);
            }
            Event::Start(Tag::CodeBlock(_)) => {
                flush(&mut spans, &mut blocks, &mut item, quote, heading);
                code = Some(String::new());
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(c) = code.take() {
                    blocks.push(Block::Code(c.trim_end().to_string()));
                }
            }
            Event::Text(t) => match &mut code {
                Some(c) => c.push_str(&t),
                None => push(&mut spans, &t, bold > 0, italic > 0, false),
            },
            Event::Code(t) => push(&mut spans, &t, bold > 0, italic > 0, true),
            Event::SoftBreak => push(&mut spans, " ", bold > 0, italic > 0, false),
            Event::HardBreak => push(&mut spans, "\n", bold > 0, italic > 0, false),
            Event::Html(t) | Event::InlineHtml(t) => push(&mut spans, &t, bold > 0, italic > 0, false),
            Event::Rule => flush(&mut spans, &mut blocks, &mut item, quote, heading),
            _ => {}
        }
    }
    flush(&mut spans, &mut blocks, &mut item, quote, heading);
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(text: &str) -> Span {
        Span { text: text.into(), ..Default::default() }
    }

    #[test]
    fn markdown_to_blocks() {
        let blocks = parse("Two **PIN diodes**\ngive *four* states.\n\n- low loss\n- wide\n  1. a\n  2. b\n\n> quoted\n\n# Head");
        assert_eq!(
            blocks[0],
            Block::Paragraph(vec![
                plain("Two "),
                Span { text: "PIN diodes".into(), bold: true, ..Default::default() },
                plain(" give "),
                Span { text: "four".into(), italic: true, ..Default::default() },
                plain(" states."),
            ])
        );
        assert_eq!(blocks[1], Block::Item { level: 0, marker: "•".into(), spans: vec![plain("low loss")] });
        assert_eq!(blocks[2], Block::Item { level: 0, marker: "•".into(), spans: vec![plain("wide")] });
        assert_eq!(blocks[3], Block::Item { level: 1, marker: "1.".into(), spans: vec![plain("a")] });
        assert_eq!(blocks[4], Block::Item { level: 1, marker: "2.".into(), spans: vec![plain("b")] });
        assert_eq!(blocks[5], Block::Quote(vec![plain("quoted")]));
        assert_eq!(blocks[6], Block::Heading(vec![plain("Head")]));
        assert_eq!(parse("  \n"), vec![]);
    }
}
