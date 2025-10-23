//! Helpers for encoding redb keys and values.

use std::{ffi::OsStr, path::Path};

use n2o5::db::{BuildHash, BuildInfo, FileInfo};
use redb::{Key, TypeName, Value};

#[derive(Debug)]
pub(crate) struct PathKey;

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
        let os = unsafe { OsStr::from_encoded_bytes_unchecked(data) };
        Path::new(os)
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

#[derive(Debug)]
pub(crate) struct BuildHashKey;

impl Key for BuildHashKey {
    fn compare(data1: &[u8], data2: &[u8]) -> std::cmp::Ordering {
        data1.cmp(data2)
    }
}

impl Value for BuildHashKey {
    type SelfType<'a> = &'a BuildHash;
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
        // Safety: BuildHash is #[repr(transparent)] over [u8; 16]
        unsafe { &*(array as *const [u8; 16] as *const BuildHash) }
    }

    fn as_bytes<'a, 'b: 'a>(value: &'a Self::SelfType<'b>) -> Self::AsBytes<'a>
    where
        Self: 'b,
    {
        &value.0
    }

    fn type_name() -> TypeName {
        TypeName::new(std::any::type_name::<BuildHash>())
    }
}

#[derive(Debug)]
pub(crate) struct FileInfoValue;

impl Value for FileInfoValue {
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

#[derive(Debug)]
pub(crate) struct BuildInfoValue;

impl Value for BuildInfoValue {
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
