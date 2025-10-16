//! Very simple, serialize-based DB

use std::{
    fs::File,
    io::{Read, Seek},
    path::Path,
    sync::{Arc, RwLock},
};

use crate::{
    ExecDb,
    db::in_memory::{self, Reader, Writer},
};

/// File-backed im-memory [`ExecDb`] for small tasks and single runs.
///
/// Loads the entire database into memory on open, holds an exclusive file lock
/// for the lifetime of the instance, and persists the whole state back to disk
/// on drop. Reads and writes during execution are in-memory only; there is no
/// mid-run durability. If the on-disk data cannot be deserialized, a fresh
/// empty database is used.
///
/// Use when:
/// - Small graphs and low write throughput (e.g. incremental builds in
///   build.rs), needing just enough build caching to avoid rebuilding
///   everything when data changes.
/// - Losing the build cache is acceptable.
///
/// Avoid when:
/// - You expect interrupts mid-run.
/// - Your graphs are large, or you generate a lot of files.
pub struct DumbDb {
    inner: Arc<DumbDbInner>,
}

struct DumbDbInner {
    file: File,
    data: RwLock<in_memory::DbInner>,
}

impl DumbDb {
    /// Open and read the database from the given file.
    ///
    /// This operation will try to acquire the lock on the corresponding file.
    /// This operation will block until a lock can be acquired, which might be
    /// a long time if another process is using the database.
    pub fn new(path: impl AsRef<Path>) -> std::io::Result<DumbDb> {
        // Open the file first for reading the contents, and then hold the FD
        // till the end to write. Holding the lock at the meantime.
        let mut file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(path)?;
        file.lock()?;

        let mut data = vec![];
        file.read_to_end(&mut data)?;
        // An error in deserialization likely means it's corrupted for
        // whatever reason. Just create a new one later.
        let deserialized = postcard::from_bytes(&data).unwrap_or_default();

        Ok(DumbDb {
            inner: Arc::new(DumbDbInner {
                file,
                data: RwLock::new(deserialized),
            }),
        })
    }
}

impl Drop for DumbDbInner {
    fn drop(&mut self) {
        self.file
            .rewind()
            .expect("Failed to seek to start in DumbDb");
        self.file
            .set_len(0)
            .expect("Failed to truncate DumbDb file");
        let data = self.data.get_mut().expect("lock poisoned");
        postcard::to_io(&data, &mut self.file)
            .expect("Failed to write the new database file to DumbDb");
    }
}

impl ExecDb for DumbDb {
    fn get_schema_version(&self) -> u64 {
        panic!("will remove this")
    }

    fn reset(&self) {
        panic!("will remove this")
    }

    fn begin_read<'r>(&'r self) -> Box<dyn super::DbReader + 'r> {
        Box::new(Reader(self.inner.data.read().unwrap()))
    }

    fn begin_write<'w>(&'w self) -> Box<dyn super::DbWriter + 'w> {
        Box::new(Writer(self.inner.data.write().unwrap()))
    }
}
