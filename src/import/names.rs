//! Personal names in the forms reference files use.

use crate::latex;
use crate::model::Author;

/// A BibTeX name list: "Zhang, Wei and Li, Na and {IEEE Antennas Society}".
/// The input is raw LaTeX; output names are decoded to Unicode.
pub fn parse_bibtex_list(raw: &str) -> Vec<Author> {
    split_top_level(raw, " and ")
        .into_iter()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(parse_bibtex_name)
        .filter(|a| !a.family.is_empty() || !a.given.is_empty())
        .collect()
}

fn parse_bibtex_name(raw: &str) -> Author {
    let raw = raw.trim();
    if raw.eq_ignore_ascii_case("others") {
        return Author::new("others", "");
    }
    // A fully braced name is an organisation: keep it whole.
    if raw.starts_with('{') && raw.ends_with('}') && matching_brace(raw) == Some(raw.len() - 1) {
        return Author::new(latex::to_unicode(raw), "");
    }
    let parts = split_top_level(raw, ",");
    match parts.as_slice() {
        // "von Last, Jr, First"
        [last, _, first, ..] | [last, first] => {
            Author::new(latex::to_unicode(last.trim()), latex::to_unicode(first.trim()))
        }
        _ => first_last(raw),
    }
}

/// "First von Last": the family name starts at the first lower-case word
/// (a particle such as "van") or is the last word.
fn first_last(raw: &str) -> Author {
    let words = split_top_level(raw, " ");
    let words: Vec<&str> = words.into_iter().map(str::trim).filter(|w| !w.is_empty()).collect();
    if words.len() <= 1 {
        return Author::new(latex::to_unicode(raw), "");
    }
    let last = words.len() - 1;
    let von = words[..last].iter().position(|w| starts_lowercase(w)).unwrap_or(last);
    let von = von.max(1);
    Author::new(latex::to_unicode(&words[von..].join(" ")), latex::to_unicode(&words[..von].join(" ")))
}

fn starts_lowercase(word: &str) -> bool {
    latex::to_unicode(word).chars().next().is_some_and(char::is_lowercase)
}

/// "Zhang, Wei" or "Wei Zhang" (RIS, RefWorks, CSL literals).
pub fn parse_plain(name: &str) -> Author {
    let name = name.trim();
    if let Some((family, given)) = name.split_once(',') {
        Author::new(family.trim(), given.trim())
    } else {
        let a = first_last(name);
        Author::new(a.family, a.given)
    }
}

/// Splits on `sep` outside braces, case-insensitively for " and ".
pub fn split_top_level<'a>(s: &'a str, sep: &str) -> Vec<&'a str> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    let bytes = s.as_bytes();
    let sep_lower = sep.to_ascii_lowercase();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            _ => {}
        }
        if depth == 0
            && i + sep.len() <= bytes.len()
            && s.is_char_boundary(i)
            && s.is_char_boundary(i + sep.len())
            && s[i..i + sep.len()].to_ascii_lowercase() == sep_lower
        {
            out.push(&s[start..i]);
            i += sep.len();
            start = i;
            continue;
        }
        i += 1;
    }
    out.push(&s[start..]);
    out
}

fn matching_brace(s: &str) -> Option<usize> {
    let mut depth = 0;
    for (i, c) in s.char_indices() {
        match c {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bibtex_names() {
        let a = parse_bibtex_list("Zhang, Wei and Li, Na and Ludwig van Beethoven and {IEEE Antennas and Propagation Society}");
        assert_eq!(a[0], Author::new("Zhang", "Wei"));
        assert_eq!(a[1], Author::new("Li", "Na"));
        assert_eq!(a[2], Author::new("van Beethoven", "Ludwig"));
        assert_eq!(a[3], Author::new("IEEE Antennas and Propagation Society", ""));
        assert_eq!(a.len(), 4);
    }

    #[test]
    fn bibtex_name_forms() {
        let a = parse_bibtex_list("M{\\\"u}ller, J{\\\"o}rg AND Smith, Jr, John and A. B. Oliveri and Plato");
        assert_eq!(a[0], Author::new("Müller", "Jörg"));
        assert_eq!(a[1], Author::new("Smith", "John"));
        assert_eq!(a[2], Author::new("Oliveri", "A. B."));
        assert_eq!(a[3], Author::new("Plato", ""));
    }

    #[test]
    fn plain_names() {
        assert_eq!(parse_plain("Zhang, Wei"), Author::new("Zhang", "Wei"));
        assert_eq!(parse_plain("Wei Zhang"), Author::new("Zhang", "Wei"));
        assert_eq!(parse_plain("Zhang"), Author::new("Zhang", ""));
    }
}
