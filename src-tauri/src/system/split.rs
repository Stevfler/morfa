/// Split a command line shell-style: whitespace separates words, single and
/// double quotes group, backslash escapes the next char (outside single quotes).
/// No expansion of variables or globs.
#[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
pub fn split_command(s: &str) -> Vec<String> {
    let mut out = vec![];
    let mut cur = String::new();
    let mut in_word = false;
    let mut quote: Option<char> = None;
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (Some('"') | None, '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
                in_word = true;
            }
            (Some(_), c) => cur.push(c),
            (None, '\'' | '"') => {
                quote = Some(c);
                in_word = true;
            }
            (None, c) if c.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            (None, c) => {
                cur.push(c);
                in_word = true;
            }
        }
    }
    if in_word {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::split_command as s;

    #[test]
    fn splits() {
        assert_eq!(s("kitty"), ["kitty"]);
        assert_eq!(s("  alacritty   -o  font.size=12 "), ["alacritty", "-o", "font.size=12"]);
        assert_eq!(s(r#"my\ term --title "A B" 'c "d"'"#), ["my term", "--title", "A B", "c \"d\""]);
        assert_eq!(s("x ''"), ["x", ""]);
        assert!(s("   ").is_empty());
    }
}
