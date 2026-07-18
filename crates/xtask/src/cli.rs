use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormat {
    Human,
    Json,
}

#[derive(Debug, Parser)]
#[command(
    name = "cargo xtask",
    version,
    about = "RustTable repository automation"
)]
pub struct Cli {
    #[arg(long, global = true, value_enum, default_value_t = OutputFormat::Human)]
    pub format: OutputFormat,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Parity {
        #[command(subcommand)]
        command: ParityCommand,
    },
    Fixtures {
        #[command(subcommand)]
        command: FixturesCommand,
    },
    Bench {
        #[command(subcommand)]
        command: BenchCommand,
    },
    Repo {
        #[command(subcommand)]
        command: RepoCommand,
    },
    Reference {
        #[command(subcommand)]
        command: ReferenceCommand,
    },
    Ci {
        #[command(subcommand)]
        command: CiCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum ParityCommand {
    ScanDarktable(ParityScanArgs),
    Verify,
}

#[derive(Debug, Args)]
pub struct ParityScanArgs {
    #[arg(long)]
    pub source: PathBuf,
    #[arg(long, default_value = "architecture/capability-overrides.toml")]
    pub overrides: PathBuf,
    #[arg(long, default_value = "architecture/darktable-capabilities.toml")]
    pub output: PathBuf,
}

#[derive(Debug, Subcommand)]
pub enum FixturesCommand {
    Verify(FixtureArgs),
    List(FixtureArgs),
    ScrubReport(FixtureArgs),
}

#[derive(Debug, Args)]
pub struct FixtureArgs {
    #[arg(long, default_value = "fixtures/manifest.toml")]
    pub manifest: PathBuf,
}

#[derive(Debug, Subcommand)]
pub enum BenchCommand {
    Run(BenchRunArgs),
    Compare(BenchReceiptArgs),
    VerifyReceipt(BenchReceiptArgs),
}

#[derive(Debug, Args)]
pub struct BenchRunArgs {
    #[arg(long, default_value_t = false)]
    pub check: bool,
    #[arg(long)]
    pub receipt: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct BenchReceiptArgs {
    #[arg(long)]
    pub receipt: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum RepoCommand {
    #[command(name = "verify-dag")]
    Dag,
    #[command(name = "verify-files")]
    Files,
    #[command(name = "verify-workflows")]
    Workflows,
}

#[derive(Debug, Subcommand)]
pub enum ReferenceCommand {
    Probe(ReferenceArgs),
    Render(ReferenceArgs),
}

#[derive(Debug, Args)]
pub struct ReferenceArgs {
    #[arg(long)]
    pub executable: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum CiCommand {
    Precommit,
    Prepush,
    Pr,
    Main,
}
