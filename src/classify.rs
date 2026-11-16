//! Hard/soft classification.
//!
//! Reliability order: catalog declaration, then an ingested experiment, then a
//! heuristic. The tool never runs a chaos experiment.

use crate::models::{Criticality, CriticalitySource};

/// Evidence for one dependency. Higher sources replace lower ones.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Evidence {
    /// Named in `depgraph/soft-dependencies`.
    pub declared_soft: bool,
    /// `Some(true)` means the client kept working. `None` means no experiment.
    pub experiment_survived: Option<bool>,
    /// Fallback, circuit breaker, or async was noted.
    pub heuristic: bool,
}

/// Classify one edge.
#[must_use]
pub fn classify(evidence: Evidence) -> (Criticality, CriticalitySource) {
    if evidence.declared_soft {
        return (Criticality::Soft, CriticalitySource::Declared);
    }
    if let Some(survived) = evidence.experiment_survived {
        return if survived {
            (Criticality::Soft, CriticalitySource::Experiment)
        } else {
            (Criticality::Hard, CriticalitySource::Experiment)
        };
    }
    if evidence.heuristic {
        return (Criticality::Soft, CriticalitySource::Heuristic);
    }
    (Criticality::Unknown, CriticalitySource::Unknown)
}

/// Whether a heuristic token counts as evidence of softness.
#[must_use]
pub fn is_soft_heuristic(token: &str) -> bool {
    matches!(
        token.trim().to_ascii_lowercase().as_str(),
        "fallback" | "circuit-breaker" | "circuit_breaker" | "async"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declaration_beats_experiment_and_heuristic() {
        let (criticality, source) = classify(Evidence {
            declared_soft: true,
            experiment_survived: Some(false),
            heuristic: true,
        });
        assert_eq!(criticality, Criticality::Soft);
        assert_eq!(source, CriticalitySource::Declared);
    }

    #[test]
    fn a_failed_experiment_is_hard_and_beats_a_heuristic() {
        let (criticality, source) = classify(Evidence {
            declared_soft: false,
            experiment_survived: Some(false),
            heuristic: true,
        });
        assert_eq!(criticality, Criticality::Hard);
        assert_eq!(source, CriticalitySource::Experiment);
    }

    #[test]
    fn a_heuristic_alone_is_evidence_of_soft() {
        let (criticality, source) = classify(Evidence {
            declared_soft: false,
            experiment_survived: None,
            heuristic: true,
        });
        assert_eq!(criticality, Criticality::Soft);
        assert_eq!(source, CriticalitySource::Heuristic);
        assert!(is_soft_heuristic("circuit-breaker"));
        assert!(!is_soft_heuristic("cache"));
    }
}
