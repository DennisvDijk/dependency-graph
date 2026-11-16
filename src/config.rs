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
    /// Pilot file. An empty app list means every app in the configured groups.
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
    /// Primary runtime signal. ADR-003 selects Istio.
    pub runtime_source: RuntimeSource,
    /// Optional Tempo source. Off until metric names are verified.
    pub tempo: TempoConfig,
    /// Library nodes and `uses_library` edges. Off: there are no internal libraries.
    pub libraries: LibraryConfig,
    /// Optional read-only Datadog status. Key fields are environment variable names.
    pub datadog: DatadogConfig,
    /// Where CycloneDX documents come from, and which packages are internal.
    pub sbom: SbomConfig,
    /// Globs for API contracts.
    pub contract_paths: Vec<String>,
    /// Days after `last_seen` before an edge is expired.
    pub edge_expiry_days: u32,
    /// Days a snapshot is kept.
    pub retention_days: u32,
    /// MCP resource server. OAuth stays with the external issuer.
    #[serde(default)]
    pub mcp: McpConfig,
    /// Optional IdP preset. `generic` is the default resource server.
    #[serde(default)]
    pub auth: AuthConfig,
    /// Path-score hardness factors. Assumptions, not measurements.
    #[serde(default)]
    pub scoring: ScoringConfig,
    /// MR check policy. Blocking is off until a team opts in.
    #[serde(default)]
    pub mr_check: MrCheckConfig,
    /// Opt-in live AWS read with the `aws` CLI. Off by default.
    #[serde(default)]
    pub aws: AwsConfig,
    /// Opt-in live Azure read with the `az` CLI. Off by default.
    #[serde(default)]
    pub azure: AzureConfig,
}

/// Live AWS discovery. No key fields: the `aws` CLI's own login is used.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AwsConfig {
    /// Read the account on every `depgraph collect`. `--aws` does it once.
    #[serde(default)]
    pub enabled: bool,
    /// AWS CLI profile name. Empty uses the default credential chain.
    #[serde(default)]
    pub profile: Option<String>,
    /// Regions to read. Empty uses the CLI's default region.
    #[serde(default)]
    pub regions: Vec<String>,
    /// Environment of this account (`dev`, `acc`, `prod` or a mapped name).
    /// Empty: an env word in each service name is used (`orders-prod`).
    #[serde(default)]
    pub env: Option<String>,
    /// Discover default-format VPC flow log groups automatically; off by default.
    #[serde(default)]
    pub flow_logs: bool,
    /// Optional explicit CloudWatch log group with default-format VPC flow logs.
    #[serde(default)]
    pub flow_log_group: Option<String>,
    /// Also read the X-Ray service graph for the calls that really happen.
    #[serde(default = "default_true")]
    pub xray: bool,
}

impl Default for AwsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            profile: None,
            regions: Vec::new(),
            env: None,
            flow_log_group: None,
            flow_logs: false,
            xray: true,
        }
    }
}

/// Live Azure discovery. No key fields: the `az` CLI's own login is used.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AzureConfig {
    /// Read Azure on every `depgraph collect`. `--azure` does it once.
    #[serde(default)]
    pub enabled: bool,
    /// Subscription ids or names. Empty uses the `az` default subscription.
    #[serde(default)]
    pub subscriptions: Vec<String>,
    /// Log Analytics workspace ids that hold Application Insights data.
    #[serde(default)]
    pub log_analytics_workspaces: Vec<String>,
    /// Environment of these subscriptions. Empty: an env word in the name.
    #[serde(default)]
    pub env: Option<String>,
    /// Discover VNet Traffic Analytics workspaces and network flows. Off by default.
    #[serde(default)]
    pub flow_logs: bool,
}

/// GitLab connection settings. `base_url` is a placeholder until a human confirms it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GitlabConfig {
    /// API origin, without userinfo. Example: `https://gitlab.example.com`.
    pub base_url: String,
    /// Optional group or subgroup paths. Empty means every project the token can see.
    /// One path is recursive: `platform` or `org/infra/platform`.
    #[serde(default)]
    pub groups: Vec<String>,
    /// Explicit project paths, fetched one by one and unioned with `groups`.
    /// Example: `org/other-team/ingress-gateway`.
    #[serde(default)]
    pub projects: Vec<String>,
    /// Globs on `path_with_namespace`. Empty keeps every listed project.
    #[serde(default)]
    pub include_paths: Vec<String>,
    /// Globs on `path_with_namespace` to drop after listing.
    #[serde(default)]
    pub exclude_paths: Vec<String>,
    /// Keep a project when it has any of these topics. Empty keeps all.
    #[serde(default)]
    pub topics: Vec<String>,
    /// Drop forks. Default true.
    #[serde(default = "default_true")]
    pub exclude_forks: bool,
    /// Drop archived projects. Default true.
    #[serde(default = "default_true")]
    pub exclude_archived: bool,
    /// Oldest last activity to keep, such as `180d`, `24h`, or `2w`.
    #[serde(default)]
    pub min_last_activity: Option<String>,
    /// `public`, `internal`, or `private`. Absent keeps every visibility.
    #[serde(default)]
    pub visibility: Option<String>,
    /// Catalog file name looked up in each project.
    pub catalog_file: String,
    /// Environment variable that holds the bot token for MR notes and drift issues.
    /// The token is not stored here.
    #[serde(default = "default_bot_token_env")]
    pub bot_token_env: String,
    /// Page size for list endpoints. GitLab allows at most 100.
    pub per_page: u32,
    /// Parallel requests one collector may have in flight.
    pub max_concurrency: u32,
}

fn default_true() -> bool {
    true
}

fn default_bot_token_env() -> String {
    "DEPGRAPH_GITLAB_BOT_TOKEN".to_owned()
}

/// MCP server settings. Values are placeholders until an IdP is connected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct McpConfig {
    /// Issuer that signs access tokens.
    pub issuer: String,
    /// Expected audience / resource indicator (RFC 8707).
    pub audience: String,
    /// JWKS URL. Keys are cached and refreshed when a `kid` is unknown.
    pub jwks_url: String,
    /// Listen address. `--insecure-local` only allows a loopback address.
    pub bind: String,
}

/// `generic` keeps issuer and JWKS in `mcp`. `entra` derives them from the tenant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthProvider {
    /// Any OAuth 2.1 issuer with a JWKS document.
    #[default]
    Generic,
    /// Microsoft Entra ID v2 endpoints.
    Entra,
}

/// Identity provider settings. Placeholder IDs only; real values may come from the environment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct AuthConfig {
    /// `generic` or `entra`.
    pub provider: AuthProvider,
    /// Entra tenant GUID. Overridden by `DEPGRAPH_ENTRA_TENANT_ID` when set.
    pub tenant_id: String,
    /// Entra application id. Overridden by `DEPGRAPH_ENTRA_CLIENT_ID` when set.
    pub client_id: String,
    /// Resource indicator. Empty becomes `api://<client_id>` for Entra.
    pub audience: String,
    /// App roles that may call the API, in addition to the read role.
    pub allowed_roles: Vec<String>,
    /// Group object ids that may call the API. Empty means no group restriction.
    pub allowed_groups: Vec<String>,
}

/// Hardness factors stored as text so the config struct stays `Eq`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct ScoringConfig {
    /// Factor for a hard edge.
    pub hard: String,
    /// Factor for an unknown edge.
    pub unknown: String,
    /// Factor for a soft edge.
    pub soft: String,
}

impl Default for ScoringConfig {
    fn default() -> Self {
        Self {
            hard: "1.0".into(),
            unknown: "0.5".into(),
            soft: "0.2".into(),
        }
    }
}

impl ScoringConfig {
    /// Parse the factors.
    ///
    /// # Errors
    ///
    /// Returns an error when a factor is not a finite number from 0 to 1.
    pub fn hardness(&self) -> Result<crate::score::Hardness, Error> {
        Ok(crate::score::Hardness {
            hard: parse_factor("scoring.hard", &self.hard)?,
            unknown: parse_factor("scoring.unknown", &self.unknown)?,
            soft: parse_factor("scoring.soft", &self.soft)?,
        })
    }
}

fn parse_factor(field: &str, value: &str) -> Result<f64, Error> {
    let number: f64 = value
        .parse()
        .map_err(|_| Error::Config(format!("{field} must be a number")))?;
    if !(0.0..=1.0).contains(&number) {
        return Err(Error::Config(format!("{field} must be between 0 and 1")));
    }
    Ok(number)
}

/// Opt-in gate for merge requests. Off in the example config.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct MrCheckConfig {
    /// When true, a contract change with observed prod consumers needs approval.
    pub blocking: bool,
    /// A label that records an explicit override.
    pub override_label: String,
}

impl Default for MrCheckConfig {
    fn default() -> Self {
        Self {
            blocking: false,
            override_label: "depgraph-override".into(),
        }
    }
}

impl Default for AuthConfig {
    fn default() -> Self {
        Self {
            provider: AuthProvider::Generic,
            tenant_id: "00000000-0000-0000-0000-000000000000".to_owned(),
            client_id: "11111111-1111-1111-1111-111111111111".to_owned(),
            audience: "api://11111111-1111-1111-1111-111111111111".to_owned(),
            allowed_roles: Vec::new(),
            allowed_groups: Vec::new(),
        }
    }
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            issuer: "https://idp.example.com".to_owned(),
            audience: "http://127.0.0.1:8080".to_owned(),
            jwks_url: "https://idp.example.com/.well-known/jwks.json".to_owned(),
            bind: "127.0.0.1:8080".to_owned(),
        }
    }
}

impl GitlabConfig {
    /// Filters applied after the GitLab list returns.
    ///
    /// # Errors
    ///
    /// Returns an error when `min_last_activity` or `visibility` is invalid.
    pub fn project_filter(&self) -> Result<crate::gitlab::ProjectFilter, Error> {
        let min_last_activity = match &self.min_last_activity {
            Some(value) => Some(crate::gitlab::parse_activity_window(value)?),
            None => None,
        };
        if let Some(visibility) = &self.visibility
            && !matches!(visibility.as_str(), "public" | "internal" | "private")
        {
            return Err(Error::Config(format!(
                "gitlab.visibility `{visibility}` must be public, internal, or private"
            )));
        }
        Ok(crate::gitlab::ProjectFilter {
            include_paths: self.include_paths.clone(),
            exclude_paths: self.exclude_paths.clone(),
            topics: self.topics.clone(),
            exclude_forks: self.exclude_forks,
            exclude_archived: self.exclude_archived,
            min_last_activity,
            visibility: self.visibility.clone(),
        })
    }
}

/// Kubernetes settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KubernetesConfig {
    /// When true, also read workloads with kubectl. Default false: repos only.
    #[serde(default)]
    pub live_read: bool,
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
    /// Query window in days. Istio queries use this unless retention is shorter.
    pub window_days: u32,
    /// How long Prometheus keeps samples. Below `window_days`, queries use `[1d]`.
    #[serde(default)]
    pub retention_days: Option<u32>,
    /// Request paths dropped as health checks. Matched against `request_path` or `path`.
    #[serde(default)]
    pub health_paths: Vec<String>,
    /// Source workloads dropped as known test clients.
    #[serde(default)]
    pub test_clients: Vec<String>,
    /// Environment variable that holds a bearer token. The token is not stored here.
    #[serde(default)]
    pub token_env: Option<String>,
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
    /// No runtime source.
    None,
}

/// Optional Tempo endpoint. The URL is a placeholder, never a credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TempoConfig {
    /// When false, collectors skip Tempo.
    pub enabled: bool,
    /// Base URL without userinfo.
    pub url: String,
}

/// Whether collectors may emit library nodes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LibraryConfig {
    /// Default false. The model types stay available for a later decision.
    pub enabled: bool,
}

/// Optional Datadog status source. Secrets stay in the named env vars.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatadogConfig {
    /// When false, the Datadog collector is not selected.
    pub enabled: bool,
    /// Site hostname, such as `api.datadog.example`. Not a URL and not a key.
    pub site: String,
    /// Environment variable that holds the API key at runtime.
    pub api_key_env: String,
    /// Environment variable that holds the application key at runtime.
    pub app_key_env: String,
}

/// SBOM inputs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SbomConfig {
    /// Artifact globs or URLs. URLs must not carry credentials.
    #[serde(default)]
    pub sources: Vec<String>,
    /// Package prefixes that become `Library` nodes. Must be empty while libraries are disabled.
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
    /// Opt-in: use the guess for an unmapped namespace (`shop-prod` -> prod)
    /// instead of leaving its calls out. Off by default (ADR-008). The build
    /// lists every guessed namespace so it can be checked.
    #[serde(default)]
    pub guess_namespaces: bool,
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

/// Case-insensitive index of source environment names and namespaces.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnvIndex {
    by_name: BTreeMap<String, Env>,
    by_namespace: BTreeMap<String, Env>,
    guess: bool,
}

impl EnvIndex {
    /// Resolve a source environment name.
    #[must_use]
    pub fn resolve(&self, name: &str) -> Option<Env> {
        self.by_name.get(&name.to_ascii_lowercase()).copied()
    }

    /// Resolve a Kubernetes namespace. Unknown namespaces are not guessed,
    /// unless `guess_namespaces` is on.
    #[must_use]
    pub fn resolve_namespace(&self, name: &str) -> Option<Env> {
        self.by_namespace
            .get(&name.to_ascii_lowercase())
            .copied()
            .or_else(|| {
                if self.guess {
                    self.suggest_namespace(name)
                } else {
                    None
                }
            })
    }

    /// The guessed environment, when `guess_namespaces` is on and the
    /// namespace is not mapped. Used to list guesses after a build.
    #[must_use]
    pub fn guessed_namespace(&self, name: &str) -> Option<Env> {
        if !self.guess || self.by_namespace.contains_key(&name.to_ascii_lowercase()) {
            return None;
        }
        self.suggest_namespace(name)
    }

    /// Suggest an environment for an unmapped namespace, for a setup hint only.
    ///
    /// The first or last `-` segment must be a canonical env (`dev`, `acc`,
    /// `prod`) or a mapped GitLab/OTel name, and all matches must agree.
    /// The graph never uses this: unmapped namespaces stay unresolved (ADR-008).
    #[must_use]
    pub fn suggest_namespace(&self, name: &str) -> Option<Env> {
        let lower = name.to_ascii_lowercase();
        let segments: Vec<&str> = lower.split('-').filter(|part| !part.is_empty()).collect();
        let candidates = match segments.as_slice() {
            [] => Vec::new(),
            [only] => vec![*only],
            [first, .., last] => vec![*first, *last],
        };
        let mut found: Option<Env> = None;
        for segment in candidates {
            let env = match segment {
                "dev" => Some(Env::Dev),
                "acc" => Some(Env::Acc),
                "prod" => Some(Env::Prod),
                other => self.by_name.get(other).copied(),
            };
            match (found, env) {
                (_, None) => {}
                (None, Some(env)) => found = Some(env),
                (Some(left), Some(right)) if left == right => {}
                (Some(_), Some(_)) => return None,
            }
        }
        found
    }
}

/// Alias table. Each alias names exactly one component.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Aliases {
    /// Alias to canonical component name, without the `component:` prefix.
    pub aliases: BTreeMap<String, String>,
}

/// `config/pilot.yaml`. Empty `apps` means all applications in the GitLab groups.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct PilotFile {
    /// Informational names. They do not shrink the pilot. Scope is every project in the configured groups.
    #[serde(default)]
    pub apps: Vec<PilotApp>,
}

/// Projects in `groups` are in scope. `pilot.apps` never removes a project (ADR-004).
#[must_use]
pub fn pilot_project_paths<'a>(
    groups: &[impl AsRef<str>],
    project_paths: &'a [impl AsRef<str>],
) -> Vec<&'a str> {
    project_paths
        .iter()
        .map(AsRef::as_ref)
        .filter(|path| groups.iter().any(|group| in_group(path, group.as_ref())))
        .collect()
}

fn in_group(path: &str, group: &str) -> bool {
    path == group || path.starts_with(&format!("{group}/"))
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
        Ok(EnvIndex {
            by_name,
            by_namespace: namespaces,
            guess: self.guess_namespaces,
        })
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
    for group in &config.gitlab.groups {
        if !is_gitlab_path(group) {
            return Err(config_error(format!(
                "gitlab group `{group}` must be a group or subgroup path such as platform or org/infra/platform"
            )));
        }
    }
    for project in &config.gitlab.projects {
        if !is_project_path(project) {
            return Err(config_error(format!(
                "gitlab project `{project}` must be a project path such as org/other-team/ingress-gateway"
            )));
        }
    }
    for pattern in config
        .gitlab
        .include_paths
        .iter()
        .chain(config.gitlab.exclude_paths.iter())
    {
        if pattern.trim().is_empty() {
            return Err(config_error("gitlab path globs must not be empty"));
        }
    }
    let _ = config.gitlab.project_filter()?;
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
    check_http_url("tempo.url", &config.tempo.url)?;
    if !config.libraries.enabled && !config.sbom.internal_package_prefixes.is_empty() {
        return Err(config_error(
            "libraries are disabled; internal_package_prefixes must be empty",
        ));
    }
    if !is_datadog_site(&config.datadog.site) {
        return Err(config_error(
            "datadog.site must be a hostname, not a URL or a key",
        ));
    }
    for (field, name) in [
        ("datadog.api_key_env", config.datadog.api_key_env.as_str()),
        ("datadog.app_key_env", config.datadog.app_key_env.as_str()),
    ] {
        if !is_env_var_name(name) || redact::secret_kind(name).is_some() {
            return Err(config_error(format!(
                "{field} must be an environment variable name, not key material"
            )));
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

fn is_project_path(value: &str) -> bool {
    is_gitlab_path(value) && value.contains('/')
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

fn is_datadog_site(value: &str) -> bool {
    !value.is_empty()
        && !value.contains("://")
        && !value.contains('@')
        && !value.contains('/')
        && value.contains('.')
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
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
        assert!(project.config.gitlab.groups.is_empty());
        assert!(project.config.gitlab.projects.is_empty());
        assert!(project.config.gitlab.include_paths.is_empty());
        assert!(project.config.gitlab.exclude_paths.is_empty());
        assert!(project.config.gitlab.topics.is_empty());
        assert!(project.config.gitlab.exclude_forks);
        assert!(project.config.gitlab.exclude_archived);
        assert!(project.config.gitlab.min_last_activity.is_none());
        assert!(project.config.gitlab.visibility.is_none());
        assert_eq!(
            project.config.gitlab.bot_token_env,
            "DEPGRAPH_GITLAB_BOT_TOKEN"
        );
        assert_eq!(project.config.gitlab.catalog_file, "catalog-info.yaml");
        assert_eq!(project.config.gitlab.per_page, 100);
        assert_eq!(project.config.gitlab.max_concurrency, 4);
        assert!(!project.config.kubernetes.live_read);
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
        assert_eq!(project.config.prometheus.retention_days, Some(15));
        assert_eq!(
            project.config.prometheus.health_paths,
            vec!["/health", "/healthz", "/ready", "/live"]
        );
        assert_eq!(project.config.prometheus.test_clients, vec!["k6"]);
        assert!(project.config.prometheus.token_env.is_none());
        assert_eq!(project.config.runtime_source, RuntimeSource::Istio);
        assert!(!project.config.tempo.enabled);
        assert_eq!(project.config.tempo.url, "https://tempo.example.com");
        assert!(!project.config.libraries.enabled);
        assert!(!project.config.datadog.enabled);
        assert_eq!(project.config.datadog.site, "api.datadog.example");
        assert_eq!(
            project.config.datadog.api_key_env,
            "DEPGRAPH_DATADOG_API_KEY"
        );
        assert_eq!(
            project.config.datadog.app_key_env,
            "DEPGRAPH_DATADOG_APP_KEY"
        );
        assert!(project.config.sbom.sources.is_empty());
        assert!(project.config.sbom.internal_package_prefixes.is_empty());
        assert_eq!(
            project.config.contract_paths,
            vec!["openapi/**", "**/*.proto", "schema/**"]
        );
        assert_eq!(project.config.edge_expiry_days, 30);
        assert_eq!(project.config.retention_days, 90);
        assert!(!project.config.mr_check.blocking);
        assert_eq!(project.config.scoring.hard, "1.0");
        assert_eq!(project.config.mcp.issuer, "https://idp.example.com");
        assert_eq!(project.config.mcp.audience, "http://127.0.0.1:8080");
        assert_eq!(project.config.mcp.bind, "127.0.0.1:8080");
        assert_eq!(project.config.auth.provider, AuthProvider::Generic);
        assert_eq!(
            project.config.auth.tenant_id,
            "00000000-0000-0000-0000-000000000000"
        );
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
    fn disabled_libraries_reject_prefixes_and_datadog_rejects_raw_keys() {
        let with_prefix = include_str!("../config/config.yaml").replace(
            "internal_package_prefixes: []",
            "internal_package_prefixes:\n    - pkg:cargo/example-",
        );
        let error = parse_config(&with_prefix).unwrap_err();
        assert!(error.to_string().contains("libraries are disabled"));

        let with_key = include_str!("../config/config.yaml").replace(
            "api_key_env: DEPGRAPH_DATADOG_API_KEY",
            "api_key_env: glpat-ABCDEFGHIJKLMNOPQRST",
        );
        let error = parse_config(&with_key).unwrap_err();
        assert!(error.to_string().contains("environment variable name"));
        assert!(!error.to_string().contains("ABCDEFG"));
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
    fn namespace_suggestions_use_the_first_or_last_segment() {
        let index = parse_environments(
            "dev:\n  gitlab: [development]\nacc:\n  gitlab: [staging]\nprod:\n  gitlab: [production]\n",
        )
        .unwrap()
        .index()
        .unwrap();
        assert_eq!(index.suggest_namespace("shop-prod"), Some(Env::Prod));
        assert_eq!(index.suggest_namespace("staging-shop"), Some(Env::Acc));
        assert_eq!(index.suggest_namespace("dev"), Some(Env::Dev));
        assert_eq!(index.suggest_namespace("qa"), None);
        assert_eq!(index.suggest_namespace("prod-to-dev"), None);
        assert_eq!(index.suggest_namespace("shop-prod"), Some(Env::Prod));
        assert_eq!(index.resolve_namespace("shop-prod"), None);
        assert_eq!(index.guessed_namespace("shop-prod"), None);
    }

    #[test]
    fn guessing_namespaces_is_opt_in() {
        let yaml = r"
guess_namespaces: true
dev:
  gitlab: [development]
acc:
  gitlab: [staging]
prod:
  gitlab: [production]
  k8s_namespaces: [shop]
";
        let index = parse_environments(yaml).unwrap().index().unwrap();
        assert_eq!(index.resolve_namespace("shop-prod"), Some(Env::Prod));
        assert_eq!(index.guessed_namespace("shop-prod"), Some(Env::Prod));
        assert_eq!(index.resolve_namespace("shop"), Some(Env::Prod));
        assert_eq!(index.guessed_namespace("shop"), None);
        assert_eq!(index.resolve_namespace("qa"), None);
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

    #[test]
    fn group_or_subgroup_and_project_paths_are_accepted() {
        let text = include_str!("../config/config.yaml")
            .replace(
                "groups: []",
                "groups:\n    - platform\n    - org/infra/platform",
            )
            .replace(
                "projects: []",
                "projects:\n    - org/other-team/ingress-gateway",
            );
        let config = parse_config(&text).unwrap();
        assert_eq!(
            config.gitlab.groups,
            vec!["platform".to_owned(), "org/infra/platform".to_owned()]
        );
        assert_eq!(
            config.gitlab.projects,
            vec!["org/other-team/ingress-gateway".to_owned()]
        );
        let bad = include_str!("../config/config.yaml")
            .replace("projects: []", "projects:\n    - not a path");
        assert!(parse_config(&bad).is_err());
    }

    #[test]
    fn example_aliases_load_and_duplicates_still_fail() {
        let example = parse_aliases(include_str!("../config/aliases.example.yaml")).unwrap();
        assert_eq!(
            example.aliases.get("payments").map(String::as_str),
            Some("payments-api")
        );
        assert_eq!(
            example.aliases.get("billing").map(String::as_str),
            Some("billing-api")
        );
        let duplicated = "aliases:\n  payments: payments-api\n  payments: other-api\n";
        assert!(
            parse_aliases(duplicated)
                .unwrap_err()
                .to_string()
                .contains("more than once")
        );
    }

    #[test]
    fn pilot_scope_is_every_project_in_the_configured_groups() {
        let groups = ["example-group/pilot"];
        let projects = [
            "example-group/pilot/payments-api".to_owned(),
            "example-group/other/skip-me".to_owned(),
            "example-group/pilot/billing-api".to_owned(),
        ];
        let scope = pilot_project_paths(&groups, &projects);
        assert_eq!(
            scope,
            vec![
                "example-group/pilot/payments-api",
                "example-group/pilot/billing-api",
            ]
        );
    }

    #[test]
    fn environment_names_map_to_exactly_one_canonical_env() {
        let environments = parse_environments(include_str!("../config/environments.yaml")).unwrap();
        let index = environments.index().unwrap();
        let cases = [
            ("development", Env::Dev),
            ("dev", Env::Dev),
            ("acceptance", Env::Acc),
            ("staging", Env::Acc),
            ("acc", Env::Acc),
            ("production", Env::Prod),
            ("prod", Env::Prod),
        ];
        for (source, expected) in cases {
            assert_eq!(index.resolve(source), Some(expected), "{source}");
        }
        assert_eq!(index.resolve("qa"), None);
    }
}
