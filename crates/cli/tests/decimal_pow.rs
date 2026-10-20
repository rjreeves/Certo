use std::{fs, process::Command};

// BACKLOG item 422 — `Decimal ** Int` is exact repeated multiplication.
fn build(src: &str) -> (bool, String, String, std::process::Output) {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.cto");
    fs::write(&file, src).unwrap();
    let exe = root.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    let ok = b.status.success();
    let berr = String::from_utf8_lossy(&b.stderr).to_string();
    if !ok { return (false, String::new(), berr, b); }
    let r = Command::new(&exe).output().unwrap();
    (true, String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n"), String::from_utf8_lossy(&r.stderr).to_string(), r)
}

#[test]
fn decimal_to_an_int_power_is_exact() {
    let (ok, out, err, _) = build("module P\nfn main(): Unit [io] = {\n  println(Decimal.toText(d\"2.50\" ** 2))\n  println(Decimal.toText(d\"1.5\" ** 0))\n  println(Decimal.toText(d\"1.1\" ** 3))\n}\n");
    assert!(ok, "{err}");
    assert_eq!(out, "6.2500\n1\n1.331\n");
}

#[test]
fn decimal_negative_exponent_stops_with_a_message() {
    let (ok, _o, err, r) = build("module P\nfn main(): Unit [io] = {\n  val n = 0 - 1\n  println(Decimal.toText(d\"2.0\" ** n))\n}\n");
    assert!(ok, "{err}");
    assert!(!r.status.success());
    assert!(String::from_utf8_lossy(&r.stderr).contains("negative exponent"));
}

#[test]
fn decimal_to_a_decimal_power_is_a_type_error() {
    let (ok, _o, err, _) = build("module P\nfn main(): Unit [io] = {\n  println(Decimal.toText(d\"2.0\" ** d\"2\"))\n}\n");
    assert!(!ok && !err.contains("clang"), "{err}");
}
