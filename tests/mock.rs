//! Test in a mock environment

use std::{
    any::Any,
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
};

use n2o4::{
    exec::BuildStatusKind,
    graph::{BuildCommand, BuildMethod},
    world::World,
};
use smol_str::SmolStr;

/// A mock [`World`] implementation that works entirely in-memory.
pub struct MockWorld {
    inner: Mutex<MockWorldInner>,
}

pub type MockCallback =
    Box<dyn Fn(&dyn Any, &BuildMethod) -> std::io::Result<BuildStatusKind> + Send + Sync>;

struct MockWorldInner {
    /// A number that roughly represent a mocked system time. Increases every
    /// time a file is touched.
    epoch: u64,
    /// Map from in-memory file list to their modification epoch
    files: HashMap<PathBuf, u64>,
    /// A log of executed commands
    exec_log: Vec<MockExecResult>,
    /// Execution callback
    callback: Option<MockCallback>,
}

#[derive(Debug, Clone)]
pub enum MockExecResult {
    Subcommand(BuildCommand),
    Callback(SmolStr),
    Phony,
}

impl World for MockWorld {
    fn exists(&self, path: &std::path::Path) -> bool {
        let inner = self.inner.lock().unwrap();
        inner.files.contains_key(path)
    }

    fn mtime(&self, path: &std::path::Path) -> std::io::Result<std::time::SystemTime> {
        let inner = self.inner.lock().unwrap();
        let epoch = inner
            .files
            .get(path)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "file not found"))?;
        Ok(std::time::UNIX_EPOCH + std::time::Duration::from_secs(*epoch))
    }

    fn execute(
        &self,
        state: &dyn std::any::Any,
        cmd: &n2o4::graph::BuildMethod,
    ) -> std::io::Result<BuildStatusKind> {
        let mut inner = self.inner.lock().unwrap();
        match cmd {
            n2o4::graph::BuildMethod::Phony => {
                inner.exec_log.push(MockExecResult::Phony);
            }
            n2o4::graph::BuildMethod::SubCommand(cmd) => {
                inner.exec_log.push(MockExecResult::Subcommand(cmd.clone()));
            }
            n2o4::graph::BuildMethod::Callback(name, _) => {
                inner.exec_log.push(MockExecResult::Callback(name.clone()));
                // We don't actually call the callback in the mock world.
            }
        }
        if let Some(cb) = &inner.callback {
            cb(state, cmd)
        } else {
            Ok(BuildStatusKind::Succeeded)
        }
    }
}

#[allow(unused)]
impl MockWorld {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(MockWorldInner {
                epoch: 0,
                files: HashMap::new(),
                exec_log: Vec::new(),
                callback: None,
            }),
        }
    }

    /// Set a file as existing, updating its modification time to the current epoch.
    pub fn touch_file(&self, path: impl AsRef<Path>) {
        let mut inner = self.inner.lock().unwrap();
        inner.epoch += 1;
        let epoch = inner.epoch;
        if let Some(file_epoch) = inner.files.get_mut(path.as_ref()) {
            *file_epoch = epoch;
        } else {
            inner.files.insert(path.as_ref().to_owned(), epoch);
        }
    }

    /// Remove a file from the mock world.
    pub fn remove_file(&self, path: impl AsRef<Path>) {
        let mut inner = self.inner.lock().unwrap();
        inner.files.remove(path.as_ref());
    }

    /// Take and clear the execution log.
    pub fn take_log(&self) -> Vec<MockExecResult> {
        let mut inner = self.inner.lock().unwrap();
        std::mem::take(&mut inner.exec_log)
    }

    /// Set an execution callback to customize command execution behavior.
    pub fn set_callback(&self, callback: MockCallback) {
        let mut inner = self.inner.lock().unwrap();
        inner.callback = Some(callback);
    }
}

impl Default for MockWorld {
    fn default() -> Self {
        Self::new()
    }
}
