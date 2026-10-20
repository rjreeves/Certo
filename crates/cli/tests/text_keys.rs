use std::{fs, process::Command};

// BACKLOG item 392 — Text compares by content in List.contains/distinct/groupBy and as Map keys.
fn run(src: &str) -> String {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.cto");
    fs::write(&file, src).unwrap();
    let exe = root.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(b.status.success(), "build failed: {}", String::from_utf8_lossy(&b.stderr));
    let r = Command::new(&exe).output().unwrap();
    assert!(r.status.success(), "run failed: {:?}", r.status);
    String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n")
}

#[test]
fn text_compares_by_content_in_lists_and_maps() {
    let out = run(r####"module P
fn main(): Unit [io] = {
  val words = Text.split("a b a c a b", " ")
  println(intToText(Map.len(List.groupBy(words, (w) => w))))
  println(intToText(List.len(List.distinct(words))))
  println(boolToText(List.contains(words, "c")))
  var counts: Map<Text, Int> = Map.empty()
  for w in words {
    counts = Map.insert(counts, w, (Map.get(counts, w) ?? 0) + 1)
  }
  println(intToText(Map.len(counts)))
  println(intToText(Map.get(counts, "a") ?? 0))
  println(intToText(Map.get(counts, Text.slice("xa", 1, 2)) ?? -1))
  println(intToText(Map.get(counts, "zz") ?? -1))
  println(boolToText(Map.contains(counts, "b")))
  val c2 = Map.remove(counts, "a")
  println(intToText(Map.len(c2)))
  println(intToText(Map.get(c2, "b") ?? -1))
  val nums = Map.insert(Map.insert(Map.empty(), 1, "x"), 2, "y")
  println(Map.get(nums, 2) ?? "none")
}
"####);
    assert_eq!(out, "3\n3\ntrue\n3\n3\n3\n-1\ntrue\n2\n2\ny\n");
}
