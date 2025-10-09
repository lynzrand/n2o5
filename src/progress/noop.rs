//! No-op progress reporter

use crate::{BuildGraph, BuildId};

use super::{Progress, ProgressConfig, ProgressStatus};

/// A no-op implementation of the Progress trait.
/// Useful for tests or environments where progress reporting is not desired.
pub struct NoopProgress;

impl Progress for NoopProgress {
    fn prepare(&self, _config: &ProgressConfig) {}

    fn build_started(&self, _graph: &BuildGraph, _id: BuildId, _status: &ProgressStatus) {}

    fn stdout_line(&self, _graph: &BuildGraph, _id: BuildId, _chunk: &[u8]) {}

    fn build_finished(
        &self,
        _graph: &BuildGraph,
        _id: BuildId,
        _success: bool,
        _status: &ProgressStatus,
    ) {
    }

    fn finish(&self) {}
}

/// A global no-op progress instance for convenience.
pub static NOOP_PROGRESS: NoopProgress = NoopProgress;
