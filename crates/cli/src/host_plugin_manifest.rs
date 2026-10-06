use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::{Path, PathBuf},
};

use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};

const HOST_API_VERSION: &str = "1.0.0";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    id: String,
    version: String,
    #[allow(dead_code)]
    description: Option<String>,
    host_api: VersionRange,
    entry: Entry,
    capabilities: Vec<Capability>,
    #[serde(default)]
    dependencies: Vec<Dependency>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct VersionRange {
    min: String,
    max_exclusive: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    source: String,
    module: String,
    factory: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Capability {
    id: String,
    version: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Dependency {
    id: String,
    version: VersionRange,
    #[serde(default)]
    optional: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ResolvedPlugin {
    pub id: String,
    pub version: Version,
    pub manifest_path: PathBuf,
    pub source_path: PathBuf,
    pub module: String,
    pub factory: String,
    pub capabilities: Vec<(String, Version)>,
    pub dependencies: Vec<ResolvedDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDependency {
    pub id: String,
    pub min: Version,
    pub max_exclusive: Version,
    pub optional: bool,
    pub resolved_version: Option<Version>,
}

/// Stable source fingerprint for a resolved static-plugin composition. The
/// input is deliberately semantic (ordered IDs, versions, entry metadata and
/// source bytes), never absolute paths or JSON formatting.
pub fn fingerprint(plugins: &[ResolvedPlugin]) -> Result<String, String> {
    fingerprint_with_host_api(plugins, HOST_API_VERSION)
}

pub fn build_graph_snapshot(plugins: &[ResolvedPlugin]) -> String {
    build_graph_snapshot_with_host_api(plugins, HOST_API_VERSION)
}

fn build_graph_snapshot_with_host_api(plugins: &[ResolvedPlugin], host_api: &str) -> String {
    let mut snapshot = format!("certo.host.plugins/v2\nhost-api {host_api}\n");
    for plugin in plugins {
        snapshot.push_str(&format!("plugin {}@{} module={} factory={}\n",
            plugin.id, plugin.version, plugin.module, plugin.factory));
        for (id, version) in &plugin.capabilities {
            snapshot.push_str(&format!("  capability {id}@{version}\n"));
        }
        for dependency in &plugin.dependencies {
            let resolved = dependency.resolved_version.as_ref()
                .map(ToString::to_string).unwrap_or_else(|| "absent".into());
            snapshot.push_str(&format!(
                "  dependency {} [{},{}) optional={} resolved={}\n",
                dependency.id, dependency.min, dependency.max_exclusive,
                dependency.optional, resolved));
        }
    }
    snapshot
}

fn fingerprint_with_host_api(plugins: &[ResolvedPlugin], host_api: &str) -> Result<String, String> {
    let mut digest = Sha256::new();
    fn feed(digest: &mut Sha256, value: &[u8]) {
        digest.update((value.len() as u64).to_le_bytes());
        digest.update(value);
    }
    feed(&mut digest, build_graph_snapshot_with_host_api(plugins, host_api).as_bytes());
    for plugin in plugins {
        let source = fs::read(&plugin.source_path).map_err(|error| format!(
            "[HostPluginManifest/UnreadableEntry] {}: {}", plugin.source_path.display(), error))?;
        feed(&mut digest, &source);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn diagnostic(category: &str, path: &Path, field: &str, message: impl AsRef<str>) -> String {
    format!(
        "[HostPluginManifest/{category}] {}: {field}: {}",
        path.display(),
        message.as_ref()
    )
}

fn parse_version(path: &Path, field: &str, value: &str) -> Result<Version, String> {
    Version::parse(value).map_err(|_| diagnostic(
        "InvalidSemanticVersion", path, field, "expected a SemVer value such as 1.2.3"))
}

fn parse_range(path: &Path, field: &str, range: &VersionRange) -> Result<(Version, Version), String> {
    let min = parse_version(path, &format!("{field}.min"), &range.min)?;
    let max = parse_version(path, &format!("{field}.max_exclusive"), &range.max_exclusive)?;
    if min >= max {
        return Err(diagnostic("InvalidVersionRange", path, field, "min must be less than max_exclusive"));
    }
    Ok((min, max))
}

fn valid_id(id: &str) -> bool {
    id.contains('.')
        && id.len() <= 128
        && id.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && id.bytes().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'.' || c == b'-')
        && id.split(['.', '-']).all(|part| !part.is_empty())
}

fn canonical_child(root: &Path, listed: &str) -> Result<PathBuf, String> {
    let candidate = root.join(listed);
    let resolved = candidate.canonicalize().map_err(|error| diagnostic(
        "UnreadableManifest", &candidate, "manifest", error.to_string()))?;
    if !resolved.starts_with(root) {
        return Err(diagnostic("ManifestOutsideProject", &candidate, "manifest", "path escapes the project root"));
    }
    Ok(resolved)
}

pub fn discover(project_root: &Path, listed: &[String]) -> Result<Vec<ResolvedPlugin>, String> {
    let root = project_root.canonicalize().map_err(|error| error.to_string())?;
    let host_version = Version::parse(HOST_API_VERSION).expect("constant host API version");
    let mut manifests = BTreeMap::<String, (Manifest, PathBuf, Version, PathBuf)>::new();
    let mut capability_owners = BTreeMap::<String, String>::new();

    let mut paths = listed.iter().map(|path| canonical_child(&root, path)).collect::<Result<Vec<_>, _>>()?;
    paths.sort_by_key(|path| path.to_string_lossy().to_lowercase());

    for path in paths {
        let bytes = fs::read(&path).map_err(|error| diagnostic("UnreadableManifest", &path, "manifest", error.to_string()))?;
        let manifest: Manifest = serde_json::from_slice(&bytes).map_err(|error| diagnostic(
            "InvalidJson", &path, "manifest", error.to_string()))?;
        if manifest.schema_version != 1 {
            return Err(diagnostic("UnsupportedSchemaVersion", &path, "schema_version", "only version 1 is supported"));
        }
        if !valid_id(&manifest.id) {
            return Err(diagnostic("InvalidPluginId", &path, "id", "expected a lowercase namespaced identifier"));
        }
        let version = parse_version(&path, "version", &manifest.version)?;
        let (host_min, host_max) = parse_range(&path, "host_api", &manifest.host_api)?;
        if host_version < host_min || host_version >= host_max {
            return Err(diagnostic("IncompatibleHostApi", &path, "host_api", format!(
                "host {host_version} is outside [{host_min}, {host_max})")));
        }
        if manifests.contains_key(&manifest.id) {
            return Err(diagnostic("DuplicatePluginId", &path, "id", &manifest.id));
        }
        let package = path.parent().expect("manifest has parent").canonicalize().map_err(|e| e.to_string())?;
        let source = package.join(&manifest.entry.source).canonicalize().map_err(|error| diagnostic(
            "UnreadableEntry", &path, "entry.source", error.to_string()))?;
        if !source.starts_with(&package) {
            return Err(diagnostic("EntryOutsidePackage", &path, "entry.source", "path escapes the plugin package"));
        }
        for capability in &manifest.capabilities {
            if !valid_id(&capability.id) {
                return Err(diagnostic("InvalidCapabilityId", &path, "capabilities.id", &capability.id));
            }
            parse_version(&path, "capabilities.version", &capability.version)?;
            if let Some(owner) = capability_owners.insert(capability.id.clone(), manifest.id.clone()) {
                return Err(diagnostic("DuplicateCapability", &path, "capabilities.id", format!(
                    "{} is already provided by {owner}", capability.id)));
            }
        }
        manifests.insert(manifest.id.clone(), (manifest, path, version, source));
    }

    let mut edges = BTreeMap::<String, BTreeSet<String>>::new();
    for (id, (manifest, path, _, _)) in &manifests {
        edges.entry(id.clone()).or_default();
        for dependency in &manifest.dependencies {
            let (min, max) = parse_range(path, "dependencies.version", &dependency.version)?;
            match manifests.get(&dependency.id) {
                Some((_, _, found, _)) if found >= &min && found < &max => {
                    edges.entry(id.clone()).or_default().insert(dependency.id.clone());
                }
                Some(_) => return Err(diagnostic("IncompatibleDependency", path, "dependencies.version", &dependency.id)),
                None if !dependency.optional => return Err(diagnostic("MissingDependency", path, "dependencies.id", &dependency.id)),
                None => {}
            }
        }
    }

    let mut ordered = Vec::new();
    while ordered.len() < manifests.len() {
        let resolved: BTreeSet<_> = ordered.iter().cloned().collect();
        let next = edges.iter().find(|(id, dependencies)| {
            !resolved.contains(*id) && dependencies.iter().all(|dependency| resolved.contains(dependency))
        }).map(|(id, _)| id.clone());
        let Some(id) = next else {
            return Err(diagnostic("DependencyCycle", &root.join("certo.toml"), "host-plugins.manifests", "plugin dependency graph contains a cycle"));
        };
        ordered.push(id);
    }

    let resolved_versions = manifests.iter().map(|(id, (_, _, version, _))|
        (id.clone(), version.clone())).collect::<BTreeMap<_, _>>();
    Ok(ordered.into_iter().map(|id| {
        let (manifest, manifest_path, version, source_path) = manifests.remove(&id).unwrap();
        let mut capabilities = manifest.capabilities.iter().map(|capability| (
            capability.id.clone(),
            Version::parse(&capability.version).expect("capability version already validated"),
        )).collect::<Vec<_>>();
        capabilities.sort();
        let mut dependencies = manifest.dependencies.iter().map(|dependency| ResolvedDependency {
            id: dependency.id.clone(),
            min: Version::parse(&dependency.version.min).expect("dependency min already validated"),
            max_exclusive: Version::parse(&dependency.version.max_exclusive)
                .expect("dependency max already validated"),
            optional: dependency.optional,
            resolved_version: resolved_versions.get(&dependency.id).cloned(),
        }).collect::<Vec<_>>();
        dependencies.sort_by(|left, right| left.id.cmp(&right.id)
            .then(left.min.cmp(&right.min))
            .then(left.max_exclusive.cmp(&right.max_exclusive)));
        ResolvedPlugin {
            id,
            version,
            manifest_path,
            source_path,
            module: manifest.entry.module,
            factory: manifest.entry.factory,
            capabilities,
            dependencies,
        }
    }).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(root: &Path, name: &str, manifest: &str) -> String {
        let directory = root.join(name);
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join("plugin.cto"), format!(
            "module {}\npub fn plugin(): HostPlugin = panic(\"not compiled by discovery\")\n",
            name.replace('-', "")))
        .unwrap();
        fs::write(directory.join("certo-plugin.json"), manifest).unwrap();
        format!("{name}/certo-plugin.json")
    }

    fn manifest(id: &str, dependencies: &str) -> String {
        format!(r#"{{
  "schema_version": 1,
  "id": "{id}",
  "version": "1.0.0",
  "host_api": {{ "min": "1.0.0", "max_exclusive": "2.0.0" }},
  "entry": {{ "source": "plugin.cto", "module": "Plugin", "factory": "plugin" }},
  "capabilities": [],
  "dependencies": {dependencies}
}}"#)
    }

    #[test]
    fn providers_precede_consumers_with_lexical_ties() {
        let root = tempfile::tempdir().unwrap();
        let provider = package(root.path(), "provider", &manifest("dev.test.provider", "[]"));
        let consumer = package(root.path(), "consumer", &manifest(
            "dev.test.consumer",
            r#"[{"id":"dev.test.provider","version":{"min":"1.0.0","max_exclusive":"2.0.0"}}]"#,
        ));
        let independent = package(root.path(), "independent", &manifest("dev.test.independent", "[]"));
        let resolved = discover(root.path(), &[consumer, provider, independent]).unwrap();
        let ids: Vec<_> = resolved.iter().map(|plugin| plugin.id.as_str()).collect();
        assert_eq!(ids, ["dev.test.independent", "dev.test.provider", "dev.test.consumer"]);
    }

    #[test]
    fn duplicate_ids_and_incompatible_host_are_rejected() {
        let root = tempfile::tempdir().unwrap();
        let first = package(root.path(), "one", &manifest("dev.test.same", "[]"));
        let second = package(root.path(), "two", &manifest("dev.test.same", "[]"));
        assert!(discover(root.path(), &[first, second]).unwrap_err().contains("DuplicatePluginId"));

        let incompatible = package(root.path(), "future", &manifest("dev.test.future", "[]"));
        let path = root.path().join(&incompatible);
        let text = fs::read_to_string(&path).unwrap().replace(
            r#""min": "1.0.0", "max_exclusive": "2.0.0""#,
            r#""min": "2.0.0", "max_exclusive": "3.0.0""#);
        fs::write(path, text).unwrap();
        assert!(discover(root.path(), &[incompatible]).unwrap_err().contains("IncompatibleHostApi"));
    }

    fn fingerprint_graph(root: &Path, capability_version: &str, dependency_max: &str) -> Vec<String> {
        let provider = package(root, "provider", &format!(r#"{{
  "schema_version": 1,
  "id": "dev.test.provider",
  "version": "1.0.0",
  "host_api": {{ "min": "1.0.0", "max_exclusive": "2.0.0" }},
  "entry": {{ "source": "plugin.cto", "module": "Plugin", "factory": "plugin" }},
  "capabilities": [{{ "id": "dev.test.storage", "version": "{capability_version}" }}],
  "dependencies": []
}}"#));
        let consumer = package(root, "consumer", &format!(r#"{{
  "schema_version": 1,
  "id": "dev.test.consumer",
  "version": "1.0.0",
  "host_api": {{ "min": "1.0.0", "max_exclusive": "2.0.0" }},
  "entry": {{ "source": "plugin.cto", "module": "Plugin", "factory": "plugin" }},
  "capabilities": [],
  "dependencies": [{{
    "id": "dev.test.provider",
    "version": {{ "min": "1.0.0", "max_exclusive": "{dependency_max}" }}
  }}]
}}"#));
        vec![consumer, provider]
    }

    #[test]
    fn fingerprint_is_path_and_manifest_order_independent() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        let listed = fingerprint_graph(first.path(), "1.0.0", "2.0.0");
        let mut reversed = fingerprint_graph(second.path(), "1.0.0", "2.0.0");
        reversed.reverse();
        let first_plugins = discover(first.path(), &listed).unwrap();
        let same_root_reversed = discover(first.path(), &listed.iter().cloned().rev().collect::<Vec<_>>()).unwrap();
        let second_plugins = discover(second.path(), &reversed).unwrap();
        let expected = fingerprint(&first_plugins).unwrap();
        assert_eq!(expected, fingerprint(&same_root_reversed).unwrap());
        assert_eq!(expected, fingerprint(&second_plugins).unwrap());
        assert_eq!(build_graph_snapshot(&first_plugins), concat!(
            "certo.host.plugins/v2\n",
            "host-api 1.0.0\n",
            "plugin dev.test.provider@1.0.0 module=Plugin factory=plugin\n",
            "  capability dev.test.storage@1.0.0\n",
            "plugin dev.test.consumer@1.0.0 module=Plugin factory=plugin\n",
            "  dependency dev.test.provider [1.0.0,2.0.0) optional=false resolved=1.0.0\n",
        ));
    }

    #[test]
    fn fingerprint_changes_for_every_build_graph_input() {
        let baseline_root = tempfile::tempdir().unwrap();
        let baseline_list = fingerprint_graph(baseline_root.path(), "1.0.0", "2.0.0");
        let baseline_plugins = discover(baseline_root.path(), &baseline_list).unwrap();
        let baseline = fingerprint(&baseline_plugins).unwrap();

        let source_root = tempfile::tempdir().unwrap();
        let source_list = fingerprint_graph(source_root.path(), "1.0.0", "2.0.0");
        fs::write(source_root.path().join("provider/plugin.cto"),
            "module provider\npub fn plugin(): HostPlugin = panic(\"changed\")\n").unwrap();
        assert_ne!(baseline, fingerprint(&discover(source_root.path(), &source_list).unwrap()).unwrap());

        let version_root = tempfile::tempdir().unwrap();
        let version_list = fingerprint_graph(version_root.path(), "1.0.0", "2.0.0");
        let provider_manifest = version_root.path().join("provider/certo-plugin.json");
        let changed_version = fs::read_to_string(&provider_manifest).unwrap()
            .replacen(r#""version": "1.0.0""#, r#""version": "1.1.0""#, 1);
        fs::write(provider_manifest, changed_version).unwrap();
        assert_ne!(baseline, fingerprint(&discover(version_root.path(), &version_list).unwrap()).unwrap());

        let capability_root = tempfile::tempdir().unwrap();
        let capability_list = fingerprint_graph(capability_root.path(), "1.1.0", "2.0.0");
        assert_ne!(baseline, fingerprint(&discover(capability_root.path(), &capability_list).unwrap()).unwrap());

        let dependency_root = tempfile::tempdir().unwrap();
        let dependency_list = fingerprint_graph(dependency_root.path(), "1.0.0", "3.0.0");
        assert_ne!(baseline, fingerprint(&discover(dependency_root.path(), &dependency_list).unwrap()).unwrap());

        assert_ne!(baseline,
            fingerprint_with_host_api(&baseline_plugins, "1.0.1").unwrap());
    }
}
