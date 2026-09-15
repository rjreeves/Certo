use std::{fs, path::PathBuf, process::Command};

fn example(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(path)
}

#[test]
fn operational_host_example_builds() {
    let directory = tempfile::tempdir().expect("temporary build directory");
    let executable = directory.path().join(if cfg!(windows) {
        "host-probes.exe"
    } else {
        "host-probes"
    });
    let output = Command::new(env!("CARGO_BIN_EXE_certo"))
        .arg("build")
        .arg(example("host_probes.cto"))
        .arg("-o")
        .arg(&executable)
        .output()
        .expect("build operational host example");
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
    assert!(executable.exists());
}

#[test]
fn service_manager_templates_preserve_shutdown_contract() {
    let systemd = fs::read_to_string(example("deploy/systemd/certo-host.service")).unwrap();
    assert!(systemd.contains("KillSignal=SIGTERM"));
    assert!(systemd.contains("TimeoutStopSec=90s"));
    assert!(systemd.contains("Restart=on-failure"));

    let compose: serde_yaml::Value = serde_yaml::from_str(
        &fs::read_to_string(example("deploy/container/compose.yaml")).unwrap(),
    ).expect("valid Compose YAML");
    assert_eq!(compose["services"]["certo-host"]["stop_grace_period"], "90s");

    let kubernetes: serde_yaml::Value = serde_yaml::from_str(
        &fs::read_to_string(example("deploy/kubernetes/deployment.yaml")).unwrap(),
    ).expect("valid Kubernetes YAML");
    let pod = &kubernetes["spec"]["template"]["spec"];
    assert_eq!(pod["terminationGracePeriodSeconds"], 90);
    let container = &pod["containers"][0];
    assert_eq!(container["livenessProbe"]["httpGet"]["path"], "/live");
    assert_eq!(container["readinessProbe"]["httpGet"]["path"], "/ready");

    let windows = fs::read_to_string(example("deploy/windows/Stop-CertoHost.ps1")).unwrap();
    assert!(windows.contains("Authorization = \"Bearer $AdminToken\""));
    assert!(windows.contains("StatusCode -ne 202"));
}
