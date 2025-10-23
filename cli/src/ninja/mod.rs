pub mod convert;
pub mod model;
pub mod parser;
pub mod run;
mod tokenizer;

use crate::{cli::NinjaSubcommand, ninja::parser::ParseSource};

use anyhow::{Context, anyhow};
use n2o5::exec::{ExecConfig, Executor};
use n2o5::progress::fancy::FancyConsoleProgress;
use n2o5_redb::ExecRedb;

static NINJA_DEFAULT_FILENAME: &str = "build.ninja";
static NINJA_DB_FILENAME: &str = "n2o5_ninja.db";

pub fn run(cmd: &NinjaSubcommand) -> anyhow::Result<()> {
    assert!(!cmd.quiet, "Quiet mode not yet implemented");
    assert!(!cmd.dry_run, "Dry-run mode not yet implemented");

    // Change working directory if requested
    if let Some(path) = &cmd.chdir {
        std::env::set_current_dir(path).context("failed to change directory")?;
    }

    // Parse Ninja file
    let parse_source = ParseSource::new(NINJA_DEFAULT_FILENAME);
    let parsed = parser::parse(&parse_source, parse_source.main_file())
        .context("Failed to parse the ninja build file")?;

    // Convert to n2o5 graph
    let converted = convert::ninja_to_n2o5(&parsed)?;
    let db = ExecRedb::open(NINJA_DB_FILENAME)
        .context("Failed to open or create the n2o5_ninja.db database file")?;

    // Map jobs -> parallelism; default to available parallelism
    let parallelism = match cmd.jobs {
        Some(n) if n > 0 => n,
        _ => std::thread::available_parallelism()
            .map(|nz| nz.get())
            .unwrap_or(1),
    };
    let cfg = ExecConfig { parallelism };

    // Build executor
    let progress = FancyConsoleProgress::new();
    let mut exec = Executor::new(&cfg, &converted.graph, &db, &progress, &());

    // Resolve targets (skip dry-run; we always run)
    let wanted = run::resolve_targets_to_build_ids(&cmd.targets, &parsed, &converted);
    if wanted.is_empty() && !cmd.targets.is_empty() {
        // Explicit targets provided but no matching builds
        return Err(anyhow!("No matching builds for targets: {:?}", cmd.targets));
    }
    exec.want(wanted);

    // Execute
    exec.run().context("Executor run failed")?;

    Ok(())
}
