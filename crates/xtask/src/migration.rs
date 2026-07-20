use clap::Subcommand;
use std::path::Path;

use crate::{
    Result, color, gpu, memory, operations, organization, pixelpipe, pixelpipe_cancellation,
    pixelpipe_mode, pixelpipe_scheduler, shaders,
};

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
            SourceMapCommand::Verify { issue } if issue == 290 => {
                gpu::verify_source_map(root, issue)?;
                eprintln!("migration source map passed (issue={issue})");
                Ok(())
            }
            SourceMapCommand::Verify { issue } if issue == 293 => {
                gpu::verify_resource_source_map(root, issue)?;
                gpu::verify_resource_architecture(root)?;
                eprintln!("migration source map passed (issue={issue})");
                Ok(())
            }
            SourceMapCommand::Verify { issue } if issue == 294 => {
                gpu::verify_transfer_source_map(root, issue)?;
                gpu::verify_transfer_architecture(root)?;
                eprintln!("migration source map passed (issue={issue})");
                Ok(())
            }
            SourceMapCommand::Verify { issue } if issue == 292 => {
                shaders::verify_source_map(root)?;
                eprintln!("migration source map passed (issue={issue})");
                Ok(())
            }
            SourceMapCommand::Verify { issue } if issue == 189 => {
                organization::verify_source_map(root, issue)?;
                eprintln!("migration source map passed (issue={issue})");
                Ok(())
            }
            SourceMapCommand::Verify { issue } if issue == 263 || issue == 264 => {
                operations::verify_source_map(root, issue)?;
                eprintln!("migration source map passed (issue={issue})");
                Ok(())
            }
            SourceMapCommand::Verify { issue } if issue == 265 => {
                operations::verify_registry_source_map(root)?;
                eprintln!("migration source map passed (issue={issue})");
                Ok(())
            }
            SourceMapCommand::Verify { issue } if issue == 266 => {
                pixelpipe::verify_source_map(root, issue)?;
                eprintln!("migration source map passed (issue={issue})");
                Ok(())
            }
            SourceMapCommand::Verify { issue } if issue == 267 => {
                pixelpipe::verify_roi_source_map(root)?;
                eprintln!("migration source map passed (issue={issue})");
                Ok(())
            }
            SourceMapCommand::Verify { issue } if issue == 268 => {
                pixelpipe::verify_tiling_source_map(root, issue)?;
                eprintln!("migration source map passed (issue={issue})");
                Ok(())
            }
            SourceMapCommand::Verify { issue } if issue == 269 => {
                memory::verify_source_map(root, issue)?;
                eprintln!("migration source map passed (issue={issue})");
                Ok(())
            }
            SourceMapCommand::Verify { issue } if issue == 270 => {
                pixelpipe::verify_cache_source_map(root, issue)?;
                eprintln!("migration source map passed (issue={issue})");
                Ok(())
            }
            SourceMapCommand::Verify { issue } if issue == 271 => {
                pixelpipe_mode::verify_mode_source_map(root, issue)?;
                eprintln!("migration source map passed (issue={issue})");
                Ok(())
            }
            SourceMapCommand::Verify { issue } if issue == 272 => {
                pixelpipe_cancellation::verify_source_map(root, issue)?;
                eprintln!("migration source map passed (issue={issue})");
                Ok(())
            }
            SourceMapCommand::Verify { issue } if issue == 273 => {
                pixelpipe_scheduler::verify_source_map(root, issue)?;
                eprintln!("migration source map passed (issue={issue})");
                Ok(())
            }
            SourceMapCommand::Verify { issue } => Err(format!(
                "migration source map: no source-map verifier for issue {issue}"
            )),
        },
    }
}
