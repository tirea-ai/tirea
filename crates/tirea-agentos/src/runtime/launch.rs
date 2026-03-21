#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunPersistence {
    DurableRunRecord,
    ThreadOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunLineageMode {
    Preserve,
    Strip,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunLaunchSpec {
    persistence: RunPersistence,
    lineage: RunLineageMode,
}

impl RunLaunchSpec {
    pub const DURABLE_PRESERVE_LINEAGE: Self =
        Self::new(RunPersistence::DurableRunRecord, RunLineageMode::Preserve);

    pub const THREAD_ONLY_PRESERVE_LINEAGE: Self =
        Self::new(RunPersistence::ThreadOnly, RunLineageMode::Preserve);

    pub const THREAD_ONLY_STRIP_LINEAGE: Self =
        Self::new(RunPersistence::ThreadOnly, RunLineageMode::Strip);

    pub const HTTP_RUN_API: Self = Self::DURABLE_PRESERVE_LINEAGE;
    pub const HTTP_DIALOG: Self = Self::THREAD_ONLY_STRIP_LINEAGE;
    pub const BACKGROUND_TASK: Self = Self::DURABLE_PRESERVE_LINEAGE;

    pub const fn new(persistence: RunPersistence, lineage: RunLineageMode) -> Self {
        Self {
            persistence,
            lineage,
        }
    }

    pub const fn persistence(self) -> RunPersistence {
        self.persistence
    }

    pub const fn lineage(self) -> RunLineageMode {
        self.lineage
    }

    pub const fn persist_run_mapping(self) -> bool {
        matches!(self.persistence, RunPersistence::DurableRunRecord)
    }

    pub const fn strip_lineage(self) -> bool {
        matches!(self.lineage, RunLineageMode::Strip)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_dialog_is_thread_only_and_strips_lineage() {
        assert_eq!(
            RunLaunchSpec::HTTP_DIALOG.persistence(),
            RunPersistence::ThreadOnly
        );
        assert_eq!(RunLaunchSpec::HTTP_DIALOG.lineage(), RunLineageMode::Strip);
        assert!(!RunLaunchSpec::HTTP_DIALOG.persist_run_mapping());
        assert!(RunLaunchSpec::HTTP_DIALOG.strip_lineage());
    }

    #[test]
    fn background_task_is_durable_and_preserves_lineage() {
        assert_eq!(
            RunLaunchSpec::BACKGROUND_TASK.persistence(),
            RunPersistence::DurableRunRecord
        );
        assert_eq!(
            RunLaunchSpec::BACKGROUND_TASK.lineage(),
            RunLineageMode::Preserve
        );
        assert!(RunLaunchSpec::BACKGROUND_TASK.persist_run_mapping());
        assert!(!RunLaunchSpec::BACKGROUND_TASK.strip_lineage());
    }
}
