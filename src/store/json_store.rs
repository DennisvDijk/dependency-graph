//! JSON files on disk: `snapshot.json`, `meta.json`, and `raw/<collector>.jsonl`.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::models::{GraphSnapshot, to_canonical_json};
use crate::normalize::validate;
use crate::raw::RawRecord;
use crate::redact;
use crate::store::base::{GraphStore, Meta, validate_collector_name};

/// A directory of JSON artifacts.
#[derive(Debug, Clone)]
pub struct JsonStore {
    root: PathBuf,
}

impl JsonStore {
    /// Store files under `root`. The directory is created on write.
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn snapshot_path(&self) -> PathBuf {
        self.root.join("snapshot.json")
    }

    fn meta_path(&self) -> PathBuf {
        self.root.join("meta.json")
    }

    fn raw_path(&self, collector: &str) -> PathBuf {
        self.root.join("raw").join(format!("{collector}.jsonl"))
    }
}

impl GraphStore for JsonStore {
    fn load_snapshot(&self) -> Result<GraphSnapshot, Error> {
        let path = self.snapshot_path();
        let text = fs::read_to_string(&path).map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                Error::Store(format!("snapshot not found at {}", path.display()))
            } else {
                Error::Store(err.to_string())
            }
        })?;
        let mut graph: GraphSnapshot = serde_json::from_str(&text)
            .map_err(|err| Error::Store(redact::redact(&err.to_string())))?;
        validate(&graph, None).into_result()?;
        graph.canonicalize();
        Ok(graph)
    }

    fn save_snapshot(&self, graph: &GraphSnapshot) -> Result<(), Error> {
        validate(graph, None).into_result()?;
        let text = to_canonical_json(graph).map_err(|err| Error::Store(err.to_string()))?;
        atomic_write(&self.snapshot_path(), text.as_bytes())
    }

    fn load_meta(&self) -> Result<Meta, Error> {
        let path = self.meta_path();
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text)
                .map_err(|err| Error::Store(redact::redact(&err.to_string()))),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Meta::default()),
            Err(err) => Err(Error::Store(err.to_string())),
        }
    }

    fn save_meta(&self, meta: &Meta) -> Result<(), Error> {
        let mut text =
            serde_json::to_string_pretty(meta).map_err(|err| Error::Store(err.to_string()))?;
        text.push('\n');
        atomic_write(&self.meta_path(), text.as_bytes())
    }

    fn write_raw(&self, collector: &str, records: &[RawRecord]) -> Result<(), Error> {
        validate_collector_name(collector)?;
        let mut lines = Vec::with_capacity(records.len());
        for record in records {
            lines.push(
                serde_json::to_string(record)
                    .map_err(|err| Error::Store(redact::redact(&err.to_string())))?,
            );
        }
        lines.sort();
        let mut body = lines.join("\n");
        if !body.is_empty() {
            body.push('\n');
        }
        atomic_write(&self.raw_path(collector), body.as_bytes())
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let parent = path
        .parent()
        .ok_or_else(|| Error::Store(format!("path {} has no parent", path.display())))?;
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .ok_or_else(|| Error::Store(format!("path {} has no file name", path.display())))?;
    let temporary = parent.join(format!(".{}.tmp", file_name.to_string_lossy()));
    fs::write(&temporary, bytes)?;
    fs::rename(&temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{RunStatus, Timestamp};
    use crate::raw::RawRecord;
    use serde_json::json;
    use std::collections::BTreeMap;

    fn empty_graph() -> GraphSnapshot {
        GraphSnapshot {
            schema_version: "0.1.0".to_owned(),
            run_id: "empty".to_owned(),
            generated_at: Timestamp::parse("2026-01-01T00:00:00Z").unwrap(),
            sources: BTreeMap::new(),
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }

    #[test]
    fn failure_keeps_the_previous_snapshot_and_redacts_the_message() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::new(dir.path());
        store.save_snapshot(&empty_graph()).unwrap();
        let snapshot_bytes = fs::read(store.snapshot_path()).unwrap();
        let finished = Timestamp::parse("2026-01-02T00:00:00Z").unwrap();
        let first = RawRecord {
            collector: "gitlab_inventory".to_owned(),
            kind: "project".to_owned(),
            payload: json!({"path": "example-group/pilot"}),
        };
        store
            .record_collector("gitlab_inventory", &Ok(vec![first]), &finished)
            .unwrap();
        let raw_bytes = fs::read(store.raw_path("gitlab_inventory")).unwrap();

        let failure = Err(Error::Config(
            "GET https://user:password@gitlab.example.com/api/v4/projects failed".to_owned(),
        ));
        store
            .record_collector("gitlab_inventory", &failure, &finished)
            .unwrap();

        assert_eq!(fs::read(store.snapshot_path()).unwrap(), snapshot_bytes);
        assert_eq!(
            fs::read(store.raw_path("gitlab_inventory")).unwrap(),
            raw_bytes
        );
        let meta = store.load_meta().unwrap();
        let entry = &meta.collectors["gitlab_inventory"];
        assert_eq!(entry.status, RunStatus::Stale);
        assert_eq!(entry.count, 1);
        let message = entry.message.as_deref().unwrap();
        assert!(!message.contains("password"));
        assert!(message.contains("***"));
    }

    #[test]
    fn secret_payloads_are_not_written() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::new(dir.path());
        let finished = Timestamp::parse("2026-01-02T00:00:00Z").unwrap();
        let leaked = RawRecord {
            collector: "gitlab_inventory".to_owned(),
            kind: "project".to_owned(),
            payload: json!({"url": "https://user:password@gitlab.example.com"}),
        };
        store
            .record_collector("gitlab_inventory", &Ok(vec![leaked]), &finished)
            .unwrap();
        assert!(!store.raw_path("gitlab_inventory").exists());
        let meta = store.load_meta().unwrap();
        assert_eq!(meta.collectors["gitlab_inventory"].status, RunStatus::Stale);
        assert!(
            !meta.collectors["gitlab_inventory"]
                .message
                .as_deref()
                .unwrap()
                .contains("password")
        );
    }

    #[test]
    fn roundtrip_is_canonical() {
        let dir = tempfile::tempdir().unwrap();
        let store = JsonStore::new(dir.path());
        store.save_snapshot(&empty_graph()).unwrap();
        let loaded = store.load_snapshot().unwrap();
        assert_eq!(loaded, empty_graph());
        assert!(
            store
                .save_snapshot(&{
                    let mut graph = empty_graph();
                    graph.schema_version = "nope".to_owned();
                    graph
                })
                .is_err()
        );
    }
}
