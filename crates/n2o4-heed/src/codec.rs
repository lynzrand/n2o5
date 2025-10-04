//! Encoding and decoding of values

use std::{borrow::Cow, os::unix::ffi::OsStrExt, path::Path};

use heed::{BytesDecode, BytesEncode};
use n2o4::db::{BuildHash, BuildInfo, FileInfo};

pub(crate) struct PathKey;

impl<'a> BytesEncode<'a> for PathKey {
    type EItem = Path;

    fn bytes_encode(item: &'a Self::EItem) -> Result<Cow<'a, [u8]>, heed::BoxedError> {
        Ok(item.as_os_str().as_encoded_bytes().into())
    }
}

impl<'a> BytesDecode<'a> for PathKey {
    type DItem = &'a Path;

    fn bytes_decode(bytes: &'a [u8]) -> Result<Self::DItem, heed::BoxedError> {
        Ok(Path::new(std::ffi::OsStr::from_bytes(bytes)))
    }
}

pub(crate) struct FileInfoWrap;

impl<'a> BytesEncode<'a> for FileInfoWrap {
    type EItem = FileInfo;

    fn bytes_encode(item: &'a Self::EItem) -> Result<Cow<'a, [u8]>, heed::BoxedError> {
        let vec = postcard::to_allocvec(item)?;
        Ok(vec.into())
    }
}

impl<'a> BytesDecode<'a> for FileInfoWrap {
    type DItem = FileInfo;

    fn bytes_decode(bytes: &'a [u8]) -> Result<Self::DItem, heed::BoxedError> {
        let info = postcard::from_bytes(bytes)?;
        Ok(info)
    }
}

pub(crate) struct BuildInfoWrap;

impl<'a> BytesEncode<'a> for BuildInfoWrap {
    type EItem = BuildInfo;

    fn bytes_encode(item: &'a Self::EItem) -> Result<Cow<'a, [u8]>, heed::BoxedError> {
        let vec = postcard::to_allocvec(item)?;
        Ok(vec.into())
    }
}

impl<'a> BytesDecode<'a> for BuildInfoWrap {
    type DItem = BuildInfo;

    fn bytes_decode(bytes: &'a [u8]) -> Result<Self::DItem, heed::BoxedError> {
        let info = postcard::from_bytes(bytes)?;
        Ok(info)
    }
}

pub(crate) struct BuildHashKey;

impl<'a> BytesEncode<'a> for BuildHashKey {
    type EItem = BuildHash;

    fn bytes_encode(item: &'a Self::EItem) -> Result<Cow<'a, [u8]>, heed::BoxedError> {
        Ok(Cow::Borrowed(&item.0))
    }
}

impl<'a> BytesDecode<'a> for BuildHashKey {
    type DItem = BuildHash;

    fn bytes_decode(bytes: &'a [u8]) -> Result<Self::DItem, heed::BoxedError> {
        if bytes.len() != 16 {
            return Err(format!("Invalid BuildHash length: {}", bytes.len()).into());
        }
        let mut arr = [0u8; 16];
        arr.copy_from_slice(bytes);
        Ok(BuildHash(arr))
    }
}
