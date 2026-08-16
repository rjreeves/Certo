//! Structured parsing of `certo.toml`, matching the full schema documented in
//! `docs/Certo_Language_Specification.md` section 11.4. Replaces the ad-hoc,
//! per-field `.lines()` scanning that used to be duplicated across `cmd_build`
//! and `read_schema_sync_flag` in `main.rs` — every field below is now a real
//! TOML key read through `serde`/`toml`, not a brittle exact-string match.
//!
//! Every section is optional so a minimal `certo.toml` (just `[project]`/`[build]`)
//! still parses. `database.migrations` is read by `crates/cli/src/main.rs`'s
//! `migrations_dir()` (item 169) — every `certo db`/`certo migrate` subcommand
//! reads/writes migration files there, defaulting to `migrations/` if unset.
//! Fields that no other part of the compiler currently *acts* on
//! (`database.seeds`, `server.*`, `dependencies`/`dev-dependencies`) are still
//! parsed and validated as real TOML, but are reserved for future toolchain
//! work (see BACKLOG items 91, `certo add`; the `[server]` section has no
//! consumer yet since `certo run --port` was deliberately not built — see
//! BACKLOG item 126's sibling note in item 125) — they are not silently
//! dropped, but nothing downstream reads them yet.
#![allow(dead_code)] // reserved schema fields — see module doc above.

use std::collections::HashMap;
use std::path::Path;

use serde::Deserialize;

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CertoToml {
    pub project: Option<ProjectSection>,
    pub build: Option<BuildSection>,
    pub database: Option<DatabaseSection>,
    pub server: Option<ServerSection>,
    pub dependencies: Option<HashMap<String, String>>,
    #[serde(rename = "dev-dependencies")]
    pub dev_dependencies: Option<HashMap<String, String>>,
    pub features: Option<FeaturesSection>,
    pub targets: Option<HashMap<String, TargetSection>>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectSection {
    pub name: Option<String>,
    pub version: Option<String>,
    pub edition: Option<String>,
    pub authors: Option<Vec<String>>,
    pub license: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildSection {
    #[serde(rename = "type")]
    pub ty: Option<String>,
    pub target: Option<String>,
    pub output: Option<String>,
    pub entry: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseSection {
    pub schema: Option<String>,
    pub migrations: Option<String>,
    pub seeds: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServerSection {
    pub port: Option<u16>,
    pub host: Option<String>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FeaturesSection {
    #[serde(rename = "strict-nulls")]
    pub strict_nulls: Option<bool>,
    #[serde(rename = "query-logging")]
    pub query_logging: Option<bool>,
    #[serde(rename = "schema-sync")]
    pub schema_sync: Option<bool>,
    #[serde(rename = "effect-checking")]
    pub effect_checking: Option<bool>,
}

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetSection {
    pub optimize: Option<bool>,
    #[serde(rename = "strip-debug")]
    pub strip_debug: Option<bool>,
    pub schema: Option<String>,
}

/// Loads and parses `<project_root>/certo.toml`. Returns `Ok(None)` if the file
/// doesn't exist (a project with no manifest is not an error — callers fall
/// back to explicit CLI args). A malformed file is `Err` with a message that
/// includes the `toml` crate's own line/column diagnostic, not a silent
/// best-effort partial parse.
pub fn load(project_root: &Path) -> Result<Option<CertoToml>, String> {
    let path = project_root.join("certo.toml");
    let src = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("could not read {}: {}", path.display(), e)),
    };
    match toml::from_str::<CertoToml>(&src) {
        Ok(cfg) => Ok(Some(cfg)),
        Err(e) => Err(format!("failed to parse {}:\n{}", path.display(), e)),
    }
}

/// Expands `${VAR}` references against the process environment. Used for
/// `database.schema` / `targets.*.schema`, matching the spec's own example
/// (`schema = "${DATABASE_URL}"`) of keeping a real connection string out of
/// version control. A reference to an unset variable is left verbatim (not
/// silently blanked) so a misconfigured environment fails loudly downstream
/// (e.g. as a connection error) rather than connecting to an empty string.
pub fn expand_env_vars(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        let Some(end_rel) = rest[start..].find('}') else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..start]);
        let var_name = &rest[start + 2..start + end_rel];
        match std::env::var(var_name) {
            Ok(val) => out.push_str(&val),
            Err(_) => out.push_str(&rest[start..start + end_rel + 1]),
        }
        rest = &rest[start + end_rel + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_spec_example() {
        let src = r#"
[project]
name    = "billing-system"
version = "1.2.0"
edition = "2026"
authors = ["Alice Smith <alice@example.com>"]
license = "MIT"

[build]
target     = "native"
output     = "dist/"
entry      = "src/main.cto"

[database]
schema     = "postgresql://localhost/billing_dev"
migrations = "db/migrations/"
seeds      = "db/seeds/"

[server]
port    = 8080
host    = "0.0.0.0"

[dependencies]
stripe      = "4.2.0"
sendgrid    = "2.1.0"

[dev-dependencies]
test-fixtures = "1.0.0"

[features]
strict-nulls     = true
query-logging    = true
schema-sync      = true
effect-checking  = true

[targets.production]
optimize    = true
strip-debug = true
schema      = "${DATABASE_URL}"
"#;
        let cfg: CertoToml = toml::from_str(src).expect("valid spec example must parse");
        assert_eq!(cfg.project.as_ref().unwrap().name.as_deref(), Some("billing-system"));
        assert_eq!(cfg.build.as_ref().unwrap().entry.as_deref(), Some("src/main.cto"));
        assert_eq!(cfg.features.as_ref().unwrap().schema_sync, Some(true));
        let prod = cfg.targets.as_ref().unwrap().get("production").expect("production target");
        assert_eq!(prod.optimize, Some(true));
        assert_eq!(prod.strip_debug, Some(true));
        assert_eq!(prod.schema.as_deref(), Some("${DATABASE_URL}"));
    }

    #[test]
    fn minimal_manifest_parses() {
        let src = "[project]\nname = \"x\"\n";
        let cfg: CertoToml = toml::from_str(src).expect("minimal manifest must parse");
        assert!(cfg.build.is_none());
        assert!(cfg.features.is_none());
        assert!(cfg.targets.is_none());
    }

    #[test]
    fn missing_file_is_not_an_error() {
        let dir = std::env::temp_dir().join("certo_toml_test_missing_dir_does_not_exist_xyz");
        assert!(load(&dir).unwrap().is_none());
    }

    #[test]
    fn malformed_toml_reports_error() {
        let dir = std::env::temp_dir().join(format!("certo_toml_test_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("certo.toml"), "[project\nname = \"x\"").unwrap();
        let err = load(&dir).unwrap_err();
        assert!(err.contains("failed to parse"), "unexpected message: {err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn unknown_key_is_rejected() {
        let src = "[project]\nname = \"x\"\nbogus-field = 1\n";
        let result: Result<CertoToml, _> = toml::from_str(src);
        assert!(result.is_err(), "unknown top-level project key should be rejected");
    }

    #[test]
    fn env_var_expansion() {
        unsafe { std::env::set_var("CERTO_TOML_TEST_VAR", "postgres://x"); }
        assert_eq!(expand_env_vars("${CERTO_TOML_TEST_VAR}"), "postgres://x");
        assert_eq!(expand_env_vars("prefix-${CERTO_TOML_TEST_VAR}-suffix"), "prefix-postgres://x-suffix");
        assert_eq!(expand_env_vars("no vars here"), "no vars here");
        unsafe { std::env::remove_var("CERTO_TOML_TEST_VAR"); }
    }

    #[test]
    fn env_var_expansion_leaves_unset_var_verbatim() {
        unsafe { std::env::remove_var("CERTO_TOML_TEST_DEFINITELY_UNSET"); }
        assert_eq!(
            expand_env_vars("${CERTO_TOML_TEST_DEFINITELY_UNSET}"),
            "${CERTO_TOML_TEST_DEFINITELY_UNSET}"
        );
    }
}
