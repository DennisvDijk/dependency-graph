//! `depgraph init`: minimal config, no tokens.

use std::fs;
use std::path::Path;

use crate::error::Error;

const CONFIG: &str = r#"# Written by `depgraph init`. example.com only — put no tokens here.
schema_version: "0.1.0"
gitlab:
  base_url: https://gitlab.example.com
  groups: []
  projects: []
  exclude_forks: true
  exclude_archived: true
  catalog_file: catalog-info.yaml
  per_page: 100
  max_concurrency: 4
  bot_token_env: DEPGRAPH_GITLAB_BOT_TOKEN
# Opt-in live cloud discovery: off until you turn it on. depgraph runs the
# `aws` / `az` CLI with list, describe and read-only query calls only, using
# the login that CLI already has. No keys here. docs/tutorials/09-cloud-discovery.md
aws:
  enabled: false
  # profile: readonly        # AWS CLI profile name; empty uses the default chain
  regions: []                # empty: the CLI's default region
  # env: prod                # environment of this account; empty: from names like orders-prod
  xray: true                 # also read the X-Ray service map (calls that really happen)
azure:
  enabled: false
  subscriptions: []          # empty: the az default subscription
  log_analytics_workspaces: []  # workspace ids with Application Insights data
  # env: prod
kubernetes:
  live_read: false
  clusters:
    - name: main
      kubeconfig_env: DEPGRAPH_KUBECONFIG_MAIN
prometheus:
  url: https://prometheus.example.com
  env_label: env
  window_days: 7
  retention_days: 15
  health_paths: ["/health", "/healthz", "/ready", "/live"]
  test_clients: []
runtime_source: istio
tempo:
  enabled: false
  url: https://tempo.example.com
libraries:
  enabled: false
datadog:
  enabled: false
  site: api.datadog.example
  api_key_env: DEPGRAPH_DATADOG_API_KEY
  app_key_env: DEPGRAPH_DATADOG_APP_KEY
sbom:
  sources: []
  internal_package_prefixes: []
contract_paths:
  - openapi/**
  - "**/*.proto"
  - schema/**
edge_expiry_days: 30
retention_days: 90
auth:
  provider: generic
mcp:
  issuer: https://idp.example.com
  audience: http://127.0.0.1:8080
  jwks_url: https://idp.example.com/.well-known/jwks.json
  bind: 127.0.0.1:8080
"#;

const ENVIRONMENTS: &str = r#"# Set to true to use the guess for unmapped namespaces (shop-prod -> prod);
# `depgraph build` lists every guess. A mapping below always wins.
guess_namespaces: false
dev:
  gitlab: [development]
  otel: [dev, development]
  k8s_namespaces: []
acc:
  gitlab: [acceptance, staging]
  otel: [acc, staging]
  k8s_namespaces: []
prod:
  gitlab: [production]
  otel: [prod, production]
  k8s_namespaces: []
"#;

const ALIASES: &str = "aliases: {}\n";
const PILOT: &str = "apps: []\n";
const SDK: &str = "hints: []\n";

/// Write missing config files. Does not store a token.
///
/// `glab_host` is a hostname from `glab config get host`, or `None`.
///
/// # Errors
///
/// Returns an error when a file cannot be written or the host is not a hostname.
pub fn init(root: &Path, glab_host: Option<&str>, force: bool) -> Result<Vec<String>, Error> {
    let config_dir = root.join("config");
    fs::create_dir_all(&config_dir)?;
    let host = match glab_host {
        Some(host) if is_hostname(host) => Some(host.to_owned()),
        Some(_) => {
            return Err(Error::Config(
                "glab host was not a hostname. next: edit config/config.yaml gitlab.base_url"
                    .into(),
            ));
        }
        None => None,
    };
    let mut config = CONFIG.to_owned();
    if let Some(host) = &host {
        config = config.replace("https://gitlab.example.com", &format!("https://{host}"));
    }
    let files = [
        ("config.yaml", config.as_str()),
        ("environments.yaml", ENVIRONMENTS),
        ("aliases.yaml", ALIASES),
        ("pilot.yaml", PILOT),
        ("sdk_hints.yaml", SDK),
    ];
    let mut written = Vec::new();
    for (name, body) in files {
        let path = config_dir.join(name);
        if path.exists() && !force {
            continue;
        }
        fs::write(&path, body)?;
        written.push(format!("config/{name}"));
    }
    Ok(written)
}

fn is_hostname(host: &str) -> bool {
    !host.is_empty()
        && host.len() < 253
        && !host.contains([' ', '/', '@', ':'])
        && host
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
}
