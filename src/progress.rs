//! Progress reporting and output capture facility

#[cfg(feature = "progress-dumb")]
pub mod dumb;
#[cfg(feature = "progress-fancy")]
pub mod fancy;
pub mod noop;

pub use noop::{NOOP_PROGRESS, NoopProgress};

#[cfg(feature = "progress-dumb")]
pub use dumb::DumbConsoleProgress;

#[cfg(feature = "progress-fancy")]
pub use fancy::FancyConsoleProgress;

use crate::{BuildGraph, BuildId};

/// Trait for reporting build progress and capturing output.
///
/// Methods of this type may be called from multiple threads, so implementations
/// must be thread-safe.
///
// TODO: Should we pass the graph only once in prepare, or have a separate
// factory type that creates Progress instances for a given graph instead?
pub trait Progress: Send + Sync {
    /// Prepare the progress reporter with the given configuration.
    fn prepare(&self, config: &ProgressConfig);

    /// Callback when a build starts.
    fn build_started(&self, graph: &BuildGraph, id: BuildId, status: &ProgressStatus);

    /// Callback when a chunk of stdout is produced by a build.
    fn stdout_line(&self, graph: &BuildGraph, id: BuildId, chunk: &[u8]);

    /// Callback when a build finishes.
    fn build_finished(
        &self,
        graph: &BuildGraph,
        id: BuildId,
        success: bool,
        status: &ProgressStatus,
    );

    /// Called when a progress session has finished
    fn finish(&self);
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
    /// The number of builds that have started, including those that are finished.
    pub started: usize,
    /// The total number of builds finished, either successfully, unsuccessfully
    /// or skipped.
    pub done: usize,
    /// The number of builds that explicitly failed, not counting skipped ones.
    pub failed: usize,
}
