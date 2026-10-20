use std::{fs, process::Command};

// BACKLOG item 367 — integer `/` and `%` by zero stop with a message (after flushing stdout)
// instead of trapping or printing garbage.
fn build_run(src: &str) -> std::process::Output {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.cto");
    fs::write(&file, src).unwrap();
    let exe = root.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(b.status.success(), "build failed: {}", String::from_utf8_lossy(&b.stderr));
    Command::new(&exe).output().unwrap()
}

#[test]
fn division_and_remainder_still_work() {
    let r = build_run("module P\nfn main(): Unit [io] = {\n  val a = 17\n  val b = 5\n  println(intToText(a / b) ++ \" \" ++ intToText(a % b) ++ \" \" ++ intToText(0 - a / b) ++ \" \" ++ intToText(0 - a % b))\n  println(floatToText(7.0 / 2.0))\n}\n");
    assert!(r.status.success());
    assert_eq!(String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n"), "3 2 -3 -2\n3.5\n");
}

#[test]
fn division_by_zero_stops_with_a_message_after_flushing_output() {
    for (op, word) in [("/", "division"), ("%", "remainder")] {
        let r = build_run(&format!("module P\nfn main(): Unit [io] = {{\n  println(\"before\")\n  val n = 0\n  println(intToText(10 {op} n))\n  println(\"after\")\n}}\n"));
        assert!(!r.status.success());
        let out = String::from_utf8_lossy(&r.stdout);
        let err = String::from_utf8_lossy(&r.stderr);
        assert!(out.contains("before") && !out.contains("after"), "{out}");
        assert!(err.contains(&format!("integer {word} by zero")), "{err}");
    }
}
