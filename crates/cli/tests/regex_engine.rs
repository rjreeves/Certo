use std::{fs, process::Command};

// BACKLOG item 401 — the Stdlib.Regex engine is a real compiled backtracking matcher:
// alternation, group quantifiers, escapes, POSIX classes, and invalid patterns stop the
// program with a message instead of crashing.
fn run(src: &str) -> (bool, String, String) {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.cto");
    fs::write(&file, src).unwrap();
    let exe = root.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let build = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(build.status.success(), "build failed: {}", String::from_utf8_lossy(&build.stderr));
    let out = Command::new(&exe).output().unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n"),
        String::from_utf8_lossy(&out.stderr).replace("\r\n", "\n"),
    )
}

#[test]
fn regex_supports_alternation_groups_escapes_and_classes() {
    let (ok, out, err) = run(r####"module P
fn b(v: Bool): Text = if v then "T" else "F"

fn main(): Unit [io] = {
  println(b(Regex.match("cat|dog", "hotdog")))
  println(b(Regex.match("^(cat|dog)$", "dog")))
  println(b(Regex.match("^(cat|dog)$", "dogs")))
  println(b(Regex.match("^(ab)+c$", "ababc")))
  println(b(Regex.match("^(ab)+c$", "abac")))
  println(Regex.find("\\d+", "order 4521 shipped"))
  println(b(Regex.match("a\\.b", "a.b")))
  println(b(Regex.match("a\\.b", "axb")))
  println(Regex.find("[[:digit:]]+", "ab 77 cd"))
  println(Regex.find("\\w+@\\w+\\.com", "mail bob@site.com now"))
  println(Regex.find("x{2,3}", "axxxxb"))
  println(Regex.find("<.+?>", "<a><b>"))
  println(Regex.find("<.+>", "<a><b>"))
  println(b(Regex.match("\\bcat\\b", "a cat sat")))
  println(b(Regex.match("\\bcat\\b", "concat")))
  println(b(Regex.match("^$", "")))
  println(b(Regex.match("a{", "a{")))
  println(b(Regex.match("é+", "caféé")))
  println(Regex.find(".", "éa"))
}
"####);
    assert!(ok, "{err}");
    let want = "T\nT\nF\nT\nF\n4521\nT\nF\n77\nbob@site.com\nxxx\n<a>\n<a><b>\nT\nF\nT\nT\nT\né\n";
    assert_eq!(out, want);
}

#[test]
fn regex_captures_replace_and_split() {
    let (ok, out, err) = run(r####"module P
fn show(xs: List<Text>): Text = "[" ++ Text.join(xs, "|") ++ "]"

fn main(): Unit [io] = {
  println(show(Regex.captures("(\\w+)@(\\w+)", "bob@site x")))
  println(show(Regex.captures("(a)|(b)", "b")))
  println(show(Regex.captures("(a)(b)?", "a")))
  println(show(Regex.captures("(?:ab)+(c)", "ababc")))
  println(show(Regex.captures("(x)", "no")))
  println(Regex.replace("[0-9]+", "a1b22", "#"))
  println(Regex.replace("z", "abc", "#"))
  println(show(Regex.split(",\\s*", "a, b,c")))
  println(show(Regex.split("[0-9]", "a1b2")))
  println(show(Regex.split("x", "")))
  println(show(Regex.split("", "abc")))
  println(show(Regex.split("a*", "baac")))
}
"####);
    assert!(ok, "{err}");
    let want = "[bob|site]\n[|b]\n[a|]\n[c]\n[]\na#b22\nabc\n[a|b|c]\n[a|b|]\n[]\n[a|b|c]\n[b|c]\n";
    assert_eq!(out, want);
}

#[test]
fn regex_invalid_patterns_stop_with_a_message_not_a_crash() {
    for (pat, needle) in [
        ("(", "unmatched ("),
        (")", "unmatched )"),
        ("[abc", "unterminated ["),
        ("*a", "nothing to repeat"),
        ("(a)\\\\1", "backreferences"),
        ("a{3,2}", "out of order"),
        ("a{5000}", "too large"),
    ] {
        let (ok, _out, err) = run(&format!(r####"module P
fn main(): Unit [io] = {{
  println(Regex.find("{pat}", "abc"))
}}
"####));
        assert!(!ok, "pattern {pat} should stop the program");
        assert!(err.contains("invalid regular expression") && err.contains(needle), "pattern {pat}: {err}");
    }
}

#[test]
fn regex_pathological_pattern_hits_the_step_limit_instead_of_hanging() {
    let (ok, _out, err) = run(r####"module P
fn main(): Unit [io] = {
  println(Regex.find("(a*)*b", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaac"))
  println(Regex.find("(a|aa)+$", "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaab"))
}
"####);
    // Either it finishes (nothing to find) or stops with the documented message; it must not hang.
    assert!(ok || err.contains("step limit"), "{err}");
}
