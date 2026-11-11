//! The synthesis matrix as CSV, Markdown or a LaTeX table.

use crate::latex::{collapse_ws, escape};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Row>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    /// Shown in the first column: "Zhang et al. (2025)".
    pub label: String,
    pub citekey: String,
    pub cells: Vec<String>,
}

pub fn to_csv(t: &Table) -> String {
    let quote = |s: &str| {
        if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
    };
    let mut out = String::new();
    let header: Vec<String> = ["Paper", "Key"].iter().map(|s| s.to_string()).chain(t.headers.iter().map(|h| quote(h))).collect();
    out.push_str(&header.join(","));
    out.push_str("\r\n");
    for r in &t.rows {
        let line: Vec<String> =
            [quote(&r.label), quote(&r.citekey)].into_iter().chain(r.cells.iter().map(|c| quote(c))).collect();
        out.push_str(&line.join(","));
        out.push_str("\r\n");
    }
    out
}

pub fn to_markdown(t: &Table) -> String {
    let cell = |s: &str| s.replace('|', "\\|").replace("\r\n", "<br>").replace('\n', "<br>");
    let mut out = String::new();
    out.push_str("| Paper |");
    for h in &t.headers {
        out.push_str(&format!(" {} |", cell(h)));
    }
    out.push_str("\n|---|");
    for _ in &t.headers {
        out.push_str("---|");
    }
    out.push('\n');
    for r in &t.rows {
        out.push_str(&format!("| {} |", cell(&r.label)));
        for c in &r.cells {
            out.push_str(&format!(" {} |", cell(c)));
        }
        out.push('\n');
    }
    out
}

/// A `tabular` with wrapping columns; the first column cites each paper.
pub fn to_latex(t: &Table) -> String {
    let n = t.headers.len().max(1);
    let first = 0.16;
    let width = (0.84 / n as f64 * 100.0).floor() / 100.0;
    let mut cols = format!(">{{\\raggedright\\arraybackslash}}p{{{first}\\linewidth}}");
    for _ in 0..n {
        cols.push_str(&format!(">{{\\raggedright\\arraybackslash}}p{{{width:.2}\\linewidth}}"));
    }
    let mut out = String::new();
    out.push_str("% Needs \\usepackage{array} and \\usepackage{booktabs}.\n");
    out.push_str("\\begin{table}[htbp]\n  \\centering\n  \\scriptsize\n");
    out.push_str(&format!("  \\begin{{tabular}}{{{cols}}}\n    \\toprule\n"));
    let headers: Vec<String> = t.headers.iter().map(|h| format!("\\textbf{{{}}}", escape(h))).collect();
    out.push_str(&format!("    \\textbf{{Paper}} & {} \\\\\n    \\midrule\n", headers.join(" & ")));
    for r in &t.rows {
        let cells: Vec<String> = r.cells.iter().map(|c| escape(&collapse_ws(c))).collect();
        out.push_str(&format!("    {} \\cite{{{}}} & {} \\\\\n", escape(&r.label), r.citekey, cells.join(" & ")));
    }
    out.push_str("    \\bottomrule\n  \\end{tabular}\n  \\caption{Comparison of the reviewed works.}\n");
    out.push_str("  \\label{tab:literature-comparison}\n\\end{table}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Table {
        Table {
            headers: vec!["Contribution".into(), "Frequency range".into()],
            rows: vec![Row {
                label: "Zhang et al. (2025)".into(),
                citekey: "zhang2025Wideband".into(),
                cells: vec!["2-bit RIS, \"low\" SLL\nwith 2 PINs".into(), "24.1–27.7 GHz & more".into()],
            }],
        }
    }

    #[test]
    fn csv() {
        assert_eq!(
            to_csv(&table()),
            "Paper,Key,Contribution,Frequency range\r\nZhang et al. (2025),zhang2025Wideband,\"2-bit RIS, \"\"low\"\" SLL\nwith 2 PINs\",24.1–27.7 GHz & more\r\n"
        );
    }

    #[test]
    fn markdown() {
        let md = to_markdown(&table());
        assert!(md.starts_with("| Paper | Contribution | Frequency range |\n|---|---|---|\n"));
        assert!(md.contains("| Zhang et al. (2025) | 2-bit RIS, \"low\" SLL<br>with 2 PINs | 24.1–27.7 GHz & more |"));
    }

    #[test]
    fn latex() {
        let tex = to_latex(&table());
        assert!(tex.contains("Zhang et al. (2025) \\cite{zhang2025Wideband} & 2-bit RIS, \"low\" SLL with 2 PINs & 24.1–27.7 GHz \\& more \\\\"));
        assert!(tex.contains("p{0.42\\linewidth}"));
        assert!(tex.contains("\\textbf{Frequency range}"));
    }
}
