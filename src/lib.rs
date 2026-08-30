//! n2o5 is a build system as a library: an embedded executor for explicit,
//! in-memory build graphs.
//!
//! Unlike Make or Ninja, the build graph is never reconstructed from the
//! artifacts that build commands emit. You construct the graph directly from
//! build nodes and the edges between them; files are only consulted to decide
//! whether a node is up to date. Build steps can be plain subprocesses or
//! first-class Rust callbacks, and the execution environment is abstracted
//! behind the [`World`] trait, so builds can run against any "world" --
//! including a fully mocked one in tests.
//!
//! # Example
//!
//! Build "hello.txt" through an in-process callback, executing the graph
//! with an in-memory database:
//!
//! ```
//! use n2o5::graph::{BuildMethod, BuildNode};
//! use n2o5::progress::NoopProgress;
//! use n2o5::{ExecConfig, Executor, GraphBuilder, InMemoryDb};
//!
//! let mut builder = GraphBuilder::new();
//! let out = builder.add_file("hello.txt");
//!
//! // A build node that writes hello.txt. In a real build the callback
//! // would do the writing; here it does nothing.
//! let node = BuildNode {
//!     command: BuildMethod::Callback(
//!         "write_hello".into(),
//!         Box::new(|_user_state| Ok(())),
//!     ),
//!     ins: vec![],
//!     outs: vec![out],
//!     description: None,
//! };
//! let id = builder.add_build(node);
//! let graph = builder.build().expect("graph should be acyclic");
//!
//! let config = ExecConfig::default();
//! let db = InMemoryDb::new();
//! let user_state = (); // available to build callbacks
//! let progress = NoopProgress;
//!
//! let mut executor =
//!     Executor::new(&config, &graph, &db, &progress, &user_state);
//! executor.want([id]);
//! executor.run().expect("build should succeed");
//! ```
//!
//! # Pluggable parts
//!
//! - **World** ([`World`], [`LocalWorld`]): file system access and process
//!   spawning. Mock it for tests, or port it to another environment.
//! - **Database** ([`ExecDb`], [`InMemoryDb`]): caches build state between
//!   runs. The crate ships an in-memory database and a simple file-backed
//!   one (`db::dumb::DumbDb`); embedded-database backends live in the
//!   repository but are not yet published to crates.io.
//! - **Progress** ([`progress::Progress`], [`progress::NoopProgress`]):
//!   build status reporting and output capture.
//!
//! See the repository README for the full design rationale.

pub mod db;
pub mod exec;
pub mod graph;
pub mod progress;
pub mod world;

// Re-exports for convenience
pub use db::ExecDb;
pub use db::in_memory::InMemoryDb;
pub use exec::{ExecConfig, Executor};
pub use graph::{BuildGraph, BuildId, FileId, GraphBuilder};
pub use world::{LocalWorld, World};
