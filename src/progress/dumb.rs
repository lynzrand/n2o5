//! Dumb console progress reporting

use std::io::Write;

use crate::progress::Progress;

pub struct DumbConsoleProgress;

impl Progress for DumbConsoleProgress {
    fn prepare(&self, _config: &super::ProgressConfig) {}

    fn build_started(
        &self,
        graph: &crate::BuildGraph,
        id: crate::BuildId,
        status: &super::ProgressStatus,
    ) {
        print!("[{}/{}] ", status.started + 1, status.total);
        let cmd = graph.lookup_build(id).expect("invalid build id");
        println!("{}", cmd.human_readable());
    }

    fn stdout_line(&self, _graph: &crate::BuildGraph, _id: crate::BuildId, chunk: &[u8]) {
        std::io::stdout().write_all(chunk).unwrap();
    }

    fn build_finished(
        &self,
        _graph: &crate::BuildGraph,
        _id: crate::BuildId,
        _success: bool,
        _status: &super::ProgressStatus,
    ) {
    }

    fn finish(&self) {}
}
