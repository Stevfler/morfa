//! The search syntax, shared by the live walk and the index so the two agree on what matches:
//! every word must appear in the name; `*.pdf` / `ext:pdf` filter by extension.
use unicode_normalization::{char::compose, UnicodeNormalization};

/// Case-folded, decomposed. A typed composed letter matches a decomposed name (usual on macOS) and
/// the other way round, and a base letter alone still matches an accented one: `cafe` finds `café`,
/// `е` finds `ё`. Marks that make a separate letter stay attached, so `и` doesn't find `й`, nor `は` find `ぱ`.
pub fn fold(s: &str) -> String {
    let low = s.to_lowercase();
    if low.is_ascii() {
        return low;
    }
    let mut out = String::with_capacity(low.len());
    let mut held: Option<char> = None;
    for c in low.nfd() {
        if let Some(whole) = held.filter(|&b| separate_letter(b, c)).and_then(|b| compose(b, c)) {
            held = Some(whole);
            continue;
        }
        // Accents go, in names and queries alike, so `елка` finds `ёлка` mid-word too. Only this block:
        // elsewhere (Indic, Hebrew, Arabic) combining marks can change what the letter is.
        if matches!(c, '\u{0300}'..='\u{036F}') {
            continue;
        }
        out.extend(held.replace(c));
    }
    out.extend(held);
    out
}

/// Base and mark that together are a letter of their own: Cyrillic й, ў, ї and kana voicing.
fn separate_letter(base: char, mark: char) -> bool {
    matches!((base, mark), ('\u{0438}' | '\u{0443}', '\u{0306}') | ('\u{0456}', '\u{0308}')) || (matches!(mark, '\u{3099}' | '\u{309A}') && matches!(base, '\u{3040}'..='\u{30FF}'))
}

/// `fold` of a raw name. Non-UTF-8 bytes become U+FFFD, like the live walk's `to_string_lossy`.
pub fn fold_bytes(raw: &[u8]) -> Vec<u8> {
    if raw.is_ascii() {
        raw.to_ascii_lowercase()
    } else {
        fold(&String::from_utf8_lossy(raw)).into_bytes()
    }
}

pub struct Query {
    pub words: Vec<Vec<u8>>,
    pub exts: Vec<Vec<u8>>,
}

impl Query {
    pub fn parse(query: &str) -> Query {
        let (mut words, mut exts) = (vec![], vec![]);
        for t in fold(query).split_whitespace() {
            if let Some(e) = t.strip_prefix("*.").or_else(|| t.strip_prefix("ext:")) {
                exts.push(format!(".{e}").into_bytes());
            } else {
                words.push(t.as_bytes().to_vec());
            }
        }
        Query { words, exts }
    }

    /// `name` is already folded.
    pub fn matches_folded(&self, name: &[u8]) -> bool {
        self.words.iter().all(|w| find(name, w).is_some()) && (self.exts.is_empty() || self.exts.iter().any(|e| name.ends_with(e)))
    }

    pub fn matches(&self, name: &str) -> bool {
        self.matches_folded(fold(name).as_bytes())
    }

    /// Lower is better: whole name or stem, then prefix, then a word start, then anywhere.
    pub fn tier(&self, name: &[u8]) -> u8 {
        let stem = name.iter().rposition(|&b| b == b'.').filter(|&i| i > 0).map_or(name, |i| &name[..i]);
        self.words.iter().fold(0u8, |sum, w| {
            let t = if name == &w[..] || stem == &w[..] {
                0
            } else if name.starts_with(w) {
                1
            } else if boundary_hit(name, w) {
                2
            } else {
                3
            };
            sum.saturating_add(t)
        })
    }
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    memchr::memmem::find(hay, needle)
}

fn boundary_hit(name: &[u8], w: &[u8]) -> bool {
    memchr::memmem::find_iter(name, w).any(|i| i > 0 && !name[i - 1].is_ascii_alphanumeric() && name[i - 1] < 0x80)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_syntax_as_before() {
        let q = Query::parse("Report *.PDF");
        assert!(q.matches("Annual report.pdf"));
        assert!(!q.matches("Annual report.txt"));
        assert!(!q.matches("notes.pdf"));
        assert!(Query::parse("ext:rs").matches("main.RS"));
        assert!(Query::parse("*.a *.b").matches("x.b"), "extensions are alternatives");
    }

    #[test]
    fn decomposed_and_cyrillic_names_match() {
        assert!(Query::parse("й").matches("Мой\u{0438}\u{0306}.txt"));
        assert!(Query::parse("\u{0438}\u{0306}").matches("мой.txt"));
        assert!(Query::parse("ОТЧЁТ").matches("отчёт.pdf"));
        for name in ["cafe\u{0301}.txt", "caf\u{e9}.txt"] {
            assert!(Query::parse("cafe").matches(name), "base letters");
            assert!(Query::parse("caf\u{e9}").matches(name), "composed query");
            assert!(Query::parse("cafe\u{0301}").matches(name), "decomposed query");
        }
    }

    #[test]
    fn marks_that_make_a_letter_stay_apart() {
        for name in ["\u{0439}\u{043E}\u{0434}", "\u{0438}\u{0306}\u{043E}\u{0434}"] {
            assert!(!Query::parse("\u{0438}").matches(name), "\u{0438} is not \u{0439}");
            assert!(Query::parse("\u{0439}").matches(name));
        }
        for name in ["\u{3071}", "\u{306F}\u{309A}", "\u{3070}", "\u{306F}\u{3099}"] {
            assert!(!Query::parse("\u{306F}").matches(name), "kana voicing");
        }
        assert!(Query::parse("\u{3071}").matches("\u{306F}\u{309A}"));
        assert!(Query::parse("\u{30D1}").matches("\u{30CF}\u{309A}"));
        for name in ["\u{0451}\u{043B}\u{043A}\u{0430}", "\u{0435}\u{0308}\u{043B}\u{043A}\u{0430}"] {
            assert!(Query::parse("\u{0435}\u{043B}").matches(name), "\u{0435} finds \u{0451}");
            assert!(Query::parse("\u{0451}").matches(name));
        }
    }

    #[test]
    fn invalid_utf8_folds_like_lossy() {
        assert_eq!(fold_bytes(b"A\xFFb"), "a\u{FFFD}b".as_bytes());
    }

    #[test]
    fn tiers_order_name_matches() {
        let q = Query::parse("note");
        let t = |n: &str| q.tier(n.as_bytes());
        assert!(t("note.txt") < t("notebook.txt"));
        assert!(t("notebook.txt") < t("my-note.txt"));
        assert!(t("my-note.txt") < t("keynote.txt"));
    }
}
