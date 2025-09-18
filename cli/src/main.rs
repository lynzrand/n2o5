use clap::Parser;

use crate::cli::{Args, NinjaSubcommand};

mod cli;
mod ninja;

fn main() {
    let argv0 = std::env::args().next();
    if let Some(v) = argv0
        && v.starts_with("ninja")
    {
        let argv = NinjaSubcommand::parse();
        ninja::run(&argv);
    } else {
        let argv = Args::parse();
        match argv.subcommand {
            cli::Subcommand::Ninja(ninja_subcommand) => ninja::run(&ninja_subcommand),
        }
    }
}
