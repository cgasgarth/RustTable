#![forbid(unsafe_code)]

mod cli;
mod commands;
mod output;
mod process;
mod root;

use std::process::ExitCode;

use clap::Parser;
use cli::Cli;
use output::Output;

fn main() -> ExitCode {
    let cli = Cli::parse();
    let output = Output::new(cli.format);
    match commands::run(&cli) {
        Ok(report) => {
            if let Err(error) = output.emit(&report) {
                eprintln!("xtask: {error}");
                return ExitCode::FAILURE;
            }
            ExitCode::SUCCESS
        }
        Err(error) => {
            output.emit_error(&error.to_string());
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}
