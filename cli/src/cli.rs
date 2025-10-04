use std::path::PathBuf;

#[derive(Debug, clap::Parser)]
#[clap(name = "n2o5", version, author)]
pub struct Args {
    #[command(subcommand)]
    pub subcommand: Subcommand,
}

#[derive(Debug, clap::Subcommand)]
pub enum Subcommand {
    Ninja(Box<NinjaSubcommand>),
}

#[derive(Debug, clap::Parser)]
pub struct NinjaSubcommand {
    /// The targets to build
    pub targets: Vec<String>,

    /// Change to DIR before doing anything else
    #[clap(short = 'C', name = "DIR")]
    pub chdir: Option<PathBuf>,

    /// Show all command lines while building
    #[clap(short, long)]
    pub verbose: bool,

    /// Don't show progress status, just command output
    #[clap(long)]
    pub quiet: bool,

    /// Run N jobs in parallel (N > 0; default: number of CPU cores)
    #[clap(short, long, name = "N")]
    pub jobs: Option<usize>,

    /// Dry run (don't commands but act like they succeeded)
    #[clap(short = 'n', long)]
    pub dry_run: bool,
}
