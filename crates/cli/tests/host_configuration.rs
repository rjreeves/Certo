use std::{fs, process::Command};

fn run_certo(source: &str) -> std::process::Output {
    let dir = tempfile::tempdir().expect("temporary test directory");
    let source_path = dir.path().join("host_configuration_test.cto");
    fs::write(&source_path, source).expect("write Certo source");
    Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("run")
        .arg(&source_path)
        .output()
        .expect("run certo")
}

fn assert_success(output: &std::process::Output) -> String {
    assert!(
        output.status.success(),
        "certo failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn build_and_run(
    source: &str,
    manifest: &str,
    environment: &[(&str, &str)],
    arguments: &[&str],
) -> std::process::Output {
    let dir = tempfile::tempdir().expect("temporary test directory");
    let source_path = dir.path().join("main.cto");
    let executable = dir.path().join(if cfg!(windows) { "app.exe" } else { "app" });
    fs::write(&source_path, source).expect("write Certo source");
    fs::write(dir.path().join("certo.toml"), manifest).expect("write manifest");
    let build = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("build")
        .arg(&source_path)
        .arg("-o")
        .arg(&executable)
        .current_dir(dir.path())
        .output()
        .expect("build Certo program");
    assert!(build.status.success(), "build failed: {}", String::from_utf8_lossy(&build.stderr));
    let mut command = Command::new(&executable);
    command.current_dir(dir.path()).args(arguments);
    for (key, value) in environment { command.env(key, value); }
    command.output().expect("run generated program")
}

#[test]
fn typed_programmatic_configuration_is_bound_validated_and_immutable() {
    let stdout = assert_success(&run_certo(r#"module HostTypedConfigurationTest

fn countKey(): ConfigKey<Int> = Host.configKey("worker.count", parseCount)
fn parseCount(value: Text): Result<Int, Text> =
    if value == "12" then Ok(12) else Err("not a supported count")
fn positive(value: Int): Result<Unit, Text> =
    if value > 0 then Ok(()) else Err("must be positive")
fn serviceKey(): ServiceKey<Int> = Host.serviceKey("configured-count")
fn create(c: HostContext): Result<Int, Text> [io] = {
    println(intToText(HostContext.configValue(c, countKey())))
    Ok(1)
}
fn dispose(c: HostContext): Result<Unit, Text> = Ok(())
fn start(c: HostContext): Result<Unit, Text> [io] = {
    println(intToText(HostContext.configValue(c, countKey())))
    Ok(())
}
fn stop(c: HostContext): Result<Unit, Text> = Ok(())

fn main(): Unit [io] = {
    val host = Host.new()
        .configure("worker.count", "12")
        .requireConfig(countKey())
        .validateConfig(countKey(), positive)
        .provideFactory(serviceKey(), create, dispose)
        .add(Host.plugin("reader", start, stop))
    match Host.start(host) {
        Ok(_) => {
            val stopped = Host.stop(host)
        }
        Err(error) => println(error)
    }
    val defaulted = Host.new()
        .defaultConfig(countKey(), 5)
        .validateConfig(countKey(), positive)
        .add(Host.plugin("default-reader", start, stop))
    val defaultStarted = Host.start(defaulted)
    val defaultStopped = Host.stop(defaulted)
}
"#));

    assert_eq!(stdout.lines().collect::<Vec<_>>(), ["12", "12", "5"]);
}

#[test]
fn configuration_failures_are_ordered_typed_and_precede_callbacks() {
    let stdout = assert_success(&run_certo(r#"module HostConfigurationFailureTest

fn missingKey(): ConfigKey<Int> = Host.configKey("first.missing", parseValue)
fn invalidKey(): ConfigKey<Int> = Host.configKey("second.invalid", parseValue)
fn rejectedKey(): ConfigKey<Int> = Host.configKey("third.rejected", parseValue)
fn parseValue(value: Text): Result<Int, Text> =
    if value == "valid" then Ok(1) else Err("invalid integer")
fn positive(value: Int): Result<Unit, Text> =
    if value > 0 then Ok(()) else Err("must be positive")
fn create(c: HostContext): Result<Int, Text> [io] = {
    println("unexpected factory")
    Ok(1)
}

fn dispose(c: HostContext): Result<Unit, Text> = Ok(())
fn start(c: HostContext): Result<Unit, Text> [io] = {
    println("unexpected plugin")
    Ok(())
}
fn stop(c: HostContext): Result<Unit, Text> = Ok(())

fn main(): Unit [io] = {
    val service = Host.serviceKey("service")
    val host = Host.new()
        .configure("second.invalid", "bad-secret-value")
        .requireConfig(missingKey())
        .requireConfig(invalidKey())
        .defaultConfig(rejectedKey(), -1)
        .validateConfig(rejectedKey(), positive)
        .provideFactory(service, create, dispose)
        .add(Host.plugin("plugin", start, stop))
    match Host.startTyped(host) {
        Ok(_) => println("unexpected success")
        Err(failure) => {
            println(HostFailureKind.name(HostLifecycleError.kind(failure)))
            println(HostLifecycleError.phase(failure))
            println(HostLifecycleError.subject(failure))
            for error in HostLifecycleError.configurationErrors(failure) {
                println(HostConfigurationError.key(error))
                println(HostConfigurationError.source(error))
                println(HostConfigurationError.location(error))
                println(HostConfigurationError.category(error))
                println(HostConfigurationError.message(error))
            }
        }
    }
}
"#));

    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        [
            "ConfigurationFailure",
            "configuration",
            "first.missing",
            "first.missing",
            "None",
            "",
            "Missing",
            "required configuration value is missing",
            "second.invalid",
            "Programmatic",
            "Host.configure",
            "Parse",
            "invalid integer",
            "third.rejected",
            "Default",
            "typed default",
            "Validation",
            "must be positive",
        ],
        "{stdout}"
    );
    assert!(!stdout.contains("bad-secret-value"), "raw values must not leak: {stdout}");
    assert!(!stdout.contains("unexpected"), "callbacks ran before validation: {stdout}");
}

#[test]
fn external_sources_follow_default_toml_environment_cli_programmatic_precedence() {
    let output = build_and_run(
        r#"module HostConfigurationPrecedenceTest

fn parse(value: Text): Result<Text, Text> = Ok(value)
fn defaultKey(): ConfigKey<Text> = Host.configKey("default.value", parse)
fn tomlKey(): ConfigKey<Text> = Host.configKey("toml.value", parse)
fn envKey(): ConfigKey<Text> = Host.configKey("env.value", parse)
fn cliKey(): ConfigKey<Text> = Host.configKey("cli.value", parse)
fn programKey(): ConfigKey<Text> = Host.configKey("program.value", parse)
fn start(c: HostContext): Result<Unit, Text> [io] = {
    println(HostContext.configValue(c, defaultKey()))
    println(HostContext.configValue(c, tomlKey()))
    println(HostContext.configValue(c, envKey()))
    println(HostContext.configValue(c, cliKey()))
    println(HostContext.configValue(c, programKey()))
    Ok(())
}
fn stop(c: HostContext): Result<Unit, Text> = Ok(())
fn main(): Unit [io] = {
    val host = Host.new()
        .defaultConfig(defaultKey(), "default")
        .defaultConfig(tomlKey(), "default")
        .defaultConfig(envKey(), "default")
        .defaultConfig(cliKey(), "default")
        .defaultConfig(programKey(), "default")
        .configure("program.value", "old")
        .configure("program.value", "programmatic")
        .add(Host.plugin("reader", start, stop))
    val started = Host.start(host)
    val stopped = Host.stop(host)
}
"#,
        "[host.toml]\nvalue = \"toml\"\n[host.env]\nvalue = \"toml\"\n[host.cli]\nvalue = \"toml\"\n[host.program]\nvalue = \"toml\"\n",
        &[("CERTO__ENV__VALUE", "environment"), ("CERTO__CLI__VALUE", "environment")],
        &["--config", "cli.value=old", "--config=cli.value=command-line"],
    );
    let stdout = assert_success(&output);
    assert_eq!(
        stdout.lines().collect::<Vec<_>>(),
        ["default", "toml", "environment", "command-line", "programmatic"]
    );
}
