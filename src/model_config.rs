//! Per-user model settings and credentials.
use crate::error::Error;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    base_url: String,
    model: String,
    keyring: bool,
}

fn path() -> Result<PathBuf, Error> {
    #[cfg(target_os = "windows")]
    let root = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let root = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let root = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|path| PathBuf::from(path).is_absolute())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    root.map(|p| p.join("depgraph").join("model.json"))
        .ok_or_else(|| Error::Config("cannot find user configuration directory".into()))
}
fn read_settings() -> Result<Option<Settings>, Error> {
    let path = path()?;
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| Error::Config("invalid depgraph model settings".into())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn prompt(label: &str) -> Result<String, Error> {
    eprint!("{label}: ");
    io::stderr().flush()?;
    let mut value = String::new();
    io::stdin().read_line(&mut value)?;
    let value = value.trim().to_owned();
    if value.is_empty() {
        return Err(Error::Usage(format!("{label} cannot be empty")));
    }
    Ok(value)
}
fn account(url: &str) -> String {
    hex::encode(Sha256::digest(url.as_bytes()))
}
fn entry(url: &str) -> Result<keyring::Entry, Error> {
    if !cfg!(any(
        target_os = "macos",
        target_os = "windows",
        target_os = "linux"
    )) {
        return Err(Error::Store("system secret storage is unsupported on this platform; use OPENAI_API_KEY_FILE or OPENAI_API_KEY".into()));
    }
    keyring::Entry::new("depgraph-model", &account(url))
        .map_err(|_| Error::Store("could not open the system secret store".into()))
}
/// Validate and normalize the configured endpoint; never permit credentialed URLs.
///
/// # Errors
/// Returns an error for a URL that is unsafe or unusable.
pub fn endpoint(base: &str) -> Result<reqwest::Url, Error> {
    let mut url = reqwest::Url::parse(base)
        .map_err(|_| Error::Usage("model endpoint must be a valid URL".into()))?;
    if url.query().is_some()
        || url.fragment().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Error::Usage(
            "model endpoint must not contain credentials, a query, or a fragment".into(),
        ));
    }
    let local = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if url.scheme() != "https" && !(url.scheme() == "http" && local) {
        return Err(Error::Usage(
            "model endpoint must use HTTPS, except for localhost".into(),
        ));
    }
    let path = url.path().trim_end_matches('/').to_owned();
    url.set_path(&path);
    Ok(url)
}
fn ensure_bound(configured: &reqwest::Url, requested: &reqwest::Url) -> Result<(), Error> {
    if configured != requested {
        return Err(Error::Usage("endpoint differs from the saved key's endpoint; provide an explicit key for this endpoint".into()));
    }
    Ok(())
}

/// Configure an endpoint and model in the user profile and optionally store a key natively.
///
/// # Errors
/// Returns an error when input, settings or the system secret store are unavailable.
pub fn setup() -> Result<(), Error> {
    if !io::stdin().is_terminal() {
        return Err(Error::Usage(
            "model setup requires an interactive terminal".into(),
        ));
    }
    let base_url = endpoint(&prompt(
        "OpenAI-compatible base URL (for example https://example.com/v1)",
    )?)?
    .to_string()
    .trim_end_matches('/')
    .to_owned();
    let model = prompt("Model name")?;
    eprint!("API key (leave empty for an endpoint without authentication): ");
    io::stderr().flush()?;
    let secret = rpassword::read_password()?;
    let use_keyring = !secret.is_empty();
    if use_keyring {
        entry(&base_url)?.set_password(&secret)
            .map_err(|_| Error::Store("could not save the key in your system secret store; check it is available and unlocked".into()))?;
    }
    let file = path()?;
    std::fs::create_dir_all(file.parent().expect("settings has parent"))?;
    let settings = Settings {
        base_url,
        model,
        keyring: use_keyring,
    };
    let text = serde_json::to_vec_pretty(&settings)
        .map_err(|_| Error::Store("could not serialize model settings".into()))?;
    std::fs::write(file, text)?;
    println!(
        "Model settings saved{}.",
        if use_keyring {
            " and API key stored in the system secret store"
        } else {
            " without an API key"
        }
    );
    Ok(())
}

/// Resolve model settings and credentials, giving explicit process inputs precedence.
///
/// # Errors
/// Returns an error when required settings, a requested secret file, or the configured key are unavailable.
pub fn resolve(model: Option<&str>) -> Result<(reqwest::Url, String, Option<String>), Error> {
    let settings = read_settings()?;
    let env_base = std::env::var("OPENAI_BASE_URL").ok();
    let base = env_base
        .as_deref()
        .or_else(|| settings.as_ref().map(|s| s.base_url.as_str()))
        .ok_or_else(|| Error::Usage("run depgraph model-setup or set OPENAI_BASE_URL".into()))?;
    let url = endpoint(base)?;
    let model = model
        .map(str::to_owned)
        .or_else(|| std::env::var("OPENAI_MODEL").ok())
        .or_else(|| settings.as_ref().map(|s| s.model.clone()))
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            Error::Usage("set a model with --model-id, OPENAI_MODEL or depgraph model-setup".into())
        })?;
    let file = std::env::var_os("OPENAI_API_KEY_FILE");
    let env_key = std::env::var("OPENAI_API_KEY")
        .ok()
        .filter(|s| !s.is_empty());
    if file.is_some() && env_key.is_some() {
        return Err(Error::Usage(
            "set only one of OPENAI_API_KEY_FILE and OPENAI_API_KEY".into(),
        ));
    }
    let key = if let Some(path) = file {
        let value = std::fs::read_to_string(path)
            .map_err(|_| Error::Store("cannot read OPENAI_API_KEY_FILE".into()))?;
        let value = value.trim_end_matches(['\r', '\n']).to_owned();
        if value.is_empty() {
            return Err(Error::Store("OPENAI_API_KEY_FILE is empty".into()));
        }
        Some(value)
    } else if env_key.is_some() {
        env_key
    } else if let Some(settings) = settings.as_ref().filter(|s| s.keyring) {
        let configured = endpoint(&settings.base_url)?;
        ensure_bound(&configured, &url)?;
        Some(entry(&settings.base_url)?.get_password().map_err(|_| Error::Store("saved API key is unavailable; unlock your secret store or provide a key explicitly".into()))?)
    } else {
        None
    };
    Ok((url, model, key))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn saved_key_is_bound_to_exact_base_url() {
        let saved = endpoint("https://models.example.com/v1/").unwrap();
        assert!(ensure_bound(&saved, &endpoint("https://models.example.com/v1").unwrap()).is_ok());
        assert!(ensure_bound(&saved, &endpoint("https://other.example.com/v1").unwrap()).is_err());
        assert!(
            ensure_bound(
                &saved,
                &endpoint("https://models.example.com/other").unwrap()
            )
            .is_err()
        );
    }
    #[test]
    fn url_validation_disallows_credentials_and_insecure_remote() {
        assert!(endpoint("https://user:secret@example.com/v1").is_err());
        assert!(endpoint("http://example.com/v1").is_err());
        assert!(endpoint("http://127.0.0.1:8080/v1").is_ok());
    }
}
