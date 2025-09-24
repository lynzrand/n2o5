//! Build graph execution

use std::{
    any::Any,
    collections::{HashMap, HashSet},
    path::PathBuf,
    process::Command,
    sync::{Arc, mpsc},
    time::SystemTime,
};

use rayon::Scope;
use tracing::{info, warn};

use crate::{
    db::{BuildHash, BuildInfo, ExecDb, InputHash},
    graph::{BuildGraph, BuildId, BuildNode, FileId, hash_build, hash_input_set},
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
enum BuildStatusKind {
    /// The build hasn't been checked yet
    Fresh,
    /// The build has been started
    Started,
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

/// Some internal shared state that is passed to each build task.
struct SharedState<'a> {
    cfg: &'a ExecConfig,
    graph: &'a BuildGraph,
    db: Box<dyn ExecDb>,
    pool: rayon::ThreadPool,
    user_state: &'a (dyn Any + Send + Sync),
}

/// The executor that runs a build graph.
pub struct Executor<'a> {
    state: Arc<SharedState<'a>>,

    /// The starting nodes that have yet to be executed for the build
    starts: HashSet<BuildId>,
    /// The current status of each build node
    builds: HashMap<BuildId, BuildStatus>,
    started: usize,
    finished: usize,
    failed: usize,

    build_started: bool,
}

impl<'a> Executor<'a> {
    pub fn new(
        cfg: &'a ExecConfig,
        graph: &'a BuildGraph,
        db: Box<dyn ExecDb>,
        user_state: &'a (dyn Any + Send + Sync),
    ) -> Self {
        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(cfg.parallelism)
            .build()
            .unwrap();

        let state = SharedState {
            cfg,
            graph,
            db,
            pool,
            user_state,
        };
        Self {
            state: Arc::new(state),

            starts: Default::default(),
            builds: Default::default(),
            started: 0,
            finished: 0,
            failed: 0,

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
            for node in self.state.graph.build_dependencies(build) {
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
    pub fn run(&mut self) -> Result<(), std::io::Error> {
        self.build_started = true;

        let state = self.state.clone();
        state.pool.in_place_scope(|pool| self.run_inner(pool))?;

        Ok(())
    }

    fn run_inner<'scope>(&mut self, pool: &Scope<'scope>) -> Result<(), std::io::Error>
    where
        'a: 'scope,
    {
        // TODO: should we prevent it from running more than once?
        let (tx, rx) = mpsc::channel::<BuildNodeResult>();

        // Starting nodes
        for node in self.starts.iter().cloned().collect::<Vec<_>>() {
            self.start_build(pool, tx.clone(), node);
        }

        while let Ok(msg) = rx.recv() {
            let id = msg.id;
            let stat = match msg.result {
                Ok(res) => res,
                Err(e) => {
                    warn!("Our build executor has encountered a problem: {e}");
                    return Err(e);
                }
            };
            if !stat.is_finished() {
                panic!(
                    "Build {:?} returned non-finished status {:?}. This is a bug.",
                    msg.id, stat
                );
            }

            self.finished += 1;

            let build = self.builds.get_mut(&msg.id).expect("Build should exist");

            if build.kind.is_finished() {
                panic!(
                    "Build {:?} has already finished with status {:?}, cannot finish again with {:?}. This is a bug.",
                    msg.id, build.kind, stat
                );
            }
            build.kind = stat;

            // Drive the state machine forward.
            //
            // - If a build is successful (succeeded or up-to-date), it
            //   counts as a valid input of its dependent builds. This
            //   increments the `n_inputs_finished` count of the dependent
            //   builds.
            //
            //   If the number of finished inputs reaches the total number
            //   of inputs, the dependent build can be started by spawning
            //   a new task.
            //
            // - If a build fails, it is considered finished, but does not
            //   count as a valid input. It will send `Skipped` to all
            //   dependent builds, which will propagate the failure
            //   downstream.
            //
            // - If a build is skipped, it propagates the skip to all
            //   dependent builds.
            match stat {
                BuildStatusKind::Fresh => panic!("Build cannot be fresh after running"),
                BuildStatusKind::Started => panic!("Build cannot be started after running"),
                BuildStatusKind::Succeeded | BuildStatusKind::UpToDate => {
                    for node in self.state.graph.build_dependents(id) {
                        let dep = self.builds.get_mut(&node).expect("Build should exist");
                        dep.n_inputs_finished += 1;
                        if dep.n_inputs == dep.n_inputs_finished {
                            // All inputs finished, can start build
                            self.started += 1;
                            self.start_build(pool, tx.clone(), node);
                        }
                    }
                }
                BuildStatusKind::Failed | BuildStatusKind::Skipped => {
                    self.failed += 1;
                    for node in self.state.graph.build_dependents(id) {
                        tx.send(BuildNodeResult {
                            id: node,
                            result: Ok(BuildStatusKind::Skipped),
                        })
                        .expect("Failed to send build result");
                    }
                }
            }

            // Avoid infinite waiting while tx is not dropped, detect
            // bailout conditions.
            if self.finished == self.started {
                info!("All builds finished");
                break;
            }
            if self.failed > 0 {
                warn!("Build failed, aborting remaining builds");
                break;
            }
        }

        Ok(())
    }

    fn start_build<'scope>(
        &mut self,
        pool: &Scope<'scope>,
        tx: mpsc::Sender<BuildNodeResult>,
        node: BuildId,
    ) where
        'a: 'scope,
    {
        let state = self.state.clone();
        self.builds.get_mut(&node).expect("Build should exist").kind = BuildStatusKind::Started;
        pool.spawn(move |_p| run_build(state, node, tx));
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
    Missing(FileId),
    CannotRead(PathBuf, std::io::Error),
}

/// Determine if the node is up-to-date by checking its inputs.
#[tracing::instrument(skip_all)]
fn stat_node(
    db: &dyn ExecDb,
    graph: &BuildGraph,
    node: &BuildNode,
    build_hash: BuildHash,
    input_hash: InputHash,
) -> NodeInputKind {
    let txn = db.begin_read();

    // Get metadata of build
    let Some(build_info) = txn.get_build_info(build_hash) else {
        tracing::debug!("Outdated: no build info for build {build_hash:?}");
        return NodeInputKind::Outdated; // Never built before
    };

    // Check if output files are out of date
    for &out in &node.outs {
        let path = graph.lookup_path(out).expect("File should exist");
        let Some(info) = txn.get_file_info(path) else {
            tracing::debug!("Outdated: File {path:?} has no info in DB");
            return NodeInputKind::Outdated;
        };
        if info.generated_by != build_hash {
            tracing::debug!(
                "Outdated: File {path:?} was generated by {:?}, expected {:?}",
                info.generated_by,
                build_hash
            );
            return NodeInputKind::Outdated;
        }
        if let Some(last_end) = build_info.last_end
            && info.last_seen > last_end
        {
            tracing::debug!(
                "Outdated: File {path:?} last_seen {:?} is after build last_end {:?}",
                info.last_seen,
                last_end
            );
            return NodeInputKind::Outdated; // File changed since last build
        }
    }

    drop(txn);

    // Validate input set equivalence
    if build_info.input_set_digest != input_hash {
        tracing::debug!(
            "Outdated: input set digest changed (was {:?}, now {:?})",
            build_info.input_set_digest,
            input_hash
        );
        return NodeInputKind::Outdated; // Input set changed
    }

    // Check if input files are up-to-date
    //
    // The modify time should be before the start time of the last build run,
    // i.e. they should not be modified after the build started.
    let mtime_should_before = build_info.last_start;
    for &file in &node.ins {
        let path = graph.lookup_path(file).expect("File should exist");
        if !path.exists() {
            tracing::debug!("Outdated: input file {path:?} does not exist");
            return NodeInputKind::Missing(file);
        }
        let mtime = match mtime(path) {
            Ok(value) => value,
            Err(value) => return value,
        };
        if mtime > mtime_should_before {
            tracing::debug!(
                "Outdated: input file {path:?} modified at {:?} after build last_start {:?}",
                mtime,
                mtime_should_before
            );
            return NodeInputKind::Outdated;
        }
    }
    // Check additional inputs
    // Note: we don't know these files beforehand, so we can't include them
    // in the DB.
    for file in &build_info.additional_inputs {
        // If the file itself is missing, it might be because other aspects of
        // the build command have changed. This is not a hard error, unlike the
        // fixed input files. We simply mark it as outdated.
        if !file.exists() {
            tracing::debug!("Outdated: additional input file {file:?} does not exist");
            return NodeInputKind::Outdated;
        }
        let mtime = match mtime(file) {
            Ok(value) => value,
            Err(value) => return value,
        };
        if mtime > mtime_should_before {
            tracing::debug!(
                "Outdated: additional input file {file:?} modified at {:?} after build last_start {:?}",
                mtime,
                mtime_should_before
            );
            return NodeInputKind::Outdated;
        }
    }

    tracing::debug!("Up-to-date: build {build_hash:?} is up-to-date");
    NodeInputKind::UpToDate
}

fn mtime(path: &PathBuf) -> Result<SystemTime, NodeInputKind> {
    let meta = path
        .metadata()
        .map_err(|e| NodeInputKind::CannotRead(path.to_owned(), e))?;
    let mtime = meta
        .modified()
        .map_err(|e| NodeInputKind::CannotRead(path.to_owned(), e))?;
    Ok(mtime)
}

#[tracing::instrument(skip_all)]
fn write_build(
    db: &dyn ExecDb,
    graph: &BuildGraph,
    build: &BuildNode,
    build_hash: BuildHash,
    input_hash: InputHash,
) {
    let mut txn = db.begin_write();

    let now = SystemTime::now();

    // Write build info
    let build_info = BuildInfo {
        last_start: now,
        last_end: None,
        input_set_digest: input_hash,
        additional_inputs: vec![], // TODO: detect such inputs
    };
    txn.set_build_info(build_hash, build_info);

    // Write info for outputs
    for &out in &build.outs {
        let path = graph.lookup_path(out).expect("File should exist");
        let file_info = crate::db::FileInfo {
            last_seen: now,
            generated_by: build_hash,
        };
        txn.set_file_info(path, file_info);
    }

    txn.commit();
}

fn invalidate_build(db: &dyn ExecDb, graph: &BuildGraph, build: &BuildNode, build_hash: BuildHash) {
    let mut txn = db.begin_write();

    // Invalidate build info
    txn.invalidate_build(build_hash);

    // Invalidate info for outputs
    for &out in &build.outs {
        let path = graph.lookup_path(out).expect("File should exist");
        txn.invalidate_file(path);
    }

    txn.commit();
}

/// Runs the build node
fn run_build(state: Arc<SharedState<'_>>, id: BuildId, report: mpsc::Sender<BuildNodeResult>) {
    let graph = state.graph;
    let db = &*state.db;

    let build = graph.lookup_build(id).expect("Node should exist");

    let span = tracing::info_span!("run_build", ?id, ?build);
    let _guard = span.enter();

    let build_id = hash_build(build, graph);
    let input_hash = hash_input_set(id, graph);

    let node_stat = stat_node(db, graph, build, build_id, input_hash);

    let result_kind = match node_stat {
        NodeInputKind::UpToDate => Ok(BuildStatusKind::UpToDate),
        NodeInputKind::CannotRead(path_buf, error) => Err(std::io::Error::other(format!(
            "Cannot read input file {path_buf:?}: {error}"
        ))),
        NodeInputKind::Missing(_id) => {
            Ok(BuildStatusKind::Failed) // TODO: report missing file
        }
        NodeInputKind::Outdated => {
            let cmd = &build.command;
            let build_result = run_build_inner(state.user_state, cmd);
            match &build_result {
                Ok(BuildStatusKind::Succeeded) => {
                    write_build(db, graph, build, build_id, input_hash);
                }
                Ok(BuildStatusKind::UpToDate) => {
                    // This should not happen, but we allow it.
                    warn!(
                        "Build {:?} returned UpToDate when it was Outdated. This is unexpected.",
                        id
                    );
                    write_build(db, graph, build, build_id, input_hash);
                }
                Ok(BuildStatusKind::Failed) | Err(_) => {
                    invalidate_build(db, graph, build, build_id);
                }
                Ok(other) => {
                    panic!(
                        "Build {:?} returned unexpected status {:?}. This is a bug.",
                        id, other
                    );
                }
            }
            build_result
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
                Ok(BuildStatusKind::Succeeded)
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
        crate::graph::BuildMethod::Phony => Ok(BuildStatusKind::Succeeded),
    }
}
