//! Node and edge identifiers.
//!
//! Edge ids are a SHA-256 of the canonical tuple. The algorithm is frozen in
//! `docs/naming.md`; changing it is a schema break.

use sha2::{Digest, Sha256};

use crate::models::{EdgeType, Env};

/// A parsed node id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParsedNodeId {
    /// `component:<kebab>`.
    Component {
        /// Canonical component name.
        name: String,
    },
    /// `deployment:<kebab>@<env>`.
    Deployment {
        /// Component name this deployment belongs to.
        name: String,
        /// Environment of the deployment.
        env: Env,
    },
    /// `resource:<kind>:<kebab>`.
    Resource {
        /// Resource kind.
        kind: ResourceKind,
        /// Resource name.
        name: String,
    },
    /// `library:<purl-without-version>`.
    Library {
        /// Package URL without version, qualifiers, or subpath.
        purl: String,
    },
}

/// Resource kinds that may appear in a resource id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ResourceKind {
    /// A database.
    Database,
    /// A publish/subscribe topic.
    Topic,
    /// A queue.
    Queue,
    /// An API outside the estate.
    ExternalApi,
    /// A model inference endpoint.
    ModelEndpoint,
    /// An identity provider.
    IdentityProvider,
    /// A Kubernetes NetworkPolicy. Accepted in ADR-007.
    NetworkPolicy,
    /// A firewall rule declared in a repo. Accepted in ADR-007.
    FirewallRule,
    /// An infrastructure component from IaC. Accepted in ADR-007.
    Infra,
    /// Anything else that still needs a node.
    Other,
}

impl ResourceKind {
    /// Wire value used inside the id.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Database => "database",
            Self::Topic => "topic",
            Self::Queue => "queue",
            Self::ExternalApi => "external-api",
            Self::ModelEndpoint => "model-endpoint",
            Self::IdentityProvider => "identity-provider",
            Self::NetworkPolicy => "network-policy",
            Self::FirewallRule => "firewall-rule",
            Self::Infra => "infra",
            Self::Other => "other",
        }
    }

    /// Parse a resource kind.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "database" => Self::Database,
            "topic" => Self::Topic,
            "queue" => Self::Queue,
            "external-api" => Self::ExternalApi,
            "model-endpoint" => Self::ModelEndpoint,
            "identity-provider" => Self::IdentityProvider,
            "network-policy" => Self::NetworkPolicy,
            "firewall-rule" => Self::FirewallRule,
            "infra" => Self::Infra,
            "other" => Self::Other,
            _ => return None,
        })
    }
}

/// Whether `name` is lowercase kebab-case.
#[must_use]
pub fn is_kebab_case(name: &str) -> bool {
    let mut parts = name.split('-');
    match parts.next() {
        Some(first) if is_kebab_part(first) => parts.all(is_kebab_part),
        _ => false,
    }
}

fn is_kebab_part(part: &str) -> bool {
    !part.is_empty()
        && part
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

/// Build `component:<name>`.
///
/// # Errors
///
/// Returns an error when `name` is not lowercase kebab-case.
pub fn component_id(name: &str) -> Result<String, String> {
    if is_kebab_case(name) {
        Ok(format!("component:{name}"))
    } else {
        Err(format!(
            "component name `{name}` must be lowercase kebab-case"
        ))
    }
}

/// Build `deployment:<name>@<env>`.
///
/// # Errors
///
/// Returns an error when `name` is not lowercase kebab-case.
pub fn deployment_id(name: &str, env: Env) -> Result<String, String> {
    if is_kebab_case(name) {
        Ok(format!("deployment:{name}@{}", env.as_str()))
    } else {
        Err(format!(
            "deployment name `{name}` must be lowercase kebab-case"
        ))
    }
}

/// Build `resource:<kind>:<name>`.
///
/// # Errors
///
/// Returns an error when `name` is not lowercase kebab-case.
pub fn resource_id(kind: ResourceKind, name: &str) -> Result<String, String> {
    if is_kebab_case(name) {
        Ok(format!("resource:{}:{name}", kind.as_str()))
    } else {
        Err(format!(
            "resource name `{name}` must be lowercase kebab-case"
        ))
    }
}

/// Build `library:<purl>` for a versionless package URL.
///
/// # Errors
///
/// Returns an error when `purl` has a version, qualifier, or subpath.
pub fn library_id(purl: &str) -> Result<String, String> {
    if is_versionless_purl(purl) {
        Ok(format!("library:{purl}"))
    } else {
        Err(format!(
            "library purl `{purl}` must look like pkg:<type>/<name> without version"
        ))
    }
}

/// Parse a node id.
///
/// # Errors
///
/// Returns an error when `id` does not follow the convention.
pub fn parse_node_id(id: &str) -> Result<ParsedNodeId, String> {
    if let Some(name) = id.strip_prefix("component:") {
        if is_kebab_case(name) {
            return Ok(ParsedNodeId::Component {
                name: name.to_owned(),
            });
        }
        return Err(format!("invalid component id `{id}`"));
    }
    if let Some(rest) = id.strip_prefix("deployment:") {
        let Some((name, env)) = rest.rsplit_once('@') else {
            return Err(format!("invalid deployment id `{id}`"));
        };
        if !is_kebab_case(name) {
            return Err(format!("invalid deployment id `{id}`"));
        }
        let env = match env {
            "dev" => Env::Dev,
            "acc" => Env::Acc,
            "prod" => Env::Prod,
            _ => return Err(format!("invalid deployment id `{id}`")),
        };
        return Ok(ParsedNodeId::Deployment {
            name: name.to_owned(),
            env,
        });
    }
    if let Some(rest) = id.strip_prefix("resource:") {
        let Some((kind, name)) = rest.split_once(':') else {
            return Err(format!("invalid resource id `{id}`"));
        };
        let Some(kind) = ResourceKind::parse(kind) else {
            return Err(format!("invalid resource id `{id}`"));
        };
        if !is_kebab_case(name) {
            return Err(format!("invalid resource id `{id}`"));
        }
        return Ok(ParsedNodeId::Resource {
            kind,
            name: name.to_owned(),
        });
    }
    if let Some(purl) = id.strip_prefix("library:") {
        if is_versionless_purl(purl) {
            return Ok(ParsedNodeId::Library {
                purl: purl.to_owned(),
            });
        }
        return Err(format!("invalid library id `{id}`"));
    }
    Err(format!("invalid node id `{id}`"))
}

/// Stable edge id: `edge:` plus the lowercase hex SHA-256 of
/// `type\nfrom\nto\nenv`. `env` is empty when the edge has no environment.
#[must_use]
pub fn edge_id(edge_type: EdgeType, from_id: &str, to_id: &str, env: Option<Env>) -> String {
    let env_part = env.map(Env::as_str).unwrap_or("");
    let canonical = format!("{}\n{from_id}\n{to_id}\n{env_part}", edge_type.as_str());
    let digest = Sha256::digest(canonical.as_bytes());
    format!("edge:{}", hex::encode(digest))
}

fn is_versionless_purl(purl: &str) -> bool {
    let Some(rest) = purl.strip_prefix("pkg:") else {
        return false;
    };
    let Some((package_type, name)) = rest.split_once('/') else {
        return false;
    };
    if package_type.is_empty() || name.is_empty() {
        return false;
    }
    if purl.contains(['@', '?', '#', ' ']) {
        return false;
    }
    let type_ok = package_type
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'.' | b'-'));
    let name_ok = name.bytes().all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(byte, b'/' | b'.' | b'_' | b'~' | b'%' | b'+' | b'-')
    });
    type_ok && name_ok && !name.starts_with('/') && !name.ends_with('/') && !name.contains("//")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_canonical_ids() {
        assert_eq!(
            component_id("payments-api").unwrap(),
            "component:payments-api"
        );
        assert_eq!(
            deployment_id("payments-api", Env::Prod).unwrap(),
            "deployment:payments-api@prod"
        );
        assert_eq!(
            resource_id(ResourceKind::ExternalApi, "payments-gateway").unwrap(),
            "resource:external-api:payments-gateway"
        );
        assert_eq!(
            library_id("pkg:npm/%40scope/name").unwrap(),
            "library:pkg:npm/%40scope/name"
        );
        assert!(parse_node_id("library:pkg:golang/k8s.io/client-go").is_ok());
        assert!(parse_node_id("resource:identity-provider:keycloak").is_ok());
        assert!(parse_node_id("resource:model-endpoint:ranker").is_ok());
        assert!(parse_node_id("resource:other:legacy").is_ok());
        assert!(parse_node_id("resource:network-policy:payments-net").is_ok());
        assert!(parse_node_id("resource:firewall-rule:payments-egress").is_ok());
        assert!(parse_node_id("resource:infra:payments-api").is_ok());
    }

    #[test]
    fn rejects_bad_ids() {
        assert!(component_id("Payments").is_err());
        assert!(component_id("-a").is_err());
        assert!(component_id("a--b").is_err());
        assert!(parse_node_id("deployment:payments-api@qa").is_err());
        assert!(parse_node_id("deployment:payments-api").is_err());
        assert!(parse_node_id("resource:bucket:payments").is_err());
        assert!(parse_node_id("library:serde").is_err());
        assert!(parse_node_id("library:pkg:cargo/serde@1.0.0").is_err());
        assert!(parse_node_id("library:pkg:cargo/serde?arch=x86").is_err());
    }

    #[test]
    fn edge_id_is_stable_and_env_sensitive() {
        let left = edge_id(
            EdgeType::DeclaredDependsOn,
            "component:payments-api",
            "component:billing-api",
            None,
        );
        let again = edge_id(
            EdgeType::DeclaredDependsOn,
            "component:payments-api",
            "component:billing-api",
            None,
        );
        let with_env = edge_id(
            EdgeType::DeclaredDependsOn,
            "component:payments-api",
            "component:billing-api",
            Some(Env::Prod),
        );
        assert_eq!(left, again);
        assert_ne!(left, with_env);
        assert_eq!(
            left,
            "edge:6aaec4d166acdcc592284a22feecf0cab4312bafc2f78939ce9f6d6a323479b1"
        );
    }
}
