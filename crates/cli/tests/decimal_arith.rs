use std::{fs, process::Command};

// BACKLOG items 386/387 — Decimal text keeps the sign below 1, arithmetic panics on overflow
// instead of wrapping, division adapts its precision, round handles negatives and negative places.
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
fn decimal_text_arithmetic_and_rounding() {
    let r = build_run(r####"module P
fn t(d: Decimal): Unit [io] = println(Decimal.toText(d))
fn main(): Unit [io] = {
  t(d"-0.05")
  t(Money.fromCents(-5))
  t(Decimal.sub(d"0.10", d"0.15"))
  t(Decimal.mul(d"-0.5", d"0.5"))
  t(d"-1.05")
  t(d"0.007")
  t(d"12")
  t(d"100.00" * d"100.00")
  t(d"1000000000000.00" / d"3")
  t(d"10.00" / d"3")
  t(Decimal.round(d"1234.5678", -2))
  t(Decimal.round(d"-1.5", 0))
  t(Decimal.round(d"-1.45", 1))
  t(Decimal.round(d"2.345", 2))
  t(Decimal.round(d"1250", -2))
  println(intToText(Money.toCents(d"5")))
  println(intToText(Money.toCents(d"-0.055")))
  println(boolToText(d"-0.05" < d"0"))
  println(boolToText(d"1.50" == d"1.5"))
  println(boolToText(Text.toDecimal("99999999999999999999.99") == None))
}
"####);
    assert!(r.status.success());
    let out = String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n");
    assert_eq!(out, "-0.05\n-0.05\n-0.05\n-0.25\n-1.05\n0.007\n12\n10000.0000\n333333333333.3333333\n3.33333333\n1200\n-2\n-1.5\n2.35\n1300\n500\n-6\ntrue\ntrue\ntrue\n");
}

#[test]
fn decimal_overflow_and_out_of_range_literals_stop_with_a_message() {
    for (body, needle) in [
        ("println(Decimal.toText(d\"10000000000.00\" * d\"10000000000.00\" * d\"10000000000.00\"))", "decimal overflow"),
        ("println(Decimal.toText(d\"99999999999999999999.99\"))", "out of range"),
        ("println(Decimal.toText(d\"9000000000000000000\" + d\"9000000000000000000\"))", "decimal overflow"),
    ] {
        let r = build_run(&format!("module P\nfn main(): Unit [io] = {{\n  {body}\n}}\n"));
        assert!(!r.status.success(), "{body}");
        assert!(String::from_utf8_lossy(&r.stderr).contains(needle), "{body}: {}", String::from_utf8_lossy(&r.stderr));
    }
}
