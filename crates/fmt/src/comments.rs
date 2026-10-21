//! Putting comments back into formatted source (BACKLOG item 445).
//!
//! The lexer skips `//` comments, so the AST the formatter prints from has none. Instead of
//! threading trivia through the parser, the comments are read from the gaps between the source's
//! tokens, the formatted text is lexed too, the two token streams are aligned, and each comment is
//! re-attached to the line that holds the token it sat next to: a comment on its own line goes
//! above the line of the token that followed it, a comment after code goes at the end of the line of
//! the token that preceded it.

use certo_lexer::{lex, Spanned};

struct Comment {
    text: String,
    /// Written after code on the same line.
    trailing: bool,
    /// Index of the source token before / after it.
    prev: Option<usize>,
    next: Option<usize>,
}

fn extract(src: &str, toks: &[Spanned<'_>]) -> Vec<Comment> {
    let mut out = Vec::new();
    let mut gap_start = 0usize;
    let mut scan = |gap: &str, base: usize, prev: Option<usize>, next: Option<usize>, out: &mut Vec<Comment>| {
        let _ = base;
        let mut seen_newline = false;
        let mut rest = gap;
        while !rest.is_empty() {
            if let Some(stripped) = rest.strip_prefix("//") {
                let end = stripped.find('\n').unwrap_or(stripped.len());
                let text = format!("//{}", stripped[..end].trim_end());
                out.push(Comment { text, trailing: prev.is_some() && !seen_newline, prev, next });
                rest = &stripped[end..];
            } else {
                let c = rest.chars().next().unwrap();
                if c == '\n' { seen_newline = true; }
                rest = &rest[c.len_utf8()..];
            }
        }
    };
    for (i, t) in toks.iter().enumerate() {
        scan(&src[gap_start..t.span.start], gap_start, if i > 0 { Some(i - 1) } else { None }, Some(i), &mut out);
        gap_start = t.span.end;
    }
    let last = toks.len().checked_sub(1);
    scan(&src[gap_start..], gap_start, last, None, &mut out);
    out
}

/// For each source token, the output token it became (matching text, tolerating a few tokens the
/// formatter added or dropped).
fn align(s: &[Spanned<'_>], o: &[Spanned<'_>]) -> Vec<Option<usize>> {
    let mut map = vec![None; s.len()];
    let (mut i, mut j) = (0usize, 0usize);
    while i < s.len() && j < o.len() {
        if s[i].token == o[j].token {
            map[i] = Some(j);
            i += 1;
            j += 1;
            continue;
        }
        let mut resynced = false;
        for k in 1..=24 {
            if j + k < o.len() && s[i].token == o[j + k].token { j += k; resynced = true; break; }
            if i + k < s.len() && s[i + k].token == o[j].token { i += k; resynced = true; break; }
        }
        if !resynced { i += 1; j += 1; }
    }
    map
}

/// `formatted` with the comments of `src` put back. If either text does not lex, `formatted` is
/// returned as it is.
pub fn restore(src: &str, formatted: String) -> String {
    let Ok(stoks) = lex(src) else { return formatted };
    let comments = extract(src, &stoks);
    if comments.is_empty() { return formatted; }
    let Ok(otoks) = lex(&formatted) else { return formatted };
    let map = align(&stoks, &otoks);

    let lines: Vec<&str> = formatted.split('\n').collect();
    // Byte offset of each line start, to turn a token offset into a line number.
    let mut starts = Vec::with_capacity(lines.len());
    let mut pos = 0usize;
    for l in &lines { starts.push(pos); pos += l.len() + 1; }
    let line_of = |off: usize| match starts.binary_search(&off) { Ok(i) => i, Err(i) => i.saturating_sub(1) };

    let mut before: Vec<Vec<String>> = vec![Vec::new(); lines.len()];
    let mut after: Vec<Option<String>> = vec![None; lines.len()];
    let mut tail: Vec<String> = Vec::new();

    // The output token a source token became, or the nearest following / preceding one.
    let out_tok = |idx: usize, forward: bool| -> Option<usize> {
        if forward { (idx..map.len()).find_map(|k| map[k]) } else { (0..=idx).rev().find_map(|k| map[k]) }
    };

    for c in &comments {
        if c.trailing {
            if let Some(q) = c.prev.and_then(|p| out_tok(p, false)) {
                let ln = line_of(otoks[q].span.end.saturating_sub(1));
                match &mut after[ln] {
                    Some(existing) => { existing.push(' '); existing.push_str(&c.text); }
                    slot => *slot = Some(c.text.clone()),
                }
                continue;
            }
        }
        match c.next.and_then(|n| out_tok(n, true)) {
            Some(q) => {
                let ln = line_of(otoks[q].span.start);
                let line = lines[ln];
                let mut indent: String = line.chars().take_while(|ch| *ch == ' ').collect();
                // A comment before a closing brace belongs to the block's body.
                if line.trim_start().starts_with('}') { indent.push_str("    "); }
                before[ln].push(format!("{indent}{}", c.text));
            }
            None => tail.push(c.text.clone()),
        }
    }

    let mut out: Vec<String> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        out.extend(before[i].iter().cloned());
        match &after[i] {
            Some(c) => out.push(format!("{line} {c}")),
            None => out.push((*line).to_string()),
        }
    }
    if !tail.is_empty() {
        // After the code, before the trailing newline `format_source` always ends with.
        while out.last().is_some_and(|l| l.is_empty()) { out.pop(); }
        out.push(String::new());
        out.extend(tail);
        out.push(String::new());
    }
    out.join("\n")
}
