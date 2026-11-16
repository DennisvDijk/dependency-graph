//! Catalog-info validation. The schema is embedded so the binary does not
//! depend on a checkout of `schemas/`.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::error::Error;
use crate::redact;

/// Schema completed in F0-5.
pub const CATALOG_SCHEMA: &str = include_str!("../schemas/catalog-info.schema.json");

/// One catalog entity after schema validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogEntity {
    /// Source path, for error messages.
    pub path: String,
    /// `Component` or `Resource`.
    pub kind: String,
    /// `metadata.name`.
    pub name: String,
    /// `spec.dependsOn` entity refs.
    pub depends_on: Vec<String>,
}

/// Validate every catalog file. `dependsOn` refs must resolve inside the set.
///
/// # Errors
///
/// Returns an error when a file fails the schema or a reference is missing.
pub fn validate_catalog_files(paths: &[impl AsRef<Path>]) -> Result<Vec<CatalogEntity>, Error> {
    if paths.is_empty() {
        return Err(Error::Validation(
            "validate-catalog needs at least one catalog file".to_owned(),
        ));
    }
    let schema: Value = serde_json::from_str(CATALOG_SCHEMA)
        .map_err(|err| Error::Validation(format!("catalog schema is not JSON: {err}")))?;
    let validator = jsonschema::validator_for(&schema)
        .map_err(|err| Error::Validation(format!("catalog schema failed to compile: {err}")))?;
    let mut entities = Vec::new();
    for path in paths {
        let path = path.as_ref();
        let text = fs::read_to_string(path)
            .map_err(|err| Error::Validation(format!("reading {}: {err}", path.display())))?;
        for doc in yaml_documents(&text) {
            let value: Value = yaml_serde::from_str(doc).map_err(|err| {
                Error::Validation(redact::redact(&format!(
                    "{} is not YAML: {err}",
                    path.display()
                )))
            })?;
            if let Err(error) = validator.validate(&value) {
                return Err(Error::Validation(format!(
                    "{} failed catalog schema: {error}",
                    path.display()
                )));
            }
            entities.push(entity_from_value(path.display().to_string(), &value)?);
        }
    }
    let known: BTreeSet<(String, String)> = entities
        .iter()
        .map(|entity| (entity.kind.to_ascii_lowercase(), entity.name.clone()))
        .collect();
    let mut errors = Vec::new();
    for entity in &entities {
        for reference in &entity.depends_on {
            match parse_entity_ref(reference) {
                Ok((kind, name)) => {
                    if !known.contains(&(kind.clone(), name.clone())) {
                        errors.push(format!(
                            "{} dependsOn {reference} does not resolve",
                            entity.path
                        ));
                    }
                }
                Err(detail) => errors.push(format!("{} {detail}", entity.path)),
            }
        }
    }
    if errors.is_empty() {
        Ok(entities)
    } else {
        Err(Error::Validation(errors.join("\n")))
    }
}

/// Split a YAML stream on `---` separator lines. Backstage allows several
/// entities in one `catalog-info.yaml`. Chunks with only comments or blank
/// lines are dropped.
#[must_use]
pub fn yaml_documents(text: &str) -> Vec<&str> {
    let mut docs = Vec::new();
    let mut start = 0;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if line.trim_end() == "---" {
            docs.push(&text[start..offset]);
            start = offset + line.len();
        }
        offset += line.len();
    }
    docs.push(&text[start..]);
    docs.into_iter()
        .filter(|doc| {
            doc.lines().any(|line| {
                let line = line.trim();
                !line.is_empty() && !line.starts_with('#')
            })
        })
        .collect()
}

fn entity_from_value(path: String, value: &Value) -> Result<CatalogEntity, Error> {
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    let name = value
        .pointer("/metadata/name")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_owned();
    if name.is_empty() {
        return Err(Error::Validation(format!(
            "{path} is missing metadata.name"
        )));
    }
    let depends_on = value
        .pointer("/spec/dependsOn")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default();
    Ok(CatalogEntity {
        path,
        kind,
        name,
        depends_on,
    })
}

fn parse_entity_ref(reference: &str) -> Result<(String, String), String> {
    let Some((kind, rest)) = reference.split_once(':') else {
        return Err(format!("dependsOn `{reference}` is not an entity ref"));
    };
    let Some((_, name)) = rest.split_once('/') else {
        return Err(format!(
            "dependsOn `{reference}` is not kind:namespace/name"
        ));
    };
    if name.is_empty() || !crate::ids::is_kebab_case(name) {
        return Err(format!("dependsOn `{reference}` has a non-canonical name"));
    }
    Ok((kind.to_ascii_lowercase(), name.to_owned()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn yaml_documents_splits_on_separator_lines_only() {
        let text = "---\nkind: Component\nmetadata:\n  name: a\n  description: \"a---b\"\n---\n# comment only\n---\nkind: Resource\n";
        let docs = super::yaml_documents(text);
        assert_eq!(docs.len(), 2);
        assert!(docs[0].contains("a---b"));
        assert!(docs[1].starts_with("kind: Resource"));
    }

    use super::*;

    #[test]
    fn embedded_schema_matches_the_file() {
        let on_disk = include_str!("../schemas/catalog-info.schema.json");
        assert_eq!(CATALOG_SCHEMA, on_disk);
    }
}
