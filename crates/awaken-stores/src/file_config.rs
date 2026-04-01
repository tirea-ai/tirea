//! File-system backed [`ConfigStore`] implementation.
//!
//! Layout:
//! ```text
//! <base_path>/config/
//!   <namespace>/
//!     <id>.json
//! ```
//!
//! Uses atomic write (tmp + rename) for crash safety, matching [`FileStore`](super::file::FileStore).

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use awaken_contract::contract::config_store::ConfigStore;
use awaken_contract::contract::storage::StorageError;
use serde_json::Value;
use tokio::io::AsyncWriteExt;

/// File-system backed configuration store.
pub struct FileConfigStore {
    base_path: PathBuf,
}

impl FileConfigStore {
    /// Create a new store rooted at `base_path/config/`.
    pub fn new(base_path: impl Into<PathBuf>) -> Self {
        Self {
            base_path: base_path.into().join("config"),
        }
    }

    fn namespace_dir(&self, namespace: &str) -> PathBuf {
        self.base_path.join(namespace)
    }
}

#[async_trait]
impl ConfigStore for FileConfigStore {
    async fn get(&self, namespace: &str, id: &str) -> Result<Option<Value>, StorageError> {
        validate_segment(namespace, "namespace")?;
        validate_segment(id, "id")?;

        let path = self.namespace_dir(namespace).join(format!("{id}.json"));
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => {
                let value = serde_json::from_str(&content)
                    .map_err(|e| StorageError::Serialization(e.to_string()))?;
                Ok(Some(value))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(StorageError::Io(e.to_string())),
        }
    }

    async fn list(
        &self,
        namespace: &str,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<(String, Value)>, StorageError> {
        validate_segment(namespace, "namespace")?;

        let dir = self.namespace_dir(namespace);
        if !dir.exists() {
            return Ok(Vec::new());
        }

        let mut entries = Vec::new();
        let mut read_dir = tokio::fs::read_dir(&dir)
            .await
            .map_err(|e| StorageError::Io(e.to_string()))?;

        while let Some(entry) = read_dir
            .next_entry()
            .await
            .map_err(|e| StorageError::Io(e.to_string()))?
        {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            // Skip hidden/tmp files
            if name.starts_with('.') || !name.ends_with(".json") {
                continue;
            }
            let id = name.trim_end_matches(".json").to_string();
            let content = tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| StorageError::Io(e.to_string()))?;
            let value = serde_json::from_str(&content)
                .map_err(|e| StorageError::Serialization(e.to_string()))?;
            entries.push((id, value));
        }

        entries.sort_by(|a, b| a.0.cmp(&b.0));
        Ok(entries.into_iter().skip(offset).take(limit).collect())
    }

    async fn put(&self, namespace: &str, id: &str, value: &Value) -> Result<(), StorageError> {
        validate_segment(namespace, "namespace")?;
        validate_segment(id, "id")?;

        let dir = self.namespace_dir(namespace);
        let content = serde_json::to_string_pretty(value)
            .map_err(|e| StorageError::Serialization(e.to_string()))?;
        atomic_write(&dir, &format!("{id}.json"), &content).await
    }

    async fn delete(&self, namespace: &str, id: &str) -> Result<(), StorageError> {
        validate_segment(namespace, "namespace")?;
        validate_segment(id, "id")?;

        let path = self.namespace_dir(namespace).join(format!("{id}.json"));
        match tokio::fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StorageError::Io(e.to_string())),
        }
    }
}

fn validate_segment(value: &str, label: &str) -> Result<(), StorageError> {
    if value.trim().is_empty() {
        return Err(StorageError::Io(format!("{label} cannot be empty")));
    }
    if value.contains('/')
        || value.contains('\\')
        || value.contains("..")
        || value.contains('\0')
        || value.chars().any(|c| c.is_control())
    {
        return Err(StorageError::Io(format!(
            "{label} contains invalid characters: {value:?}"
        )));
    }
    Ok(())
}

async fn atomic_write(dir: &Path, filename: &str, content: &str) -> Result<(), StorageError> {
    if !dir.exists() {
        tokio::fs::create_dir_all(dir)
            .await
            .map_err(|e| StorageError::Io(e.to_string()))?;
    }

    let target = dir.join(filename);
    let tmp_path = dir.join(format!(
        ".{}.{}.tmp",
        filename.trim_end_matches(".json"),
        uuid::Uuid::now_v7().simple()
    ));

    let write_result = async {
        let mut file = tokio::fs::File::create(&tmp_path)
            .await
            .map_err(|e| StorageError::Io(e.to_string()))?;
        file.write_all(content.as_bytes())
            .await
            .map_err(|e| StorageError::Io(e.to_string()))?;
        file.flush()
            .await
            .map_err(|e| StorageError::Io(e.to_string()))?;
        file.sync_all()
            .await
            .map_err(|e| StorageError::Io(e.to_string()))?;
        drop(file);
        tokio::fs::rename(&tmp_path, &target)
            .await
            .map_err(|e| StorageError::Io(e.to_string()))?;
        Ok::<(), StorageError>(())
    }
    .await;

    if let Err(e) = write_result {
        let _ = tokio::fs::remove_file(&tmp_path).await;
        return Err(e);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    fn make_store() -> (TempDir, FileConfigStore) {
        let td = TempDir::new().unwrap();
        let store = FileConfigStore::new(td.path());
        (td, store)
    }

    #[tokio::test]
    async fn put_and_get() {
        let (_td, store) = make_store();
        let val = json!({"id": "a1", "name": "alpha"});
        store.put("agents", "a1", &val).await.unwrap();

        let got = store.get("agents", "a1").await.unwrap().unwrap();
        assert_eq!(got, val);
    }

    #[tokio::test]
    async fn get_missing() {
        let (_td, store) = make_store();
        assert!(store.get("agents", "missing").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn put_overwrite() {
        let (_td, store) = make_store();
        store.put("agents", "a1", &json!({"v": 1})).await.unwrap();
        store.put("agents", "a1", &json!({"v": 2})).await.unwrap();
        let got = store.get("agents", "a1").await.unwrap().unwrap();
        assert_eq!(got["v"], 2);
    }

    #[tokio::test]
    async fn delete_existing() {
        let (_td, store) = make_store();
        store
            .put("agents", "a1", &json!({"id": "a1"}))
            .await
            .unwrap();
        store.delete("agents", "a1").await.unwrap();
        assert!(store.get("agents", "a1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn delete_nonexistent_ok() {
        let (_td, store) = make_store();
        store.delete("agents", "nope").await.unwrap();
    }

    #[tokio::test]
    async fn list_sorted_with_pagination() {
        let (_td, store) = make_store();
        for name in ["charlie", "alpha", "bravo"] {
            store
                .put("agents", name, &json!({"id": name}))
                .await
                .unwrap();
        }

        let all = store.list("agents", 0, 100).await.unwrap();
        let ids: Vec<&str> = all.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(ids, vec!["alpha", "bravo", "charlie"]);

        let page = store.list("agents", 1, 1).await.unwrap();
        assert_eq!(page.len(), 1);
        assert_eq!(page[0].0, "bravo");
    }

    #[tokio::test]
    async fn list_empty_namespace() {
        let (_td, store) = make_store();
        let list = store.list("empty", 0, 100).await.unwrap();
        assert!(list.is_empty());
    }

    #[tokio::test]
    async fn namespaces_isolated() {
        let (_td, store) = make_store();
        store
            .put("agents", "x", &json!({"ns": "agents"}))
            .await
            .unwrap();
        store
            .put("models", "x", &json!({"ns": "models"}))
            .await
            .unwrap();

        let a = store.get("agents", "x").await.unwrap().unwrap();
        let m = store.get("models", "x").await.unwrap().unwrap();
        assert_eq!(a["ns"], "agents");
        assert_eq!(m["ns"], "models");

        store.delete("agents", "x").await.unwrap();
        assert!(store.get("agents", "x").await.unwrap().is_none());
        assert!(store.get("models", "x").await.unwrap().is_some());
    }

    #[tokio::test]
    async fn rejects_invalid_namespace() {
        let (_td, store) = make_store();
        let err = store.put("../escape", "id", &json!({})).await.unwrap_err();
        assert!(err.to_string().contains("invalid characters"));
    }

    #[tokio::test]
    async fn rejects_invalid_id() {
        let (_td, store) = make_store();
        let err = store
            .put("ns", "../../etc/passwd", &json!({}))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid characters"));
    }

    #[tokio::test]
    async fn skips_hidden_files_in_list() {
        let (_td, store) = make_store();
        store.put("ns", "visible", &json!({})).await.unwrap();

        // Manually create a hidden file
        let dir = store.namespace_dir("ns");
        tokio::fs::write(dir.join(".hidden.json"), "{}")
            .await
            .unwrap();

        let list = store.list("ns", 0, 100).await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].0, "visible");
    }
}
