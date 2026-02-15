use super::*;

pub struct FileStore {
    base_path: PathBuf,
}

impl FileStore {
    /// Create a new file storage with the given base path.
    pub fn new(base_path: impl Into<PathBuf>) -> Self {
        Self {
            base_path: base_path.into(),
        }
    }

    pub(super) fn thread_path(&self, thread_id: &str) -> Result<PathBuf, ThreadStoreError> {
        Self::validate_thread_id(thread_id)?;
        Ok(self.base_path.join(format!("{}.json", thread_id)))
    }

    /// Validate that a session ID is safe for use as a filename.
    /// Rejects path separators, `..`, and control characters.
    fn validate_thread_id(thread_id: &str) -> Result<(), ThreadStoreError> {
        if thread_id.is_empty() {
            return Err(ThreadStoreError::InvalidId(
                "thread id cannot be empty".to_string(),
            ));
        }
        if thread_id.contains('/')
            || thread_id.contains('\\')
            || thread_id.contains("..")
            || thread_id.contains('\0')
        {
            return Err(ThreadStoreError::InvalidId(format!(
                "thread id contains invalid characters: {thread_id:?}"
            )));
        }
        if thread_id.chars().any(|c| c.is_control()) {
            return Err(ThreadStoreError::InvalidId(format!(
                "thread id contains control characters: {thread_id:?}"
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl ThreadWriter for FileStore {
    async fn create(&self, thread: &Thread) -> Result<Committed, ThreadStoreError> {
        let path = self.thread_path(&thread.id)?;
        if path.exists() {
            return Err(ThreadStoreError::AlreadyExists);
        }
        // Serialize with version=0 embedded
        let head = ThreadHead {
            thread: thread.clone(),
            version: 0,
        };
        self.save_head(&head).await?;
        Ok(Committed { version: 0 })
    }

    async fn append(
        &self,
        thread_id: &str,
        delta: &ThreadDelta,
    ) -> Result<Committed, ThreadStoreError> {
        let head = self
            .load_head(thread_id)
            .await?
            .ok_or_else(|| ThreadStoreError::NotFound(thread_id.to_string()))?;

        let mut thread = head.thread;
        delta.apply_to(&mut thread);
        let new_version = head.version + 1;
        let new_head = ThreadHead {
            thread,
            version: new_version,
        };
        self.save_head(&new_head).await?;
        Ok(Committed {
            version: new_version,
        })
    }

    async fn delete(&self, thread_id: &str) -> Result<(), ThreadStoreError> {
        let path = self.thread_path(thread_id)?;
        if path.exists() {
            tokio::fs::remove_file(&path).await?;
        }
        Ok(())
    }
}

#[async_trait]
impl ThreadReader for FileStore {
    async fn load(&self, thread_id: &str) -> Result<Option<ThreadHead>, ThreadStoreError> {
        self.load_head(thread_id).await
    }

    async fn list_threads(
        &self,
        query: &ThreadListQuery,
    ) -> Result<ThreadListPage, ThreadStoreError> {
        // Read directory for all thread IDs
        let mut all = if !self.base_path.exists() {
            Vec::new()
        } else {
            let mut entries = tokio::fs::read_dir(&self.base_path).await?;
            let mut ids = Vec::new();
            while let Some(entry) = entries.next_entry().await? {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "json") {
                    if let Some(stem) = path.file_stem() {
                        if let Some(id) = stem.to_str() {
                            ids.push(id.to_string());
                        }
                    }
                }
            }
            ids
        };

        // Filter by resource_id if specified.
        if let Some(ref resource_id) = query.resource_id {
            let mut filtered = Vec::new();
            for id in &all {
                if let Some(head) = self.load(id).await? {
                    if head.thread.resource_id.as_deref() == Some(resource_id.as_str()) {
                        filtered.push(id.clone());
                    }
                }
            }
            all = filtered;
        }

        // Filter by parent_thread_id if specified.
        if let Some(ref parent_thread_id) = query.parent_thread_id {
            let mut filtered = Vec::new();
            for id in &all {
                if let Some(head) = self.load(id).await? {
                    if head.thread.parent_thread_id.as_deref() == Some(parent_thread_id.as_str()) {
                        filtered.push(id.clone());
                    }
                }
            }
            all = filtered;
        }

        all.sort();
        let total = all.len();
        let limit = query.limit.clamp(1, 200);
        let offset = query.offset.min(total);
        let end = (offset + limit + 1).min(total);
        let slice = &all[offset..end];
        let has_more = slice.len() > limit;
        let items: Vec<String> = slice.iter().take(limit).cloned().collect();
        Ok(ThreadListPage {
            items,
            total,
            has_more,
        })
    }
}

impl FileStore {
    /// Load a thread head (thread + version) from file.
    async fn load_head(&self, thread_id: &str) -> Result<Option<ThreadHead>, ThreadStoreError> {
        let path = self.thread_path(thread_id)?;
        if !path.exists() {
            return Ok(None);
        }
        let content = tokio::fs::read_to_string(&path).await?;
        // Try to parse as ThreadHead first (new format with version).
        if let Ok(head) = serde_json::from_str::<VersionedThread>(&content) {
            let thread: Thread = serde_json::from_str(&content)
                .map_err(|e| ThreadStoreError::Serialization(e.to_string()))?;
            Ok(Some(ThreadHead {
                thread,
                version: head._version.unwrap_or(0),
            }))
        } else {
            let thread: Thread = serde_json::from_str(&content)
                .map_err(|e| ThreadStoreError::Serialization(e.to_string()))?;
            Ok(Some(ThreadHead { thread, version: 0 }))
        }
    }

    /// Save a thread head (thread + version) to file atomically.
    async fn save_head(&self, head: &ThreadHead) -> Result<(), ThreadStoreError> {
        if !self.base_path.exists() {
            tokio::fs::create_dir_all(&self.base_path).await?;
        }
        let path = self.thread_path(&head.thread.id)?;

        // Embed version into the JSON
        let mut v = serde_json::to_value(&head.thread)
            .map_err(|e| ThreadStoreError::Serialization(e.to_string()))?;
        if let Some(obj) = v.as_object_mut() {
            obj.insert("_version".to_string(), serde_json::json!(head.version));
        }
        let content = serde_json::to_string_pretty(&v)
            .map_err(|e| ThreadStoreError::Serialization(e.to_string()))?;

        let tmp_path = self.base_path.join(format!(
            ".{}.{}.tmp",
            head.thread.id,
            uuid::Uuid::new_v4().simple()
        ));

        let write_result = async {
            let mut file = tokio::fs::File::create(&tmp_path).await?;
            file.write_all(content.as_bytes()).await?;
            file.flush().await?;
            file.sync_all().await?;
            drop(file);
            match tokio::fs::rename(&tmp_path, &path).await {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    tokio::fs::remove_file(&path).await?;
                    tokio::fs::rename(&tmp_path, &path).await?;
                }
                Err(e) => return Err(e),
            }
            Ok::<(), std::io::Error>(())
        }
        .await;

        if let Err(e) = write_result {
            let _ = tokio::fs::remove_file(&tmp_path).await;
            return Err(ThreadStoreError::Io(e));
        }
        Ok(())
    }
}

/// Helper for extracting the `_version` field from serialized thread JSON.
#[derive(Deserialize)]
struct VersionedThread {
    #[serde(default)]
    _version: Option<Version>,
}
