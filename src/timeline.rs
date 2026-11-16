//! Deployments and incidents on one timeline.
//!
//! Datadog stays optional and read-only. Tests pass fixtures. This module does
//! not call Datadog or GitLab.

use crate::models::Env;

/// One deployment or incident.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimelineEvent {
    /// RFC 3339 time.
    pub at: String,
    /// `deployment` or `incident`.
    pub kind: String,
    /// Canonical environment.
    pub env: String,
    /// Short description. No secrets.
    pub summary: String,
}

/// A GitLab deployment row from a fixture.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct DeploymentFixture {
    /// When it was created.
    pub created_at: String,
    /// Source environment name, such as `production`.
    pub environment: String,
    /// Component name.
    pub component: String,
    /// Version or ref.
    pub version: String,
}

/// An incident row. A Datadog export can be reduced to this shape offline.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct IncidentFixture {
    /// When it started.
    pub started_at: String,
    /// Canonical env, or a source name already mapped by the caller.
    pub env: String,
    /// Title. No credentials.
    pub title: String,
}

/// Merge deployments and incidents for `env` at or after `since`.
#[must_use]
pub fn correlate(
    deployments: &[DeploymentFixture],
    incidents: &[IncidentFixture],
    env: Env,
    since: &str,
    env_name: impl Fn(&str) -> Option<Env>,
) -> Vec<TimelineEvent> {
    let mut events = Vec::new();
    for deployment in deployments {
        let Some(mapped) = env_name(&deployment.environment) else {
            continue;
        };
        if mapped != env || deployment.created_at.as_str() < since {
            continue;
        }
        events.push(TimelineEvent {
            at: deployment.created_at.clone(),
            kind: "deployment".into(),
            env: env.as_str().into(),
            summary: format!("{} {}", deployment.component, deployment.version),
        });
    }
    for incident in incidents {
        if incident.env != env.as_str() || incident.started_at.as_str() < since {
            continue;
        }
        events.push(TimelineEvent {
            at: incident.started_at.clone(),
            kind: "incident".into(),
            env: env.as_str().into(),
            summary: incident.title.clone(),
        });
    }
    events.sort_by(|left, right| left.at.cmp(&right.at).then(left.kind.cmp(&right.kind)));
    events
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeline_keeps_prod_events_after_since() {
        let deployments = vec![DeploymentFixture {
            created_at: "2026-02-01T00:00:00Z".into(),
            environment: "production".into(),
            component: "payments-api".into(),
            version: "1.4.0".into(),
        }];
        let incidents = vec![IncidentFixture {
            started_at: "2026-02-01T01:00:00Z".into(),
            env: "prod".into(),
            title: "elevated errors".into(),
        }];
        let events = correlate(
            &deployments,
            &incidents,
            Env::Prod,
            "2026-01-15T00:00:00Z",
            |name| (name == "production").then_some(Env::Prod),
        );
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].kind, "deployment");
        assert_eq!(events[1].kind, "incident");
    }
}
