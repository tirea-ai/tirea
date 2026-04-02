# 线程模型

Thread 表示持久化会话。`Thread` 本身只保存 thread 元信息；消息和 run 历史通过存储 trait 单独管理。

## Thread

```rust,ignore
pub struct Thread {
    pub id: String,
    pub metadata: ThreadMetadata,
}
```

### 构造函数

```rust,ignore
fn new() -> Self
fn with_id(id: impl Into<String>) -> Self
```

### Builder 方法

```rust,ignore
fn with_title(self, title: impl Into<String>) -> Self
```

## ThreadMetadata

```rust,ignore
pub struct ThreadMetadata {
    pub created_at: Option<u64>,
    pub updated_at: Option<u64>,
    pub title: Option<String>,
    pub custom: HashMap<String, Value>,
}
```

## 存储

消息不直接嵌在 `Thread` 里，而是通过 `ThreadStore` 读写：

```rust,ignore
#[async_trait]
pub trait ThreadStore: Send + Sync {
    async fn load_thread(&self, thread_id: &str) -> Result<Option<Thread>, StorageError>;
    async fn save_thread(&self, thread: &Thread) -> Result<(), StorageError>;
    async fn delete_thread(&self, thread_id: &str) -> Result<(), StorageError>;
    async fn list_threads(&self, offset: usize, limit: usize) -> Result<Vec<String>, StorageError>;
    async fn load_messages(&self, thread_id: &str) -> Result<Option<Vec<Message>>, StorageError>;
    async fn save_messages(&self, thread_id: &str, messages: &[Message]) -> Result<(), StorageError>;
    async fn delete_messages(&self, thread_id: &str) -> Result<(), StorageError>;
    async fn update_thread_metadata(&self, id: &str, metadata: ThreadMetadata) -> Result<(), StorageError>;
}
```

## ThreadRunStore

`ThreadRunStore` 在 `ThreadStore + RunStore` 基础上增加了原子 checkpoint：

```rust,ignore
#[async_trait]
pub trait ThreadRunStore: ThreadStore + RunStore + Send + Sync {
    async fn checkpoint(
        &self,
        thread_id: &str,
        messages: &[Message],
        run: &RunRecord,
    ) -> Result<(), StorageError>;
}
```

## RunStore

```rust,ignore
#[async_trait]
pub trait RunStore: Send + Sync {
    async fn create_run(&self, record: &RunRecord) -> Result<(), StorageError>;
    async fn load_run(&self, run_id: &str) -> Result<Option<RunRecord>, StorageError>;
    async fn latest_run(&self, thread_id: &str) -> Result<Option<RunRecord>, StorageError>;
    async fn list_runs(&self, query: &RunQuery) -> Result<RunPage, StorageError>;
}
```

## RunRecord

```rust,ignore
pub struct RunRecord {
    pub run_id: String,
    pub thread_id: String,
    pub agent_id: String,
    pub parent_run_id: Option<String>,
    pub status: RunStatus,
    pub termination_code: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
    pub steps: usize,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub state: Option<PersistedState>,
}
```

## 相关

- [使用文件存储](../how-to/use-file-store.md)
- [使用 Postgres 存储](../how-to/use-postgres-store.md)
