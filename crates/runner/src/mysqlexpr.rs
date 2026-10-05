//! MySQL's spelling of column defaults and CHECK bodies, brought to the PostgreSQL-flavoured text the rest of the runner
//! understands (`drift::normalize`, `pgexpr::parse`): backtick identifiers become double-quoted, string literals lose their
//! character-set introducers and MySQL's backslash escapes, and a few functions get the name SDL knows them by
//! (`char_length` is `length`, `utc_timestamp(6)` is `now()`, ...).
//!
//! `information_schema` hands back expression text escaped one more time (`_utf8mb4\'user\'`): [`from_server`] undoes that
//! first; [`from_render`] is for text this crate wrote itself (`certo_sql::render_expr`).

/// Text read from `information_schema`.
pub fn from_server(text: &str) -> String {
    from_render(&unescape_once(text))
}

/// `\'` is `'` and `\\` is `\`: the server escapes the whole expression as if it were a string.
fn unescape_once(s: &str) -> String {
    let mut out = String::new();
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\\' && matches!(it.peek(), Some('\'' | '\\' | '"')) {
            out.push(it.next().unwrap());
        } else {
            out.push(c);
        }
    }
    out
}

/// MySQL SQL text to PostgreSQL-flavoured text (see the module comment).
pub fn from_render(text: &str) -> String {
    let c: Vec<char> = text.chars().collect();
    let mut out = String::new();
    let mut i = 0;
    while i < c.len() {
        match c[i] {
            '`' => {
                i += 1;
                let mut v = String::new();
                while i < c.len() {
                    if c[i] == '`' {
                        if c.get(i + 1) == Some(&'`') {
                            v.push('`');
                            i += 2;
                            continue;
                        }
                        i += 1;
                        break;
                    }
                    v.push(c[i]);
                    i += 1;
                }
                out.push_str(&format!("\"{}\"", v.replace('"', "\"\"")));
            }
            '\'' => {
                i += 1;
                let mut v = String::new();
                while i < c.len() {
                    match c[i] {
                        '\'' if c.get(i + 1) == Some(&'\'') => {
                            v.push('\'');
                            i += 2;
                        }
                        '\'' => {
                            i += 1;
                            break;
                        }
                        '\\' if i + 1 < c.len() => {
                            v.push(match c[i + 1] {
                                'n' => '\n',
                                't' => '\t',
                                'r' => '\r',
                                '0' => '\0',
                                other => other,
                            });
                            i += 2;
                        }
                        ch => {
                            v.push(ch);
                            i += 1;
                        }
                    }
                }
                out.push_str(&format!("'{}'", v.replace('\'', "''")));
            }
            ch if ch.is_alphabetic() || ch == '_' => {
                let start = i;
                while i < c.len() && (c[i].is_alphanumeric() || c[i] == '_' || c[i] == '$') {
                    i += 1;
                }
                let word: String = c[start..i].iter().collect();
                // a character-set introducer: `_utf8mb4'text'`
                if word.starts_with('_') && word.len() > 1 && c.get(i) == Some(&'\'') {
                    continue;
                }
                let lower = word.to_lowercase();
                let call = c.get(i) == Some(&'(');
                match (lower.as_str(), call) {
                    ("utc_timestamp" | "current_timestamp" | "now" | "localtimestamp", _) => {
                        out.push_str("now()");
                        i = skip_args(&c, i);
                    }
                    ("utc_date" | "curdate" | "current_date", _) => {
                        out.push_str("today()");
                        i = skip_args(&c, i);
                    }
                    ("char_length" | "character_length", true) => out.push_str("length"),
                    _ => out.push_str(&word),
                }
            }
            ch => {
                out.push(ch);
                i += 1;
            }
        }
    }
    out
}

/// After a function name: skip `()` or `(6)` if there is one.
fn skip_args(c: &[char], mut i: usize) -> usize {
    if c.get(i) == Some(&'(') {
        let mut depth = 0;
        while i < c.len() {
            match c[i] {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 {
                        return i + 1;
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_server_text_becomes_postgres_flavoured() {
        assert_eq!(from_server(r"_utf8mb4\'user\'"), "'user'");
        assert_eq!(from_server(r"_utf8mb4\'it\\\'s\'"), "'it''s'");
        assert_eq!(from_server("(`n` >= 0)"), "(\"n\" >= 0)");
        assert_eq!(from_server(r"((`n` >= 0) and ((`name` <> _utf8mb4\'bad\') or (`n` in (1,2))))"), "((\"n\" >= 0) and ((\"name\" <> 'bad') or (\"n\" in (1,2))))");
        assert_eq!(from_server("utc_timestamp(6)"), "now()");
        assert_eq!(from_server("utc_date()"), "today()");
        assert_eq!(from_server("(char_length(`name`) > 0)"), "(length(\"name\") > 0)");
        assert_eq!(from_server("-(5)"), "-(5)");
    }

    #[test]
    fn rendered_text_gets_the_same_treatment() {
        assert_eq!(from_render("(`a` = 'x\\\\y')"), "(\"a\" = 'x\\y')");
        assert_eq!(from_render("UTC_TIMESTAMP(6)"), "now()");
        assert_eq!(from_render("CHAR_LENGTH(`n`)"), "length(\"n\")");
        assert_eq!(from_render("CONCAT(`a`, 'it''s')"), "CONCAT(\"a\", 'it''s')");
    }
}
