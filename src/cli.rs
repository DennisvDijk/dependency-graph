//! `depgraph` command line.
//!
//! `validate` is implemented. Every other command is wired to its module and
//! returns [`crate::Error::NotImplemented`] so a pipeline can see the stub
//! without touching a source system.

use std::fs;
use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::analysis::{self, OutputFormat, ReadRequest};
use crate::collectors::{self, CollectSet};
use crate::config::{self, Aliases};
use crate::error::Error;
use crate::models::Env;
use crate::normalize::{self, validate};
use crate::publish;

/// Self-hosted dependency graph for runtime dependencies and deployments.
#[derive(Debug, Parser)]
#[command(name = "depgraph", version, about)]
pub struct Cli {
    /// Command to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Collect raw records from source systems.
    Collect(CollectArgs),
    /// Build a graph snapshot from raw records.
    Build,
    /// Generate the static site.
    Site,
    /// Publish a snapshot or a drift issue.
    Publish {
        /// What to publish.
        #[command(subcommand)]
        target: PublishCommand,
    },
    /// Show one node.
    Show(ReadArgs),
    /// List dependencies of a component.
    Deps(ReadArgs),
    /// List consumers of a component.
    Consumers(ReadArgs),
    /// Show transitive impact.
    Impact(ReadArgs),
    /// Propose a deploy order.
    DeployOrder(ReadArgs),
    /// List cycles.
    Cycles(ReadArgs),
    /// Show components affected by a library.
    LibraryImpact(ReadArgs),
    /// Show declared versus observed drift.
    Drift(ReadArgs),
    /// List hard edges.
    Critical(ReadArgs),
    /// List nodes with no edges.
    Orphans(ReadArgs),
    /// Diff two snapshots.
    Diff(ReadArgs),
    /// Show the graph at a point in time.
    At(ReadArgs),
    /// Validate a graph snapshot.
    Validate(ValidateArgs),
}

/// `depgraph collect`.
#[derive(Debug, clap::Args)]
pub struct CollectArgs {
    /// Collector set.
    #[arg(long, value_enum)]
    pub set: CollectSetArg,
}

/// `depgraph publish`.
#[derive(Debug, Subcommand)]
pub enum PublishCommand {
    /// Publish the graph snapshot.
    Snapshot,
    /// Open or update the drift issue.
    DriftIssue,
}

/// Flags shared by read commands.
#[derive(Debug, clap::Args)]
pub struct ReadArgs {
    /// Component, library, or other target.
    pub target: Option<String>,
    /// Second target, reserved for diff.
    pub against: Option<String>,
    /// Canonical environment filter.
    #[arg(long, value_enum)]
    pub env: Option<EnvArg>,
    /// Output format.
    #[arg(long, value_enum, default_value = "table")]
    pub format: OutputFormatArg,
    /// Graph snapshot to read.
    #[arg(long)]
    pub snapshot: Option<PathBuf>,
}

/// `depgraph validate`.
#[derive(Debug, clap::Args)]
pub struct ValidateArgs {
    /// Path to a graph snapshot JSON file.
    pub graph: PathBuf,
    /// Alias file. Defaults to `config/aliases.yaml` when that file exists.
    #[arg(long)]
    pub aliases: Option<PathBuf>,
}

/// Collector set on the command line.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum CollectSetArg {
    /// Incremental collectors.
    Fast,
    /// Every collector.
    Full,
}

/// Canonical environment on the command line.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum EnvArg {
    /// Development.
    Dev,
    /// Acceptance.
    Acc,
    /// Production.
    Prod,
}

/// Output format on the command line.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormatArg {
    /// Aligned text.
    Table,
    /// One JSON document.
    Json,
    /// A Mermaid diagram.
    Mermaid,
}

impl CollectSetArg {
    fn into_set(self) -> CollectSet {
        match self {
            Self::Fast => CollectSet::Fast,
            Self::Full => CollectSet::Full,
        }
    }
}

impl EnvArg {
    fn into_env(self) -> Env {
        match self {
            Self::Dev => Env::Dev,
            Self::Acc => Env::Acc,
            Self::Prod => Env::Prod,
        }
    }
}

impl OutputFormatArg {
    fn into_format(self) -> OutputFormat {
        match self {
            Self::Table => OutputFormat::Table,
            Self::Json => OutputFormat::Json,
            Self::Mermaid => OutputFormat::Mermaid,
        }
    }
}

/// Parse process arguments and run one command.
///
/// # Errors
///
/// Returns a usage error, a validation error, or a not-implemented error.
pub fn run() -> Result<(), Error> {
    run_from(std::env::args_os())
}

/// Parse `args` and run one command. The first item is the program name.
///
/// # Errors
///
/// Returns a usage error, a validation error, or a not-implemented error.
pub fn run_from<I, T>(args: I) -> Result<(), Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    match Cli::try_parse_from(args) {
        Ok(cli) => execute(cli),
        Err(err) => {
            use clap::error::ErrorKind;
            if matches!(
                err.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) {
                err.print().ok();
                Ok(())
            } else {
                Err(Error::Usage(err.to_string()))
            }
        }
    }
}

fn execute(cli: Cli) -> Result<(), Error> {
    match cli.command {
        Command::Collect(args) => collectors::collect(args.set.into_set()),
        Command::Build => normalize::merge::merge(),
        Command::Site => publish::site(),
        Command::Publish { target } => match target {
            PublishCommand::Snapshot => publish::publish_snapshot(),
            PublishCommand::DriftIssue => publish::drift_issue(),
        },
        Command::Show(args) => analysis::show(&read_request("show", args)),
        Command::Deps(args) => analysis::deps(&read_request("deps", args)),
        Command::Consumers(args) => analysis::consumers(&read_request("consumers", args)),
        Command::Impact(args) => analysis::impact(&read_request("impact", args)),
        Command::DeployOrder(args) => analysis::deploy_order(&read_request("deploy-order", args)),
        Command::Cycles(args) => analysis::cycles(&read_request("cycles", args)),
        Command::LibraryImpact(args) => {
            analysis::library_impact(&read_request("library-impact", args))
        }
        Command::Drift(args) => analysis::drift(&read_request("drift", args)),
        Command::Critical(args) => analysis::critical(&read_request("critical", args)),
        Command::Orphans(args) => analysis::orphans(&read_request("orphans", args)),
        Command::Diff(args) => analysis::diff(&read_request("diff", args)),
        Command::At(args) => analysis::at(&read_request("at", args)),
        Command::Validate(args) => validate_path(&args),
    }
}

fn read_request(command: &'static str, args: ReadArgs) -> ReadRequest {
    ReadRequest {
        command,
        target: args.target,
        against: args.against,
        env: args.env.map(EnvArg::into_env),
        format: args.format.into_format(),
        snapshot: args.snapshot,
    }
}

fn validate_path(args: &ValidateArgs) -> Result<(), Error> {
    let text = fs::read_to_string(&args.graph)
        .map_err(|err| Error::Store(format!("reading {}: {err}", args.graph.display())))?;
    let graph: crate::models::GraphSnapshot = serde_json::from_str(&text)
        .map_err(|err| Error::Validation(crate::redact::redact(&err.to_string())))?;
    let aliases = load_aliases(args.aliases.as_deref())?;
    let report = validate(&graph, aliases.as_ref());
    report.into_result()?;
    println!("valid");
    Ok(())
}

fn load_aliases(path: Option<&std::path::Path>) -> Result<Option<Aliases>, Error> {
    let path = match path {
        Some(path) => path.to_path_buf(),
        None => {
            let default = PathBuf::from("config/aliases.yaml");
            if default.is_file() {
                default
            } else {
                return Ok(None);
            }
        }
    };
    let text = fs::read_to_string(&path)
        .map_err(|err| Error::Config(format!("reading {}: {err}", path.display())))?;
    config::parse_aliases(&text).map(Some)
}
