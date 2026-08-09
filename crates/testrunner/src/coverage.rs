//! Merge per-subprocess `.profraw` coverage files and produce a per-line
//! report keyed to the original `.cto` source (BACKLOG item 126).
//!
//! `llvm-cov`'s own file/line attribution is driven entirely by the
//! *physical* file it compiled — it ignores `#line` directives completely
//! (confirmed by direct testing against LLVM 22: neither `llvm-cov report`'s
//! file grouping nor `llvm-cov show`'s line numbers shift for a `#line`
//! pragma, even when the target file exists on disk). So instead of relying
//! on the compiler to do the remapping, this module re-derives the same
//! `#line` semantics itself from the exact `c_src` string that was compiled,
//! then remaps `llvm-cov export`'s per-line hit data through that table
//! before printing anything.

use std::path::Path;
use std::process::Command;

use crate::compile::find_llvm_tool;
use crate::error::TestRunnerError;

/// `remap[i]` (0-based index, i.e. `remap[line - 1]`) is the `.cto` file and
/// line number that generated-C physical line `line` (1-based) belongs to.
///
/// Unlike *real* C `#line` semantics (where each physical line after a
/// directive implicitly increments the logical line number), every physical
/// line here shares the *same* target until the next directive appears —
/// `crates/codegen/src/emit_mir.rs` only emits a fresh `#line` when the
/// underlying MIR statement's source line actually changes (see its
/// `last_line` dedup), so a run of generated C lines produced from one
/// Certo statement/expression all point at that one source line, not a
/// sequentially incrementing run of them.
///
/// The directive line itself, and any generated lines before the first
/// directive, map to `None` (synthetic runtime/harness code with no `.cto`
/// counterpart) — as does any run following the `<generated>` sentinel
/// filename, which `emit_mir.rs` emits to explicitly end a mapped run
/// (function boundary, or a synthetic unspanned statement) so attribution
/// doesn't bleed into unrelated trailing code.
fn build_line_remap(c_src: &str) -> Vec<Option<(String, u32)>> {
    let mut remap = Vec::new();
    let mut current: Option<(String, u32)> = None;
    for line in c_src.lines() {
        if let Some(rest) = line.trim_start().strip_prefix("#line ") {
            if let Some((num, file)) = parse_line_directive(rest) {
                current = if file == "<generated>" { None } else { Some((file, num)) };
                remap.push(None);
                continue;
            }
        }
        remap.push(current.clone());
    }
    remap
}

/// Parse the tail of a `#line` directive (everything after `"#line "`):
/// `123 "some\\path.cto"` -> `(123, "some\path.cto")`, undoing the
/// backslash/quote escaping `crates/codegen/src/emit_mir.rs`'s `LineMap`
/// applies when it writes the directive out.
fn parse_line_directive(rest: &str) -> Option<(u32, String)> {
    let rest = rest.trim();
    let space = rest.find(' ')?;
    let num: u32 = rest[..space].parse().ok()?;
    let quoted = rest[space..].trim();
    let inner = quoted.strip_prefix('"')?.strip_suffix('"')?;
    let unescaped = inner.replace("\\\"", "\"").replace("\\\\", "\\");
    Some((num, unescaped))
}

/// Merge every `*.profraw` file in `dir` into `<dir>/merged.profdata`, ask
/// `llvm-cov export` for per-line hit counts against `binary`, remap those
/// counts back to `c_src`'s original `.cto` lines via `build_line_remap`,
/// and return a human-readable per-line report. `c_src` must be the exact
/// string that was compiled into `binary` (so physical line numbers line
/// up) — `binary` must have been compiled with coverage instrumentation.
pub fn report(dir: &Path, binary: &Path, c_src: &str) -> Result<String, TestRunnerError> {
    let profraws: Vec<_> = std::fs::read_dir(dir)
        .map_err(|e| TestRunnerError::Io(e.to_string()))?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("profraw"))
        .collect();

    if profraws.is_empty() {
        return Err(TestRunnerError::Io(
            "no .profraw files were produced — no test in this file ran to completion".into(),
        ));
    }

    let merged = dir.join("merged.profdata");
    let profdata_tool = find_llvm_tool("llvm-profdata");
    // NOTE: `-o <file>` must come *after* the input files — this build of
    // llvm-profdata on Windows misparses `-o <file> <inputs...>`, silently
    // treating the output path as an extra (nonexistent) input and leaving
    // `merged` incomplete/empty. Confirmed by direct repro.
    let status = Command::new(&profdata_tool)
        .arg("merge")
        .arg("-sparse")
        .args(&profraws)
        .arg("-o").arg(&merged)
        .status()
        .map_err(|e| TestRunnerError::CompilerNotFound { compiler: profdata_tool.clone(), detail: e.to_string() })?;
    if !status.success() {
        return Err(TestRunnerError::Io(format!("{} failed to merge coverage data", profdata_tool)));
    }

    let cov_tool = find_llvm_tool("llvm-cov");
    // NOTE: `-instr-profile=<file>` (the `=` form) is silently misparsed by
    // this LLVM build too — must be passed as two separate args. Confirmed
    // by direct repro alongside the `-o` ordering issue above.
    let output = Command::new(&cov_tool)
        .arg("export")
        .arg(binary)
        .arg("-instr-profile").arg(&merged)
        .arg("-format=text")
        .output()
        .map_err(|e| TestRunnerError::CompilerNotFound { compiler: cov_tool.clone(), detail: e.to_string() })?;

    if !output.status.success() {
        return Err(TestRunnerError::Io(format!(
            "{} export failed: {}",
            cov_tool,
            String::from_utf8_lossy(&output.stderr),
        )));
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| TestRunnerError::Io(format!("failed to parse llvm-cov export output: {}", e)))?;

    render_report(&json, c_src)
}

/// Forward-fill `file["segments"]` into a per-physical-line execution count,
/// covering every line from the first region entry through `total_lines` —
/// see `render_report`'s doc comment for why this can't just read counts
/// off each segment's own line. `total_lines` must reach end-of-file (not
/// just the last segment's own line): a region's count applies until a
/// later segment supersedes it, and nothing guarantees the *last* function
/// in the file gets its own closing segment, so stopping early would
/// silently drop trailing code from the report.
fn line_active_counts(file: &serde_json::Value, total_lines: u32) -> std::collections::BTreeMap<u32, u64> {
    let mut segs: Vec<(u32, u32, u64, bool, bool)> = file["segments"].as_array()
        .map(|arr| arr.iter().filter_map(|seg| {
            let s = seg.as_array()?;
            if s.len() < 5 { return None; }
            Some((
                s[0].as_u64()? as u32,
                s[1].as_u64()? as u32,
                s[2].as_u64().unwrap_or(0),
                s[3].as_bool().unwrap_or(false),
                s[4].as_bool().unwrap_or(false),
            ))
        }).collect())
        .unwrap_or_default();
    segs.sort_by_key(|s| (s.0, s.1));

    let mut result = std::collections::BTreeMap::new();
    let mut active: Option<u64> = None;
    let mut i = 0;
    for line in 1..=total_lines {
        while i < segs.len() && segs[i].0 <= line {
            let (_, _, count, has_count, is_region_entry) = segs[i];
            if is_region_entry && has_count { active = Some(count); }
            i += 1;
        }
        if let Some(a) = active { result.insert(line, a); }
    }
    result
}

/// Walk the `llvm-cov export -format=text` JSON, forward-fill per generated-C
/// line via its `segments` array (`[line, col, count, hasCount,
/// isRegionEntry, isGapRegion]` — the same data `llvm-cov show` renders line
/// numbers from), remap each line through `build_line_remap`, and format a
/// `.cto`-keyed summary.
///
/// A `segments` entry only marks where an execution-count *region begins* —
/// it does not repeat itself for every line the region spans. So a whole
/// straight-line function body, say, gets exactly one entry (at its opening
/// brace) whose count applies to every line after it until the next entry.
/// `line_active_counts` below reproduces that forward-fill (the same
/// algorithm `llvm-cov show` uses to print a count next to *every* source
/// line, not just the ones with their own entry) — without it, lines
/// covered only by an earlier region's entry are silently dropped from the
/// `.cto`-side report instead of showing as covered.
fn render_report(json: &serde_json::Value, c_src: &str) -> Result<String, TestRunnerError> {
    let remap = build_line_remap(c_src);
    let files = json["data"][0]["files"].as_array().cloned().unwrap_or_default();
    let file = match files.first() { Some(f) => f, None => &serde_json::Value::Null };
    let c_line_counts = line_active_counts(file, remap.len() as u32);

    // Remap into per-.cto-file, per-line max counts.
    let mut per_file: std::collections::BTreeMap<String, std::collections::BTreeMap<u32, u64>> =
        std::collections::BTreeMap::new();
    for (c_line, count) in &c_line_counts {
        let idx = (*c_line as usize).wrapping_sub(1);
        if let Some(Some((file, cto_line))) = remap.get(idx) {
            let entry = per_file.entry(file.clone()).or_default().entry(*cto_line).or_insert(0);
            if *count > *entry { *entry = *count; }
        }
    }

    if per_file.is_empty() {
        return Ok(
            "no coverage data mapped back to a .cto source line (no #line-covered code ran)\n".to_string(),
        );
    }

    let mut out = String::new();
    for (file, lines) in &per_file {
        let total = lines.len();
        let covered = lines.values().filter(|&&c| c > 0).count();
        let percent = if total == 0 { 0.0 } else { 100.0 * covered as f64 / total as f64 };
        out.push_str(&format!("{}\n", file));
        out.push_str(&format!("  {}/{} lines covered ({:.1}%)\n", covered, total, percent));
        let missed: Vec<String> = lines.iter()
            .filter(|(_, &c)| c == 0)
            .map(|(line, _)| line.to_string())
            .collect();
        if !missed.is_empty() {
            out.push_str(&format!("  missed lines: {}\n", missed.join(", ")));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_line_directive_simple() {
        assert_eq!(parse_line_directive("3 \"foo.cto\""), Some((3, "foo.cto".to_string())));
    }

    #[test]
    fn parse_line_directive_unescapes_windows_path() {
        assert_eq!(
            parse_line_directive("12 \"C:\\\\Users\\\\bob\\\\cov_test.cto\""),
            Some((12, "C:\\Users\\bob\\cov_test.cto".to_string())),
        );
    }

    #[test]
    fn parse_line_directive_rejects_garbage() {
        assert_eq!(parse_line_directive("not a directive"), None);
    }

    #[test]
    fn build_line_remap_shares_line_across_consecutive_statements() {
        let c_src = "int f(void) {\n#line 6 \"a.cto\"\n    x = 1;\n    y = 2;\n}\n";
        let remap = build_line_remap(c_src);
        // line 1 (signature): no mapping yet
        assert_eq!(remap[0], None);
        // line 2 is the directive itself: no mapping
        assert_eq!(remap[1], None);
        // lines 3 and 4 both belong to cto line 6 (no auto-increment)
        assert_eq!(remap[2], Some(("a.cto".to_string(), 6)));
        assert_eq!(remap[3], Some(("a.cto".to_string(), 6)));
    }

    #[test]
    fn build_line_remap_generated_sentinel_resets() {
        let c_src = "#line 6 \"a.cto\"\n    x = 1;\n#line 1 \"<generated>\"\n    y = 2;\n";
        let remap = build_line_remap(c_src);
        assert_eq!(remap[1], Some(("a.cto".to_string(), 6)));
        assert_eq!(remap[2], None); // the reset directive line itself
        assert_eq!(remap[3], None); // back to untracked after the reset
    }

    #[test]
    fn line_active_counts_forward_fills_across_lines_without_their_own_entry() {
        // One region entry at line 1 (count 5); no entry at lines 2-3, so
        // they should inherit 5 too, matching how `llvm-cov show` renders.
        let file = json!({
            "segments": [
                [1, 1, 5, true, true, false],
                [4, 1, 0, true, true, false],
            ]
        });
        let counts = line_active_counts(&file, 4);
        assert_eq!(counts.get(&1), Some(&5));
        assert_eq!(counts.get(&2), Some(&5));
        assert_eq!(counts.get(&3), Some(&5));
        assert_eq!(counts.get(&4), Some(&0));
    }

    #[test]
    fn render_report_marks_uncalled_line_as_missed() {
        let c_src = "int f(void) {\n#line 3 \"a.cto\"\n    x = 1;\n}\nint g(void) {\n#line 8 \"a.cto\"\n    y = 2;\n}\n";
        // f() ran (count 1), g() never did (count 0).
        let json = json!({
            "data": [{
                "files": [{
                    "segments": [
                        [1, 1, 1, true, true, false],
                        [5, 1, 0, true, true, false],
                    ]
                }]
            }]
        });
        let report = render_report(&json, c_src).unwrap();
        assert!(report.contains("1/2 lines covered"));
        assert!(report.contains("missed lines: 8"));
        assert!(!report.contains("missed lines: 3"));
    }
}
