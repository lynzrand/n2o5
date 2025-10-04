//! DB Reader and writer

use std::path::Path;

use heed::WithTls;
use n2o5::db::{BuildHash, BuildInfo, DbReader, DbWriter, FileInfo};

use crate::codec::{BuildHashKey, BuildInfoWrap, FileInfoWrap, PathKey};
use crate::{BUILD_INFO_DB_NAME, FILE_INFO_DB_NAME};

pub struct DbRead<'a> {
    pub(crate) env: &'a heed::Env,
    pub(crate) txn: heed::RoTxn<'a, WithTls>,
}

impl<'a> DbReader for DbRead<'a> {
    fn get_build_info(&self, hash: n2o5::db::BuildHash) -> Option<n2o5::db::BuildInfo> {
        let db = self
            .env
            .open_database::<BuildHashKey, BuildInfoWrap>(&self.txn, Some(BUILD_INFO_DB_NAME))
            .ok()??;
        db.get(&self.txn, &hash).ok()?
    }

    fn get_file_info(&self, path: &Path) -> Option<FileInfo> {
        let db = self
            .env
            .open_database::<PathKey, FileInfoWrap>(&self.txn, Some(FILE_INFO_DB_NAME))
            .ok()??;
        db.get(&self.txn, path).ok()?
    }
}

pub struct DbWrite<'a> {
    pub(crate) env: &'a heed::Env<WithTls>,
    pub(crate) txn: heed::RwTxn<'a>,
}

impl<'a> DbWriter for DbWrite<'a> {
    fn set_build_info(&mut self, hash: BuildHash, info: BuildInfo) {
        let db = self
            .env
            .open_database::<BuildHashKey, BuildInfoWrap>(&self.txn, Some(BUILD_INFO_DB_NAME))
            .expect("Failed to open build database")
            .expect("Build database not found");
        db.put(&mut self.txn, &hash, &info)
            .expect("Failed to insert into build database");
    }

    fn invalidate_build(&mut self, hash: BuildHash) {
        let db = self
            .env
            .open_database::<BuildHashKey, BuildInfoWrap>(&self.txn, Some(BUILD_INFO_DB_NAME))
            .expect("Failed to open build database")
            .expect("Build database not found");
        db.delete(&mut self.txn, &hash)
            .expect("Failed to delete from build database");
    }

    fn set_file_info(&mut self, path: &Path, info: FileInfo) {
        let db = self
            .env
            .open_database::<PathKey, FileInfoWrap>(&self.txn, Some(FILE_INFO_DB_NAME))
            .expect("Failed to open file database")
            .expect("File database not found");
        db.put(&mut self.txn, path, &info)
            .expect("Failed to insert into file database");
    }

    fn invalidate_file(&mut self, path: &Path) {
        let db = self
            .env
            .open_database::<PathKey, FileInfoWrap>(&self.txn, Some(FILE_INFO_DB_NAME))
            .expect("Failed to open file database")
            .expect("File database not found");
        db.delete(&mut self.txn, path)
            .expect("Failed to delete from file database");
    }

    fn commit(self: Box<Self>) {
        self.txn.commit().expect("Failed to commit transaction");
    }
}
