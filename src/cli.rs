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
    Build(BuildArgs),
    /// Generate the static site.
    Site(SiteArgs),
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
    Drift(DriftArgs),
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
    /// Serve the read-only MCP endpoint.
    ServeMcp(ServeMcpArgs),
    /// Comment on a merge request with prod consumers and contract changes.
    MrCheck(MrCheckArgs),
}

/// `depgraph collect`.
#[derive(Debug, clap::Args)]
pub struct CollectArgs {
    /// Collector set.
    #[arg(long, value_enum)]
    pub set: CollectSetArg,
    /// Fixture directory. Omit to read GitLab.
    #[arg(long)]
    pub fixture: Option<PathBuf>,
    /// Directory for raw JSONL. Defaults to `data`.
    #[arg(long)]
    pub output: Option<PathBuf>,
    /// GitLab base URL override.
    #[arg(long)]
    pub base_url: Option<String>,
    /// Group or subgroup path. Repeat to override `gitlab.groups`.
    #[arg(long)]
    pub group: Vec<String>,
    /// Project path. Repeat to override `gitlab.projects`.
    #[arg(long = "project")]
    pub project_path: Vec<String>,
    /// Recorded Prometheus responses (`calls.json`, `errors.json`, `latency.json`).
    #[arg(long)]
    pub prometheus_fixture: Option<PathBuf>,
    /// Query `prometheus.url` with read-only GETs.
    #[arg(long)]
    pub prometheus: bool,
}

/// `depgraph drift`.
#[derive(Debug, clap::Args)]
pub struct DriftArgs {
    /// Shared snapshot flags.
    #[command(flatten)]
    pub read: ReadArgs,
    /// Directory for `dev.md`, `acc.md`, and `prod.md`. Defaults to `drift_report`.
    #[arg(long)]
    pub output: Option<PathBuf>,
}

/// `depgraph serve-mcp`.
#[derive(Debug, clap::Args)]
pub struct ServeMcpArgs {
    /// Disable auth. The bind address must be loopback.
    #[arg(long)]
    pub insecure_local: bool,
    /// Listen address. Defaults to `mcp.bind`.
    #[arg(long)]
    pub bind: Option<String>,
    /// Snapshot to serve. Defaults to `data/snapshot.json`.
    #[arg(long)]
    pub snapshot: Option<PathBuf>,
}

/// `depgraph mr-check`.
#[derive(Debug, clap::Args)]
pub struct MrCheckArgs {
    /// Snapshot path. Defaults to `data/snapshot.json`.
    #[arg(long)]
    pub snapshot: Option<PathBuf>,
    /// GitLab project path. Defaults to `CI_PROJECT_PATH`.
    #[arg(long)]
    pub project: Option<String>,
    /// GitLab origin. Defaults to config.
    #[arg(long)]
    pub base_url: Option<String>,
    /// Merge request IID. Defaults to `CI_MERGE_REQUEST_IID`.
    #[arg(long)]
    pub mr_iid: Option<u64>,
    /// Changed file. Repeat for each path.
    #[arg(long)]
    pub changed_file: Vec<String>,
    /// Print the note and do not call GitLab.
    #[arg(long)]
    pub dry_run: bool,
}

/// `depgraph publish drift-issue`.
#[derive(Debug, clap::Args)]
pub struct DriftIssueArgs {
    /// Snapshot path. Defaults to `data/snapshot.json`.
    #[arg(long)]
    pub snapshot: Option<PathBuf>,
    /// GitLab project path.
    #[arg(long)]
    pub project: Option<String>,
    /// GitLab origin. Defaults to config.
    #[arg(long)]
    pub base_url: Option<String>,
    /// Print the issue body and do not call GitLab.
    #[arg(long)]
    pub dry_run: bool,
}

/// `depgraph build`.
#[derive(Debug, clap::Args)]
pub struct BuildArgs {
    /// Directory of collector JSONL files. Defaults to `data/raw`.
    #[arg(long)]
    pub raw: Option<PathBuf>,
    /// Snapshot path. Defaults to `data/snapshot.json`.
    #[arg(long)]
    pub output: Option<PathBuf>,
    /// Alias file. Defaults to `config/aliases.yaml`.
    #[arg(long)]
    pub aliases: Option<PathBuf>,
}

/// `depgraph publish`.
#[derive(Debug, Subcommand)]
pub enum PublishCommand {
    /// Publish the graph snapshot.
    Snapshot(SnapshotArgs),
    /// Open or update the drift issue.
    DriftIssue(DriftIssueArgs),
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
    /// How many edges to walk. Omit to walk the whole reachable graph.
    #[arg(long)]
    pub depth: Option<usize>,
}

/// `depgraph site`.
#[derive(Debug, clap::Args)]
pub struct SiteArgs {
    /// Snapshot to render. Defaults to `data/snapshot.json`.
    #[arg(long)]
    pub snapshot: Option<PathBuf>,
    /// Output directory. Defaults to `public`.
    #[arg(long)]
    pub output: Option<PathBuf>,
}

/// `depgraph publish snapshot`.
#[derive(Debug, clap::Args)]
pub struct SnapshotArgs {
    /// Snapshot to copy. Defaults to `data/snapshot.json`.
    #[arg(long)]
    pub snapshot: Option<PathBuf>,
    /// Destination. Defaults to `data/published.json`.
    #[arg(long)]
    pub output: Option<PathBuf>,
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
        Command::Collect(args) => run_collect(args),
        Command::Build(args) => run_build(args),
        Command::Site(args) => run_site(args),
        Command::Publish { target } => match target {
            PublishCommand::Snapshot(args) => run_publish_snapshot(args),
            PublishCommand::DriftIssue(args) => run_drift_issue(args),
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
        Command::Drift(args) => {
            let output = args.output.unwrap_or_else(|| PathBuf::from("drift_report"));
            analysis::drift(&read_request("drift", args.read), &output)
        }
        Command::Critical(args) => analysis::critical(&read_request("critical", args)),
        Command::Orphans(args) => analysis::orphans(&read_request("orphans", args)),
        Command::Diff(args) => analysis::diff(&read_request("diff", args)),
        Command::At(args) => analysis::at(&read_request("at", args)),
        Command::Validate(args) => validate_path(&args),
        Command::Inventory(args) => run_inventory(args),
        Command::ValidateCatalog(args) => validate_catalog(&args),
        Command::ResolveEnv(args) => resolve_env(&args),
        Command::ProposeCatalog(args) => propose_catalog(args),
        Command::ServeMcp(args) => run_serve_mcp(args),
        Command::MrCheck(args) => run_mr_check(args),
    }
}

fn run_collect(args: CollectArgs) -> Result<(), Error> {
    let set = match args.set.into_set() {
        collectors::CollectSet::Fast => collectors::GatherSet::Fast,
        collectors::CollectSet::Full => collectors::GatherSet::Full,
    };
    let prometheus_only =
        args.fixture.is_none() && (args.prometheus_fixture.is_some() || args.prometheus);
    let runtime = prometheus_records(&args)?;
    let mut records = if let Some(fixture) = &args.fixture {
        collectors::records_from_fixture(fixture, set)?
    } else if prometheus_only {
        Vec::new()
    } else {
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
        let catalog_file = project.config.gitlab.catalog_file.clone();
        let per_page = project.config.gitlab.per_page;
        let token = crate::gitlab::resolve_token(&base_url)?;
        let client = crate::gitlab::GitLabClient::new(base_url, token, per_page)?;
        let live = live_reader(&project.config.kubernetes)?;
        let (records, warnings) = tokio_runtime()?.block_on(collectors::records_from_gitlab(
            &client,
            collectors::GitlabScope {
                groups: &groups,
                projects: &projects,
                filter: &filter,
                now: chrono::Utc::now(),
            },
            &catalog_file,
            set,
            live.as_ref()
                .map(|reader| reader as &dyn collectors::LiveRead),
        ))?;
        for warning in &warnings {
            eprintln!("warning: {warning}");
        }
        records
    };
    records.extend(runtime);
    let output = args.output.unwrap_or_else(|| PathBuf::from("data"));
    write_raw_dir(&output.join("raw"), &records)?;
    println!(
        "wrote {} raw records to {}",
        records.len(),
        output.display()
    );
    Ok(())
}

fn prometheus_records(args: &CollectArgs) -> Result<Vec<crate::raw::RawRecord>, Error> {
    if args.prometheus_fixture.is_none() && !args.prometheus {
        return Ok(Vec::new());
    }
    let project = config::load_project(Path::new("."))?;
    let prom = &project.config.prometheus;
    let window = collectors::query_window(prom.window_days, prom.retention_days);
    let filter = collectors::RuntimeFilter {
        health_paths: prom.health_paths.clone(),
        test_clients: prom.test_clients.clone(),
    };
    if let Some(dir) = &args.prometheus_fixture {
        return collectors::records_from_prometheus_dir(dir, window, &filter);
    }
    let token = match &prom.token_env {
        Some(name) => Some(std::env::var(name).map_err(|_| {
            Error::Config(format!(
                "environment variable {name} is not set. The token was not read."
            ))
        })?),
        None => None,
    };
    tokio_runtime()?.block_on(collectors::query_prometheus(
        &prom.url,
        window,
        &filter,
        token.as_deref(),
    ))
}

fn live_reader(
    kubernetes: &config::KubernetesConfig,
) -> Result<Option<collectors::KubectlRead>, Error> {
    if !kubernetes.live_read {
        return Ok(None);
    }
    let Some(cluster) = kubernetes.clusters.first() else {
        return Err(Error::Config(
            "kubernetes.live_read needs a cluster".to_owned(),
        ));
    };
    let path = std::env::var(&cluster.kubeconfig_env).map_err(|_| {
        Error::Config(format!(
            "environment variable {} is not set. The kubeconfig was not read.",
            cluster.kubeconfig_env
        ))
    })?;
    Ok(Some(collectors::KubectlRead {
        kubeconfig: PathBuf::from(path),
    }))
}

fn write_raw_dir(dir: &Path, records: &[crate::raw::RawRecord]) -> Result<(), Error> {
    fs::create_dir_all(dir)?;
    let mut by_collector: std::collections::BTreeMap<&str, Vec<&crate::raw::RawRecord>> =
        std::collections::BTreeMap::new();
    for record in records {
        by_collector
            .entry(record.collector.as_str())
            .or_default()
            .push(record);
    }
    for (name, group) in by_collector {
        let mut lines = Vec::new();
        for record in group {
            lines.push(serde_json::to_string(record).map_err(|err| Error::Store(err.to_string()))?);
        }
        lines.sort();
        let mut body = lines.join("\n");
        if !body.is_empty() {
            body.push('\n');
        }
        fs::write(dir.join(format!("{name}.jsonl")), body)?;
    }
    Ok(())
}

fn run_build(args: BuildArgs) -> Result<(), Error> {
    let raw_dir = args.raw.unwrap_or_else(|| PathBuf::from("data/raw"));
    let records = read_raw_dir(&raw_dir)?;
    let aliases = if let Some(path) = &args.aliases {
        config::parse_aliases(&fs::read_to_string(path)?)?
    } else {
        config::load_project(Path::new("."))?.aliases
    };
    let environments =
        config::parse_environments(&fs::read_to_string("config/environments.yaml")?)?.index()?;
    let generated_at = generated_at_from(&records);
    let loaded = config::load_project(Path::new(".")).ok();
    let libraries = loaded
        .as_ref()
        .map(|project| project.config.libraries.enabled)
        .unwrap_or(false);
    let edge_expiry_days = loaded
        .as_ref()
        .map(|project| project.config.edge_expiry_days)
        .unwrap_or(30);
    let retention_days = loaded
        .as_ref()
        .map(|project| project.config.retention_days)
        .unwrap_or(90);
    let built = normalize::merge::merge_records(
        &records,
        &normalize::merge::MergeOptions {
            aliases: &aliases,
            environments: &environments,
            generated_at: &generated_at,
            run_id: "local",
            libraries_enabled: libraries,
            edge_expiry_days,
            retention_days,
        },
    )?;
    let output = args
        .output
        .unwrap_or_else(|| PathBuf::from("data/snapshot.json"));
    publish::write_snapshot(&built.graph, &output)?;
    let unresolved_path = output.with_file_name("unresolved.csv");
    fs::write(
        &unresolved_path,
        normalize::merge::unresolved_csv(&built.unresolved),
    )?;
    let meta_path = output.with_file_name("meta.json");
    let meta = crate::store::Meta {
        collectors: Default::default(),
        deployment_mismatches: built.mismatches,
        coverage: built.coverage,
    };
    fs::write(
        &meta_path,
        serde_json::to_string_pretty(&meta).map_err(|err| Error::Store(err.to_string()))?,
    )?;
    println!(
        "wrote {} ({} unresolved)",
        output.display(),
        built.unresolved.len()
    );
    Ok(())
}

fn read_raw_dir(dir: &Path) -> Result<Vec<crate::raw::RawRecord>, Error> {
    let mut records = Vec::new();
    if !dir.is_dir() {
        return Err(Error::Config(format!(
            "raw directory {} does not exist",
            dir.display()
        )));
    }
    let mut paths: Vec<_> = fs::read_dir(dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .collect();
    paths.sort();
    for path in paths {
        if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
            continue;
        }
        let text = fs::read_to_string(&path)?;
        for line in text.lines().filter(|line| !line.is_empty()) {
            records.push(
                serde_json::from_str(line)
                    .map_err(|err| Error::Store(format!("{}: {err}", path.display())))?,
            );
        }
    }
    Ok(records)
}

fn generated_at_from(records: &[crate::raw::RawRecord]) -> crate::models::Timestamp {
    let mut latest = "1970-01-01T00:00:00Z".to_owned();
    for record in records {
        for field in ["created_at", "last_seen"] {
            if let Some(created) = record.payload.get(field).and_then(|value| value.as_str())
                && created > latest.as_str()
            {
                latest = created.to_owned();
            }
        }
    }
    crate::models::Timestamp::parse(latest)
        .unwrap_or_else(|_| crate::models::Timestamp::parse("1970-01-01T00:00:00Z").expect("epoch"))
}

fn run_site(args: SiteArgs) -> Result<(), Error> {
    let snapshot = args
        .snapshot
        .unwrap_or_else(|| PathBuf::from("data/snapshot.json"));
    let graph: crate::models::GraphSnapshot = serde_json::from_str(&fs::read_to_string(&snapshot)?)
        .map_err(|err| Error::Validation(err.to_string()))?;
    let output = args.output.unwrap_or_else(|| PathBuf::from("public"));
    let aliases = config::load_project(Path::new("."))
        .map(|project| project.aliases.aliases)
        .unwrap_or_default();
    publish::write_site(&graph, &aliases, &output)?;
    println!("wrote {}", output.join("index.html").display());
    Ok(())
}

fn run_publish_snapshot(args: SnapshotArgs) -> Result<(), Error> {
    let snapshot = args
        .snapshot
        .unwrap_or_else(|| PathBuf::from("data/snapshot.json"));
    let graph: crate::models::GraphSnapshot = serde_json::from_str(&fs::read_to_string(&snapshot)?)
        .map_err(|err| Error::Validation(err.to_string()))?;
    let output = args
        .output
        .unwrap_or_else(|| PathBuf::from("data/published.json"));
    publish::write_snapshot(&graph, &output)?;
    println!("wrote {}", output.display());
    Ok(())
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

fn run_serve_mcp(args: ServeMcpArgs) -> Result<(), Error> {
    let project = config::load_project(Path::new("."))?;
    let bind = args.bind.unwrap_or(project.config.mcp.bind);
    let snapshot = args
        .snapshot
        .unwrap_or_else(|| PathBuf::from("data/snapshot.json"));
    crate::mcp::serve_blocking(
        &bind,
        args.insecure_local,
        &project.config.mcp.issuer,
        &project.config.mcp.audience,
        &project.config.mcp.jwks_url,
        &snapshot,
        project.aliases,
    )
}

fn run_mr_check(args: MrCheckArgs) -> Result<(), Error> {
    let project = config::load_project(Path::new("."))?;
    let snapshot_path = args
        .snapshot
        .unwrap_or_else(|| PathBuf::from("data/snapshot.json"));
    let text = fs::read_to_string(&snapshot_path)?;
    let snapshot: crate::models::GraphSnapshot = serde_json::from_str(&text)
        .map_err(|err| Error::Validation(crate::redact::redact(&err.to_string())))?;
    let project_path = args
        .project
        .or_else(|| std::env::var("CI_PROJECT_PATH").ok())
        .ok_or_else(|| Error::Config("mr-check needs --project or CI_PROJECT_PATH".to_owned()))?;
    let note = publish::build_note(&publish::MrCheck {
        project_path: &project_path,
        snapshot: &snapshot,
        aliases: &project.aliases,
        contract_paths: &project.config.contract_paths,
        changed_files: &args.changed_file,
    });
    if args.dry_run {
        println!("{}", note.body);
        return Ok(());
    }
    let iid = args
        .mr_iid
        .or_else(|| {
            std::env::var("CI_MERGE_REQUEST_IID")
                .ok()
                .and_then(|value| value.parse().ok())
        })
        .ok_or_else(|| Error::Config("mr-check needs --mr-iid or CI_MERGE_REQUEST_IID".into()))?;
    let base_url = args.base_url.unwrap_or(project.config.gitlab.base_url);
    let token = std::env::var(&project.config.gitlab.bot_token_env).map_err(|_| {
        Error::Config(format!(
            "environment variable {} is not set. The token was not read.",
            project.config.gitlab.bot_token_env
        ))
    })?;
    tokio_runtime()?.block_on(publish::upsert_note(
        &base_url,
        &project_path,
        iid,
        &token,
        &note.body,
    ))
}

fn run_drift_issue(args: DriftIssueArgs) -> Result<(), Error> {
    let project = config::load_project(Path::new("."))?;
    let snapshot_path = args
        .snapshot
        .unwrap_or_else(|| PathBuf::from("data/snapshot.json"));
    let text = fs::read_to_string(&snapshot_path)?;
    let snapshot: crate::models::GraphSnapshot = serde_json::from_str(&text)
        .map_err(|err| Error::Validation(crate::redact::redact(&err.to_string())))?;
    let body = publish::drift_issue_body(&snapshot);
    if args.dry_run {
        println!("{body}");
        return Ok(());
    }
    let project_path = args
        .project
        .or_else(|| std::env::var("CI_PROJECT_PATH").ok())
        .ok_or_else(|| {
            Error::Config("drift-issue needs --project or CI_PROJECT_PATH".to_owned())
        })?;
    let base_url = args.base_url.unwrap_or(project.config.gitlab.base_url);
    let token = std::env::var(&project.config.gitlab.bot_token_env).map_err(|_| {
        Error::Config(format!(
            "environment variable {} is not set. The token was not read.",
            project.config.gitlab.bot_token_env
        ))
    })?;
    tokio_runtime()?.block_on(publish::upsert_drift_issue(
        &base_url,
        &project_path,
        &token,
        &body,
    ))
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
        depth: args.depth,
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
