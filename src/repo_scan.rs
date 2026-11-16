//! Pure repository classification. No network and no clock.
//!
//! Inventory and catalog drafts share these rules so a fixture and a live
//! GitLab tree produce the same flags.

/// What a repository contains that depgraph cares about.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RepoFlags {
    /// `catalog-info.yaml` (or the configured catalog file) is present.
    pub has_catalog_info: bool,
    /// `.gitlab-ci.yml` declares `environment:`.
    pub uses_environment: bool,
    /// A job artifact looks like CycloneDX.
    pub has_sbom_artifact: bool,
    /// Kubernetes manifests live in the repo.
    pub has_k8s_manifests: bool,
    /// A Helm chart is present.
    pub has_helm: bool,
    /// A kustomize base is present.
    pub has_kustomize: bool,
    /// Terraform (or other `.tf`) is present.
    pub has_terraform: bool,
    /// Istio config is present.
    pub has_istio_config: bool,
}

/// A CI job reduced to the names we match on.
#[derive(Debug, Clone)]
pub struct JobHint {
    /// Job name.
    pub name: String,
    /// Artifact file names.
    pub filenames: Vec<String>,
}

/// Classify blob paths, optional CI text, and job hints.
#[must_use]
pub fn classify(
    paths: &[impl AsRef<str>],
    catalog_file: &str,
    ci_text: Option<&str>,
    jobs: &[JobHint],
) -> RepoFlags {
    let mut flags = RepoFlags::default();
    for path in paths {
        let path = path.as_ref();
        if is_catalog_path(path, catalog_file) {
            flags.has_catalog_info = true;
        }
        if is_helm_path(path) {
            flags.has_helm = true;
        }
        if is_kustomize_path(path) {
            flags.has_kustomize = true;
        }
        if path.ends_with(".tf") {
            flags.has_terraform = true;
        }
        if is_k8s_manifest_path(path) {
            flags.has_k8s_manifests = true;
        }
        if is_istio_path(path) {
            flags.has_istio_config = true;
        }
    }
    if ci_text.is_some_and(declares_environment) {
        flags.uses_environment = true;
    }
    if jobs.iter().any(job_has_sbom) {
        flags.has_sbom_artifact = true;
    }
    flags
}

/// `app.kubernetes.io/name` values found in YAML text.
#[must_use]
pub fn kubernetes_app_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(value) = trimmed
            .strip_prefix("app.kubernetes.io/name:")
            .or_else(|| trimmed.strip_prefix("\"app.kubernetes.io/name\":"))
        else {
            continue;
        };
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if !value.is_empty() && !names.iter().any(|existing: &String| existing == value) {
            names.push(value.to_owned());
        }
    }
    names
}

/// Helm chart name from a `Chart.yaml` document.
#[must_use]
pub fn helm_chart_name(text: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("name:") {
            let value = value.trim().trim_matches('"').trim_matches('\'');
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

/// GitLab CI environment names (`environment:` or `environment.name`).
#[must_use]
pub fn ci_environment_names(text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            continue;
        }
        if let Some(value) = trimmed.strip_prefix("environment:") {
            let value = value.trim();
            if !value.is_empty() && !value.starts_with('|') && !value.starts_with('>') {
                push_unique(&mut names, value.trim_matches('"').trim_matches('\''));
                continue;
            }
            if let Some(next) = lines.get(index + 1) {
                let next = next.trim();
                if let Some(name) = next.strip_prefix("name:") {
                    let name = name.trim().trim_matches('"').trim_matches('\'');
                    if !name.is_empty() {
                        push_unique(&mut names, name);
                    }
                }
            }
        }
    }
    names
}

fn push_unique(names: &mut Vec<String>, value: &str) {
    if !names.iter().any(|existing| existing == value) {
        names.push(value.to_owned());
    }
}

fn is_catalog_path(path: &str, catalog_file: &str) -> bool {
    path == catalog_file || path.ends_with(&format!("/{catalog_file}"))
}

fn is_helm_path(path: &str) -> bool {
    path == "Chart.yaml" || path.ends_with("/Chart.yaml")
}

fn is_kustomize_path(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    file == "kustomization.yaml" || file == "kustomization.yml"
}

fn is_k8s_manifest_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    if !(lower.ends_with(".yaml") || lower.ends_with(".yml")) {
        return false;
    }
    let parts: Vec<&str> = path.split('/').collect();
    parts
        .iter()
        .any(|part| matches!(*part, "deploy" | "k8s" | "manifests" | "kubernetes"))
}

fn is_istio_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    path.split('/')
        .any(|part| part.eq_ignore_ascii_case("istio"))
        || lower.contains("virtualservice")
        || lower.contains("virtual-service")
        || lower.contains("destinationrule")
        || lower.contains("destination-rule")
}

fn declares_environment(ci_text: &str) -> bool {
    !ci_environment_names(ci_text).is_empty()
}

fn job_has_sbom(job: &JobHint) -> bool {
    let name = job.name.to_ascii_lowercase();
    if name.contains("sbom") || name.contains("cyclonedx") || name.contains("dependency_scanning") {
        return true;
    }
    job.filenames.iter().any(|filename| {
        let lower = filename.to_ascii_lowercase();
        lower.ends_with(".cdx.json") || lower.contains("cyclonedx") || lower.contains("sbom")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_a_repo_that_holds_the_context() {
        let paths = [
            "catalog-info.yaml",
            ".gitlab-ci.yml",
            "deploy/deployment.yaml",
            "chart/Chart.yaml",
            "gitops/kustomization.yaml",
            "terraform/main.tf",
            "istio/virtual-service.yaml",
        ];
        let jobs = [JobHint {
            name: "cyclonedx".to_owned(),
            filenames: vec!["sbom.cdx.json".to_owned()],
        }];
        let flags = classify(
            &paths,
            "catalog-info.yaml",
            Some("deploy:\n  environment:\n    name: production\n"),
            &jobs,
        );
        assert!(flags.has_catalog_info);
        assert!(flags.uses_environment);
        assert!(flags.has_sbom_artifact);
        assert!(flags.has_k8s_manifests);
        assert!(flags.has_helm);
        assert!(flags.has_kustomize);
        assert!(flags.has_terraform);
        assert!(flags.has_istio_config);
    }

    #[test]
    fn a_plain_readme_repo_has_no_flags() {
        let flags = classify(
            &["README.md"],
            "catalog-info.yaml",
            Some("test:\n  script: [echo]\n"),
            &[],
        );
        assert_eq!(flags, RepoFlags::default());
    }

    #[test]
    fn extracts_labels_chart_names_and_environments() {
        assert_eq!(
            kubernetes_app_names(
                "metadata:\n  labels:\n    app.kubernetes.io/name: payments-api\n"
            ),
            vec!["payments-api".to_owned()]
        );
        assert_eq!(
            helm_chart_name("apiVersion: v2\nname: payments-api\n"),
            Some("payments-api".to_owned())
        );
        assert_eq!(
            ci_environment_names("deploy:\n  environment: production\n"),
            vec!["production".to_owned()]
        );
    }
}
