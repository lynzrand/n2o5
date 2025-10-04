//! Caches the current build status onto disk.

pub mod in_memory;
pub mod redb;

use std::{
    fmt::Debug,
    path::{Path, PathBuf},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};

/// A hash that uniquely identifies a build command.
///
/// Generate one with [`crate::graph::hash_build`].
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct BuildHash(#[serde(with = "serde_bytes")] pub [u8; 16]);

impl Debug for BuildHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "BuildHash(")?;
        for byte in &self.0 {
            write!(f, "{:02x}", byte)?;
        }
        write!(f, ")")?;
        Ok(())
    }
}

/// A hash of a build's input environments.
///
/// Generate one with [`crate::graph::hash_input_set`].
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct InputHash(#[serde(with = "serde_bytes")] pub [u8; 16]);

impl Debug for InputHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "InputHash(")?;
        for byte in &self.0 {
            write!(f, "{:02x}", byte)?;
        }
        write!(f, ")")?;
        Ok(())
    }
}

/// The information associated with a specific file in the DB
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FileInfo {
    /// The timestamp of the file when it was last checked in the build system
    pub last_seen: SystemTime,
    /// The build that generated this file
    pub generated_by: BuildHash,
}

/// The information associated with a specific build in the DB
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct BuildInfo {
    /// The last time this build was started
    pub last_start: SystemTime,
    /// The last time this build was successfully completed
    pub last_end: Option<SystemTime>,
    /// The hash of the fixed input set (file, env var, etc.) to this build when
    /// it last ran.
    pub input_set_digest: InputHash,
    /// Additional inputs that was not part of the input set hash, but
    /// should be considered as dependencies for this build.
    pub additional_inputs: Vec<PathBuf>,
}

/// A trait for the database caching build and file information.
pub trait ExecDb: Send + Sync {
    /// Get the schema version of stored data.
    fn get_schema_version(&self) -> u64;

    /// Destroy all stored data and reset to an empty state.
    ///
    /// This might be used on schema version mismatch.
    fn reset(&self);

    /// Begin a read transaction. The database may block during this process.
    fn begin_read<'r>(&'r self) -> Box<dyn DbReader + 'r>;

    /// Begin a write transaction. The database may block during this process.
    fn begin_write<'w>(&'w self) -> Box<dyn DbWriter + 'w>;
}

pub trait DbReader {
    fn get_build_info(&self, hash: BuildHash) -> Option<BuildInfo>;
    fn get_file_info(&self, path: &Path) -> Option<FileInfo>;
}

pub trait DbWriter {
    fn set_build_info(&mut self, hash: BuildHash, info: BuildInfo);
    fn invalidate_build(&mut self, hash: BuildHash);
    fn set_file_info(&mut self, path: &Path, info: FileInfo);
    fn invalidate_file(&mut self, path: &Path);

    fn commit(self: Box<Self>);
}
