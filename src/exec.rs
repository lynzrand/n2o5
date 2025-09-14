//! Build graph execution

use std::{
    any::Any,
    collections::{HashMap, HashSet},
    process::Command,
    sync::{Arc, mpsc},
};

use crate::graph::{BuildGraph, BuildId, BuildNode};

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
enum BuildStatus {
    /// The build hasn't been checked yet
    Fresh,
    /// The build is currently outdated and needs running
    Outdated,
    /// The build is up-to-date and does not need running
    UpToDate,
    /// Building has failed
    Failed,
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
    pub fn new(cfg: &'a ExecConfig, graph: &'a BuildGraph) -> Self {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(cfg.parallelism)
            .build()
            .unwrap();
        Self {
            cfg,
            graph: Arc::new(graph),
            pool,

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

            self.builds.insert(build, BuildStatus::Fresh);
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
        }

        affected_nodes
    }

    /// Perform the build.
    pub fn run(&mut self) {
        // TODO: should we prevent it from running more than once?
        let (tx, rx) = mpsc::channel();

        self.build_started = true;

        self.pool.in_place_scope(|pool| {
            // Starting nodes
            for &node in &self.starts {
                let graph = Arc::clone(&self.graph);
                let tx = tx.clone();
                pool.spawn(move |_p| {
                    run_build(
                        node,
                        graph.lookup_build(node).expect("Node should exist"),
                        &(),
                        tx,
                    )
                });
            }

            todo!("Receive and process results")
        })
    }
}

struct BuildNodeResult {
    id: BuildId,
    /// The result of the build. Only `Err` if an error on our side fails it.
    result: std::io::Result<BuildStatus>,
}

/// Runs the build node
fn run_build(
    id: BuildId,
    build: &BuildNode,
    state: &dyn Any,
    report: mpsc::Sender<BuildNodeResult>,
) {
    let cmd = &build.command;
    let res = 'res: {
        match cmd {
            crate::graph::BuildMethod::SubCommand(build_cmd) => {
                // FIXME: n2 reports that `Command::spawn` leaks file descriptors.
                // Replace with a manual call to spawn instead.
                // See: https://github.com/rust-lang/rust/issues/95584
                let mut cmd = Command::new(&build_cmd.executable);
                cmd.args(&build_cmd.args);

                let mut child = match cmd.spawn() {
                    Ok(c) => c,
                    Err(e) => break 'res Err(e),
                };
                let output = match child.wait() {
                    Ok(o) => o,
                    Err(e) => break 'res Err(e),
                };

                if output.success() {
                    Ok(BuildStatus::UpToDate)
                } else {
                    Ok(BuildStatus::Failed)
                }
            }
            crate::graph::BuildMethod::Callback(_name, callback) => match callback(state) {
                Ok(_) => Ok(BuildStatus::UpToDate),
                Err(e) => {
                    eprintln!("Failed to execute build step {_name}: {e}");
                    Ok(BuildStatus::Failed)
                }
            },
            crate::graph::BuildMethod::Phony => Ok(BuildStatus::UpToDate),
        }
    };
    report
        .send(BuildNodeResult { id, result: res })
        .expect("Failed to send build result");
}
