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
    Foundation {
        #[command(subcommand)]
        command: FoundationCommand,
    },
    Platform {
        #[command(subcommand)]
        command: PlatformCommand,
    },
    Coverage {
        #[command(subcommand)]
        command: CoverageCommand,
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
    #[command(name = "plan-issue-reconciliation")]
    PlanIssueReconciliation(IssueReconciliationArgs),
    #[command(name = "apply-issue-reconciliation")]
    ApplyIssueReconciliation(IssueReconciliationArgs),
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

#[derive(Debug, Args)]
pub struct IssueReconciliationArgs {
    #[arg(
        long,
        default_value = "target/validation/issue-reconciliation.plan.json"
    )]
    pub plan: PathBuf,
    #[arg(long)]
    pub api_fixture: Option<PathBuf>,
    #[arg(long)]
    pub specifications: Option<PathBuf>,
    #[arg(long, default_value_t = false)]
    pub confirm: bool,
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
    Compare(BenchCompareArgs),
    VerifyReceipt(BenchReceiptArgs),
    VerifyBenchmarkReceipt(BenchReceiptArgs),
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
    pub receipt: PathBuf,
}

#[derive(Debug, Args)]
pub struct BenchCompareArgs {
    /// Baseline benchmark receipt used by `bench compare`.
    #[arg(long)]
    pub baseline: PathBuf,
    /// Current benchmark receipt used by `bench compare`.
    #[arg(long)]
    pub current: PathBuf,
}

#[derive(Debug, Subcommand)]
pub enum RepoCommand {
    #[command(name = "verify-dag")]
    Dag(DagArgs),
    #[command(name = "verify-files")]
    Files(FilePolicyArgs),
    #[command(name = "verify-workflows")]
    Workflows,
    #[command(name = "verify-native-boundaries")]
    NativeBoundaries(NativeBoundariesArgs),
}

#[derive(Debug, Args)]
pub struct FilePolicyArgs {
    /// Select the immutable source of repository bytes.
    #[arg(long, value_enum, default_value_t = FileSource::Auto)]
    pub source: FileSource,
    /// Tree-ish used by --source tree.
    #[arg(long, default_value = "HEAD")]
    pub treeish: String,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum FileSource {
    Auto,
    Index,
    Tree,
    Worktree,
}

#[derive(Debug, Args)]
pub struct NativeBoundariesArgs {
    /// Write the deterministic boundary evidence receipt to this path.
    #[arg(long)]
    pub receipt: Option<PathBuf>,
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
    #[command(name = "provision")]
    Provision(ReferenceProvisionArgs),
    #[command(name = "verify-bundle")]
    VerifyBundle(ReferenceArgs),
    #[command(name = "run-qualification")]
    RunQualification(ReferenceArgs),
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
    #[arg(long, default_value_t = false)]
    pub cpu: bool,
    #[arg(long, default_value_t = 1)]
    pub repeat: u32,
}

#[derive(Debug, Args)]
pub struct ReferenceProvisionArgs {
    #[arg(long, default_value = "fixtures/reference/darktable.toml")]
    pub identity: PathBuf,
    #[arg(long)]
    pub source: PathBuf,
    #[arg(long)]
    pub executable: PathBuf,
    #[arg(long)]
    pub data_dir: PathBuf,
    #[arg(long)]
    pub source_alias: Option<PathBuf>,
    #[arg(long)]
    pub executable_alias: Option<PathBuf>,
    #[arg(long)]
    pub data_alias: Option<PathBuf>,
    #[arg(long, default_value = "5.7.0")]
    pub version: String,
    #[arg(long, default_value = "cfe57f3bbf5269bfacf31e832267279caa6938ad")]
    pub commit: String,
    #[arg(long)]
    pub build_options_hash: String,
    #[arg(long)]
    pub compiler: String,
    #[arg(long)]
    pub native_library_identity: String,
    #[arg(long, default_value = "x86_64-unknown-linux-gnu")]
    pub target: String,
    #[arg(long, default_value = "x86_64")]
    pub architecture: String,
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
    Main {
        /// Omit a merge-only parallel group run by a dedicated merge job.
        #[arg(long = "skip-group")]
        skip_groups: Vec<String>,
        /// Restrict main validation to one independent contract group.
        #[arg(long)]
        group: Option<String>,
    },
}

#[derive(Debug, Subcommand)]
pub enum GithubCommand {
    #[command(name = "verify-pr-contract")]
    VerifyPrContract(VerifyPrContractArgs),
    #[command(name = "verify-queue", alias = "queue")]
    VerifyQueue(VerifyQueueArgs),
    #[command(name = "refresh-issue-spec-snapshot")]
    RefreshIssueSpecSnapshot(IssueSpecArgs),
    #[command(name = "verify-issue-specs")]
    VerifyIssueSpecs(IssueSpecArgs),
    #[command(name = "ready-issues")]
    ReadyIssues(IssueSpecArgs),
    #[command(name = "apply-issue-spec-plan")]
    ApplyIssueSpecPlan(IssueSpecArgs),
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
pub struct IssueSpecArgs {
    #[arg(long, default_value = "architecture/issue-spec-snapshot.json")]
    pub snapshot: PathBuf,
    #[arg(long, default_value = "quality/issue-spec-policy.toml")]
    pub policy: PathBuf,
    #[arg(long)]
    pub api_fixture: Option<PathBuf>,
    #[arg(long)]
    pub reviewed_plan: Option<PathBuf>,
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
    UpgradeDiff(UpgradeDiffArgs),
    #[command(name = "refresh-baseline")]
    RefreshBaseline(RefreshBaselineArgs),
    Dependencies {
        #[command(subcommand)]
        command: DependencyCommand,
    },
    Channels {
        #[command(subcommand)]
        command: ChannelsCommand,
    },
}

#[derive(Debug, Subcommand)]
pub enum FoundationCommand {
    Verify(FoundationVerifyArgs),
}

#[derive(Debug, Subcommand)]
pub enum PlatformCommand {
    Verify(PlatformVerifyArgs),
}

#[derive(Debug, Args)]
pub struct PlatformVerifyArgs {
    #[arg(long)]
    pub all_targets: bool,
    #[arg(long)]
    pub runtime_current: bool,
    #[arg(long)]
    pub verify_startup_preflight: bool,
}

#[derive(Debug, Subcommand)]
pub enum CoverageCommand {
    Run(CoverageRunArgs),
    Verify(CoverageVerifyArgs),
    Summarize(CoverageSummarizeArgs),
}

#[derive(Debug, Args)]
pub struct CoverageRunArgs {
    #[arg(long, default_value = "target/coverage")]
    pub output_dir: PathBuf,
    #[arg(long, default_value = "quality/coverage.toml")]
    pub policy: PathBuf,
}

#[derive(Debug, Args)]
pub struct CoverageVerifyArgs {
    #[arg(long, default_value = "target/coverage/coverage.json")]
    pub report: PathBuf,
    #[arg(long, default_value = "target/coverage/coverage.lcov")]
    pub lcov: PathBuf,
    #[arg(long, default_value = "quality/coverage.toml")]
    pub policy: PathBuf,
}

#[derive(Debug, Args)]
pub struct CoverageSummarizeArgs {
    #[arg(long, default_value = "target/coverage/coverage.lcov")]
    pub lcov: PathBuf,
    #[arg(long, default_value = "target/coverage/coverage.json")]
    pub output: PathBuf,
    #[arg(long, default_value = "quality/coverage.toml")]
    pub policy: PathBuf,
}

#[derive(Debug, Args)]
pub struct FoundationVerifyArgs {
    #[arg(long, value_enum, default_value_t = FoundationMode::Pr)]
    pub mode: FoundationMode,
    #[arg(long, default_value = "quality/foundation-gate.toml")]
    pub contract: PathBuf,
    #[arg(long, default_value = "target/validation/foundation")]
    pub receipts: PathBuf,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq, ValueEnum)]
pub enum FoundationMode {
    Pr,
    Merge,
}

#[derive(Debug, Args)]
pub struct BaselineVerifyArgs {
    #[arg(long)]
    pub receipt: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct UpgradeDiffArgs {
    #[arg(long)]
    pub candidate: PathBuf,
}

#[derive(Debug, Args)]
pub struct RefreshBaselineArgs {
    #[arg(long)]
    pub candidate: PathBuf,
}

#[derive(Debug, Subcommand)]
pub enum DependencyCommand {
    #[command(name = "verify-policy")]
    VerifyPolicy,
    #[command(name = "vendor-closure")]
    VendorClosure(OfflineClosureArgs),
    #[command(name = "verify-offline")]
    VerifyOffline(OfflineClosureArgs),
}

#[derive(Debug, Args)]
pub struct OfflineClosureArgs {
    #[arg(long)]
    pub receipt: Option<PathBuf>,
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
