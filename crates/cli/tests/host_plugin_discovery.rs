use std::{fs, path::Path, process::Command};

fn write_project(root: &Path, host_min: &str, host_max: &str) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir_all(root.join("plugins/example")).unwrap();
    fs::write(root.join("src/main.cto"),
        "module Main\nfn main(): Unit = {\n    val host = Host.discovered(Host.new())\n    ()\n}\n").unwrap();
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
    let emitted = fs::read_to_string(root.path().join("out.c")).unwrap();
    assert!(emitted.contains("host = certo_host_add(host, certo_host_plugin_dev_d_certo_d_example__plugin());"));
    assert!(emitted.contains("#define CERTO_HOST_PLUGIN_FINGERPRINT \""));
    let native = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("build").arg("-o")
        .arg(root.path().join(if cfg!(windows) { "app.exe" } else { "app" }))
        .current_dir(root.path()).output().unwrap();
    assert!(native.status.success(), "{}", String::from_utf8_lossy(&native.stderr));
}

#[test]
fn composition_calls_factories_in_resolved_dependency_order() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("src")).unwrap();
    fs::create_dir_all(root.path().join("plugins/provider")).unwrap();
    fs::create_dir_all(root.path().join("plugins/consumer")).unwrap();
    fs::write(root.path().join("src/main.cto"),
        "module Main\nfn main(): Unit = {\n    val host = Host.discovered(Host.new())\n    ()\n}\n").unwrap();
    fs::write(root.path().join("certo.toml"),
        "[project]\nname = \"order-test\"\nversion = \"1.0.0\"\n\n[build]\nentry = \"src/main.cto\"\n\n[host-plugins]\nmanifests = [\"plugins/consumer/certo-plugin.json\", \"plugins/provider/certo-plugin.json\"]\n").unwrap();
    fs::write(root.path().join("plugins/provider/plugin.cto"),
        "module Provider\npub fn providerPlugin(): HostPlugin = panic(\"unused\")\n").unwrap();
    fs::write(root.path().join("plugins/consumer/plugin.cto"),
        "module Consumer\npub fn consumerPlugin(): HostPlugin = panic(\"unused\")\n").unwrap();
    fs::write(root.path().join("plugins/provider/certo-plugin.json"), r#"{
  "schema_version": 1, "id": "dev.certo.provider", "version": "1.0.0",
  "host_api": { "min": "1.0.0", "max_exclusive": "2.0.0" },
  "entry": { "source": "plugin.cto", "module": "Provider", "factory": "providerPlugin" },
  "capabilities": []
}"#).unwrap();
    fs::write(root.path().join("plugins/consumer/certo-plugin.json"), r#"{
  "schema_version": 1, "id": "dev.certo.consumer", "version": "1.0.0",
  "host_api": { "min": "1.0.0", "max_exclusive": "2.0.0" },
  "entry": { "source": "plugin.cto", "module": "Consumer", "factory": "consumerPlugin" },
  "capabilities": [],
  "dependencies": [{ "id": "dev.certo.provider", "version": { "min": "1.0.0", "max_exclusive": "2.0.0" } }]
}"#).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("build").arg("--emit-c").arg("-o").arg(root.path().join("out.c"))
        .current_dir(root.path()).output().unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    let emitted = fs::read_to_string(root.path().join("out.c")).unwrap();
    let provider = emitted.find("host = certo_host_add(host, certo_host_plugin_dev_d_certo_d_provider__provider_plugin());").unwrap();
    let consumer = emitted.find("host = certo_host_add(host, certo_host_plugin_dev_d_certo_d_consumer__consumer_plugin());").unwrap();
    assert!(provider < consumer, "provider factory must be invoked first");
}

#[test]
fn plugins_may_reuse_factory_and_private_function_names() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir_all(root.path().join("src")).unwrap();
    for package in ["alpha", "beta"] {
        fs::create_dir_all(root.path().join(format!("plugins/{package}"))).unwrap();
        fs::write(root.path().join(format!("plugins/{package}/plugin.cto")), format!(
            "module {package}\ntype PluginName = Text\nval pluginName: PluginName = \"{package}\"\nfn name(): PluginName = pluginName\nfn start(c: HostContext): Result<Unit, Text> = Ok(())\nfn stop(c: HostContext): Result<Unit, Text> = Ok(())\npub fn plugin(): HostPlugin = Host.plugin(name(), start, stop)\n"
        )).unwrap();
        fs::write(root.path().join(format!("plugins/{package}/certo-plugin.json")), format!(r#"{{
  "schema_version": 1, "id": "dev.certo.{package}", "version": "1.0.0",
  "host_api": {{ "min": "1.0.0", "max_exclusive": "2.0.0" }},
  "entry": {{ "source": "plugin.cto", "module": "{package}", "factory": "plugin" }},
  "capabilities": []
}}"#)).unwrap();
    }
    fs::write(root.path().join("src/main.cto"),
        "module Main\nfn main(): Unit = {\n    val host = Host.discovered(Host.new())\n    ()\n}\n").unwrap();
    fs::write(root.path().join("certo.toml"),
        "[project]\nname = \"isolation-test\"\nversion = \"1.0.0\"\n\n[build]\nentry = \"src/main.cto\"\n\n[host-plugins]\nmanifests = [\"plugins/alpha/certo-plugin.json\", \"plugins/beta/certo-plugin.json\"]\n").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("build").arg("-o")
        .arg(root.path().join(if cfg!(windows) { "app.exe" } else { "app" }))
        .current_dir(root.path()).output().unwrap();
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
