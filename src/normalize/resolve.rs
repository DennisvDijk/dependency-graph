//! Resolve source environment names through the config index.
//!
//! This is a pure lookup. It does not read the clock or the network.

use crate::config::EnvIndex;
use crate::error::Error;
use crate::models::Env;

/// Map a GitLab or telemetry environment name onto `dev`, `acc`, or `prod`.
///
/// # Errors
///
/// Returns an error when `name` is not in the index.
pub fn resolve_source_env(index: &EnvIndex, name: &str) -> Result<Env, Error> {
    index.resolve(name).ok_or_else(|| {
        Error::Config(format!(
            "source env `{name}` is not mapped to a canonical environment"
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::parse_environments;
    use crate::models::Env;

    #[test]
    fn resolves_only_configured_names() {
        let environments =
            parse_environments(include_str!("../../config/environments.yaml")).unwrap();
        let index = environments.index().unwrap();
        assert_eq!(resolve_source_env(&index, "staging").unwrap(), Env::Acc);
        assert_eq!(resolve_source_env(&index, "production").unwrap(), Env::Prod);
        assert!(resolve_source_env(&index, "qa").is_err());
    }
}
