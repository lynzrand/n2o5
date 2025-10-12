//! Build graph representation and construction.

use std::{
    any::Any,
    borrow::Cow,
    error::Error,
    ffi::OsStr,
    fmt::Debug,
    io::Write,
    path::{Path, PathBuf},
};

use indexmap::IndexSet;
use petgraph::prelude::DiGraphMap;
use smol_str::SmolStr;

mod hash;
pub use hash::{hash_build, hash_input_set};

/// The build graph to be executed.
///
/// This type is immutable. To build it, use [`GraphBuilder`].
#[derive(Default, Debug)]
pub struct BuildGraph {
    nodes: Vec<BuildNode>,
    files: IndexSet<PathBuf>,
    pub(crate) graph: DiGraphMap<BuildId, ()>,
}

impl BuildGraph {
    pub fn nodes(&self) -> impl Iterator<Item = (BuildId, &BuildNode)> {
        self.nodes.iter().enumerate().map(|(i, n)| (BuildId(i), n))
    }

    pub fn files(&self) -> impl Iterator<Item = (FileId, &PathBuf)> {
        self.files.iter().enumerate().map(|(i, p)| (FileId(i), p))
    }

    pub fn build_dependencies(&self, build_id: BuildId) -> impl Iterator<Item = BuildId> + '_ {
        self.graph
            .neighbors_directed(build_id, petgraph::Direction::Outgoing)
    }

    pub fn build_dependents(&self, build_id: BuildId) -> impl Iterator<Item = BuildId> + '_ {
        self.graph
            .neighbors_directed(build_id, petgraph::Direction::Incoming)
    }

    pub fn lookup_fileid(&self, path: impl AsRef<Path>) -> Option<FileId> {
        self.files.get_index_of(path.as_ref()).map(FileId)
    }

    pub fn lookup_path(&self, file_id: FileId) -> Option<&PathBuf> {
        self.files.get_index(file_id.0)
    }

    pub fn lookup_build(&self, build_id: BuildId) -> Option<&BuildNode> {
        self.nodes.get(build_id.0)
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

/// The builder to build a [`BuildGraph`].
///
/// This builder is append-only. You will not be able to remove already-added
/// files or build nodes.
///
/// # Notes
///
/// This graph builder explicitly **does not** track the relationship between
/// build nodes using the file list in each build node, *i.e.* adding one node
/// outputting one file and another with that file as input does not
/// automatically make the latter node depend on the former. The file list is
/// only for checking build timestamps. You must manually add edges using
/// [`Self::add_build_dep`] to ensure the correct build order.
///
/// Currently, you still need to declare all input and output files in the build
/// nodes, including the input files that correspond to the output of other build
/// nodes. This will be relaxed in the future to reduce duplication.
//
// FIXME: Use the following version once we are done with node shapes & implicit
// inputs.
//
// A consequence of this is that you can actually skip defining inputs for
// files you have already declared as output of preceding commands, and use the
// build edge instead. You still need that output file for timestamp checking,
// or else the build will always be considered out-of-date.
#[derive(Default, Debug)]
pub struct GraphBuilder {
    graph: BuildGraph,
}

/// An index that uniquely identifies an (input or output) file in the build graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct FileId(usize);

/// An index that uniquely identifies a build node in the build graph.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BuildId(usize);

impl GraphBuilder {
    /// Create a new, empty build graph.
    pub fn new() -> Self {
        Default::default()
    }

    /// Add a file to the graph, returning its ID.
    ///
    /// This method accpets an owned `PathBuf`, avoiding an unnecessary clone if
    /// you already own the path.
    pub fn add_file_owned(&mut self, path: impl Into<PathBuf>) -> FileId {
        let path = path.into();
        if let Some(id) = self.graph.files.get_index_of(&path) {
            return FileId(id);
        }
        let id = self.graph.files.len();
        self.graph.files.insert(path);
        FileId(id)
    }

    /// Add a file to the graph, returning its ID.
    ///
    /// This method accepts a reference to a `Path`, cloning it if necessary.
    pub fn add_file(&mut self, path: impl AsRef<Path>) -> FileId {
        let path = path.as_ref();
        if let Some(id) = self.graph.files.get_index_of(path) {
            return FileId(id);
        }
        let id = self.graph.files.len();
        self.graph.files.insert(path.to_owned());
        FileId(id)
    }

    /// Add a build node to the graph, returning its ID.
    pub fn add_build(&mut self, build: BuildNode) -> BuildId {
        let id = self.graph.nodes.len();
        let build_id = BuildId(id);
        self.graph.nodes.push(build);
        build_id
    }

    /// Add a build dependency edge, where `dependent` relies on the finish of
    /// `dependency` to start.
    pub fn add_build_dep(&mut self, dependent: BuildId, dependency: BuildId) {
        self.graph.graph.add_edge(dependent, dependency, ());
    }

    /// Lookup a file ID by its path.
    pub fn lookup_fileid(&self, path: impl AsRef<Path>) -> Option<FileId> {
        self.graph.lookup_fileid(path)
    }

    /// Lookup a path by its file ID.
    pub fn lookup_path(&self, file_id: FileId) -> Option<&PathBuf> {
        self.graph.lookup_path(file_id)
    }

    /// Lookup a build node by its build ID.
    pub fn lookup_build(&self, build_id: BuildId) -> Option<&BuildNode> {
        self.graph.lookup_build(build_id)
    }

    /// Finish building the graph, returning it if valid.
    pub fn build(self) -> Result<BuildGraph, BuildError> {
        if petgraph::algo::is_cyclic_directed(&self.graph.graph) {
            return Err(BuildError::ContainsCycle);
        }
        Ok(self.graph)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum BuildError {
    #[error("The build graph contains a cycle")]
    ContainsCycle,
}

/// Represents a single node being built.
#[derive(Debug)]
pub struct BuildNode {
    pub command: BuildMethod,
    pub ins: Vec<FileId>,
    pub outs: Vec<FileId>,
    pub description: Option<Cow<'static, str>>,
    // pub restat: bool,
}

/// A callback to invoke as a build step.
///
/// It should be a function that accepts the build environment (as [`Any`]),
/// performs the necessary actions, and returns either `Ok(())` when succeeding,
/// or an error if something went wrong.
///
/// The callback will be executed in a threadpool. It should not spawn new
/// threads on its own. This might be changed in the future.
type BuildCallback =
    Box<dyn Fn(&dyn Any) -> Result<(), Box<dyn Error + Send + Sync>> + Send + Sync>;

/// Represents the method to build the target within a build node.
pub enum BuildMethod {
    /// A real, command-line command to run.
    SubCommand(BuildCommand),

    /// A callback function to invoke. Includes a name for version checking & debugging.
    ///
    /// For more info and constraints on the callback, see [`BuildCallback`].
    Callback(SmolStr, BuildCallback),

    /// A phony command that does nothing.
    Phony,
}

impl Debug for BuildMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SubCommand(arg0) => f.debug_tuple("SubCommand").field(arg0).finish(),
            Self::Callback(name, callback) => {
                let cb_ptr = (&**callback) as *const _;
                f.debug_tuple("Callback")
                    .field(name)
                    .field(&cb_ptr)
                    .finish()
            }
            Self::Phony => f.debug_tuple("Phony").finish(),
        }
    }
}

impl BuildMethod {
    pub fn write_human_readable(&self, mut w: impl Write) -> std::io::Result<()> {
        match self {
            BuildMethod::SubCommand(cmd) => {
                let quoted_cmd =
                    shlex::bytes::try_quote(cmd.executable.as_os_str().as_encoded_bytes())
                        .map_err(|x| std::io::Error::new(std::io::ErrorKind::InvalidFilename, x))?;
                w.write_all(&quoted_cmd)?;

                for arg in &cmd.args {
                    w.write_all(b" ")?;
                    let quoted_arg = shlex::bytes::try_quote(arg.as_encoded_bytes())
                        .map_err(|x| std::io::Error::new(std::io::ErrorKind::InvalidData, x))?;
                    w.write_all(&quoted_arg)?;
                }

                Ok(())
            }
            BuildMethod::Callback(smol_str, _) => write!(w, "<callback: {}>", smol_str),
            BuildMethod::Phony => write!(w, "<phony>"),
        }
    }
}

#[derive(Clone, Debug)]
pub struct BuildCommand {
    pub executable: PathBuf,
    pub args: Vec<Cow<'static, OsStr>>,
}
