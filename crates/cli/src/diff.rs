//! Minimal line-oriented unified diff, for `certo fmt --diff` (BACKLOG item
//! 158). No new dependency — a small LCS-based diff, self-contained like
//! this project's other hand-rolled implementations (e.g. `crates/diagnostics`'
//! own error rendering) rather than pulling in a crate for one CLI flag.

#[derive(Clone, Copy)]
enum DiffOp {
    Keep(usize),   // old line index (== new line index's content, by definition)
    Delete(usize), // old line index
    Insert(usize), // new line index
}

/// A unified diff between `old` and `new`, labeled with `path` in the
/// `---`/`+++` headers. Empty string if the two are line-for-line identical.
pub fn unified_diff(old: &str, new: &str, path: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let ops = diff_ops(&old_lines, &new_lines);
    if ops.iter().all(|op| matches!(op, DiffOp::Keep(..))) {
        return String::new();
    }
    render_unified(&old_lines, &new_lines, &ops, path)
}

/// Classic LCS dynamic-programming diff: build the LCS-length table, then
/// backtrack from (0,0) preferring a Keep whenever both lines match, and
/// otherwise following whichever side has the longer remaining LCS (the
/// standard construction for a minimal edit script).
fn diff_ops(old: &[&str], new: &[&str]) -> Vec<DiffOp> {
    let n = old.len();
    let m = new.len();
    let mut lcs = vec![vec![0u32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if old[i] == new[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }

    let mut ops = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old[i] == new[j] {
            ops.push(DiffOp::Keep(i));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            ops.push(DiffOp::Delete(i));
            i += 1;
        } else {
            ops.push(DiffOp::Insert(j));
            j += 1;
        }
    }
    while i < n {
        ops.push(DiffOp::Delete(i));
        i += 1;
    }
    while j < m {
        ops.push(DiffOp::Insert(j));
        j += 1;
    }
    ops
}

/// Render an edit script as unified-diff text: `CONTEXT` lines of
/// unchanged context around each change, nearby changes merged into one
/// hunk when their gap is within `2 * CONTEXT`.
fn render_unified(old: &[&str], new: &[&str], ops: &[DiffOp], path: &str) -> String {
    const CONTEXT: usize = 3;
    let n = ops.len();

    // Running old/new line-count cursors *before* each op index, so a
    // hunk's starting line number is correct regardless of which op type
    // (Keep/Delete/Insert) happens to open it.
    let mut old_before = vec![0usize; n + 1];
    let mut new_before = vec![0usize; n + 1];
    for (i, op) in ops.iter().enumerate() {
        let (d_old, d_new) = match op {
            DiffOp::Keep(..) => (1, 1),
            DiffOp::Delete(_) => (1, 0),
            DiffOp::Insert(_) => (0, 1),
        };
        old_before[i + 1] = old_before[i] + d_old;
        new_before[i + 1] = new_before[i] + d_new;
    }

    // Mark every op within CONTEXT of a real change as part of some hunk,
    // then merge adjacent/overlapping marked runs into hunks.
    let mut included = vec![false; n];
    for (i, op) in ops.iter().enumerate() {
        if !matches!(op, DiffOp::Keep(..)) {
            let lo = i.saturating_sub(CONTEXT);
            let hi = (i + CONTEXT + 1).min(n);
            included[lo..hi].fill(true);
        }
    }
    let mut hunk_ranges: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < n {
        if included[i] {
            let start = i;
            while i < n && included[i] { i += 1; }
            hunk_ranges.push((start, i));
        } else {
            i += 1;
        }
    }

    let mut out = format!("--- a/{path}\n+++ b/{path}\n");
    for (start, end) in hunk_ranges {
        let old_start = old_before[start];
        let new_start = new_before[start];
        let old_count = old_before[end] - old_start;
        let new_count = new_before[end] - new_start;
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            old_start + 1, old_count, new_start + 1, new_count
        ));
        for op in &ops[start..end] {
            match op {
                DiffOp::Keep(oi)   => out.push_str(&format!(" {}\n", old[*oi])),
                DiffOp::Delete(oi)  => out.push_str(&format!("-{}\n", old[*oi])),
                DiffOp::Insert(ni)  => out.push_str(&format!("+{}\n", new[*ni])),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_input_produces_no_diff() {
        assert_eq!(unified_diff("a\nb\nc\n", "a\nb\nc\n", "f.cto"), "");
    }

    #[test]
    fn single_line_change_shows_up_as_del_and_add() {
        let d = unified_diff("a\nb\nc\n", "a\nB\nc\n", "f.cto");
        assert!(d.contains("--- a/f.cto"));
        assert!(d.contains("+++ b/f.cto"));
        assert!(d.contains("-b"));
        assert!(d.contains("+B"));
        assert!(d.contains(" a")); // unchanged context line
        assert!(d.contains(" c"));
    }

    #[test]
    fn pure_insertion_is_shown() {
        let d = unified_diff("a\nc\n", "a\nb\nc\n", "f.cto");
        assert!(d.contains("+b"));
        assert!(!d.contains("-a"));
        assert!(!d.contains("-c"));
    }

    #[test]
    fn pure_deletion_is_shown() {
        let d = unified_diff("a\nb\nc\n", "a\nc\n", "f.cto");
        assert!(d.contains("-b"));
    }

    #[test]
    fn distant_changes_produce_separate_hunks() {
        // Two single-line changes far enough apart (> 2*CONTEXT unchanged
        // lines between them) must not be merged into one giant hunk.
        let old = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\n14\n15\n";
        let new = "1\nX\n3\n4\n5\n6\n7\n8\n9\n10\n11\n12\n13\nY\n15\n";
        let d = unified_diff(old, new, "f.cto");
        assert_eq!(d.matches("@@").count(), 4, "expected two hunks (two @@ markers each): {d}");
    }

    #[test]
    fn nearby_changes_merge_into_one_hunk() {
        let old = "1\n2\n3\n4\n5\n";
        let new = "1\nX\n3\nY\n5\n";
        let d = unified_diff(old, new, "f.cto");
        assert_eq!(d.matches("@@").count(), 2, "expected one merged hunk: {d}");
    }
}
