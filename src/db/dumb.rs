//! Very simple, serialize-based DB

use std::{
    fs::File,
    io::{Read, Seek, Write},
    path::Path,
    sync::{Arc, RwLock},
};

use crate::{
    ExecDb,
    db::in_memory::{self, DbInner, Reader, Writer},
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

const CFG: bincode::config::Configuration = bincode::config::standard();
const MAGIC: &[u8; 16] = b"AZ50_NTO_0000000";

impl DumbDb {
    /// Open and read the database from the given file.
    ///
    /// This operation will try to acquire the lock on the corresponding file.
    /// This operation will block until a lock can be acquired, which might be
    /// a long time if another process is using the database.
    ///
    /// If the DB cache file is in an invalid state, such as from an
    /// incompatible previous version of this crate or corrupted, the file will
    /// be unconditionally viewed as empty and cleared.
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

        // Verify magic
        let mut magic_buf = [0u8; 16];
        let Ok(_) = file.read_exact(&mut magic_buf) else {
            // Not enough bytes for magic. Just ignore it.
            tracing::warn!("Magic header does not exist, using empty DB");
            return Ok(Self::create(file, Default::default()));
        };
        if &magic_buf != MAGIC {
            // Invalid magic.
            tracing::warn!("Invalid DB magic header, using empty DB");
            return Ok(Self::create(file, Default::default()));
        }

        // An error in deserialization likely means it's corrupted for
        // whatever reason. Just create a new one later.
        let deserialized = bincode::decode_from_std_read(&mut file, CFG).unwrap_or_default();

        Ok(Self::create(file, deserialized))
    }

    fn create(file: std::fs::File, data: DbInner) -> Self {
        DumbDb {
            inner: Arc::new(DumbDbInner {
                file,
                data: RwLock::new(data),
            }),
        }
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

        self.file
            .write_all(MAGIC)
            .expect("Failed to write magic header to DumbDB");
        bincode::encode_into_std_write(&*data, &mut self.file, CFG)
            .expect("Failed to write the new database file to DumbDb");
        self.file.flush().expect("Failed to flush file");
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
