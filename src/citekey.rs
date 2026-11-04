//! Readable citation keys, e.g. `zhang2025Wideband`, built from a pattern.
//!
//! Pattern tokens (anything outside brackets is copied as is):
//! - `[auth]` first author's family name ("vanDerBerg"); `[auth:lower]`, `[auth:upper]`
//! - `[auth2]` first two family names ("zhangLi"), `[auth2:lower]`
//! - `[year]` four digits, `[shortyear]` two
//! - `[title:N]` the first N significant title words, capitalised; `[title:N:lower]`
//! - `[venue:N]` the first N significant words of the journal or proceedings

use deunicode::deunicode;

use crate::model::Biblio;

pub const DEFAULT_PATTERN: &str = "[auth:lower][year][title:1]";

const STOP_WORDS: &[&str] = &[
    "a", "an", "the", "and", "or", "but", "nor", "of", "on", "in", "at", "to", "for", "from", "by", "with",
    "without", "into", "onto", "over", "under", "via", "as", "is", "are", "be", "its", "their", "this",
    "that", "these", "those", "towards", "toward", "about", "between", "do", "does", "how", "what", "when",
    "where", "which", "who", "why", "can", "not", "no", "vs", "versus", "de", "la", "le", "der", "die",
    "das", "el", "los", "las", "et",
];

#[derive(Debug, PartialEq)]
enum Token {
    Literal(String),
    Auth { count: usize, case: Case },
    Year { short: bool },
    Title { words: usize, case: Case },
    Venue { words: usize, case: Case },
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum Case {
    AsIs,
    Lower,
    Upper,
}

fn parse(pattern: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut rest = pattern;
    while !rest.is_empty() {
        if let Some(body) = rest.strip_prefix('[') {
            let end = body.find(']').ok_or("A '[' has no matching ']'.")?;
            tokens.push(token(&body[..end])?);
            rest = &body[end + 1..];
        } else {
            let end = rest.find('[').unwrap_or(rest.len());
            let lit: String = rest[..end]
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.'))
                .collect();
            if rest[..end].contains(']') {
                return Err("A ']' has no matching '['.".into());
            }
            tokens.push(Token::Literal(lit));
            rest = &rest[end..];
        }
    }
    Ok(tokens)
}

fn token(body: &str) -> Result<Token, String> {
    let mut parts = body.split(':');
    let name = parts.next().unwrap_or_default();
    let mut case = Case::AsIs;
    let mut number = None;
    for arg in parts {
        match arg {
            "lower" => case = Case::Lower,
            "upper" => case = Case::Upper,
            n => {
                let n: usize = n.parse().map_err(|_| format!("Unknown option '{n}' in [{body}]."))?;
                if !(1..=5).contains(&n) {
                    return Err(format!("Use between 1 and 5 words in [{body}]."));
                }
                number = Some(n);
            }
        }
    }
    Ok(match name {
        "auth" => Token::Auth { count: 1, case },
        "auth2" => Token::Auth { count: 2, case },
        "year" => Token::Year { short: false },
        "shortyear" => Token::Year { short: true },
        "title" => Token::Title { words: number.unwrap_or(1), case },
        "venue" => Token::Venue { words: number.unwrap_or(1), case },
        other => return Err(format!("Unknown part [{other}]. Use auth, auth2, year, shortyear, title or venue.")),
    })
}

/// Checks a pattern, for the Settings screen.
pub fn validate(pattern: &str) -> Result<(), String> {
    let tokens = parse(pattern)?;
    if tokens.iter().all(|t| matches!(t, Token::Literal(_))) {
        return Err("The pattern needs at least one part in brackets, e.g. [auth].".into());
    }
    Ok(())
}

/// The key for `bib`, before making it unique. Falls back to the default
/// pattern if `pattern` is invalid, and to "ref" if everything is empty.
pub fn generate(pattern: &str, bib: &Biblio) -> String {
    let tokens = parse(pattern)
        .ok()
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| parse(DEFAULT_PATTERN).expect("default pattern is valid"));
    let mut key = String::new();
    for t in &tokens {
        match t {
            Token::Literal(s) => key.push_str(s),
            Token::Auth { count, case } => {
                let names: Vec<String> = bib
                    .authors
                    .iter()
                    .take(*count)
                    .map(|a| camel(&words_of(&a.family)))
                    .filter(|n| !n.is_empty())
                    .collect();
                let joined = if names.is_empty() {
                    // No author: an organisation-like stand-in from the venue.
                    significant_words(&bib.container).into_iter().next().map(|w| capitalise(&w)).unwrap_or_default()
                } else {
                    names.iter().enumerate().map(|(i, n)| if i == 0 { n.clone() } else { capitalise(n) }).collect()
                };
                key.push_str(&apply_case(&lower_first(&joined), *case));
            }
            Token::Year { short } => {
                if let Some(y) = bib.year {
                    let y = y.to_string();
                    key.push_str(if *short && y.len() == 4 { &y[2..] } else { &y });
                }
            }
            Token::Title { words, case } => {
                key.push_str(&apply_case(&title_part(&bib.title, *words), *case));
            }
            Token::Venue { words, case } => {
                key.push_str(&apply_case(&title_part(&bib.container, *words), *case));
            }
        }
    }
    let key: String = key.chars().filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.')).collect();
    if key.is_empty() { "ref".into() } else { key }
}

/// Adds a, b, … z, aa, ab… until the key is not taken.
pub fn make_unique(base: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(base) {
        return base.to_string();
    }
    for n in 0.. {
        let candidate = format!("{base}{}", suffix(n));
        if !taken(&candidate) {
            return candidate;
        }
    }
    unreachable!()
}

fn suffix(mut n: usize) -> String {
    // 0 → a, 25 → z, 26 → aa…
    let mut s = Vec::new();
    loop {
        s.push(b'a' + (n % 26) as u8);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    s.reverse();
    String::from_utf8(s).expect("ascii")
}

/// Keys worth replacing: numbers (IEEE Xplore's `10234847`), database ids,
/// DOIs and URLs, or anything with spaces or no letters.
pub fn looks_opaque(key: &str) -> bool {
    let key = key.trim();
    if key.is_empty() || key.len() > 48 || key.contains(char::is_whitespace) || key.contains("://") {
        return true;
    }
    if !key.chars().any(|c| c.is_ascii_alphabetic()) || key.starts_with("10.") {
        return true;
    }
    let lower = key.to_ascii_lowercase();
    if ["isi:", "wos:", "scopus", "pmid", "doi:", "doi_", "eid:", "ieee", "arxiv:", "ref_", "ref-", "item"]
        .iter()
        .any(|p| lower.starts_with(p))
    {
        return true;
    }
    // A run of six or more digits.
    let mut run = 0;
    for c in key.chars() {
        run = if c.is_ascii_digit() { run + 1 } else { 0 };
        if run >= 6 {
            return true;
        }
    }
    false
}

/// A key in the source file may be kept only if it is readable and valid.
pub fn is_valid_key(key: &str) -> bool {
    !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | ':' | '.' | '+' | '/'))
}

fn title_part(title: &str, n: usize) -> String {
    let words = significant_words(title);
    let picked: Vec<String> = if words.is_empty() {
        all_words(title).into_iter().take(n).collect()
    } else {
        words.into_iter().take(n).collect()
    };
    picked.iter().map(|w| capitalise(w)).collect()
}

/// Title words in ASCII, without maths, stop words or words that start with
/// a digit ("2-Bit", "5G"): those make poor reminders.
fn significant_words(text: &str) -> Vec<String> {
    all_words(text)
        .into_iter()
        .filter(|w| !STOP_WORDS.contains(&w.to_ascii_lowercase().as_str()))
        .filter(|w| !w.starts_with(|c: char| c.is_ascii_digit()))
        .collect()
}

fn all_words(text: &str) -> Vec<String> {
    let mut plain = String::new();
    let mut in_math = false;
    for ch in text.chars() {
        if ch == '$' {
            in_math = !in_math;
            plain.push(' ');
        } else if !in_math {
            plain.push(ch);
        }
    }
    deunicode(&plain)
        .split(|c: char| c.is_whitespace() || matches!(c, '/' | ':' | ';' | ',' | '.' | '(' | ')' | '"' | '?' | '!'))
        .map(|w| {
            // "mm-Wave" → "mmWave"
            w.split('-')
                .filter(|p| !p.is_empty())
                .enumerate()
                .map(|(i, p)| {
                    let p: String = p.chars().filter(char::is_ascii_alphanumeric).collect();
                    if i == 0 { p } else { capitalise(&p) }
                })
                .collect::<String>()
        })
        .filter(|w| !w.is_empty())
        .collect()
}

/// ASCII words of a name: "van der Berg" → ["van", "der", "Berg"].
fn words_of(name: &str) -> Vec<String> {
    deunicode(name)
        .split(|c: char| c.is_whitespace() || c == '-' || c == '\'')
        .map(|w| w.chars().filter(char::is_ascii_alphanumeric).collect::<String>())
        .filter(|w| !w.is_empty())
        .collect()
}

/// ["van", "der", "Berg"] → "vanDerBerg"; ["Zhang"] → "Zhang".
fn camel(words: &[String]) -> String {
    words.iter().enumerate().map(|(i, w)| if i == 0 { w.clone() } else { capitalise(w) }).collect()
}

fn capitalise(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// "Zhang" → "zhang" but keep "vanDerBerg" and acronyms like "IEEE" as they are.
fn lower_first(s: &str) -> String {
    if s.chars().filter(|c| c.is_ascii_alphabetic()).all(|c| c.is_ascii_uppercase()) {
        return s.to_string();
    }
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_lowercase().chain(c).collect(),
        None => String::new(),
    }
}

fn apply_case(s: &str, case: Case) -> String {
    match case {
        Case::AsIs => s.to_string(),
        Case::Lower => s.to_lowercase(),
        Case::Upper => s.to_uppercase(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Author;

    fn bib(family: &str, year: i32, title: &str) -> Biblio {
        Biblio {
            authors: vec![Author::new(family, "X.")],
            year: Some(year),
            title: title.into(),
            ..Default::default()
        }
    }

    #[test]
    fn default_pattern() {
        let b = bib("Zhang", 2025, "A Wideband 1-bit Reconfigurable Metasurface With Switchable Bidirectional Transmission");
        assert_eq!(generate(DEFAULT_PATTERN, &b), "zhang2025Wideband");
        let b = bib("Zhang", 2025, "A 2-Bit Wideband 5G mm-Wave RIS With Low Sidelobe Levels");
        assert_eq!(generate(DEFAULT_PATTERN, &b), "zhang2025Wideband");
        let b = bib("Oliveri", 2022, "Synthesis of Modular Electromagnetic Skins");
        assert_eq!(generate(DEFAULT_PATTERN, &b), "oliveri2022Synthesis");
    }

    #[test]
    fn particles_diacritics_and_organisations() {
        assert_eq!(generate("[auth][year]", &bib("van der Berg", 2020, "x")), "vanDerBerg2020");
        assert_eq!(generate("[auth:lower][year]", &bib("van der Berg", 2020, "x")), "vanderberg2020");
        assert_eq!(generate(DEFAULT_PATTERN, &bib("Müller", 2019, "Über Antennen")), "muller2019Uber");
        assert_eq!(generate("[auth][year]", &bib("O'Neill", 2019, "x")), "oNeill2019");
        let org = Biblio {
            title: "Study".into(),
            container: "IEEE Transactions on Antennas".into(),
            year: Some(2020),
            ..Default::default()
        };
        assert_eq!(generate(DEFAULT_PATTERN, &org), "ieee2020Study");
    }

    #[test]
    fn other_patterns() {
        let mut b = bib("Zhang", 2025, "Wideband Reconfigurable Intelligent Surface");
        b.authors.push(Author::new("Li", "Y."));
        b.container = "IEEE Transactions on Antennas and Propagation".into();
        assert_eq!(generate("[auth][title:3][year]", &b), "zhangWidebandReconfigurableIntelligent2025");
        assert_eq!(generate("[auth2]_[shortyear]", &b), "zhangLi_25");
        assert_eq!(generate("[auth:upper][year][venue:2]", &b), "ZHANG2025IEEETransactions");
        assert_eq!(generate("[auth][year][title:2:lower]", &b), "zhang2025widebandreconfigurable");
        assert_eq!(generate("[auth][year]", &Biblio::default()), "ref");
        // Maths is skipped, a broken pattern falls back to the default.
        assert_eq!(generate("[auth:lower][year][title:1]", &bib("Wu", 2021, "$\\mu$-Scale Arrays")), "wu2021Scale");
        assert_eq!(generate("[nope", &bib("Wu", 2021, "Arrays")), "wu2021Arrays");
    }

    #[test]
    fn validation() {
        assert!(validate(DEFAULT_PATTERN).is_ok());
        assert!(validate("[auth]_[year]").is_ok());
        assert!(validate("[author]").is_err());
        assert!(validate("[title:9]").is_err());
        assert!(validate("[auth").is_err());
        assert!(validate("plain").is_err());
    }

    #[test]
    fn unique_suffixes() {
        let taken = ["zhang2025Wideband", "zhang2025Widebanda"];
        assert_eq!(make_unique("zhang2025Wideband", |k| taken.contains(&k)), "zhang2025Widebandb");
        assert_eq!(make_unique("free", |_| false), "free");
        assert_eq!(suffix(0), "a");
        assert_eq!(suffix(25), "z");
        assert_eq!(suffix(26), "aa");
        assert_eq!(suffix(27), "ab");
    }

    #[test]
    fn opaque_keys() {
        for k in ["10234847", "9886689", "ISI:000123456", "WOS:000999", "10.1109/TAP.2025.3603900", "https://x.org/a", "has space", "ieee_10234847", "a123456"] {
            assert!(looks_opaque(k), "{k}");
        }
        for k in ["zhang2025Wideband", "Smith2020", "oliveri_skins_2022", "Li:2019"] {
            assert!(!looks_opaque(k), "{k}");
        }
    }
}
