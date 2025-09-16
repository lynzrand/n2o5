//! Build graph execution

use std::{
    any::Any,
    collections::{HashMap, HashSet},
    ops::ControlFlow,
    path::PathBuf,
    process::Command,
    sync::{Arc, RwLock, mpsc},
    time::SystemTime,
};

use tracing::warn;

use crate::{
    db::{BuildInfo, ExecDb},
    graph::{BuildGraph, BuildId, BuildNode, hash_build},
};

#[derive(Debug)]
pub struct ExecConfig {
    /// The maximum amount of actions that can execute in parallel.
    pub parallelism: usize,
}

impl Default for ExecConfig {
    fn default() -> Self {
        Self { parallelism: 1 }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileStatus {
    UpToDate,
    Outdated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BuildStatusKind {
    /// The build hasn't been checked yet
    Fresh,
    /// The build is currently outdated and needs running
    Outdated,
    /// The build is up-to-date and does not need running
    UpToDate,
    /// Building has failed
    Failed,
    /// Building has succeeded
    Succeeded,
    /// Cannot run because a dependency has failed
    Skipped,
}

impl BuildStatusKind {
    fn is_finished(self) -> bool {
        matches!(
            self,
            BuildStatusKind::UpToDate
                | BuildStatusKind::Failed
                | BuildStatusKind::Succeeded
                | BuildStatusKind::Skipped
        )
    }
}

#[derive(Debug, Clone)]
struct BuildStatus {
    kind: BuildStatusKind,
    n_inputs: usize,
    /// The number of input nodes of this build that has
    /// [finished](BuildStatusKind::is_finished).
    n_inputs_finished: usize,
}

/// The executor that runs a build graph.
pub struct Executor<'a> {
    cfg: &'a ExecConfig,

    /// The build graph.
    ///
    /// Allocation is required because it needs to be accessed from multiple
    /// threads without clear lifetime constraints.
    #[allow(clippy::redundant_allocation)]
    graph: Arc<&'a BuildGraph>,

    db: Arc<RwLock<dyn ExecDb>>,

    pool: rayon::ThreadPool,
    /// The starting nodes that have yet to be executed for the build
    starts: HashSet<BuildId>,
    /// The current status of each build node
    builds: HashMap<BuildId, BuildStatus>,
    started: usize,
    finished: usize,

    build_started: bool,
}

impl<'a> Executor<'a> {
    pub fn new(cfg: &'a ExecConfig, graph: &'a BuildGraph, db: Arc<RwLock<dyn ExecDb>>) -> Self {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(cfg.parallelism)
            .build()
            .unwrap();
        Self {
            cfg,
            graph: Arc::new(graph),
            pool,
            db,

            starts: Default::default(),
            builds: Default::default(),
            started: 0,
            finished: 0,

            build_started: false,
        }
    }

    /// Add a list of build nodes to be executed. Returns number of nodes affected.
    ///
    /// This method should be called before [`Self::run`].
    pub fn want(&mut self, build: impl IntoIterator<Item = BuildId>) -> usize {
        if self.build_started {
            panic!("Cannot call want() after run() has been called");
        }

        let mut dfs_stack = vec![];
        dfs_stack.extend(build);
        self.want_internal(dfs_stack)
    }

    fn want_internal(&mut self, start_stack: Vec<BuildId>) -> usize {
        let mut affected_nodes = 0;
        let mut dfs_stack = start_stack;

        while let Some(build) = dfs_stack.pop() {
            if self.builds.contains_key(&build) {
                continue;
            }

            affected_nodes += 1;

            let mut children_count: usize = 0;
            for node in self.graph.build_dependencies(build) {
                children_count += 1;
                dfs_stack.push(node);
            }
            if children_count == 0 {
                // This is a leaf node, add it to the starts
                self.starts.insert(build);
            }

            // Initialize/reinit node, no difference either case.
            self.builds.insert(
                build,
                BuildStatus {
                    kind: BuildStatusKind::Fresh,
                    n_inputs: children_count,
                    n_inputs_finished: 0,
                },
            );
        }

        affected_nodes
    }

    /// Perform the build.
    pub fn run(&mut self) {
        // TODO: should we prevent it from running more than once?
        let (tx, rx) = mpsc::channel();

        self.build_started = true;

        let res = self.pool.in_place_scope(|pool| {
            // Starting nodes
            for &node in &self.starts {
                let graph = Arc::clone(&self.graph);
                let db = self.db.clone();
                let tx = tx.clone();
                pool.spawn(move |_p| run_build(&graph, &db, node, &(), tx));
            }

            while let Ok(msg) = rx.recv() {
                let stat = match msg.result {
                    Ok(res) => res,
                    Err(e) => {
                        warn!("Our build executor has encountered a problem: {e}");
                        return Err(e);
                    }
                };

                match stat {
                    BuildStatusKind::Fresh | BuildStatusKind::Outdated => {
                        panic!("Build must return a finished status")
                    }
                    BuildStatusKind::UpToDate => todo!(),
                    BuildStatusKind::Failed => todo!(),
                    BuildStatusKind::Succeeded => todo!(),
                    BuildStatusKind::Skipped => todo!(),
                }
            }

            todo!("Receive and process results");

            Ok(())
        });
    }
}

struct BuildNodeResult {
    id: BuildId,
    /// The result of the build. Only `Err` if an error on our side fails it.
    result: std::io::Result<BuildStatusKind>,
}

enum NodeInputKind {
    UpToDate,
    Outdated,
    CannotRead(PathBuf, std::io::Error),
}

/// Determine if the node is up-to-date by checking its inputs.
fn stat_node(db: &dyn ExecDb, graph: &BuildGraph, node: &BuildNode, _id: BuildId) -> NodeInputKind {
    // Get metadata of build
    let build_id = hash_build(node, graph);

    let Some(build_info) = db.get_build_info(build_id) else {
        return NodeInputKind::Outdated; // Never built before
    };

    // Check if output files are out of date
    for &out in &node.outs {
        let path = graph.lookup_path(out).expect("File should exist");
        let Some(info) = db.get_file_info(path) else {
            return NodeInputKind::Outdated; // No info means file does not exist
        };
        if info.generated_by != build_id {
            return NodeInputKind::Outdated; // Generated by different build
        }
        if let Some(last_end) = build_info.last_end
            && info.last_seen > last_end
        {
            return NodeInputKind::Outdated; // File changed since last build
        }
    }

    let mtime_should_before = build_info.last_start;
    for &file in &node.ins {
        let path = graph.lookup_path(file).expect("File should exist");
        let meta = match path.metadata() {
            Ok(meta) => meta,
            Err(e) => return NodeInputKind::CannotRead(path.to_owned(), e),
        };
        let mtime = match meta.modified() {
            Ok(t) => t,
            Err(e) => return NodeInputKind::CannotRead(path.to_owned(), e),
        };
        if mtime > mtime_should_before {
            return NodeInputKind::Outdated;
        }
    }

    NodeInputKind::UpToDate
}

/// Runs the build node
fn run_build(
    graph: &BuildGraph,
    db: &RwLock<dyn ExecDb>,
    id: BuildId,
    state: &dyn Any,
    report: mpsc::Sender<BuildNodeResult>,
) {
    let build = graph.lookup_build(id).expect("Node should exist");

    let db_read = db.read().expect("DB lock poisoned");
    let node_stat = stat_node(&*db_read, graph, build, id);
    drop(db_read);

    let result_kind = match node_stat {
        NodeInputKind::UpToDate => Ok(BuildStatusKind::UpToDate),
        NodeInputKind::CannotRead(path_buf, error) => Err(std::io::Error::other(format!(
            "Cannot read input file {path_buf:?}: {error}"
        ))),
        NodeInputKind::Outdated => {
            let cmd = &build.command;
            run_build_inner(state, cmd)
        }
    };

    report
        .send(BuildNodeResult {
            id,
            result: result_kind,
        })
        .expect("Failed to send build result");
}

fn run_build_inner(
    state: &dyn Any,
    cmd: &crate::graph::BuildMethod,
) -> Result<BuildStatusKind, std::io::Error> {
    match cmd {
        crate::graph::BuildMethod::SubCommand(build_cmd) => {
            // FIXME: n2 reports that `Command::spawn` leaks file descriptors.
            // Replace with a manual call to spawn instead.
            // See: https://github.com/rust-lang/rust/issues/95584
            let mut cmd = Command::new(&build_cmd.executable);
            cmd.args(&build_cmd.args);

            let mut child = cmd.spawn()?;
            let output = child.wait()?;

            if output.success() {
                Ok(BuildStatusKind::UpToDate)
            } else {
                Ok(BuildStatusKind::Failed)
            }
        }
        crate::graph::BuildMethod::Callback(_name, callback) => match callback(state) {
            Ok(_) => Ok(BuildStatusKind::UpToDate),
            Err(e) => {
                eprintln!("Failed to execute build step {_name}: {e}");
                Ok(BuildStatusKind::Failed)
            }
        },
        crate::graph::BuildMethod::Phony => Ok(BuildStatusKind::UpToDate),
    }
}
