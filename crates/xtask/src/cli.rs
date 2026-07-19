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
    Github {
        #[command(subcommand)]
        command: GithubCommand,
    },
    #[command(name = "lua-conformance")]
    LuaConformance(LuaConformanceArgs),
    Ecosystem {
        #[command(subcommand)]
        command: EcosystemCommand,
    },
    #[command(name = "extension-conformance")]
    ExtensionConformance(ExtensionConformanceArgs),
    #[command(name = "template-matrix")]
    TemplateMatrix(TemplateMatrixArgs),
    #[command(name = "ui-shell")]
    UiShell(UiShellArgs),
}

#[derive(Debug, Args)]
pub struct UiShellArgs {
    #[arg(long, default_value = "all")]
    pub presets: String,
    #[arg(long)]
    pub verify_a11y: bool,
    #[arg(long)]
    pub verify_window_lifecycle: bool,
}

#[derive(Debug, Args)]
pub struct ExtensionConformanceArgs {
    #[arg(long)]
    pub all_fixtures: bool,
    #[arg(long)]
    pub verify_isolation: bool,
    #[arg(long)]
    pub verify_limits: bool,
}

#[derive(Debug, Subcommand)]
pub enum ParityCommand {
    ScanDarktable(ParityScanArgs),
    Verify,
}

#[derive(Debug, Args)]
pub struct ParityScanArgs {
    #[arg(long, default_value = "fixtures/reference/darktable.toml")]
    pub identity: PathBuf,
    #[arg(long)]
    pub source: Option<PathBuf>,
    #[arg(long)]
    pub executable: Option<PathBuf>,
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
    #[arg(long, default_value = "architecture/capability-overrides.toml")]
    pub overrides: PathBuf,
    #[arg(long, default_value = "architecture/darktable-capabilities.toml")]
    pub output: PathBuf,
    #[arg(
        long,
        default_value = "architecture/darktable-capabilities.receipt.toml"
    )]
    pub receipt: PathBuf,
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
    Dag(DagArgs),
    #[command(name = "verify-files")]
    Files,
    #[command(name = "verify-workflows")]
    Workflows,
}

#[derive(Debug, Args)]
pub struct DagArgs {
    /// Write the deterministic, bounded verification artifact to this path.
    #[arg(long)]
    pub artifact: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum ReferenceCommand {
    Probe(ReferenceArgs),
    Render(ReferenceArgs),
}

#[derive(Debug, Args)]
pub struct ReferenceArgs {
    #[arg(long, default_value = "fixtures/reference/darktable.toml")]
    pub identity: PathBuf,
    #[arg(long)]
    pub source: Option<PathBuf>,
    #[arg(long)]
    pub executable: Option<PathBuf>,
    #[arg(long)]
    pub data_dir: Option<PathBuf>,
    #[arg(long)]
    pub input: Option<PathBuf>,
    #[arg(long)]
    pub xmp: Option<PathBuf>,
    #[arg(long, default_value = "reference.fixture")]
    pub fixture_id: String,
    #[arg(long, default_value_t = 1)]
    pub width: u32,
    #[arg(long, default_value_t = 1)]
    pub height: u32,
    #[arg(long, default_value_t = false)]
    pub gpu: bool,
}

#[derive(Debug, Subcommand)]
pub enum CiCommand {
    Precommit,
    Prepush,
    Pr {
        /// Restrict pull-request validation to one independent contract group.
        #[arg(long)]
        group: Option<String>,
    },
    Main,
}

#[derive(Debug, Subcommand)]
pub enum GithubCommand {
    #[command(name = "verify-pr-contract")]
    VerifyPrContract(VerifyPrContractArgs),
    #[command(name = "verify-queue", alias = "queue")]
    VerifyQueue(VerifyQueueArgs),
}

#[derive(Debug, Args)]
pub struct VerifyPrContractArgs {
    #[arg(long)]
    pub event: PathBuf,
    #[arg(long)]
    pub api_fixture: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct TemplateMatrixArgs {
    #[arg(long)]
    pub all_builtins: bool,
    #[arg(long)]
    pub all_platforms: bool,
    #[arg(long)]
    pub verify_privacy: bool,
    #[arg(long, default_value_t = 1)]
    pub repeat: usize,
}

#[derive(Debug, Args)]
pub struct VerifyQueueArgs {
    #[arg(long)]
    pub api_fixture: Option<PathBuf>,
}

#[derive(Debug, Args)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "These flags are the issue's required independent conformance checks."
)]
pub struct LuaConformanceArgs {
    #[arg(long, default_value_t = false)]
    pub all_fixtures: bool,
    #[arg(long, default_value_t = false)]
    pub verify_isolation: bool,
    #[arg(long, default_value_t = false)]
    pub verify_limits: bool,
    #[arg(long, default_value_t = false)]
    pub verify_events: bool,
}

#[derive(Debug, Subcommand)]
pub enum EcosystemCommand {
    #[command(name = "verify-baseline")]
    VerifyBaseline(BaselineVerifyArgs),
    #[command(name = "upgrade-diff")]
    UpgradeDiff,
    Dependencies {
        #[command(subcommand)]
        command: DependencyCommand,
    },
    Channels {
        #[command(subcommand)]
        command: ChannelsCommand,
    },
}

#[derive(Debug, Args)]
pub struct BaselineVerifyArgs {
    #[arg(long)]
    pub receipt: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
pub enum DependencyCommand {
    #[command(name = "verify-policy")]
    VerifyPolicy,
}

#[derive(Debug, Subcommand)]
pub enum ChannelsCommand {
    Verify(ChannelVerifyArgs),
    Refresh(ChannelRefreshArgs),
}

#[derive(Debug, Args)]
pub struct ChannelVerifyArgs {
    #[arg(long = "channel")]
    pub channels: Vec<String>,
    #[arg(long)]
    pub receipt: Option<PathBuf>,
    #[arg(long)]
    pub artifact: bool,
}

#[derive(Debug, Args)]
pub struct ChannelRefreshArgs {
    #[arg(long)]
    pub receipt: Option<PathBuf>,
}
