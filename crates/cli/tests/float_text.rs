use std::{fs, process::Command};

// BACKLOG item 400 — `floatToText` and f-string Float interpolation printed through
// `%g`, which keeps 6 significant digits: 3.14159265358979 printed as 3.14159 and
// 1234567.891 as 1.23457e+06, silently. They now print the fewest digits that read
// back as exactly the same number.

fn run(src: &str) -> String {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("f.cto");
    fs::write(&file, src).unwrap();
    let exe = dir.path().join(if cfg!(windows) { "f.exe" } else { "f" });
    let build = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(build.status.success(), "{}{}", String::from_utf8_lossy(&build.stdout), String::from_utf8_lossy(&build.stderr));
    let out = Command::new(&exe).output().unwrap();
    assert!(out.status.success());
    String::from_utf8_lossy(&out.stdout).replace("\r\n", "\n")
}

#[test]
fn floats_print_without_losing_precision() {
    let out = run(r#"module F
import Stdlib.Core

fn main(): Unit [io] = {
    println(floatToText(3.14159265358979))
    println(floatToText(1234567.891))
    println(floatToText(123456789.125))
    println(floatToText(0.1 + 0.2))
    println(floatToText(1.0 / 3.0))
    println(floatToText(78.53981633974483))
}
"#);
    assert_eq!(out, "3.14159265358979\n1234567.891\n123456789.125\n0.30000000000000004\n0.3333333333333333\n78.53981633974483\n");
}

#[test]
fn short_values_and_whole_numbers_print_as_before() {
    let out = run(r#"module F
import Stdlib.Core

fn main(): Unit [io] = {
    println(floatToText(2.5))
    println(floatToText(0.1))
    println(floatToText(3.0))
    println(floatToText(-1.5))
    println(floatToText(100.0))
    println(floatToText(0.0))
}
"#);
    assert_eq!(out, "2.5\n0.1\n3\n-1.5\n100\n0\n");
}

#[test]
fn moderate_exponents_are_plain_decimal_and_extreme_ones_use_exponents() {
    let out = run(r#"module F
import Stdlib.Core

fn main(): Unit [io] = {
    println(floatToText(100000000.0))
    println(floatToText(12345678901234567.0))
    println(floatToText(0.00001234))
    println(floatToText(123456789012345678.0))
    println(floatToText(1000000.0 * 1000000.0 * 1000000.0 * 1000000.0))
    println(floatToText(1.0 / 10000000.0))
    println(floatToText(0.000001234))
}
"#);
    assert_eq!(out, "100000000\n12345678901234568\n0.00001234\n1.2345678901234568e+17\n1e+24\n1e-07\n1.234e-06\n");
}

#[test]
fn f_string_interpolation_uses_the_same_rule() {
    let out = run(r#"module F
import Stdlib.Core

fn main(): Unit [io] = {
    val pi = 3.14159265358979
    println(f"pi is {pi}")
}
"#);
    assert_eq!(out, "pi is 3.14159265358979\n");
}

#[test]
fn float32_uses_its_own_shorter_digit_count() {
    // A Float32 needs at most 9 significant digits, so 0.1 stays "0.1" rather
    // than the double expansion of the nearest float.
    let out = run(r#"module F
import Stdlib.Core

fn main(): Unit [io] = {
    println(float32ToText(floatToFloat32(0.1)))
    println(float32ToText(floatToFloat32(3.14159265)))
    println(float32ToText(floatToFloat32(16777216.0)))
}
"#);
    assert_eq!(out, "0.1\n3.1415927\n16777216\n");
}
