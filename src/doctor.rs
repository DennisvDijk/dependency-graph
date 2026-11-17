//! `depgraph doctor`: config, glab, and snapshot freshness.
//!
//! The report never includes a token. Connectivity to GitLab is a separate
//! `--online` step so the default check stays offline.

use std::path::Path;

use crate::config;
use crate::models::Timestamp;

/// What `glab auth status` told us. Stdout is discarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GlabStatus {
    /// `glab` is not on `PATH`.
    Missing,
    /// The command ran and exited non-zero.
    LoggedOut,
    /// The command exited 0.
    LoggedIn,
}

/// One doctor row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Check {
    /// Short name.
    pub name: &'static str,
    /// Whether the check passed.
    pub ok: bool,
    /// What is wrong, or a confirmation. No secrets.
    pub detail: String,
}

/// Offline checks. `now` is the reference time for snapshot age.
pub fn diagnose(root: &Path, now: &Timestamp, glab: GlabStatus, token_env_set: bool) -> Vec<Check> {
    let mut checks = Vec::new();
    match config::load_project(root) {
        Ok(project) => {
            checks.push(Check {
                name: "config",
                ok: true,
                detail: format!(
                    "parsed {} (gitlab {})",
                    root.join("config/config.yaml").display(),
                    project.config.gitlab.base_url
                ),
            });
            let fresh = snapshot_check(root, now);
            checks.push(fresh);
            let token_ok = token_env_set || glab == GlabStatus::LoggedIn;
            checks.push(Check {
                name: "token",
                ok: token_ok,
                detail: if token_ok {
                    "a GitLab token is available (value not shown)".to_owned()
                } else {
                    format!(
                        "no token. next: export GITLAB_TOKEN or run `glab auth login --hostname {}`",
                        host_of(&project.config.gitlab.base_url)
                    )
                },
            });
            checks.push(match glab {
                GlabStatus::LoggedIn => Check {
                    name: "glab",
                    ok: true,
                    detail: "glab auth status succeeded".to_owned(),
                },
                GlabStatus::LoggedOut => Check {
                    name: "glab",
                    ok: false,
                    detail: "glab is installed but not logged in. next: glab auth login".to_owned(),
                },
                GlabStatus::Missing => Check {
                    name: "glab",
                    ok: false,
                    detail: "glab is not on PATH. next: install glab from https://gitlab.com/gitlab-org/cli then `glab auth login`".to_owned(),
                },
            });
            checks.push(Check {
                name: "connectivity",
                ok: true,
                detail:
                    "skipped. next: depgraph doctor --online to GET /api/v4/version (read-only)"
                        .to_owned(),
            });
        }
        Err(err) => {
            checks.push(Check {
                name: "config",
                ok: false,
                detail: format!("{err}. next: depgraph init"),
            });
        }
    }
    checks
}

fn snapshot_check(root: &Path, now: &Timestamp) -> Check {
    let path = root.join("data/snapshot.json");
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(_) => {
            return Check {
                name: "snapshot",
                ok: false,
                detail: "data/snapshot.json is missing. next: depgraph collect --set full --output data && depgraph build --raw data/raw --output data/snapshot.json".to_owned(),
            };
        }
    };
    let value: serde_json::Value = match serde_json::from_str(&text) {
        Ok(value) => value,
        Err(_) => {
            return Check {
                name: "snapshot",
                ok: false,
                detail: "data/snapshot.json is not JSON. next: depgraph build".to_owned(),
            };
        }
    };
    let generated = value
        .get("generated_at")
        .and_then(|item| item.as_str())
        .unwrap_or("");
    let Ok(generated_at) = Timestamp::parse(generated) else {
        return Check {
            name: "snapshot",
            ok: false,
            detail: "snapshot generated_at is not a timestamp. next: depgraph build".to_owned(),
        };
    };
    let Ok(now_dt) = now.to_datetime() else {
        return Check {
            name: "snapshot",
            ok: false,
            detail: "doctor --now is not a timestamp".to_owned(),
        };
    };
    let Ok(generated_dt) = generated_at.to_datetime() else {
        return Check {
            name: "snapshot",
            ok: false,
            detail: "snapshot generated_at is not a timestamp".to_owned(),
        };
    };
    let age = (now_dt - generated_dt).as_hours() / 24;
    if age > 7 {
        Check {
            name: "snapshot",
            ok: false,
            detail: format!(
                "snapshot is {age} days old ({generated}). next: depgraph collect --set full && depgraph build"
            ),
        }
    } else {
        Check {
            name: "snapshot",
            ok: true,
            detail: format!("fresh ({generated})"),
        }
    }
}

fn host_of(url: &str) -> &str {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or(url)
}

/// Stable text for the CLI and tests.
#[must_use]
pub fn render(checks: &[Check]) -> String {
    let mut lines = Vec::new();
    for check in checks {
        let mark = if check.ok { "ok" } else { "fail" };
        lines.push(format!("{mark} {}: {}", check.name, check.detail));
    }
    lines.join("\n") + "\n"
}

/// Every check passed.
#[must_use]
pub fn all_ok(checks: &[Check]) -> bool {
    checks.iter().all(|check| check.ok)
}

/// Age of a snapshot in whole hours at `now`. Errors when the file is missing,
/// unreadable, or has no valid `generated_at`.
pub fn snapshot_age_hours(path: &Path, now: jiff::Timestamp) -> Result<i64, crate::error::Error> {
    use crate::error::Error;
    let text = std::fs::read_to_string(path).map_err(|e| {
        Error::Config(format!(
            "cannot read {}: {e}. next: depgraph build",
            path.display()
        ))
    })?;
    let value: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| Error::Validation(format!("{} is not JSON: {e}", path.display())))?;
    let stamp = value
        .get("generated_at")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Validation("snapshot has no generated_at".into()))?;
    let generated = stamp
        .parse::<jiff::Timestamp>()
        .map_err(|e| Error::Validation(format!("generated_at is not a timestamp: {e}")))?;
    Ok(((now - generated).as_hours()).max(0))
}

#[cfg(test)]
mod freshness_tests {
    use super::*;

    #[test]
    fn age_is_counted_in_hours_and_bad_files_fail() {
        let dir = std::env::temp_dir().join(format!("depgraph-fresh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("snapshot.json");
        std::fs::write(&file, r#"{"generated_at":"2026-11-17T00:00:00Z"}"#).unwrap();
        let now: jiff::Timestamp = "2026-11-18T06:30:00Z".parse().unwrap();
        assert_eq!(snapshot_age_hours(&file, now).unwrap(), 30);
        std::fs::write(&file, "{}").unwrap();
        assert!(snapshot_age_hours(&file, now).is_err());
        assert!(snapshot_age_hours(&dir.join("missing.json"), now).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
