//! Redb-backed `ExecDb` implementation.

use std::path::Path;

use n2o5::db::{DbReader, DbWriter, ExecDb};
use redb::{ReadableDatabase, TableDefinition};

mod codec;
mod rw;

use crate::codec::{BuildHashKey, BuildInfoValue, FileInfoValue, PathKey};
use crate::rw::{Reader, Writer};

pub(crate) static FILE_TABLE: TableDefinition<PathKey, FileInfoValue> =
    TableDefinition::new("files");
pub(crate) static BUILD_TABLE: TableDefinition<BuildHashKey, BuildInfoValue> =
    TableDefinition::new("builds");

pub struct ExecRedb {
    inner: redb::Database,
}

impl ExecRedb {
    pub fn new(inner: redb::Database) -> Self {
        Self { inner }
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self, redb::DatabaseError> {
        let db = redb::Database::create(path)?;
        let txn = db
            .begin_write()
            .expect("Failed to begin initial transaction");
        txn.open_table(FILE_TABLE)
            .expect("Failed to create file table");
        txn.open_table(BUILD_TABLE)
            .expect("Failed to create build table");
        txn.commit().expect("Failed to commit initial transaction");

        Ok(Self { inner: db })
    }
}

impl ExecDb for ExecRedb {
    fn get_schema_version(&self) -> u64 {
        // Keep parity with other backends
        0
    }

    fn reset(&self) {
        let txn = self
            .inner
            .begin_write()
            .expect("Failed to begin reset transaction");

        // Recreate tables by deleting and reopening them.
        txn.delete_table(FILE_TABLE)
            .expect("Failed to delete file table during reset");
        txn.delete_table(BUILD_TABLE)
            .expect("Failed to delete build table during reset");

        txn.open_table(FILE_TABLE)
            .expect("Failed to recreate file table during reset");
        txn.open_table(BUILD_TABLE)
            .expect("Failed to recreate build table during reset");

        txn.commit().expect("Failed to commit reset transaction");
    }

    fn begin_read<'r>(&'r self) -> Box<dyn DbReader + 'r> {
        let txn = self
            .inner
            .begin_read()
            .expect("Failed to begin read transaction");
        Box::new(Reader::new(txn))
    }

    fn begin_write<'w>(&'w self) -> Box<dyn DbWriter + 'w> {
        let txn = self
            .inner
            .begin_write()
            .expect("Failed to begin write transaction");
        Box::new(Writer::new(txn))
    }
}
