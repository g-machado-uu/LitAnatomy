//! A draft related-work outline for a collection, built from its themes and
//! the papers' notes, as Markdown (with `\cite{key}`) or LaTeX.

use crate::latex::{collapse_ws, escape};

/// One paper as the outline mentions it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutlinePaper {
    pub citekey: String,
    /// "Zhang et al. (2025)"
    pub label: String,
    pub title: String,
    /// What it contributes (from its notes), or how it fits the theme.
    pub summary: String,
    /// Its research gaps (from its notes).
    pub gaps: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutlineTheme {
    pub name: String,
    pub notes: String,
    pub papers: Vec<OutlinePaper>,
}

/// Placeholders the writer replaces; LaTeX output turns them into comments.
fn todo(text: &str) -> String {
    format!("_{text}_")
}

/// The first sentence (or 240 characters) of a note.
pub fn first_sentence(text: &str) -> String {
    let text = collapse_ws(text.trim_start_matches(['-', '*', '#', ' ']));
    let end = text
        .char_indices()
        .find(|(i, c)| matches!(c, '.' | '!' | '?') && text[i + 1..].starts_with(' ') && *i > 20)
        .map(|(i, _)| i + 1)
        .unwrap_or(text.len());
    let sentence = &text[..end];
    if sentence.chars().count() > 240 {
        format!("{}…", sentence.chars().take(240).collect::<String>().trim_end())
    } else {
        sentence.to_string()
    }
}

fn bullet(p: &OutlinePaper, text: &str) -> String {
    let what = if text.trim().is_empty() { p.title.clone() } else { first_sentence(text) };
    format!("- {} \\cite{{{}}}: {what}\n", p.label, p.citekey)
}

/// Markdown: a section per theme with its papers, then the papers in no
/// theme, then the open problems the papers' notes mention.
pub fn draft(collection: &str, themes: &[OutlineTheme], unthemed: &[OutlinePaper], all: &[OutlinePaper]) -> String {
    let mut out = format!("# Related work: {collection}\n\n");
    out.push_str(&todo("Introduce the area: the problem these papers address, and why it matters."));
    out.push_str("\n\n");
    for t in themes {
        out.push_str(&format!("## {}\n\n", t.name.trim()));
        if t.notes.trim().is_empty() {
            out.push_str(&todo("What do these papers share? Where do they agree or disagree?"));
        } else {
            out.push_str(t.notes.trim());
        }
        out.push_str("\n\n");
        for p in &t.papers {
            out.push_str(&bullet(p, &p.summary));
        }
        if !t.papers.is_empty() {
            out.push('\n');
        }
    }
    if !unthemed.is_empty() {
        out.push_str(if themes.is_empty() { "## Papers\n\n" } else { "## Other work\n\n" });
        for p in unthemed {
            out.push_str(&bullet(p, &p.summary));
        }
        out.push('\n');
    }
    let gaps: Vec<&OutlinePaper> = all.iter().filter(|p| !p.gaps.trim().is_empty()).collect();
    out.push_str("## Open problems\n\n");
    if gaps.is_empty() {
        out.push_str(&todo("What is still open? Note research gaps in the papers' anatomy to list them here."));
        out.push_str("\n\n");
    } else {
        for p in gaps {
            out.push_str(&bullet(p, &p.gaps));
        }
        out.push('\n');
    }
    out.push_str("## Summary\n\n");
    out.push_str(&todo("How your work builds on these papers, and which gap it fills."));
    out.push('\n');
    out
}

/// Markdown outline → LaTeX: headings become sections, bullets an
/// itemize, `\cite{…}` stays, placeholders become `% TODO` comments.
pub fn to_latex(markdown: &str) -> String {
    let mut out = String::new();
    let mut in_list = false;
    for line in markdown.lines() {
        let trimmed = line.trim();
        let is_item = trimmed.starts_with("- ") || trimmed.starts_with("* ");
        if in_list && !is_item {
            out.push_str("\\end{itemize}\n");
            in_list = false;
        }
        if let Some(h) = trimmed.strip_prefix("### ") {
            out.push_str(&format!("\\subsubsection{{{}}}\n", text(h)));
        } else if let Some(h) = trimmed.strip_prefix("## ") {
            out.push_str(&format!("\\subsection{{{}}}\n", text(h)));
        } else if let Some(h) = trimmed.strip_prefix("# ") {
            let h = h.strip_prefix("Related work: ").map(|_| "Related work").unwrap_or(h);
            out.push_str(&format!("\\section{{{}}}\n", text(h)));
        } else if is_item {
            if !in_list {
                out.push_str("\\begin{itemize}\n");
                in_list = true;
            }
            out.push_str(&format!("  \\item {}\n", text(&trimmed[2..])));
        } else if trimmed.len() > 2 && trimmed.starts_with('_') && trimmed.ends_with('_') {
            out.push_str(&format!("% TODO: {}\n", trimmed.trim_matches('_')));
        } else if trimmed.is_empty() {
            out.push('\n');
        } else {
            out.push_str(&text(trimmed));
            out.push('\n');
        }
    }
    if in_list {
        out.push_str("\\end{itemize}\n");
    }
    out
}

/// Escapes text for LaTeX, keeping `\cite{…}` commands as they are.
fn text(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find("\\cite{") {
        let Some(len) = rest[i..].find('}') else { break };
        out.push_str(&escape(&rest[..i]));
        out.push_str(&rest[i..i + len + 1]);
        rest = &rest[i + len + 1..];
    }
    out.push_str(&escape(rest));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(key: &str, summary: &str, gaps: &str) -> OutlinePaper {
        OutlinePaper {
            citekey: key.into(),
            label: format!("{key} et al."),
            title: format!("Title of {key}"),
            summary: summary.into(),
            gaps: gaps.into(),
        }
    }

    #[test]
    fn draft_from_themes() {
        let themes = vec![OutlineTheme {
            name: "Low-cost unit cells".into(),
            notes: "Fewer diodes, less loss.".into(),
            papers: vec![p("zhang2025", "A 2-bit cell with two PIN diodes. It also does more things here.", "")],
        }];
        let other = vec![p("li2024", "", "Wideband behaviour is untested in the field.")];
        let all = [themes[0].papers.clone(), other.clone()].concat();
        let md = draft("Viva", &themes, &other, &all);
        assert!(md.starts_with("# Related work: Viva\n"));
        assert!(md.contains("## Low-cost unit cells\n\nFewer diodes, less loss.\n\n- zhang2025 et al. \\cite{zhang2025}: A 2-bit cell with two PIN diodes.\n"), "{md}");
        assert!(md.contains("## Other work\n\n- li2024 et al. \\cite{li2024}: Title of li2024\n"), "{md}");
        assert!(md.contains("## Open problems\n\n- li2024 et al. \\cite{li2024}: Wideband behaviour is untested in the field.\n"), "{md}");

        let tex = to_latex(&md);
        assert!(tex.starts_with("\\section{Related work}\n\n% TODO: Introduce the area"), "{tex}");
        assert!(tex.contains("\\subsection{Low-cost unit cells}"), "{tex}");
        assert!(tex.contains("\\begin{itemize}\n  \\item zhang2025 et al. \\cite{zhang2025}: A 2-bit cell"), "{tex}");
        assert_eq!(tex.matches("\\begin{itemize}").count(), tex.matches("\\end{itemize}").count());
    }

    #[test]
    fn latex_escapes_text_but_not_citations() {
        assert_eq!(text("50% of \\cite{a_b} & more_"), "50\\% of \\cite{a_b} \\& more\\_");
        assert_eq!(first_sentence("- Short. Then the rest"), "Short. Then the rest");
        assert_eq!(first_sentence("We propose a new cell design. It is cheap."), "We propose a new cell design.");
    }
}
