//! LaTeX text in BibTeX fields: decoding to plain Unicode on import, and
//! escaping and case protection on export. Maths (`$…$`) passes through as is.

/// Decodes LaTeX markup to plain Unicode: accents (`{\"o}`, `\'{e}`),
/// symbols (`\ss`, `\&`), formatting commands (`\emph{x}` → x), grouping
/// braces, dashes and `~`. Whitespace is collapsed.
pub fn to_unicode(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    decode(&chars, &mut 0, &mut out, false);
    collapse_ws(&out)
}

fn decode(c: &[char], i: &mut usize, out: &mut String, stop_at_brace: bool) {
    while *i < c.len() {
        let ch = c[*i];
        match ch {
            '}' if stop_at_brace => {
                *i += 1;
                return;
            }
            '{' => {
                *i += 1;
                decode(c, i, out, true);
            }
            '}' => *i += 1,
            '$' => {
                // Keep maths verbatim, delimiters included.
                let start = *i;
                *i += 1;
                while *i < c.len() && c[*i] != '$' {
                    if c[*i] == '\\' {
                        *i += 1;
                    }
                    *i += 1;
                }
                *i = (*i + 1).min(c.len());
                out.extend(&c[start..*i]);
            }
            '~' => {
                out.push(' ');
                *i += 1;
            }
            '-' if c.get(*i + 1) == Some(&'-') => {
                if c.get(*i + 2) == Some(&'-') {
                    out.push('—');
                    *i += 3;
                } else {
                    out.push('–');
                    *i += 2;
                }
            }
            '\\' => command(c, i, out),
            _ => {
                out.push(ch);
                *i += 1;
            }
        }
    }
}

/// Reads one argument: `{…}` (decoded) or a single character.
fn argument(c: &[char], i: &mut usize) -> String {
    while *i < c.len() && c[*i] == ' ' {
        *i += 1;
    }
    let mut arg = String::new();
    match c.get(*i) {
        Some('{') => {
            *i += 1;
            decode(c, i, &mut arg, true);
        }
        Some('\\') => command(c, i, &mut arg),
        Some(&ch) => {
            arg.push(ch);
            *i += 1;
        }
        None => {}
    }
    arg
}

fn command(c: &[char], i: &mut usize, out: &mut String) {
    *i += 1; // the backslash
    let Some(&next) = c.get(*i) else { return };
    if !next.is_ascii_alphabetic() {
        *i += 1;
        match next {
            '\'' | '`' | '^' | '"' | '~' | '=' | '.' => {
                let arg = argument(c, i);
                push_accented(out, next, &arg);
            }
            '\\' => out.push(' '),
            ' ' | '\n' => out.push(' '),
            '-' => {} // discretionary hyphen
            other => out.push(other), // \& \% \$ \# \_ \{ \}
        }
        return;
    }
    let start = *i;
    while *i < c.len() && c[*i].is_ascii_alphabetic() {
        *i += 1;
    }
    let name: String = c[start..*i].iter().collect();
    // A control word eats the spaces after it.
    let eat_space = |i: &mut usize| {
        while *i < c.len() && c[*i] == ' ' {
            *i += 1;
        }
    };
    let symbol = match name.as_str() {
        "ss" => Some("ß"),
        "ae" => Some("æ"),
        "AE" => Some("Æ"),
        "oe" => Some("œ"),
        "OE" => Some("Œ"),
        "aa" => Some("å"),
        "AA" => Some("Å"),
        "o" => Some("ø"),
        "O" => Some("Ø"),
        "l" => Some("ł"),
        "L" => Some("Ł"),
        "i" => Some("ı"),
        "j" => Some("ȷ"),
        "textendash" => Some("–"),
        "textemdash" => Some("—"),
        "textquoteright" => Some("’"),
        "textquoteleft" => Some("‘"),
        "textregistered" => Some("®"),
        "texttrademark" => Some("™"),
        "textcopyright" => Some("©"),
        "textdegree" => Some("°"),
        "textmu" => Some("µ"),
        "copyright" => Some("©"),
        "dag" => Some("†"),
        "S" => Some("§"),
        "P" => Some("¶"),
        "ldots" | "dots" | "textellipsis" => Some("…"),
        "LaTeX" => Some("LaTeX"),
        "TeX" => Some("TeX"),
        "&" => Some("&"),
        _ => None,
    };
    if let Some(sym) = symbol {
        eat_space(i);
        // `{\i}` style: braces around the symbol are handled by the caller.
        out.push_str(sym);
        return;
    }
    let accent = match name.as_str() {
        "c" => Some('c'),
        "v" => Some('v'),
        "u" => Some('u'),
        "H" => Some('H'),
        "r" => Some('r'),
        "k" => Some('k'),
        "d" => Some('d'),
        "b" => Some('b'),
        "t" => Some('t'),
        _ => None,
    };
    if let Some(a) = accent {
        let arg = argument(c, i);
        push_accented(out, a, &arg);
        return;
    }
    eat_space(i);
    // Formatting and unknown commands: drop the name, keep a braced argument.
    if c.get(*i) == Some(&'{') {
        *i += 1;
        decode(c, i, out, true);
    }
}

fn push_accented(out: &mut String, accent: char, base: &str) {
    let mut chars = base.chars();
    let Some(first) = chars.next() else {
        return;
    };
    match compose(accent, first) {
        Some(composed) => out.push(composed),
        None => {
            out.push(first);
            if let Some(mark) = combining(accent) {
                out.push(mark);
            }
        }
    }
    out.extend(chars);
}

fn combining(accent: char) -> Option<char> {
    Some(match accent {
        '`' => '\u{300}',
        '\'' => '\u{301}',
        '^' => '\u{302}',
        '~' => '\u{303}',
        '=' => '\u{304}',
        'u' => '\u{306}',
        '.' => '\u{307}',
        '"' => '\u{308}',
        'r' => '\u{30A}',
        'H' => '\u{30B}',
        'v' => '\u{30C}',
        'd' => '\u{323}',
        'c' => '\u{327}',
        'k' => '\u{328}',
        'b' => '\u{331}',
        _ => return None,
    })
}

fn compose(accent: char, base: char) -> Option<char> {
    const TABLE: &[(char, &str, &str)] = &[
        ('\'', "aeiouyAEIOUYcnszCNSZ", "áéíóúýÁÉÍÓÚÝćńśźĆŃŚŹ"),
        ('`', "aeiouAEIOU", "àèìòùÀÈÌÒÙ"),
        ('^', "aeiouAEIOU", "âêîôûÂÊÎÔÛ"),
        ('"', "aeiouyAEIOUY", "äëïöüÿÄËÏÖÜŸ"),
        ('~', "anoANO", "ãñõÃÑÕ"),
        ('c', "csCS", "çşÇŞ"),
        ('v', "cszrenCSZREN", "čšžřěňČŠŽŘĚŇ"),
        ('u', "agAG", "ăğĂĞ"),
        ('H', "ouOU", "őűŐŰ"),
        ('r', "auAU", "åůÅŮ"),
        ('k', "aeAE", "ąęĄĘ"),
        ('=', "aeiouAEIOU", "āēīōūĀĒĪŌŪ"),
        ('.', "zeZEI", "żėŻĖİ"),
    ];
    let base = if base == 'ı' { 'i' } else { base };
    for (a, from, to) in TABLE {
        if *a == accent
            && let Some(pos) = from.chars().position(|c| c == base)
        {
            return to.chars().nth(pos);
        }
    }
    None
}

pub fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Escapes characters special to LaTeX, leaving `$…$` maths untouched.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 8);
    for (i, part) in split_math(s).into_iter().enumerate() {
        if i % 2 == 1 {
            out.push_str(part);
            continue;
        }
        for ch in part.chars() {
            match ch {
                '&' | '%' | '#' | '_' | '{' | '}' => {
                    out.push('\\');
                    out.push(ch);
                }
                '$' => out.push_str("\\$"),
                '~' => out.push_str("\\textasciitilde{}"),
                '^' => out.push_str("\\textasciicircum{}"),
                '\\' => out.push_str("\\textbackslash{}"),
                _ => out.push(ch),
            }
        }
    }
    out
}

/// Splits into text, maths, text, maths… (odd indices are `$…$` including
/// delimiters). An unpaired `$` stays in the text.
fn split_math(s: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut rest = s;
    loop {
        let Some(start) = rest.find('$') else { break };
        let Some(len) = rest[start + 1..].find('$') else { break };
        let end = start + 1 + len + 1;
        parts.push(&rest[..start]);
        parts.push(&rest[start..end]);
        rest = &rest[end..];
    }
    parts.push(rest);
    parts
}

/// Escapes a title and protects words whose capitals must survive a
/// sentence-case bibliography style: acronyms and mixed case such as
/// `RIS`, `5G`, `mmWave`, `LiDAR` become `{RIS}`, `{5G}`…
pub fn protect_title(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    for (i, part) in split_math(s).into_iter().enumerate() {
        if i % 2 == 1 {
            out.push_str(part);
            continue;
        }
        let mut word = String::new();
        let flush = |word: &mut String, out: &mut String| {
            if word.is_empty() {
                return;
            }
            let escaped = escape(word);
            if needs_protection(word) {
                out.push('{');
                out.push_str(&escaped);
                out.push('}');
            } else {
                out.push_str(&escaped);
            }
            word.clear();
        };
        for ch in part.chars() {
            if ch.is_whitespace() || matches!(ch, ':' | ',' | ';' | '?' | '!' | '(' | ')' | '"' | '/') {
                flush(&mut word, &mut out);
                out.push(ch);
            } else {
                word.push(ch);
            }
        }
        flush(&mut word, &mut out);
    }
    out
}

fn needs_protection(word: &str) -> bool {
    // Lower case first, upper case later: "mmWave", "mm-Wave", "iPhone".
    let first_lower = word.chars().find(|c| c.is_alphanumeric()).is_some_and(char::is_lowercase);
    if first_lower && word.chars().any(char::is_uppercase) {
        return true;
    }
    // A capital after the first character of a hyphenated part: "RIS", "5G",
    // "LiDAR". "1-Bit" and "Wi-Fi" are ordinary title case.
    word.split('-')
        .any(|part| part.chars().skip_while(|c| !c.is_alphanumeric()).skip(1).any(char::is_uppercase))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accents_and_symbols() {
        assert_eq!(to_unicode(r#"M{\"u}ller and Gr\"{o}{\ss}e"#), "Müller and Größe");
        assert_eq!(to_unicode(r"Fran\c{c}ois \'{E}cole \v{S}koda"), "François École Škoda");
        assert_eq!(to_unicode(r"{\'\i}ndice"), "índice");
        assert_eq!(to_unicode(r"Tom \& Jerry 50\% \_x"), "Tom & Jerry 50% _x");
        assert_eq!(to_unicode(r"Ole {\O}stergaard and \AA{}se"), "Ole Østergaard and Åse");
    }

    #[test]
    fn braces_commands_dashes_and_space() {
        assert_eq!(to_unicode("{A} {W}ideband   5{G}\n RIS"), "A Wideband 5G RIS");
        assert_eq!(to_unicode(r"\emph{Very} \textbf{bold} \unknown{kept}"), "Very bold kept");
        assert_eq!(to_unicode("pages 1--10 and---more"), "pages 1–10 and—more");
        assert_eq!(to_unicode("A~B"), "A B");
    }

    #[test]
    fn maths_is_kept() {
        assert_eq!(to_unicode(r"Gain of $\mu$-{Wave} $x_{1}$"), r"Gain of $\mu$-Wave $x_{1}$");
        assert_eq!(escape(r"5% at $\lambda/2$ & more"), r"5\% at $\lambda/2$ \& more");
    }

    #[test]
    fn escaping() {
        assert_eq!(escape("R&D_1 #2 {x} ~^\\"), r"R\&D\_1 \#2 \{x\} \textasciitilde{}\textasciicircum{}\textbackslash{}");
        assert_eq!(escape("Costs $5"), r"Costs \$5");
    }

    #[test]
    fn title_protection() {
        assert_eq!(
            protect_title("A Wideband RIS for 5G mm-Wave and mmWave: LiDAR Study"),
            "A Wideband {RIS} for {5G} {mm-Wave} and {mmWave}: {LiDAR} Study"
        );
        assert_eq!(protect_title("A 2-Bit Wideband Surface"), "A 2-Bit Wideband Surface");
        assert_eq!(protect_title("Gain at $\\lambda$ & R&D"), "Gain at $\\lambda$ \\& {R\\&D}");
    }
}
