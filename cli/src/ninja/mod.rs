pub mod model;
pub mod parser;
mod tokenizer;

use crate::cli::NinjaSubcommand;

use anyhow::Context;

static NINJA_DEFAULT_FILENAME: &str = "build.ninja";

pub fn run(cmd: &NinjaSubcommand) -> anyhow::Result<()> {
    if let Some(path) = &cmd.chdir {
        std::env::set_current_dir(path).context("failed to change directory")?;
    }

    let file =
        std::fs::read_to_string(NINJA_DEFAULT_FILENAME).context("failed to read ninja file")?;
    let parsed = parser::parse(&file)?;

    // TODO: translate to n2o4 graph
    println!("{:#?}", parsed);

    Ok(())
}
