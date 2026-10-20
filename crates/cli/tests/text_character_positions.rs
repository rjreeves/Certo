use std::{fs, process::Command};

// BACKLOG item 402 — `Text.length` and `Text.charAt` counted characters while
// `Text.slice`, `Text.indexOf` and `Text.split(s, "")` counted bytes, so for
// non-ASCII text `Text.slice(s, 0, Text.length(s))` dropped the last character
// and a slice could cut a multi-byte character in half. All of them now use
// character positions. `Text.len`/`Text.byteLength` stay byte counts.

fn run(src: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("t.cto");
    fs::write(&file, src).unwrap();
    let exe = dir.path().join(if cfg!(windows) { "t.exe" } else { "t" });
    let build = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(build.status.success(), "{}{}", String::from_utf8_lossy(&build.stdout), String::from_utf8_lossy(&build.stderr));
    let out = Command::new(&exe).output().unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

#[test]
fn slicing_the_whole_text_by_its_character_length_returns_all_of_it() {
    let out = run(r#"module T
import Stdlib.Core

fn main(): Unit [io] = {
    val s = "héllo"
    println(intToText(Text.length(s)))
    println(Text.slice(s, 0, Text.length(s)))
    println(Text.slice(s, 1, 3))
    println(Text.slice(s, 2, 4))
    println(Text.slice(s, 4, 99))
    println(Text.slice(s, 3, 3) ++ "|" ++ Text.slice(s, 4, 2) ++ "|" ++ Text.slice(s, -5, 2))
}
"#);
    assert_eq!(out, "5\nhéllo\nél\nll\no\n||hé\n");
}

#[test]
fn index_of_returns_a_character_position_that_slice_understands() {
    let out = run(r#"module T
import Stdlib.Core

fn main(): Unit [io] = {
    val s = "héllo"
    println(intToText(Text.indexOf(s, "llo") ?? -1))
    println(intToText(Text.indexOf(s, "é") ?? -1))
    println(intToText(Text.indexOf(s, "z") ?? -1))
    val cjk = "日本語テキスト"
    println(intToText(Text.indexOf(cjk, "テ") ?? -1))
    println(Text.slice(cjk, 2, 5))
    val emoji = "a🙂b"
    val at = Text.indexOf(emoji, "b") ?? -1
    println(intToText(at) ++ " " ++ Text.slice(emoji, 1, at))
}
"#);
    assert_eq!(out, "2\n1\n-1\n3\n語テキ\n2 🙂\n");
}

#[test]
fn splitting_on_the_empty_separator_gives_one_entry_per_character() {
    let out = run(r#"module T
import Stdlib.Core

fn main(): Unit [io] = {
    val parts = Text.split("héy", "")
    println(intToText(List.len(parts)))
    for p in parts {
        println("[" ++ p ++ "]")
    }
    println(intToText(List.len(Text.split("日本", ""))))
    println(intToText(List.len(Text.split("", ""))))
}
"#);
    assert_eq!(out, "3\n[h]\n[é]\n[y]\n2\n0\n");
}

#[test]
fn byte_counts_and_ascii_text_are_unchanged() {
    let out = run(r#"module T
import Stdlib.Core

fn main(): Unit [io] = {
    println(intToText(Text.len("héllo")))
    println(intToText(Text.byteLength("日本")))
    println(Text.slice("hello world", 6, 11))
    println(intToText(Text.indexOf("hello world", "o w") ?? -1))
    println(intToText(List.len(Text.split("a,b,c", ","))))
    println(Text.slice("abc", 1, Text.len("abc")))
}
"#);
    assert_eq!(out, "6\n6\nworld\n4\n3\nbc\n");
}
