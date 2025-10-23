//! Read/write transaction adapters for redb.

use std::path::Path;

use n2o5::db::{BuildHash, BuildInfo, DbReader, DbWriter, FileInfo};
use redb::{ReadTransaction, WriteTransaction};

use crate::{BUILD_TABLE, FILE_TABLE};

pub(crate) struct Reader {
    txn: ReadTransaction,
}

impl Reader {
    pub(crate) fn new(txn: ReadTransaction) -> Self {
        Self { txn }
    }
}

impl DbReader for Reader {
    fn get_build_info(&self, hash: BuildHash) -> Option<BuildInfo> {
        let table = self
            .txn
            .open_table(BUILD_TABLE)
            .expect("Failed to open build table");
        let guard = table.get(&hash).expect("Failed to read from build table")?;
        Some(guard.value())
    }

    fn get_file_info(&self, path: &Path) -> Option<FileInfo> {
        let table = self
            .txn
            .open_table(FILE_TABLE)
            .expect("Failed to open file table");
        let guard = table.get(path).expect("Failed to read from file table")?;
        Some(guard.value())
    }
}

pub(crate) struct Writer {
    txn: Option<WriteTransaction>,
}

impl Writer {
    pub(crate) fn new(txn: WriteTransaction) -> Self {
        Self { txn: Some(txn) }
    }

    fn txn(&mut self) -> &mut WriteTransaction {
        self.txn
            .as_mut()
            .expect("write transaction already completed")
    }
}

impl DbWriter for Writer {
    fn set_build_info(&mut self, hash: BuildHash, info: BuildInfo) {
        let txn = self.txn();
        let mut table = txn
            .open_table(BUILD_TABLE)
            .expect("Failed to open build table");
        table
            .insert(&hash, info)
            .expect("Failed to insert into build table");
    }

    fn invalidate_build(&mut self, hash: BuildHash) {
        let txn = self.txn();
        let mut table = txn
            .open_table(BUILD_TABLE)
            .expect("Failed to open build table");
        table
            .remove(&hash)
            .expect("Failed to remove from build table");
    }

    fn set_file_info(&mut self, path: &Path, info: FileInfo) {
        let txn = self.txn();
        let mut table = txn
            .open_table(FILE_TABLE)
            .expect("Failed to open file table");
        table
            .insert(path, info)
            .expect("Failed to insert into file table");
    }

    fn invalidate_file(&mut self, path: &Path) {
        let txn = self.txn();
        let mut table = txn
            .open_table(FILE_TABLE)
            .expect("Failed to open file table");
        table
            .remove(path)
            .expect("Failed to remove from file table");
    }

    fn commit(self: Box<Self>) {
        let mut this = self;
        let txn = this
            .txn
            .take()
            .expect("write transaction already completed");
        txn.commit().expect("Failed to commit transaction");
    }
}
