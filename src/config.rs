//! Loaders for `config/*.yaml`.
//!
//! Kubeconfig values are environment-variable names. The file never contains
//! the kubeconfig itself, a token, or a credentialed URL.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::Error;
use crate::models::Env;
use crate::redact;

/// Parsed project configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectConfig {
    /// Main tool config.
    pub config: Config,
    /// Canonical environment map.
    pub environments: Environments,
    /// Source-env name to canonical env.
    pub env_index: EnvIndex,
    /// Alias to exactly one component name.
    pub aliases: Aliases,
    /// Pilot applications. Empty until teams choose them.
    pub pilot: PilotFile,
    /// SDK hints. Empty until internal package prefixes are known.
    pub sdk_hints: SdkHintsFile,
}

/// Root of `config/config.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Config schema version. F0-1 accepts `0.1.0` only.
    pub schema_version: String,
    /// GitLab read settings. No token field: tokens stay in the environment.
    pub gitlab: GitlabConfig,
    /// Clusters whose kubeconfig is named by an environment variable.
    pub kubernetes: KubernetesConfig,
    /// Prometheus base settings.
    pub prometheus: PrometheusConfig,
    /// Which runtime signal phase 1 will read. `none` until that is known.
    pub runtime_source: RuntimeSource,
    /// Where CycloneDX documents come from, and which packages are internal.
    pub sbom: SbomConfig,
    /// Globs for API contracts.
    pub contract_paths: Vec<String>,
    /// Days after `last_seen` before an edge is expired.
    pub edge_expiry_days: u32,
    /// Days a snapshot is kept.
    pub retention_days: u32,
}

/// GitLab connection settings. `base_url` is a placeholder until a human confirms it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitlabConfig {
    /// API origin, without userinfo. Example: `https://gitlab.example.com`.
    pub base_url: String,
    /// Group or subgroup paths in scope.
    pub groups: Vec<String>,
    /// Catalog file name looked up in each project.
    pub catalog_file: String,
    /// Page size for list endpoints. GitLab allows at most 100.
    pub per_page: u32,
    /// Parallel requests one collector may have in flight.
    pub max_concurrency: u32,
}

/// Kubernetes settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KubernetesConfig {
    /// One entry per cluster. The default assumption is a single cluster.
    pub clusters: Vec<ClusterConfig>,
}

/// One cluster. `kubeconfig_env` is the variable name, never the file contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClusterConfig {
    /// Short cluster name.
    pub name: String,
    /// Environment variable that holds a kubeconfig path or contents at runtime.
    pub kubeconfig_env: String,
}

/// Prometheus settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrometheusConfig {
    /// Base URL without userinfo.
    pub url: String,
    /// Label that carries the environment.
    pub env_label: String,
    /// Query window in days.
    pub window_days: u32,
}

/// Runtime edge source selected for a later phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeSource {
    /// Tempo metrics, including `traces_service_graph_*` once verified.
    Tempo,
    /// Jaeger-derived edges.
    Jaeger,
    /// Istio telemetry.
    Istio,
    /// Cilium Hubble flows.
    Hubble,
    /// No runtime source until coverage is known.
    None,
}

/// SBOM inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SbomConfig {
    /// Artifact globs or URLs. URLs must not carry credentials.
    #[serde(default)]
    pub sources: Vec<String>,
    /// Package prefixes that become `Library` nodes. Empty until known.
    #[serde(default)]
    pub internal_package_prefixes: Vec<String>,
}

/// `config/environments.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Environments {
    /// Development.
    pub dev: EnvMapping,
    /// Acceptance.
    pub acc: EnvMapping,
    /// Production.
    pub prod: EnvMapping,
}

/// Names used by each source system for one canonical environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvMapping {
    /// GitLab `environment:` names.
    #[serde(default)]
    pub gitlab: Vec<String>,
    /// OpenTelemetry / Prometheus environment values.
    #[serde(default)]
    pub otel: Vec<String>,
    /// Namespaces in the single cluster. Empty until confirmed.
    #[serde(default)]
    pub k8s_namespaces: Vec<String>,
}

/// Case-insensitive index of source environment names.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnvIndex {
    by_name: BTreeMap<String, Env>,
}

impl EnvIndex {
    /// Resolve a source environment name.
    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<Env> {
        self.by_name.get(&name.to_ascii_lowercase()).copied()
    }
}

/// Alias table. Each alias names exactly one component.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Aliases {
    /// Alias to canonical component name, without the `component:` prefix.
    pub aliases: BTreeMap<String, String>,
}

/// `config/pilot.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PilotFile {
    /// Chosen applications. Empty in F0-1.
    #[serde(default)]
    pub apps: Vec<PilotApp>,
}

/// One pilot application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PilotApp {
    /// Canonical component name.
    pub name: String,
    /// GitLab project path.
    pub gitlab_path: String,
}

/// `config/sdk_hints.yaml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SdkHintsFile {
    /// Language hints. Empty until prefixes exist.
    #[serde(default)]
    pub hints: Vec<SdkHint>,
}

/// One SDK hint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SdkHint {
    /// Package prefix.
    pub prefix: String,
    /// Language or ecosystem name.
    pub language: String,
}

impl Environments {
    /// Index source names and reject ambiguous mappings.
    ///
    /// # Errors
    ///
    /// Returns an error when a source name or namespace maps to two canonical
    /// environments, or a namespace is not a DNS label.
    pub fn index(&self) -> Result<EnvIndex, Error> {
        let mut by_name = BTreeMap::new();
        let mut namespaces = BTreeMap::new();
        for (env, mapping) in [
            (Env::Dev, &self.dev),
            (Env::Acc, &self.acc),
            (Env::Prod, &self.prod),
        ] {
            for name in mapping.gitlab.iter().chain(mapping.otel.iter()) {
                insert_source(&mut by_name, name, env)?;
            }
            for namespace in &mapping.k8s_namespaces {
                insert_namespace(&mut namespaces, namespace, env)?;
            }
        }
        Ok(EnvIndex { by_name })
    }
}

/// Load every config file under `root/config`.
///
/// # Errors
///
/// Returns an error when a file is missing, malformed, or violates an invariant.
pub fn load_project(root: &Path) -> Result<ProjectConfig, Error> {
    let dir = root.join("config");
    let config = parse_config(&read_to_string(&dir.join("config.yaml"))?)?;
    let environments = parse_environments(&read_to_string(&dir.join("environments.yaml"))?)?;
    let env_index = environments.index()?;
    let aliases = parse_aliases(&read_to_string(&dir.join("aliases.yaml"))?)?;
    let pilot = parse_pilot(&read_to_string(&dir.join("pilot.yaml"))?)?;
    let sdk_hints = parse_sdk_hints(&read_to_string(&dir.join("sdk_hints.yaml"))?)?;
    Ok(ProjectConfig {
        config,
        environments,
        env_index,
        aliases,
        pilot,
        sdk_hints,
    })
}

/// Parse and check `config.yaml` text.
///
/// # Errors
///
/// Returns an error when the document is not the F0-1 shape.
pub fn parse_config(text: &str) -> Result<Config, Error> {
    let config: Config = yaml_serde::from_str(text).map_err(|err| {
        config_error(format!("config.yaml: {}", redact::redact(&err.to_string())))
    })?;
    validate_config(&config)?;
    Ok(config)
}

/// Parse and check `environments.yaml` text.
///
/// # Errors
///
/// Returns an error when a source name is ambiguous or a namespace is invalid.
pub fn parse_environments(text: &str) -> Result<Environments, Error> {
    let environments: Environments = yaml_serde::from_str(text).map_err(|err| {
        config_error(format!(
            "environments.yaml: {}",
            redact::redact(&err.to_string())
        ))
    })?;
    let _ = environments.index()?;
    Ok(environments)
}

/// Parse `aliases.yaml` text.
///
/// # Errors
///
/// Returns an error when an alias is duplicated or names more than one component.
pub fn parse_aliases(text: &str) -> Result<Aliases, Error> {
    reject_duplicate_alias_keys(text)?;
    let value: yaml_serde::Value = yaml_serde::from_str(text).map_err(|err| {
        config_error(format!(
            "aliases.yaml: {}",
            redact::redact(&err.to_string())
        ))
    })?;
    let Some(root) = value.as_mapping() else {
        return Err(config_error("aliases.yaml must be a mapping"));
    };
    for key in root.keys() {
        let Some(name) = key.as_str() else {
            return Err(config_error("aliases.yaml keys must be strings"));
        };
        if name != "aliases" {
            return Err(config_error(format!(
                "unknown field `{name}` in aliases.yaml"
            )));
        }
    }
    let Some(aliases_value) = root.get(yaml_serde::Value::String("aliases".to_owned())) else {
        return Err(config_error("aliases.yaml is missing `aliases`"));
    };
    let Some(mapping) = aliases_value.as_mapping() else {
        return Err(config_error("`aliases` must be a mapping"));
    };
    let mut aliases = BTreeMap::new();
    for (key, target) in mapping {
        let Some(alias) = key.as_str() else {
            return Err(config_error("alias keys must be strings"));
        };
        if alias.is_empty() || alias != alias.trim() {
            return Err(config_error("alias keys must be non-empty"));
        }
        if let Some(kind) = redact::secret_kind(alias) {
            return Err(config_error(format!("alias key contains {kind}")));
        }
        let component = match target {
            yaml_serde::Value::String(name) => name.clone(),
            yaml_serde::Value::Sequence(items) => {
                return Err(config_error(format!(
                    "alias `{alias}` must name exactly one component, found {}",
                    items.len()
                )));
            }
            _ => {
                return Err(config_error(format!(
                    "alias `{alias}` must name exactly one component"
                )));
            }
        };
        if let Some(kind) = redact::secret_kind(&component) {
            return Err(config_error(format!("alias `{alias}` contains {kind}")));
        }
        aliases.insert(alias.to_owned(), component);
    }
    Ok(Aliases { aliases })
}

/// Parse `pilot.yaml` text.
///
/// # Errors
///
/// Returns an error when an entry is not a kebab-case name plus a project path.
pub fn parse_pilot(text: &str) -> Result<PilotFile, Error> {
    let pilot: PilotFile = yaml_serde::from_str(text)
        .map_err(|err| config_error(format!("pilot.yaml: {}", redact::redact(&err.to_string()))))?;
    for app in &pilot.apps {
        if !crate::ids::is_kebab_case(&app.name) {
            return Err(config_error(format!(
                "pilot app `{}` must be lowercase kebab-case",
                app.name
            )));
        }
        if !is_gitlab_path(&app.gitlab_path) {
            return Err(config_error(format!(
                "pilot gitlab_path `{}` is not a project path",
                app.gitlab_path
            )));
        }
    }
    scan_config_secrets("pilot.yaml", &pilot)?;
    Ok(pilot)
}

/// Parse `sdk_hints.yaml` text.
///
/// # Errors
///
/// Returns an error when a hint is empty or contains a secret.
pub fn parse_sdk_hints(text: &str) -> Result<SdkHintsFile, Error> {
    let hints: SdkHintsFile = yaml_serde::from_str(text).map_err(|err| {
        config_error(format!(
            "sdk_hints.yaml: {}",
            redact::redact(&err.to_string())
        ))
    })?;
    for hint in &hints.hints {
        if hint.prefix.trim().is_empty() || hint.language.trim().is_empty() {
            return Err(config_error("sdk hints need a prefix and a language"));
        }
    }
    scan_config_secrets("sdk_hints.yaml", &hints)?;
    Ok(hints)
}

fn validate_config(config: &Config) -> Result<(), Error> {
    if config.schema_version != "0.1.0" {
        return Err(config_error(format!(
            "config schema_version must be 0.1.0, found {}",
            config.schema_version
        )));
    }
    check_http_url("gitlab.base_url", &config.gitlab.base_url)?;
    if config.gitlab.groups.is_empty() {
        return Err(config_error("gitlab.groups must list at least one group"));
    }
    for group in &config.gitlab.groups {
        if !is_gitlab_path(group) {
            return Err(config_error(format!("gitlab group `{group}` is invalid")));
        }
    }
    if !(1..=100).contains(&config.gitlab.per_page) {
        return Err(config_error("gitlab.per_page must be between 1 and 100"));
    }
    if !(1..=32).contains(&config.gitlab.max_concurrency) {
        return Err(config_error(
            "gitlab.max_concurrency must be between 1 and 32",
        ));
    }
    if !is_relative_file(&config.gitlab.catalog_file) {
        return Err(config_error(
            "gitlab.catalog_file must be a file name such as catalog-info.yaml",
        ));
    }
    if config.kubernetes.clusters.is_empty() {
        return Err(config_error(
            "kubernetes.clusters must list the cluster (ADR-001 assumes one)",
        ));
    }
    for cluster in &config.kubernetes.clusters {
        if !is_kebab_name(&cluster.name) {
            return Err(config_error(format!(
                "cluster name `{}` must be lowercase kebab-case",
                cluster.name
            )));
        }
        if !is_env_var_name(&cluster.kubeconfig_env) {
            return Err(config_error(format!(
                "kubeconfig_env `{}` must be an environment variable name, not file contents",
                cluster.kubeconfig_env
            )));
        }
    }
    check_http_url("prometheus.url", &config.prometheus.url)?;
    if !is_label(&config.prometheus.env_label) {
        return Err(config_error("prometheus.env_label must be a label name"));
    }
    if !(1..=90).contains(&config.prometheus.window_days) {
        return Err(config_error(
            "prometheus.window_days must be between 1 and 90",
        ));
    }
    for source in &config.sbom.sources {
        if source.contains("://") {
            check_http_url("sbom.sources", source)?;
        } else if source.trim().is_empty() {
            return Err(config_error("sbom source must not be empty"));
        }
    }
    for prefix in &config.sbom.internal_package_prefixes {
        if prefix.trim().is_empty() {
            return Err(config_error("internal package prefix must not be empty"));
        }
    }
    if config.contract_paths.is_empty() {
        return Err(config_error("contract_paths must not be empty"));
    }
    for path in &config.contract_paths {
        if path.trim().is_empty() {
            return Err(config_error("contract path must not be empty"));
        }
    }
    if config.edge_expiry_days == 0 {
        return Err(config_error("edge_expiry_days must be at least 1"));
    }
    if config.retention_days < config.edge_expiry_days {
        return Err(config_error(
            "retention_days must be greater than or equal to edge_expiry_days",
        ));
    }
    scan_config_secrets("config.yaml", config)?;
    Ok(())
}

fn insert_source(map: &mut BTreeMap<String, Env>, name: &str, env: Env) -> Result<(), Error> {
    let cleaned = clean_source_name(name)?;
    if let Some(existing) = map.get(&cleaned) {
        if *existing != env {
            return Err(config_error(format!(
                "source env `{name}` maps to both {existing} and {env}"
            )));
        }
        return Ok(());
    }
    map.insert(cleaned, env);
    Ok(())
}

fn insert_namespace(map: &mut BTreeMap<String, Env>, name: &str, env: Env) -> Result<(), Error> {
    if !is_dns_label(name) {
        return Err(config_error(format!(
            "k8s namespace `{name}` must be a DNS label"
        )));
    }
    if let Some(existing) = map.get(name) {
        if *existing != env {
            return Err(config_error(format!(
                "k8s namespace `{name}` maps to both {existing} and {env}"
            )));
        }
        return Ok(());
    }
    map.insert(name.to_owned(), env);
    Ok(())
}

fn clean_source_name(name: &str) -> Result<String, Error> {
    if name.is_empty() || name != name.trim() {
        return Err(config_error("source env name must not be empty or padded"));
    }
    Ok(name.to_ascii_lowercase())
}

fn check_http_url(field: &str, url: &str) -> Result<(), Error> {
    if redact::secret_kind(url).is_some() || url.contains('@') {
        return Err(config_error(format!(
            "{field} must not contain credentials"
        )));
    }
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .ok_or_else(|| config_error(format!("{field} must be an http(s) URL")))?;
    if rest.is_empty() || rest.contains(['?', '#', ' ']) {
        return Err(config_error(format!("{field} is not a usable base URL")));
    }
    Ok(())
}

fn scan_config_secrets(label: &str, value: &impl Serialize) -> Result<(), Error> {
    let json = serde_json::to_value(value).map_err(|err| Error::Config(err.to_string()))?;
    if let Some(hit) = redact::find_secrets(&json).into_iter().next() {
        return Err(config_error(format!(
            "{label} {} contains {}",
            hit.path, hit.kind
        )));
    }
    Ok(())
}

fn reject_duplicate_alias_keys(text: &str) -> Result<(), Error> {
    let mut seen_root = false;
    let mut in_aliases = false;
    let mut counts: BTreeMap<String, u32> = BTreeMap::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        if !in_aliases {
            if trimmed == "aliases:" {
                seen_root = true;
                in_aliases = true;
                continue;
            }
            if trimmed == "aliases: {}" {
                seen_root = true;
                continue;
            }
            if trimmed.starts_with("aliases:") {
                return Err(config_error(
                    "write aliases as a block mapping or `aliases: {}`",
                ));
            }
            continue;
        }
        if !line.starts_with([' ', '\t']) {
            in_aliases = false;
            continue;
        }
        if trimmed.starts_with("- ") {
            return Err(config_error(
                "alias targets must be a single component name, not a list",
            ));
        }
        let Some((key, _)) = trimmed.split_once(':') else {
            return Err(config_error(format!("invalid alias line `{trimmed}`")));
        };
        let key = key.trim().trim_matches('"').trim_matches('\'');
        if key.is_empty() {
            return Err(config_error("empty alias"));
        }
        *counts.entry(key.to_owned()).or_insert(0) += 1;
    }
    if !seen_root {
        return Err(config_error("alias file is missing `aliases`"));
    }
    let duplicates: Vec<_> = counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(key, _)| key)
        .collect();
    if duplicates.is_empty() {
        Ok(())
    } else {
        Err(config_error(format!(
            "alias `{}` is defined more than once",
            duplicates.join(", ")
        )))
    }
}

fn is_gitlab_path(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value.ends_with('/')
        && !value.contains("//")
        && value.split('/').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'-'))
        })
}

fn is_relative_file(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('/')
        && !value.contains("..")
        && !value.contains('\\')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn is_kebab_name(value: &str) -> bool {
    crate::ids::is_kebab_case(value)
}

fn is_env_var_name(value: &str) -> bool {
    let mut bytes = value.bytes();
    match bytes.next() {
        Some(first) if first.is_ascii_uppercase() || first == b'_' => {}
        _ => return false,
    }
    bytes.all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn is_label(value: &str) -> bool {
    let mut bytes = value.bytes();
    match bytes.next() {
        Some(first) if first.is_ascii_alphabetic() || first == b'_' => {}
        _ => return false,
    }
    bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn is_dns_label(value: &str) -> bool {
    (1..=63).contains(&value.len())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !value.starts_with('-')
        && !value.ends_with('-')
}

fn read_to_string(path: &Path) -> Result<String, Error> {
    fs::read_to_string(path)
        .map_err(|err| config_error(format!("reading {}: {err}", path.display())))
}

fn config_error(message: impl Into<String>) -> Error {
    Error::Config(redact::redact(&message.into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_root() -> &'static Path {
        Path::new(env!("CARGO_MANIFEST_DIR"))
    }

    #[test]
    fn committed_config_matches_the_phase_0_defaults() {
        let project = load_project(project_root()).unwrap();
        assert_eq!(project.config.schema_version, "0.1.0");
        assert_eq!(project.config.gitlab.base_url, "https://gitlab.example.com");
        assert_eq!(project.config.gitlab.groups, vec!["example-group/pilot"]);
        assert_eq!(project.config.gitlab.catalog_file, "catalog-info.yaml");
        assert_eq!(project.config.gitlab.per_page, 100);
        assert_eq!(project.config.gitlab.max_concurrency, 4);
        assert_eq!(project.config.kubernetes.clusters.len(), 1);
        assert_eq!(
            project.config.kubernetes.clusters[0].kubeconfig_env,
            "DEPGRAPH_KUBECONFIG_MAIN"
        );
        assert_eq!(
            project.config.prometheus.url,
            "https://prometheus.example.com"
        );
        assert_eq!(project.config.prometheus.env_label, "env");
        assert_eq!(project.config.prometheus.window_days, 7);
        assert_eq!(project.config.runtime_source, RuntimeSource::None);
        assert!(project.config.sbom.sources.is_empty());
        assert!(project.config.sbom.internal_package_prefixes.is_empty());
        assert_eq!(
            project.config.contract_paths,
            vec!["openapi/**", "**/*.proto", "schema/**"]
        );
        assert_eq!(project.config.edge_expiry_days, 30);
        assert_eq!(project.config.retention_days, 90);
        assert!(project.aliases.aliases.is_empty());
        assert!(project.pilot.apps.is_empty());
        assert!(project.sdk_hints.hints.is_empty());
        assert_eq!(project.env_index.resolve("development"), Some(Env::Dev));
        assert_eq!(project.env_index.resolve("Development"), Some(Env::Dev));
        assert_eq!(project.env_index.resolve("staging"), Some(Env::Acc));
        assert_eq!(project.env_index.resolve("acceptance"), Some(Env::Acc));
        assert_eq!(project.env_index.resolve("production"), Some(Env::Prod));
        assert_eq!(project.env_index.resolve("prod"), Some(Env::Prod));
        assert_eq!(project.env_index.resolve("qa"), None);
    }

    #[test]
    fn the_same_source_name_cannot_map_to_two_envs() {
        let yaml = r"
dev:
  gitlab: [staging]
  otel: [dev]
acc:
  gitlab: [staging]
  otel: [acc]
prod:
  gitlab: [production]
  otel: [prod]
";
        let error = parse_environments(yaml).unwrap_err();
        assert!(error.to_string().contains("staging"));
        assert!(error.to_string().contains("dev"));
        assert!(error.to_string().contains("acc"));
    }

    #[test]
    fn repeated_source_name_in_one_env_is_ok() {
        let yaml = r"
dev:
  gitlab: [development]
  otel: [dev, development]
acc:
  gitlab: [acceptance, staging]
  otel: [acc, staging]
prod:
  gitlab: [production]
  otel: [prod, production]
";
        let environments = parse_environments(yaml).unwrap();
        assert_eq!(
            environments.index().unwrap().resolve("development"),
            Some(Env::Dev)
        );
    }

    #[test]
    fn a_namespace_cannot_belong_to_two_envs() {
        let yaml = r"
dev:
  gitlab: [development]
  otel: [dev]
  k8s_namespaces: [app]
acc:
  gitlab: [acceptance]
  otel: [acc]
  k8s_namespaces: [app]
prod:
  gitlab: [production]
  otel: [prod]
";
        let error = parse_environments(yaml).unwrap_err();
        assert!(error.to_string().contains("namespace `app`"));
    }

    #[test]
    fn credentialed_urls_and_kubeconfig_bodies_are_rejected() {
        let mut text = include_str!("../config/config.yaml").to_owned();
        text = text.replace(
            "https://gitlab.example.com",
            "https://user:password@gitlab.example.com",
        );
        let error = parse_config(&text).unwrap_err();
        assert!(error.to_string().contains("credentials"));
        assert!(!error.to_string().contains("password"));

        let with_body = include_str!("../config/config.yaml").replace(
            "kubeconfig_env: DEPGRAPH_KUBECONFIG_MAIN",
            "kubeconfig: |\n        apiVersion: v1",
        );
        assert!(parse_config(&with_body).is_err());
    }

    #[test]
    fn duplicate_and_multi_target_aliases_fail() {
        let duplicated = "aliases:\n  payments: payments-api\n  payments: billing-api\n";
        assert!(
            parse_aliases(duplicated)
                .unwrap_err()
                .to_string()
                .contains("more than once")
        );
        let many = "aliases:\n  payments:\n    - payments-api\n    - billing-api\n";
        assert!(
            parse_aliases(many)
                .unwrap_err()
                .to_string()
                .contains("exactly one")
                || parse_aliases(many)
                    .unwrap_err()
                    .to_string()
                    .contains("not a list")
        );
    }

    #[test]
    fn a_single_alias_loads() {
        let aliases = parse_aliases("aliases:\n  payments: payments-api\n").unwrap();
        assert_eq!(
            aliases.aliases.get("payments").map(String::as_str),
            Some("payments-api")
        );
    }
}
