pub mod model;
pub mod parser;
mod tokenizer;

use crate::{cli::NinjaSubcommand, ninja::parser::ParseSource};

use anyhow::Context;

static NINJA_DEFAULT_FILENAME: &str = "build.ninja";

pub fn run(cmd: &NinjaSubcommand) -> anyhow::Result<()> {
    if let Some(path) = &cmd.chdir {
        std::env::set_current_dir(path).context("failed to change directory")?;
    }

    let parse_source = ParseSource::new(NINJA_DEFAULT_FILENAME);
    let parsed = parser::parse(&parse_source, parse_source.main_file())?;

    // TODO: translate to n2o4 graph
    println!("{:#?}", parsed);

    Ok(())
}
