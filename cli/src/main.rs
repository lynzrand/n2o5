use clap::Parser;

use crate::cli::{Args, NinjaSubcommand};

mod cli;

fn main() {
    let argv0 = std::env::args().next();
    if let Some(v) = argv0
        && v.starts_with("ninja")
    {
        let argv = NinjaSubcommand::parse();
        run_ninja(&argv);
    } else {
        let argv = Args::parse();
        match argv.subcommand {
            cli::Subcommand::Ninja(ninja_subcommand) => run_ninja(&ninja_subcommand),
        }
    }
}

fn run_ninja(cmd: &NinjaSubcommand) {}
