use std::{fs, path::Path, process::Command};

fn write_project(root: &Path, host_min: &str, host_max: &str) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("plugins/example")).unwrap();
    fs::write(root.join("src/main.cto"), "module Main\nfn main(): Unit = ()\n").unwrap();
    fs::write(
        root.join("certo.toml"),
        "[project]\nname = \"manifest-test\"\nversion = \"1.0.0\"\n\n[build]\nentry = \"src/main.cto\"\n\n[host-plugins]\nmanifests = [\"plugins/example/certo-plugin.json\"]\n",
    ).unwrap();
    fs::write(
        root.join("plugins/example/plugin.cto"),
        "module Example\npub fn plugin(): HostPlugin = panic(\"discovery only\")\n",
    ).unwrap();
    fs::write(
        root.join("plugins/example/certo-plugin.json"),
        format!(r#"{{
  "schema_version": 1,
  "id": "dev.certo.example",
  "version": "1.0.0",
  "host_api": {{ "min": "{host_min}", "max_exclusive": "{host_max}" }},
  "entry": {{ "source": "plugin.cto", "module": "Example", "factory": "plugin" }},
  "capabilities": []
}}"#),
    ).unwrap();
}

#[test]
fn build_discovers_explicit_local_manifest() {
    let root = tempfile::tempdir().unwrap();
    write_project(root.path(), "1.0.0", "2.0.0");
    let output = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("build")
        .arg("--emit-c")
        .arg("-o")
        .arg(root.path().join("out.c"))
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
}

#[test]
fn build_rejects_incompatible_manifest_before_codegen() {
    let root = tempfile::tempdir().unwrap();
    write_project(root.path(), "2.0.0", "3.0.0");
    let output = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("build")
        .arg("--emit-c")
        .arg("-o")
        .arg(root.path().join("out.c"))
        .current_dir(root.path())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("HostPluginManifest/IncompatibleHostApi"), "{stderr}");
    assert!(!root.path().join("out.c").exists());
}

#[test]
fn build_rejects_invalid_factory_signature_before_codegen() {
    let root = tempfile::tempdir().unwrap();
    write_project(root.path(), "1.0.0", "2.0.0");
    fs::write(
        root.path().join("plugins/example/plugin.cto"),
        "module Example\npub fn plugin(name: Text): HostPlugin = panic(name)\n",
    ).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("build").arg("--emit-c").arg("-o").arg(root.path().join("out.c"))
        .current_dir(root.path()).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("HostPluginManifest/InvalidFactorySignature"), "{stderr}");
}

#[test]
fn build_rejects_entry_module_mismatch() {
    let root = tempfile::tempdir().unwrap();
    write_project(root.path(), "1.0.0", "2.0.0");
    fs::write(
        root.path().join("plugins/example/plugin.cto"),
        "module Different\npub fn plugin(): HostPlugin = panic(\"unused\")\n",
    ).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("build").arg("--emit-c").arg("-o").arg(root.path().join("out.c"))
        .current_dir(root.path()).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(stderr.contains("HostPluginManifest/EntryModuleMismatch"), "{stderr}");
}
