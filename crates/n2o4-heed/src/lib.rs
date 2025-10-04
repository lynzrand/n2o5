//! Heed-backed ExecDb implementation, mirroring the redb backend behavior.

use std::path::Path;

use heed::EnvOpenOptions;
use n2o4::db::ExecDb;

use crate::codec::{BuildHashKey, BuildInfoWrap, FileInfoWrap, PathKey};

mod codec;
mod rw;

pub const FILE_INFO_DB_NAME: &str = "files";
pub const BUILD_INFO_DB_NAME: &str = "builds";

pub struct ExecHeedDb {
    inner: heed::Env,
}

impl ExecHeedDb {
    pub fn new(inner: heed::Env) -> Self {
        Self { inner }
    }

    pub fn open(path: impl AsRef<Path>) -> heed::Result<Self> {
        // Create or open an LMDB environment with named databases
        let env = unsafe { EnvOpenOptions::new().max_dbs(4).open(path)? };

        // Ensure DBs exist
        {
            let mut wtxn = env.write_txn()?;
            // Create if not existing; returns Ok(Some(db)) on open, Ok(None) if not found
            env.create_database::<PathKey, FileInfoWrap>(&mut wtxn, Some(FILE_INFO_DB_NAME))?;
            env.create_database::<BuildHashKey, BuildInfoWrap>(
                &mut wtxn,
                Some(BUILD_INFO_DB_NAME),
            )?;
            wtxn.commit()?;
        }

        Ok(Self { inner: env })
    }
}

impl ExecDb for ExecHeedDb {
    fn get_schema_version(&self) -> u64 {
        // Keep parity with redb backend for now
        0
    }

    fn reset(&self) {
        // Clear both databases
        let mut wtxn = self
            .inner
            .write_txn()
            .expect("Failed to begin write transaction");
        if let Ok(Some(db)) = self
            .inner
            .open_database::<PathKey, FileInfoWrap>(&wtxn, Some(FILE_INFO_DB_NAME))
        {
            db.clear(&mut wtxn).expect("Failed to clear files database");
        }
        if let Ok(Some(db)) = self
            .inner
            .open_database::<BuildHashKey, BuildInfoWrap>(&wtxn, Some(BUILD_INFO_DB_NAME))
        {
            db.clear(&mut wtxn)
                .expect("Failed to clear builds database");
        }
        wtxn.commit().expect("Failed to commit reset transaction");
    }

    fn begin_read<'r>(&'r self) -> Box<dyn n2o4::db::DbReader + 'r> {
        let txn = self
            .inner
            .read_txn()
            .expect("Failed to begin read transaction");
        Box::new(rw::DbRead {
            env: &self.inner,
            txn,
        })
    }

    fn begin_write<'w>(&'w self) -> Box<dyn n2o4::db::DbWriter + 'w> {
        let txn = self
            .inner
            .write_txn()
            .expect("Failed to begin write transaction");
        Box::new(rw::DbWrite {
            env: &self.inner,
            txn,
        })
    }
}
