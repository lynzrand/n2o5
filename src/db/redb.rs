//! Implementation using [`redb`].

use std::{ffi::OsStr, path::Path};

use redb::{
    Key, ReadTransaction, ReadableDatabase, TableDefinition, TypeName, Value, WriteTransaction,
};

use crate::db::{BuildHash, BuildInfo, DbReader, ExecDb, FileInfo};

pub struct ExecRedb {
    inner: redb::Database,
}

impl ExecRedb {
    pub fn new(inner: redb::Database) -> Self {
        Self { inner }
    }

    pub fn open(p: impl AsRef<Path>) -> Result<Self, redb::DatabaseError> {
        let db = redb::Database::create(p)?;
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
        0 // TODO
    }

    fn reset(&self) {
        todo!()
    }

    fn begin_read<'r>(&'r self) -> Box<dyn super::DbReader + 'r> {
        let txn = self
            .inner
            .begin_read()
            .expect("Failed to begin read transaction");
        Box::new(Reader(txn))
    }

    fn begin_write<'w>(&'w self) -> Box<dyn super::DbWriter + 'w> {
        let txn = self
            .inner
            .begin_write()
            .expect("Failed to begin write transaction");
        Box::new(Writer(txn))
    }
}

impl Key for BuildHash {
    fn compare(data1: &[u8], data2: &[u8]) -> std::cmp::Ordering {
        data1.cmp(data2)
    }
}

impl Value for BuildHash {
    type SelfType<'a> = &'a Self;

    type AsBytes<'a> = &'a [u8; 16];

    fn fixed_width() -> Option<usize> {
        Some(16)
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        assert_eq!(data.len(), 16);
        let array: &[u8; 16] = data.try_into().expect("slice with incorrect length");
        // Safety: We just asserted that data is 16 bytes long
        unsafe { &*(array as *const [u8; 16] as *const BuildHash) }
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'b,
    {
        &value.0
    }

    fn type_name() -> redb::TypeName {
        TypeName::new(std::any::type_name::<BuildHash>())
    }
}

#[derive(Debug)]
struct PathKey;

impl Key for PathKey {
    fn compare(data1: &[u8], data2: &[u8]) -> std::cmp::Ordering {
        data1.cmp(data2)
    }
}

impl Value for PathKey {
    type SelfType<'a> = &'a Path;

    type AsBytes<'a> = &'a [u8];

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        unsafe { OsStr::from_encoded_bytes_unchecked(data).as_ref() }
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'b,
    {
        value.as_os_str().as_encoded_bytes()
    }

    fn type_name() -> TypeName {
        TypeName::new(std::any::type_name::<Path>())
    }
}

impl Value for FileInfo {
    type SelfType<'a> = FileInfo;

    type AsBytes<'a> = Vec<u8>;

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        postcard::from_bytes(data).expect("Failed to deserialize FileInfo")
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'b,
    {
        postcard::to_stdvec(value).expect("Failed to serialize FileInfo")
    }

    fn type_name() -> TypeName {
        TypeName::new(std::any::type_name::<FileInfo>())
    }
}

impl Value for BuildInfo {
    type SelfType<'a> = BuildInfo;

    type AsBytes<'a> = Vec<u8>;

    fn fixed_width() -> Option<usize> {
        None
    }

    fn from_bytes<'a>(data: &'a [u8]) -> Self::SelfType<'a>
    where
        Self: 'a,
    {
        postcard::from_bytes(data).expect("Failed to deserialize BuildInfo")
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'b,
    {
        postcard::to_stdvec(value).expect("Failed to serialize BuildInfo")
    }

    fn type_name() -> TypeName {
        TypeName::new(std::any::type_name::<BuildInfo>())
    }
}

struct Writer(WriteTransaction);

impl super::DbWriter for Writer {
    fn set_build_info(&mut self, hash: BuildHash, info: BuildInfo) {
        let mut tbl = self
            .0
            .open_table(BUILD_TABLE)
            .expect("Failed to open build table");
        tbl.insert(&hash, info)
            .expect("Failed to insert into build table");
    }

    fn invalidate_build(&mut self, hash: BuildHash) {
        let mut tbl = self
            .0
            .open_table(BUILD_TABLE)
            .expect("Failed to open build table");
        tbl.remove(&hash)
            .expect("Failed to remove from build table");
    }

    fn set_file_info(&mut self, path: &std::path::Path, info: FileInfo) {
        let mut tbl = self
            .0
            .open_table(FILE_TABLE)
            .expect("Failed to open file table");
        tbl.insert(path, info)
            .expect("Failed to insert into file table");
    }

    fn invalidate_file(&mut self, path: &std::path::Path) {
        let mut tbl = self
            .0
            .open_table(FILE_TABLE)
            .expect("Failed to open file table");
        tbl.remove(path).expect("Failed to remove from file table");
    }

    fn commit(self: Box<Self>) {
        self.0.commit().expect("Failed to commit transaction");
    }
}

static FILE_TABLE: TableDefinition<PathKey, FileInfo> = TableDefinition::new("files");
static BUILD_TABLE: TableDefinition<BuildHash, BuildInfo> = TableDefinition::new("builds");

struct Reader(ReadTransaction);

impl DbReader for Reader {
    fn get_build_info(&self, hash: BuildHash) -> Option<BuildInfo> {
        let tbl = self
            .0
            .open_table(BUILD_TABLE)
            .expect("Failed to open build table");
        let guard = tbl.get(&hash).expect("Failed to read from build table")?;
        Some(guard.value())
    }

    fn get_file_info(&self, path: &Path) -> Option<FileInfo> {
        let tbl = self
            .0
            .open_table(FILE_TABLE)
            .expect("Failed to open file table");
        let guard = tbl.get(path).expect("Failed to read from file table")?;
        Some(guard.value())
    }
}
