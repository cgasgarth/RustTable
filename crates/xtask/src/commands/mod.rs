mod bench;
mod ci;
mod fixtures;
mod parity;
mod reference;
mod repo;

use std::fmt;

use crate::cli::{Cli, Command};
use crate::output::Report;
use crate::process::{ProcessError, ProcessRunner};
use crate::root::{RepositoryRoot, RootError};

pub(crate) type Result<T = Report> = std::result::Result<T, String>;

pub fn run(cli: &Cli) -> std::result::Result<Report, CommandError> {
    let runner = ProcessRunner::new();
    let root = RepositoryRoot::discover(&runner).map_err(CommandError::Root)?;
    match &cli.command {
        Command::Parity { command } => parity::run(&root, command),
        Command::Fixtures { command } => fixtures::run(&root, command),
        Command::Bench { command } => bench::run(&root, command, &runner),
        Command::Repo { command } => repo::run(&root, command),
        Command::Reference { command } => reference::run(&root, command, &runner),
        Command::Ci { command } => ci::run(&root, command, &runner),
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
