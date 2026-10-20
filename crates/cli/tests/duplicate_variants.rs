use std::{fs, process::Command};

// BACKLOG item 357 - two sum types may declare a variant with the same name; the shared name has
// to be written qualified (`Type.Variant`), an unqualified use is an error that says so.
fn build(src: &str) -> (bool, String, Option<std::path::PathBuf>, tempfile::TempDir) {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.cto");
    fs::write(&file, src).unwrap();
    let exe = root.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    let b = Command::new(env!("CARGO_BIN_EXE_certo")).arg("build").arg(&file).arg("-o").arg(&exe).output().unwrap();
    let text = String::from_utf8_lossy(&b.stderr).to_string();
    let ok = b.status.success();
    (ok, text, if ok { Some(exe) } else { None }, root)
}

#[test]
fn qualified_duplicate_variants_work() {
    let (ok, text, exe, _root) = build(r####"module P
type OS = Pending | Shipped
type PS = Pending | Paid
type Shape = Circle(Int) | Square(Int)
type Token = Circle | Plus
fn o(s: OS): Text = match s {
  OS.Pending => "o-pending"
  Shipped => "shipped"
}
fn p(s: PS): Text = match s {
  PS.Pending => "p-pending"
  Paid => "paid"
}
fn area(s: Shape): Int = match s {
  Shape.Circle(r) => r * 3
  Square(w) => w * w
}
fn tok(t: Token): Text = match t {
  Token.Circle => "circle-token"
  Plus => "plus"
}
fn main(): Unit [io] = {
  println(o(OS.Pending) ++ " " ++ o(Shipped) ++ " " ++ p(PS.Pending) ++ " " ++ p(Paid))
  println(intToText(area(Shape.Circle(4))) ++ " " ++ intToText(area(Square(3))))
  println(tok(Token.Circle) ++ " " ++ tok(Plus))
}
"####);
    assert!(ok, "{text}");
    let r = Command::new(exe.unwrap()).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&r.stdout).replace("\r\n", "\n"),
        "o-pending shipped p-pending paid\n12 9\ncircle-token plus\n");
}

#[test]
fn an_unqualified_shared_variant_name_is_an_error_naming_the_choices() {
    let (ok, text, _, _root) = build("module P\ntype A = Same | OnlyA\ntype B = Same | OnlyB\nfn f(a: A): Int = match a {\n  Same => 1\n  OnlyA => 2\n}\n");
    assert!(!ok);
    assert!(text.contains("`Same`") && text.contains("`A.Same`") && text.contains("`B.Same`"), "{text}");
}

#[test]
fn the_formatter_keeps_the_names_as_written() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("main.cto");
    fs::write(&file, "module P\ntype A = Same | OnlyA\ntype B = Same | OnlyB\nfn f(a: A): Int = match a {\n  A.Same => 1\n  OnlyA => 2\n}\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_certo")).arg("fmt").arg(&file).output().unwrap();
    assert!(out.status.success());
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.contains("A.Same") && !text.contains("_of_"), "{text}");
}
