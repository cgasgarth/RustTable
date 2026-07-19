mod bench;
mod channels;
mod ci;
mod coverage;
mod dag;
mod dependencies;
mod ecosystem;
mod extension_conformance;
mod file_source;
mod files;
mod fixtures;
mod foundation;
mod github;
mod github_reconciliation;
mod issue_spec;
mod lua;
mod native_boundaries;
mod offline_closure;
mod parity;
mod platform;
mod reference;
mod repo;
mod source_map;
mod template_matrix;
mod ui_shell;

use std::fmt;

use crate::cli::{Cli, Command, DependencyCommand, EcosystemCommand};
use crate::output::Report;
use crate::process::{ProcessError, ProcessRunner};
use crate::root::{RepositoryRoot, RootError};

pub(crate) type Result<T = Report> = std::result::Result<T, String>;

pub fn run(cli: &Cli) -> std::result::Result<Report, CommandError> {
    let runner = ProcessRunner::new();
    let root = RepositoryRoot::discover(&runner).map_err(CommandError::Root)?;
    match &cli.command {
        Command::Parity { command } => parity::run(&root, command, &runner),
        Command::Fixtures { command } => fixtures::run(&root, command),
        Command::Bench { command } => bench::run(&root, command, &runner),
        Command::Repo { command } => repo::run(&root, command, &runner),
        Command::Reference { command } => reference::run(&root, command, &runner),
        Command::Ci { command } => ci::run(&root, command, &runner),
        Command::Coverage { command } => coverage::run(&root, command, &runner),
        Command::Github { command } => match command {
            crate::cli::GithubCommand::RefreshIssueSpecSnapshot(arguments)
            | crate::cli::GithubCommand::VerifyIssueSpecs(arguments)
            | crate::cli::GithubCommand::ReadyIssues(arguments)
            | crate::cli::GithubCommand::ApplyIssueSpecPlan(arguments) => {
                issue_spec::run(&root, command, arguments, &runner)
            }
            _ => github::run(&root, command, &runner),
        },
        Command::LuaConformance(arguments) => lua::run(&root, arguments),
        Command::Ecosystem { command } => match command {
            EcosystemCommand::VerifyBaseline(arguments) => {
                ecosystem::verify_baseline(&root, arguments, &runner)
            }
            EcosystemCommand::UpgradeDiff(arguments) => ecosystem::upgrade_diff(&root, arguments),
            EcosystemCommand::RefreshBaseline(arguments) => {
                ecosystem::refresh_baseline(&root, arguments, &runner)
            }
            EcosystemCommand::Dependencies { command } => match command {
                DependencyCommand::VerifyPolicy => dependencies::verify_policy(&root, &runner),
                DependencyCommand::VendorClosure(arguments) => {
                    offline_closure::vendor_closure(&root, arguments, &runner)
                }
                DependencyCommand::VerifyOffline(arguments) => {
                    offline_closure::verify_offline(&root, arguments, &runner)
                }
            },
            EcosystemCommand::Channels { command } => channels::run(&root, command, &runner),
        },
        Command::Foundation { command } => foundation::run(&root, command, &runner),
        Command::Platform { command } => platform::run(&root, command),
        Command::ExtensionConformance(arguments) => extension_conformance::run(&root, arguments),
        Command::TemplateMatrix(args) => template_matrix::run(&root, args),
        Command::UiShell(args) => ui_shell::run(&root, args),
        Command::Migration { command } => source_map::run(&root, command, &runner),
    }
    .map_err(CommandError::Surface)
}

pub(crate) fn report(root: &RepositoryRoot, command: &str, data: serde_json::Value) -> Report {
    let mut object = match data {
        serde_json::Value::Object(object) => object,
        value => serde_json::Map::from_iter([(String::from("result"), value)]),
    };
    object.insert(
        "repository_root".to_owned(),
        serde_json::Value::String(root.path().display().to_string()),
    );
    Report::new(command, serde_json::Value::Object(object))
}

#[derive(Debug)]
pub enum CommandError {
    Root(RootError),
    Process(ProcessError),
    Surface(String),
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root(error) => error.fmt(formatter),
            Self::Process(error) => error.fmt(formatter),
            Self::Surface(message) => formatter.write_str(message),
        }
    }
}

impl std::error::Error for CommandError {}

impl From<ProcessError> for CommandError {
    fn from(error: ProcessError) -> Self {
        Self::Process(error)
    }
}
