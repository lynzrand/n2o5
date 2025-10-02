use std::{any::Any, path::Path, process::Command, time::SystemTime};

use crate::{exec::BuildStatusKind, graph::BuildMethod};

/// A trait that abstracts over how the executor interacts with the outside world.
///
/// All file and execution operations in the executor will be directed through
/// this trait. You may implement this trait to customize the view of the world
/// as seen by the executor. For example, a mock world can be used to test the
/// executor, or a remote world can be used to execute commands on a different
/// machine.
///
/// A default implementation is available at [`LocalWorld`].
///
/// For the database the executor uses, see [`crate::db`].
///
/// # Implementation notes
///
/// The implementation of trait will be executed in a threadpool.
pub trait World: Send + Sync {
    /// Test whether a file exists.
    fn exists(&self, path: &Path) -> bool;

    /// Get the modification time of a file.
    fn mtime(&self, path: &Path) -> std::io::Result<SystemTime>;

    /// Get the current time. Implementations may return a mocked monotonic time.
    fn now(&self) -> SystemTime;

    fn execute(&self, state: &dyn Any, cmd: &BuildMethod) -> std::io::Result<BuildStatusKind>;
}

/// The default implementation of [`World`], which interacts with the local
/// filesystem and spawns local processes.
pub struct LocalWorld;
pub static LOCAL_WORLD: LocalWorld = LocalWorld;

impl World for LocalWorld {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn mtime(&self, path: &Path) -> std::io::Result<SystemTime> {
        path.metadata()?.modified()
    }

    fn now(&self) -> SystemTime {
        SystemTime::now()
    }

    fn execute(&self, state: &dyn Any, cmd: &BuildMethod) -> std::io::Result<BuildStatusKind> {
        run_build_inner(state, cmd)
    }
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
