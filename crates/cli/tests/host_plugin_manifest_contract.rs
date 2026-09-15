use std::{collections::HashSet, fs, path::PathBuf};

use serde_yaml::Value;

fn root(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").join(path)
}

fn load(path: PathBuf) -> Value {
    serde_yaml::from_str(&fs::read_to_string(path).unwrap()).expect("valid JSON document")
}

fn semver(value: &str) -> Option<(u64, u64, u64)> {
    let core = value.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let version = (
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    );
    if parts.next().is_some() { None } else { Some(version) }
}

fn contract_error(manifest: &Value) -> Option<&'static str> {
    if semver(manifest["version"].as_str()?).is_none() {
        return Some("InvalidSemanticVersion");
    }
    let host = &manifest["host_api"];
    let Some(min) = semver(host["min"].as_str()?) else {
        return Some("InvalidSemanticVersion");
    };
    let Some(max) = semver(host["max_exclusive"].as_str()?) else {
        return Some("InvalidSemanticVersion");
    };
    if min >= max { return Some("InvalidVersionRange"); }

    let source = manifest["entry"]["source"].as_str()?;
    if PathBuf::from(source).is_absolute() || source.replace('\\', "/").split('/').any(|p| p == "..") {
        return Some("EntryOutsidePackage");
    }

    let mut capabilities = HashSet::new();
    for capability in manifest["capabilities"].as_sequence()? {
        if !capabilities.insert(capability["id"].as_str()?) {
            return Some("DuplicateCapability");
        }
    }
    None
}

#[test]
fn manifest_schema_is_closed_and_versioned() {
    let schema = load(root("schemas/host-plugin.schema.json"));
    assert_eq!(schema["$schema"], "https://json-schema.org/draft/2020-12/schema");
    assert_eq!(schema["properties"]["schema_version"]["const"], 1);
    assert_eq!(schema["additionalProperties"], false);
    let required = schema["required"].as_sequence().unwrap();
    for field in ["schema_version", "id", "version", "host_api", "entry", "capabilities"] {
        assert!(required.iter().any(|value| value == field), "missing required field {field}");
    }
}

#[test]
fn dropbox_manifest_satisfies_semantic_contract() {
    let manifest = load(root("examples/certo-plugin.json"));
    assert_eq!(contract_error(&manifest), None);
    assert_eq!(manifest["entry"]["module"], "DropboxHost");
    assert_eq!(manifest["entry"]["factory"], "dropboxPlugin");
}

#[test]
fn invalid_manifest_fixtures_have_stable_categories() {
    let fixtures = root("crates/cli/tests/fixtures/plugin_manifests");
    for (name, expected) in [
        ("invalid-range.json", "InvalidVersionRange"),
        ("duplicate-capability.json", "DuplicateCapability"),
        ("escaping-entry.json", "EntryOutsidePackage"),
        ("invalid-semver.json", "InvalidSemanticVersion"),
    ] {
        assert_eq!(contract_error(&load(fixtures.join(name))), Some(expected), "{name}");
    }
}
