use std::fs;
use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};

use crate::Result;

#[derive(Debug, Subcommand)]
pub(crate) enum FixturesCommand {
    /// Verify every registered fixture byte and qualification rule.
    Verify(FixturesArgs),
}

#[derive(Debug, Args)]
pub(crate) struct FixturesArgs {
    #[arg(long, default_value = "fixtures/manifest.toml")]
    manifest: PathBuf,
}

pub(crate) fn run(root: &Path, command: FixturesCommand) -> Result {
    match command {
        FixturesCommand::Verify(arguments) => verify(root, &arguments.manifest),
    }
}

pub(crate) fn verify(root: &Path, manifest: &Path) -> Result {
    let path = root.join(manifest);
    let source =
        fs::read_to_string(&path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let manifest = rusttable_testkit::fixtures::FixtureManifest::parse(&source)
        .map_err(|error| error.to_string())?;
    let repository = rusttable_testkit::fixtures::FixtureRepository::new(root, manifest)
        .map_err(|error| error.to_string())?;
    let _verified = repository.verify().map_err(|error| error.to_string())?;
    Ok(())
}
