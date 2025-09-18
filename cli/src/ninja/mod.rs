mod parse;

use crate::cli::NinjaSubcommand;

pub fn run(cmd: &NinjaSubcommand) {
    if let Some(path) = &cmd.chdir {
        std::env::set_current_dir(path).expect_err("failed to change directory");
    }
}
