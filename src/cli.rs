//! `depgraph` command line.
//!
//! `validate`, `inventory`, `validate-catalog`, `resolve-env`, and
//! `propose-catalog` are implemented. Collect, analysis, and publish still
//! return [`crate::Error::NotImplemented`] until phase 1.

use std::fs;
use std::path::{Path, PathBuf};

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
    /// Write inventory.csv for the configured GitLab groups.
    Inventory(InventoryArgs),
    /// Validate catalog-info.yaml files and resolve dependsOn.
    ValidateCatalog(ValidateCatalogArgs),
    /// Map a source environment name onto dev, acc, or prod.
    ResolveEnv(ResolveEnvArgs),
    /// Draft catalog-info.yaml files from a local checkout.
    ProposeCatalog(ProposeCatalogArgs),
    /// Serve the read-only MCP endpoint. OAuth is not implemented.
    ServeMcp,
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

/// `depgraph inventory`.
#[derive(Debug, clap::Args)]
pub struct InventoryArgs {
    /// CSV path. Defaults to `inventory.csv`.
    #[arg(long)]
    pub output: Option<PathBuf>,
    /// GitLab base URL. Defaults to `config/config.yaml`.
    #[arg(long)]
    pub base_url: Option<String>,
    /// Group or subgroup path. Repeat to override `gitlab.groups`.
    /// One path collects every project beneath it.
    #[arg(long)]
    pub group: Vec<String>,
    /// Project path. Repeat to override `gitlab.projects`.
    #[arg(long = "project")]
    pub project_path: Vec<String>,
    /// Page size. Defaults to config.
    #[arg(long)]
    pub per_page: Option<u32>,
    /// Parallel project inspections. Defaults to config.
    #[arg(long)]
    pub max_concurrency: Option<usize>,
}

/// `depgraph validate-catalog`.
#[derive(Debug, clap::Args)]
pub struct ValidateCatalogArgs {
    /// Catalog files. dependsOn must resolve inside this set.
    pub files: Vec<PathBuf>,
}

/// `depgraph resolve-env`.
#[derive(Debug, clap::Args)]
pub struct ResolveEnvArgs {
    /// Source environment name, such as `staging`.
    pub name: String,
}

/// `depgraph propose-catalog`.
#[derive(Debug, clap::Args)]
pub struct ProposeCatalogArgs {
    /// Checkout or a directory of checkouts.
    #[arg(long)]
    pub from: PathBuf,
    /// Where drafts are written. Defaults to `proposed-catalog`.
    #[arg(long)]
    pub output: Option<PathBuf>,
    /// Open GitLab merge requests. Never merges.
    #[arg(long)]
    pub open_mr: bool,
    /// GitLab project path for each repo, in directory order. Required with `--open-mr`.
    #[arg(long)]
    pub project: Vec<String>,
    /// GitLab base URL. Defaults to config.
    #[arg(long)]
    pub base_url: Option<String>,
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
        Command::Inventory(args) => run_inventory(args),
        Command::ValidateCatalog(args) => validate_catalog(&args),
        Command::ResolveEnv(args) => resolve_env(&args),
        Command::ProposeCatalog(args) => propose_catalog(args),
        Command::ServeMcp => crate::mcp::serve(),
    }
}

fn run_inventory(args: InventoryArgs) -> Result<(), Error> {
    let project = config::load_project(Path::new("."))?;
    let filter = project.config.gitlab.project_filter()?;
    let base_url = args
        .base_url
        .unwrap_or_else(|| project.config.gitlab.base_url.clone());
    let groups = if args.group.is_empty() {
        project.config.gitlab.groups.clone()
    } else {
        args.group
    };
    let projects = if args.project_path.is_empty() {
        project.config.gitlab.projects.clone()
    } else {
        args.project_path
    };
    let per_page = args.per_page.unwrap_or(project.config.gitlab.per_page);
    let max_concurrency = args
        .max_concurrency
        .unwrap_or(project.config.gitlab.max_concurrency as usize);
    let catalog_file = project.config.gitlab.catalog_file.clone();
    let output = args
        .output
        .unwrap_or_else(|| PathBuf::from("inventory.csv"));
    let (rows, warnings) = tokio_runtime()?.block_on(crate::inventory::collect_inventory(
        crate::inventory::InventoryQuery {
            base_url: &base_url,
            groups: &groups,
            projects: &projects,
            filter: &filter,
            catalog_file: &catalog_file,
            per_page,
            max_concurrency,
            now: chrono::Utc::now(),
        },
    ))?;
    for warning in &warnings {
        eprintln!("warning: {warning}");
    }
    let count = rows.len();
    crate::inventory::write_inventory_csv(&output, &rows)?;
    println!("wrote {} ({count} projects)", output.display());
    Ok(())
}

fn validate_catalog(args: &ValidateCatalogArgs) -> Result<(), Error> {
    let entities = crate::catalog::validate_catalog_files(&args.files)?;
    println!("valid ({} entities)", entities.len());
    Ok(())
}

fn resolve_env(args: &ResolveEnvArgs) -> Result<(), Error> {
    let environments = config::parse_environments(
        &std::fs::read_to_string("config/environments.yaml")
            .map_err(|err| Error::Config(format!("reading config/environments.yaml: {err}")))?,
    )?;
    let index = environments.index()?;
    let env = crate::normalize::resolve_source_env(&index, &args.name)?;
    println!("{env}");
    Ok(())
}

fn propose_catalog(args: ProposeCatalogArgs) -> Result<(), Error> {
    let drafts = crate::propose::draft_from_directory(&args.from)?;
    let output = args
        .output
        .unwrap_or_else(|| PathBuf::from("proposed-catalog"));
    let written = crate::propose::write_drafts(&output, &drafts)?;
    for path in &written {
        println!("wrote {}", path.display());
    }
    if args.open_mr {
        let project = config::load_project(Path::new("."))?;
        let base_url = args.base_url.unwrap_or(project.config.gitlab.base_url);
        let urls = tokio_runtime()?.block_on(crate::propose::open_merge_requests(
            &base_url,
            project.config.gitlab.per_page,
            &drafts,
            &args.project,
        ))?;
        for url in urls {
            println!("opened {url}");
        }
    }
    Ok(())
}

fn tokio_runtime() -> Result<tokio::runtime::Runtime, Error> {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(Error::Io)
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
