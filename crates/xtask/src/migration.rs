use clap::Subcommand;
use std::path::Path;

use crate::{Result, color};

#[derive(Debug, Subcommand)]
pub(crate) enum MigrationCommand {
    SourceMap {
        #[command(subcommand)]
        command: SourceMapCommand,
    },
}

#[derive(Debug, Subcommand)]
pub(crate) enum SourceMapCommand {
    Verify {
        #[arg(long)]
        issue: i64,
    },
}

pub(crate) fn run(root: &Path, command: MigrationCommand) -> Result {
    match command {
        MigrationCommand::SourceMap { command } => match command {
            SourceMapCommand::Verify { issue } if issue == 257 => {
                color::verify_source_map(root, issue)?;
                eprintln!("migration source map passed (issue={issue})");
                Ok(())
            }
            SourceMapCommand::Verify { issue } => Err(format!(
                "migration source map: no source-map verifier for issue {issue}"
            )),
        },
    }
}
