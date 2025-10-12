//! Fancy console progress bar

use std::io::Write;

use indicatif::{ProgressBar, ProgressStyle};

use crate::progress::Progress;

pub struct FancyConsoleProgress {
    progress: indicatif::ProgressBar,
}

impl FancyConsoleProgress {
    pub fn new() -> Self {
        Self {
            progress: ProgressBar::no_length().with_style(
                ProgressStyle::with_template("[{bar:30}] {pos}/{len}: {wide_msg}")
                    .expect("invalid progress style")
                    .progress_chars("=> "),
            ),
        }
    }

    fn update_progress(&self, status: &super::ProgressStatus) {
        self.progress.set_length(status.total as u64);
        self.progress.set_position((status.started + 1) as u64);
    }
}

impl Default for FancyConsoleProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl Progress for FancyConsoleProgress {
    fn prepare(&self, _config: &super::ProgressConfig) {}

    fn build_started(
        &self,
        graph: &crate::BuildGraph,
        id: crate::BuildId,
        status: &super::ProgressStatus,
    ) {
        self.update_progress(status);
        let mut human_cmd = vec![];
        graph
            .lookup_build(id)
            .expect("invalid build id")
            .command
            .write_human_readable(&mut human_cmd)
            .expect("Write to string cannot fail");
        let human_cmd = String::from_utf8_lossy(&human_cmd);
        self.progress.set_message(human_cmd.into_owned());
    }

    fn stdout_line(&self, _graph: &crate::BuildGraph, _id: crate::BuildId, chunk: &[u8]) {
        self.progress.suspend(|| {
            std::io::stdout().write_all(chunk).unwrap();
            println!()
        })
    }

    fn build_finished(
        &self,
        _graph: &crate::BuildGraph,
        _id: crate::BuildId,
        _success: bool,
        status: &super::ProgressStatus,
    ) {
        self.update_progress(status);
    }

    fn finish(&self) {
        self.progress.finish_and_clear();
    }
}
