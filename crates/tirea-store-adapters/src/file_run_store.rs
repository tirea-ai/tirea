use crate::file_utils;
use async_trait::async_trait;
use std::path::PathBuf;
use tirea_contract::storage::{
    paginate_runs_in_memory, RunPage, RunQuery, RunReader, RunRecord, RunStoreError, RunWriter,
};

/// File-based run projection store.
///
/// Each run is stored as one JSON file `<run_id>.json` under `base_path`.
pub struct FileRunStore {
    base_path: PathBuf,
}

impl FileRunStore {
    pub fn new(base_path: impl Into<PathBuf>) -> Self {
        Self {
            base_path: base_path.into(),
        }
    }

    fn run_path(&self, run_id: &str) -> Result<PathBuf, RunStoreError> {
        Self::validate_run_id(run_id)?;
        Ok(self.base_path.join(format!("{run_id}.json")))
    }

    fn validate_run_id(run_id: &str) -> Result<(), RunStoreError> {
        file_utils::validate_fs_id(run_id, "run id").map_err(RunStoreError::InvalidId)
    }

    async fn save_run(&self, record: &RunRecord) -> Result<(), RunStoreError> {
        let payload = serde_json::to_string_pretty(record)
            .map_err(|e| RunStoreError::Serialization(e.to_string()))?;
        let filename = format!("{}.json", record.run_id);
        file_utils::atomic_json_write(&self.base_path, &filename, &payload)
            .await
            .map_err(RunStoreError::Io)
    }

    async fn load_all_runs(&self) -> Result<Vec<RunRecord>, RunStoreError> {
        if !self.base_path.exists() {
            return Ok(Vec::new());
        }
        let mut entries = tokio::fs::read_dir(&self.base_path).await?;
        let mut records = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().is_none_or(|ext| ext != "json") {
                continue;
            }
            let content = tokio::fs::read_to_string(path).await?;
            let record: RunRecord = serde_json::from_str(&content)
                .map_err(|e| RunStoreError::Serialization(e.to_string()))?;
            records.push(record);
        }
        Ok(records)
    }
}

#[async_trait]
impl RunReader for FileRunStore {
    async fn load_run(&self, run_id: &str) -> Result<Option<RunRecord>, RunStoreError> {
        let path = self.run_path(run_id)?;
        if !path.exists() {
            return Ok(None);
        }
        let content = tokio::fs::read_to_string(path).await?;
        let record: RunRecord = serde_json::from_str(&content)
            .map_err(|e| RunStoreError::Serialization(e.to_string()))?;
        Ok(Some(record))
    }

    async fn list_runs(&self, query: &RunQuery) -> Result<RunPage, RunStoreError> {
        let records = self.load_all_runs().await?;
        Ok(paginate_runs_in_memory(&records, query))
    }
}

#[async_trait]
impl RunWriter for FileRunStore {
    async fn upsert_run(&self, record: &RunRecord) -> Result<(), RunStoreError> {
        self.save_run(record).await
    }

    async fn delete_run(&self, run_id: &str) -> Result<(), RunStoreError> {
        let path = self.run_path(run_id)?;
        if path.exists() {
            tokio::fs::remove_file(path).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;
    use tirea_contract::storage::{RunOrigin, RunRecordStatus};

    #[tokio::test]
    async fn run_store_roundtrip() {
        let temp = TempDir::new().expect("tempdir");
        let store = FileRunStore::new(temp.path());
        let mut record = RunRecord::new(
            "run-roundtrip",
            "thread-1",
            RunOrigin::A2a,
            RunRecordStatus::Submitted,
            100,
        );
        record.updated_at = 120;

        store.upsert_run(&record).await.expect("upsert");
        let loaded = store
            .load_run("run-roundtrip")
            .await
            .expect("load")
            .expect("exists");
        assert_eq!(loaded.thread_id, "thread-1");
        assert_eq!(loaded.updated_at, 120);

        let page = store.list_runs(&RunQuery::default()).await.expect("list");
        assert_eq!(page.total, 1);

        store.delete_run("run-roundtrip").await.expect("delete");
        assert!(store
            .load_run("run-roundtrip")
            .await
            .expect("load after delete")
            .is_none());
    }
}
