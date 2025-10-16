//! In-memory mocked implementation

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use crate::db::{BuildHash, BuildInfo, DbReader, ExecDb, FileInfo};

use super::DbWriter;

#[derive(Clone)]
pub struct InMemoryDb {
    inner: Arc<RwLock<DbInner>>,
}

impl InMemoryDb {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(RwLock::new(DbInner {
                schema_version: 1,
                build_info: HashMap::new(),
                file_info: HashMap::new(),
            })),
        }
    }
}

impl Default for InMemoryDb {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Default, serde::Deserialize, serde::Serialize)]
pub(super) struct DbInner {
    schema_version: u64,
    build_info: HashMap<BuildHash, BuildInfo>,
    file_info: HashMap<PathBuf, FileInfo>,
}

pub struct Reader<'r>(pub(super) RwLockReadGuard<'r, DbInner>);

pub struct Writer<'w>(pub(super) RwLockWriteGuard<'w, DbInner>);

impl ExecDb for InMemoryDb {
    fn get_schema_version(&self) -> u64 {
        self.inner.read().unwrap().schema_version
    }

    fn reset(&self) {
        let mut inner = self.inner.write().unwrap();
        inner.build_info.clear();
        inner.file_info.clear();
    }

    fn begin_read<'r>(&'r self) -> Box<dyn super::DbReader + 'r> {
        Box::new(Reader(self.inner.read().unwrap()))
    }

    fn begin_write<'w>(&'w self) -> Box<dyn DbWriter + 'w> {
        Box::new(Writer(self.inner.write().unwrap()))
    }
}

impl<'r> DbReader for Reader<'r> {
    fn get_build_info(&self, hash: BuildHash) -> Option<BuildInfo> {
        self.0.build_info.get(&hash).cloned()
    }

    fn get_file_info(&self, path: &std::path::Path) -> Option<FileInfo> {
        self.0.file_info.get(path).cloned()
    }
}

impl<'w> DbWriter for Writer<'w> {
    fn set_build_info(&mut self, hash: BuildHash, info: BuildInfo) {
        self.0.build_info.insert(hash, info);
    }

    fn set_file_info(&mut self, path: &Path, info: FileInfo) {
        self.0.file_info.insert(path.into(), info);
    }

    fn invalidate_build(&mut self, hash: BuildHash) {
        self.0.build_info.remove(&hash);
    }

    fn invalidate_file(&mut self, path: &std::path::Path) {
        self.0.file_info.remove(path);
    }

    fn commit(self: Box<Self>) {
        // No-op for in-memory DB
    }
}
