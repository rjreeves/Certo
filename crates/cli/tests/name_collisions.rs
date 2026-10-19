use std::{fs, process::Command};

// BACKLOG item 358 — a global `val` or an enum variant named like a runtime
// helper, or a global `val` next to a variant of the same name, used to fail in
// the C compiler ("redefinition of 'certo_identity'"). Builds and runs a real
// program with each of them.

#[test]
fn globals_and_variants_that_share_runtime_or_variant_names_compile_and_run() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("main.cto");
    fs::write(&file, r#"module C
import Stdlib.Core

type Kind = Identity | Print | Other
type Role = Admin | User

val identity = 5
val admin = 7

fn describe(k: Kind): Text = match k {
    Identity => "identity"
    Print => "print"
    Other => "other"
}

fn main(): Unit [io] = {
    println(intToText(identity + admin))
    println(describe(Identity))
    println(describe(Print))
    val role = Admin
    match role {
        Admin => println("admin role")
        User => println("user role")
    }
}
"#).unwrap();
    let exe = dir.path().join(if cfg!(windows) { "c.exe" } else { "c" });
    let build = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    assert!(
        build.status.success(),
        "build failed: {}{}",
        String::from_utf8_lossy(&build.stdout),
        String::from_utf8_lossy(&build.stderr),
    );
    let run = Command::new(&exe).output().unwrap();
    assert!(run.status.success(), "run failed: {:?}", run.status);
    let out = String::from_utf8_lossy(&run.stdout).replace("\r\n", "\n");
    assert_eq!(out, "12\nidentity\nprint\nadmin role\n");
}
