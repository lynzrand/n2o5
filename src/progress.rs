//! Progress reporting and output capture facility

use crate::BuildId;

/// Trait for reporting build progress and capturing output.
///
/// Methods of this type may be called from multiple threads, so implementations
/// must be thread-safe.
pub trait Progress: Send + Sync {
    /// Prepare the progress reporter with the given configuration.
    fn prepare(&self, config: &ProgressConfig);

    /// Callback when a build starts.
    fn build_started(&self, id: BuildId);

    /// Callback when a chunk of stdout is produced by a build.
    fn stdout_chunk(&self, id: BuildId, chunk: &[u8]);

    /// Callback when a build finishes.
    fn build_finished(&self, id: BuildId, success: bool);

    /// Callback when a build is skipped.
    fn status_update(&self, status: &ProgressStatus);
}

/// A config for the progress reporter.
#[derive(Clone, Debug)]
pub struct ProgressConfig {
    /// The maximum number of threads that might be spawn. Can be `None` if
    /// unknown.
    pub max_threads: Option<usize>,
}

/// Status of the current progress.
#[derive(Clone, Debug)]
pub struct ProgressStatus {
    /// The total number of builds to runs
    pub total: usize,
    /// The total number of builds finished, either successfully, unsuccessfully
    /// or skipped.
    pub done: usize,
    /// The number of builds that explicitly failed, not counting skipped ones.
    pub failed: usize,
}
