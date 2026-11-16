//! `depgraph` command line.
//!
//! `validate`, `inventory`, `validate-catalog`, `resolve-env`, and
//! `propose-catalog` are implemented. Collect, analysis, and publish still
//! return [`crate::Error::NotImplemented`] until phase 1.

use std::fs;
use std::path::{Path, PathBuf};

use clap::{CommandFactory, Parser, Subcommand, ValueEnum};

use crate::analysis::{self, OutputFormat, ReadRequest};
use crate::collectors::{self, CollectSet};
use crate::config::{self, Aliases};
use crate::error::Error;
use crate::models::Env;
use crate::normalize::{self, validate};
use crate::publish;

/// Self-hosted dependency graph for runtime dependencies and deployments.
#[derive(Debug, Parser)]
#[command(
    name = "depgraph",
    version,
    about,
    after_long_help = "Examples:\n  depgraph init\n  depgraph doctor\n  depgraph collect --set full --output data\n  depgraph build --raw data/raw --output data/snapshot.json\n  depgraph show payments-api --snapshot data/snapshot.json\n  depgraph serve-mcp --insecure-local --snapshot data/snapshot.json\n  depgraph mcp-config cursor"
)]
pub struct Cli {
    /// Command to run.
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Guide a discovery run and explain available evidence.
    Discover(DiscoverArgs),
    /// Collect raw records from source systems.
    Collect(CollectArgs),
    /// Build a graph snapshot from raw records.
    Build(BuildArgs),
    /// Propose aliases and namespace mappings for review.
    Suggest(SuggestArgs),
    /// Set up an OpenAI-compatible model and optional system-stored key.
    ModelSetup,
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
    /// Write a minimal config. Does not store tokens.
    Init(InitArgs),
    /// Check config, glab, and snapshot freshness.
    Doctor(DoctorArgs),
    /// Print an MCP client config snippet.
    McpConfig(McpConfigArgs),
    /// Print a shell completion script.
    Completions(CompletionArgs),
    /// List archived snapshot runs.
    Runs(RunsArgs),
    /// Delete archived runs older than retention_days.
    Retain(RetainArgs),
    /// Deployments and incidents on one timeline.
    Timeline(TimelineArgs),
    /// Edge count and local query p95.
    Stats(StatsArgs),
    /// Data-quality scorecard per owner.
    Scorecard(ScorecardArgs),
}

/// Options for reviewable suggestions.
#[derive(Debug, clap::Args)]
pub struct SuggestArgs {
    /// Unresolved report from a build.
    #[arg(long, default_value = "data/unresolved.json")]
    pub unresolved: PathBuf,
    /// Graph snapshot from the same build.
    #[arg(long, default_value = "data/snapshot.json")]
    pub snapshot: PathBuf,
    /// Environment mappings used to suggest namespace names.
    #[arg(long, default_value = "config/environments.yaml")]
    pub environments: PathBuf,
    /// Send remaining names and component names to an OpenAI-compatible API.
    #[arg(long)]
    pub llm: bool,
    /// Model name on the configured endpoint (or set OPENAI_MODEL).
    #[arg(long, requires = "llm")]
    pub model_id: Option<String>,
}

/// Options for guided discovery.
#[derive(Debug, clap::Args)]
pub struct DiscoverArgs {
    /// Include AWS discovery, including existing flow logs.
    #[arg(long, conflicts_with = "repos_only")]
    pub aws: bool,
    /// Include Azure discovery, including existing VNet Traffic Analytics.
    #[arg(long, conflicts_with = "repos_only")]
    pub azure: bool,
    /// Scan repository sources only, without live cloud or Prometheus calls.
    #[arg(long, conflicts_with_all = ["aws", "azure", "prometheus"])]
    pub repos_only: bool,
    /// Include Istio metrics from the configured Prometheus endpoint.
    #[arg(long, conflicts_with = "repos_only")]
    pub prometheus: bool,
    /// Use saved Prometheus responses (requires --prometheus).
    #[arg(long, requires = "prometheus")]
    pub prometheus_fixture: Option<PathBuf>,
    /// Use a saved repository landscape instead of GitLab.
    #[arg(long)]
    pub fixture: Option<PathBuf>,
    /// Use saved AWS data instead of live AWS (requires --aws).
    #[arg(long, requires = "aws")]
    pub aws_fixture: Option<PathBuf>,
    /// Use saved Azure data instead of live Azure (requires --azure).
    #[arg(long, requires = "azure")]
    pub azure_fixture: Option<PathBuf>,
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
    /// Read the AWS account once with the `aws` CLI (read-only), even when
    /// `aws.enabled` is false.
    #[arg(long)]
    pub aws: bool,
    /// Discover AWS VPC flow logs for this run (also enables AWS collection).
    #[arg(long)]
    pub aws_flow_logs: bool,
    /// Saved `aws` CLI output (`security-groups.json`, `service-graph.json`, ...).
    #[arg(long)]
    pub aws_fixture: Option<PathBuf>,
    /// Read Azure once with the `az` CLI (read-only), even when
    /// `azure.enabled` is false.
    #[arg(long)]
    pub azure: bool,
    /// Discover Azure VNet flow analytics for this run (also enables Azure collection).
    #[arg(long)]
    pub azure_flow_logs: bool,
    /// Saved `az` CLI output (`asgs.json`, `nsgs.json`, `dependencies.json`).
    #[arg(long)]
    pub azure_fixture: Option<PathBuf>,
    /// Skip implicit cloud configuration for guided discovery.
    #[arg(skip)]
    pub skip_config_cloud: bool,
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
    /// Opt in to the blocking check for this run. The config flag stays off.
    #[arg(long)]
    pub blocking: bool,
    /// Label already on the merge request. Repeatable.
    #[arg(long)]
    pub label: Vec<String>,
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

/// `depgraph init`.
#[derive(Debug, clap::Args)]
pub struct InitArgs {
    /// Directory that will contain `config/`. Defaults to the current directory.
    #[arg(long)]
    pub dir: Option<PathBuf>,
    /// Replace config files that already exist.
    #[arg(long)]
    pub force: bool,
}

/// `depgraph doctor`.
#[derive(Debug, clap::Args)]
pub struct DoctorArgs {
    /// Reference time for snapshot age. Defaults to the clock.
    #[arg(long)]
    pub now: Option<String>,
    /// Also GET GitLab `/api/v4/version`. Read-only.
    #[arg(long)]
    pub online: bool,
}

/// `depgraph mcp-config`.
#[derive(Debug, clap::Args)]
pub struct McpConfigArgs {
    /// `cursor`, `claude`, `vscode`, or `opencode`.
    pub client: String,
}

/// `depgraph completions`.
#[derive(Debug, clap::Args)]
pub struct CompletionArgs {
    /// Shell to generate.
    pub shell: clap_complete::Shell,
}

/// `depgraph runs`.
#[derive(Debug, clap::Args)]
pub struct RunsArgs {
    /// Directory that contains `snapshot.json`. Defaults to `data`.
    #[arg(long)]
    pub data: Option<PathBuf>,
}

/// `depgraph retain`.
#[derive(Debug, clap::Args)]
pub struct RetainArgs {
    /// Directory that contains `snapshot.json` and `runs/`. Defaults to `data`.
    #[arg(long)]
    pub data: Option<PathBuf>,
    /// Reference time. Defaults to the clock.
    #[arg(long)]
    pub now: Option<String>,
}

/// `depgraph timeline`.
#[derive(Debug, clap::Args)]
pub struct TimelineArgs {
    /// Canonical environment.
    #[arg(long, value_enum)]
    pub env: EnvArg,
    /// Include events at or after this RFC 3339 time.
    #[arg(long)]
    pub since: String,
    /// Deployment fixture. Not a live GitLab call.
    #[arg(long)]
    pub deployments: PathBuf,
    /// Incident fixture. Not a live Datadog call.
    #[arg(long)]
    pub incidents: Option<PathBuf>,
}

/// `depgraph stats`.
#[derive(Debug, clap::Args)]
pub struct StatsArgs {
    /// Snapshot to measure.
    #[arg(long)]
    pub snapshot: Option<PathBuf>,
}

/// `depgraph scorecard`.
#[derive(Debug, clap::Args)]
pub struct ScorecardArgs {
    /// Snapshot to score.
    #[arg(long)]
    pub snapshot: Option<PathBuf>,
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
    /// Archived run key for the left side of `diff`.
    #[arg(long = "from")]
    pub from_run: Option<String>,
    /// Archived run key for the right side of `diff`.
    #[arg(long = "to")]
    pub to_run: Option<String>,
    /// Date for `at`. Uses the newest stored run at or before this time.
    #[arg(long)]
    pub date: Option<String>,
    /// Directory of archived runs. Defaults to `data/runs`.
    #[arg(long)]
    pub runs: Option<PathBuf>,
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
        Command::Discover(args) => run_discover(args),
        Command::Collect(args) => run_collect(args),
        Command::Build(args) => run_build(args),
        Command::ModelSetup => crate::model_config::setup(),
        Command::Suggest(args) => crate::suggest::run(
            &args.unresolved,
            &args.snapshot,
            &args.environments,
            args.llm,
            args.model_id.as_deref(),
        ),
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
        Command::Init(args) => run_init(args),
        Command::Doctor(args) => run_doctor(args),
        Command::McpConfig(args) => run_mcp_config(args),
        Command::Completions(args) => run_completions(args),
        Command::Runs(args) => run_runs(args),
        Command::Retain(args) => run_retain(args),
        Command::Timeline(args) => run_timeline(args),
        Command::Stats(args) => run_stats(args),
        Command::Scorecard(args) => run_scorecard(args),
    }
}

fn command_available(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("--version")
        .output()
        .is_ok_and(|result| result.status.success())
}

fn aws_scope(aws: &config::AwsConfig, fixture: bool) -> String {
    if fixture {
        return "saved AWS fixture".into();
    }
    let profile = aws.profile.as_deref().unwrap_or("AWS CLI default");
    let regions = if aws.regions.is_empty() {
        let env = std::env::var("AWS_REGION")
            .ok()
            .or_else(|| std::env::var("AWS_DEFAULT_REGION").ok())
            .filter(|s| !s.trim().is_empty());
        let configured = || {
            let mut cmd = std::process::Command::new("aws");
            cmd.args(["configure", "get", "region"]);
            if let Some(profile) = &aws.profile {
                cmd.args(["--profile", profile]);
            }
            cmd.output()
                .ok()
                .filter(|out| out.status.success())
                .and_then(|out| String::from_utf8(out.stdout).ok())
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        };
        env.or_else(configured)
            .unwrap_or_else(|| "not detected (AWS CLI default)".into())
    } else {
        aws.regions.join(", ")
    };
    format!("profile {profile}, region {regions}")
}

fn azure_scope(azure: &config::AzureConfig, fixture: bool) -> String {
    if fixture {
        return "saved Azure fixture".into();
    }
    if !azure.subscriptions.is_empty() {
        return format!("subscriptions {}", azure.subscriptions.join(", "));
    }
    let selected = std::process::Command::new("az")
        .args(["account", "show", "--query", "name", "--output", "json"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .and_then(|out| serde_json::from_slice::<String>(&out.stdout).ok())
        .filter(|name| {
            !name.trim().is_empty() && name.len() <= 200 && !name.chars().any(char::is_control)
        });
    match selected {
        Some(name) => format!("subscription {name}"),
        None => "current Azure CLI subscription (not detected)".into(),
    }
}

fn run_discover(args: DiscoverArgs) -> Result<(), Error> {
    use std::io::{IsTerminal, Write};
    println!("Discovery scans repositories and can add cloud evidence.");
    println!(
        "AWS: {}. Default: current AWS profile and region; finds cloud resources and existing VPC flow logs.",
        if command_available("aws") {
            "CLI installed"
        } else {
            "CLI not found"
        }
    );
    println!(
        "Azure: {}. Default: current Azure CLI subscription; finds cloud resources and existing VNet Traffic Analytics.",
        if command_available("az") {
            "CLI installed"
        } else {
            "CLI not found"
        }
    );
    println!(
        "Cloud reads use your existing login and may incur CloudWatch or Azure Monitor charges. Nothing is enabled or created in the cloud."
    );
    println!("Network packets are not application calls; not every VM can be tied to a service.");
    let guided = !(args.aws || args.azure || args.repos_only || args.prometheus);
    if guided
        && std::io::stdin().is_terminal()
        && let Ok(project) = config::load_project(Path::new("."))
    {
        println!(
            "AWS configured scope: {}",
            aws_scope(&project.config.aws, false)
        );
        println!(
            "Azure configured scope: {}",
            azure_scope(&project.config.azure, false)
        );
        if project.config.prometheus.url != "https://prometheus.example.com"
            && !project.config.prometheus.url.is_empty()
        {
            println!("Prometheus endpoint is configured; you can choose it separately.");
        }
    }
    let (aws, azure) = if !guided {
        (args.aws, args.azure)
    } else {
        if !std::io::stdin().is_terminal() {
            return Err(Error::Usage("choose --repos-only, --prometheus, --aws, --azure, or combine sources; no cloud reads were started".into()));
        }
        println!("Choose: 1) repositories only  2) add AWS  3) add Azure  4) add both  0) cancel");
        print!("Selection: ");
        std::io::stdout().flush()?;
        let mut choice = String::new();
        std::io::stdin().read_line(&mut choice)?;
        match choice.trim() {
            "1" => (false, false),
            "2" => (true, false),
            "3" => (false, true),
            "4" => (true, true),
            "0" | "" => {
                println!("Discovery cancelled; no data was read.");
                return Ok(());
            }
            _ => {
                return Err(Error::Usage(
                    "invalid selection; run depgraph discover again".into(),
                ));
            }
        }
    };
    let project = config::load_project(Path::new(".")).map_err(|err| {
        Error::Config(format!(
            "{err}. Run depgraph init to create starter configuration."
        ))
    })?;
    let mut prometheus = args.prometheus;
    if guided
        && args.fixture.is_none()
        && !project.config.prometheus.url.is_empty()
        && project.config.prometheus.url != "https://prometheus.example.com"
    {
        println!(
            "Prometheus is configured at {}. Query it for Istio calls? [y/N]",
            project.config.prometheus.url
        );
        print!("Selection: ");
        std::io::stdout().flush()?;
        let mut choice = String::new();
        std::io::stdin().read_line(&mut choice)?;
        prometheus = match choice.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" => true,
            "" | "n" | "no" => false,
            _ => {
                return Err(Error::Usage(
                    "invalid selection; run depgraph discover again".into(),
                ));
            }
        };
    }
    if prometheus
        && args.prometheus_fixture.is_none()
        && project.config.prometheus.url == "https://prometheus.example.com"
    {
        return Err(Error::Config(
            "Prometheus still uses the example URL; set prometheus.url in config/config.yaml"
                .into(),
        ));
    }
    if args.fixture.is_none() {
        // Verify repository access before potentially chargeable cloud calls.
        crate::gitlab::resolve_token(&project.config.gitlab.base_url)?;
    }
    if aws {
        let scope = aws_scope(&project.config.aws, args.aws_fixture.is_some());
        println!(
            "AWS scope: {scope}; existing security groups, X-Ray when enabled, and eligible VPC flow logs."
        );
    }
    if azure {
        let scope = azure_scope(&project.config.azure, args.azure_fixture.is_some());
        println!(
            "Azure scope: {scope}; existing security groups, configured Application Insights and discoverable VNet Traffic Analytics."
        );
    }
    println!(
        "This run builds a fresh snapshot from the selected sources; failed sources do not contribute earlier data."
    );
    // A unique raw directory ensures opt-out doesn't accidentally reuse an older cloud scan.
    let run_dir = PathBuf::from("data/discovery").join(format!(
        "{}-{}",
        chrono::Utc::now().timestamp_millis(),
        std::process::id()
    ));
    run_collect(CollectArgs {
        set: CollectSetArg::Full,
        fixture: args.fixture,
        output: Some(run_dir.clone()),
        base_url: None,
        group: Vec::new(),
        project_path: Vec::new(),
        prometheus_fixture: args.prometheus_fixture,
        prometheus,
        aws,
        aws_flow_logs: aws,
        aws_fixture: args.aws_fixture,
        azure,
        azure_flow_logs: azure,
        azure_fixture: args.azure_fixture,
        skip_config_cloud: true,
    })?;
    run_build(BuildArgs {
        raw: Some(run_dir.join("raw")),
        output: None,
        aliases: None,
    })?;
    let graph: crate::models::GraphSnapshot =
        serde_json::from_slice(&fs::read("data/snapshot.json")?)
            .map_err(|_| Error::Store("could not read the discovery snapshot".into()))?;
    let report: normalize::merge::UnresolvedReport =
        serde_json::from_slice(&fs::read("data/unresolved.json")?)
            .map_err(|_| Error::Store("could not read the unresolved report".into()))?;
    let traffic = graph
        .edges
        .iter()
        .filter(|e| e.edge_type == crate::models::EdgeType::NetworkTraffic)
        .count();
    let observed = graph
        .edges
        .iter()
        .filter(|e| e.edge_type == crate::models::EdgeType::ObservedCalls)
        .count();
    println!(
        "Discovery: {} nodes, {} ties, {} observed calls, {} network traffic edges; {} unresolved.",
        graph.nodes.len(),
        graph.edges.len(),
        observed,
        traffic,
        report.records.len()
    );
    for (enabled, label, evidence) in [
        (aws, "AWS", "aws-vpc-flow-logs"),
        (azure, "Azure", "azure-vnet-traffic-analytics"),
    ] {
        if enabled {
            let count = graph
                .edges
                .iter()
                .filter(|edge| {
                    edge.edge_type == crate::models::EdgeType::NetworkTraffic
                        && edge
                            .sources
                            .iter()
                            .any(|source| source.evidence_ref.as_deref() == Some(evidence))
                })
                .count();
            println!(
                "{label} network evidence: {count} traffic edges. See warnings for missing sources or permissions."
            );
        }
    }
    if !prometheus {
        println!("Prometheus was not queried; Istio call evidence is not included in this run.");
    }
    if (aws || azure) && traffic == 0 {
        println!(
            "No network traffic edges were found. Logs may be unavailable, empty, unsupported or unmatched; check warnings above."
        );
    }
    println!(
        "Results: data/snapshot.json and data/unresolved.json. Packet direction does not prove call direction. Review unresolved names with depgraph suggest."
    );
    println!("Discovery raw data: {}", run_dir.join("raw").display());
    Ok(())
}

fn run_collect(args: CollectArgs) -> Result<(), Error> {
    let set = match args.set.into_set() {
        collectors::CollectSet::Fast => collectors::GatherSet::Fast,
        collectors::CollectSet::Full => collectors::GatherSet::Full,
    };
    let prometheus_only = args.fixture.is_none()
        && (args.prometheus_fixture.is_some()
            || args.prometheus
            || args.aws_fixture.is_some()
            || args.azure_fixture.is_some());
    let runtime = prometheus_records(&args)?;
    let cloud = cloud_records(&args)?;
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
    records.extend(cloud);
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

/// Opt-in AWS and Azure reads: a saved fixture, `--aws`/`--azure`, or
/// `enabled: true` in config. Nothing runs when none of these is set.
fn cloud_records(args: &CollectArgs) -> Result<Vec<crate::raw::RawRecord>, Error> {
    let project = config::load_project(Path::new(".")).ok();
    let aws = project
        .as_ref()
        .map(|p| p.config.aws.clone())
        .unwrap_or_default();
    let azure = project
        .as_ref()
        .map(|p| p.config.azure.clone())
        .unwrap_or_default();
    let uses_config = args.fixture.is_none() && !args.skip_config_cloud;
    let mut records = Vec::new();
    if let Some(dir) = &args.aws_fixture {
        records.extend(collectors::records_from_aws_dir(dir, aws.env.as_deref())?);
    } else if args.aws || args.aws_flow_logs || (uses_config && aws.enabled) {
        let live = collectors::AwsLive {
            profile: aws.profile.clone(),
            regions: aws.regions.clone(),
            env: aws.env.clone(),
            xray: aws.xray,
            flow_log_group: aws.flow_log_group.clone(),
            flow_logs: aws.flow_logs || args.aws_flow_logs,
        };
        let (found, warnings) = collectors::records_from_aws(&live, chrono::Utc::now())?;
        for warning in &warnings {
            eprintln!("warning: {warning}");
        }
        eprintln!("aws: {} records", found.len());
        records.extend(found);
    }
    if let Some(dir) = &args.azure_fixture {
        records.extend(collectors::records_from_azure_dir(
            dir,
            azure.env.as_deref(),
        )?);
    } else if args.azure || args.azure_flow_logs || (uses_config && azure.enabled) {
        let live = collectors::AzureLive {
            subscriptions: azure.subscriptions.clone(),
            workspaces: azure.log_analytics_workspaces.clone(),
            env: azure.env.clone(),
            flow_logs: azure.flow_logs || args.azure_flow_logs,
        };
        let (found, warnings) = collectors::records_from_azure(&live)?;
        for warning in &warnings {
            eprintln!("warning: {warning}");
        }
        eprintln!("azure: {} records", found.len());
        records.extend(found);
    }
    Ok(records)
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
    let unresolved_path = output.with_file_name("unresolved.json");
    fs::write(
        &unresolved_path,
        serde_json::to_string_pretty(&normalize::merge::UnresolvedReport {
            schema_version: "1.0.0".into(),
            records: built.unresolved.clone(),
        })
        .map_err(|err| Error::Store(err.to_string()))?,
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
    for line in namespace_hints(&built.unresolved, &environments) {
        eprintln!("{line}");
    }
    for line in guessed_namespaces(&records, &environments) {
        eprintln!("{line}");
    }
    match crate::store::archive_snapshot(&output) {
        Ok(path) => println!("archived {}", path.display()),
        Err(err) => eprintln!("warning: snapshot was not archived: {err}"),
    }
    Ok(())
}

/// With `guess_namespaces: true`, list every namespace whose environment was
/// guessed, so the guesses can be checked.
fn guessed_namespaces(
    records: &[crate::raw::RawRecord],
    environments: &config::EnvIndex,
) -> Vec<String> {
    let mut guessed: std::collections::BTreeMap<String, crate::models::Env> = Default::default();
    for record in records {
        for field in ["namespace", "source_namespace", "destination_namespace"] {
            if let Some(namespace) = record.payload.get(field).and_then(|value| value.as_str())
                && let Some(env) = environments.guessed_namespace(namespace)
            {
                guessed.insert(namespace.to_owned(), env);
            }
        }
    }
    if guessed.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!(
        "note: guess_namespaces is on; {} namespace(s) were guessed from their name:",
        guessed.len()
    )];
    for (namespace, env) in &guessed {
        lines.push(format!("  {namespace} -> {env}"));
    }
    lines.push(
        "  map a wrong guess under k8s_namespaces in config/environments.yaml; a mapping always wins."
            .to_string(),
    );
    lines
}

/// Setup hints for calls dropped because their namespace is not mapped.
fn namespace_hints(
    unresolved: &[normalize::merge::Unresolved],
    environments: &config::EnvIndex,
) -> Vec<String> {
    let mut dropped: std::collections::BTreeMap<String, u64> = Default::default();
    for row in unresolved
        .iter()
        .filter(|row| row.kind == "environment" && row.reason == "namespace is not mapped")
    {
        for namespace in row
            .name
            .split(" and ")
            .filter(|namespace| environments.resolve_namespace(namespace).is_none())
        {
            *dropped.entry(namespace.to_string()).or_default() += row.volume;
        }
    }
    if dropped.is_empty() {
        return Vec::new();
    }
    let mut lines = vec![format!(
        "hint: calls in {} namespace(s) were left out because config/environments.yaml does not map them:",
        dropped.len()
    )];
    let mut suggested: std::collections::BTreeMap<crate::models::Env, Vec<&str>> =
        Default::default();
    for (namespace, calls) in &dropped {
        let guess = environments.suggest_namespace(namespace);
        let advice = match guess {
            Some(env) => format!("looks like {env}"),
            None => "no guess; map it yourself or ignore it".to_string(),
        };
        lines.push(format!("  {namespace} ({calls} calls): {advice}"));
        if let Some(env) = guess {
            suggested.entry(env).or_default().push(namespace);
        }
    }
    if !suggested.is_empty() {
        lines.push(
            "  check the guesses, then add them under the matching keys in config/environments.yaml:"
                .to_string(),
        );
        for (env, namespaces) in &suggested {
            lines.push(format!("    {env}:"));
            lines.push(format!("      k8s_namespaces: [{}]", namespaces.join(", ")));
        }
    }
    lines.push(
        "  then run depgraph build again (docs/tutorials/07-troubleshooting.md).".to_string(),
    );
    lines
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
    let bind = args
        .bind
        .clone()
        .unwrap_or_else(|| project.config.mcp.bind.clone());
    let snapshot = args
        .snapshot
        .unwrap_or_else(|| PathBuf::from("data/snapshot.json"));
    let (policy, jwks) = crate::mcp::policy_from_config(&project.config)?;
    if args.insecure_local {
        eprintln!("next: depgraph mcp-config cursor");
    }
    crate::mcp::serve_blocking(
        &bind,
        args.insecure_local,
        policy,
        &jwks,
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
        blocking: args.blocking || project.config.mr_check.blocking,
        override_label: &project.config.mr_check.override_label,
        labels: &args.label,
    });
    if args.dry_run {
        println!("{}", note.body);
        if note.requires_approval {
            return Err(Error::Validation(
                "blocking MR check: extra approval or the override label is required".into(),
            ));
        }
        return Ok(());
    }
    if note.requires_approval {
        eprintln!("{}", note.body);
        return Err(Error::Validation(
            "blocking MR check: extra approval or the override label is required".into(),
        ));
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

fn run_init(args: InitArgs) -> Result<(), Error> {
    let root = args.dir.unwrap_or_else(|| PathBuf::from("."));
    let host = glab_host();
    let written = crate::quickstart::init(&root, host.as_deref(), args.force)?;
    if written.is_empty() {
        println!("config already exists. next: depgraph doctor");
    } else {
        for path in &written {
            println!("wrote {path}");
        }
        println!("next: depgraph doctor");
    }
    if host.is_none() {
        eprintln!("glab host was not detected. next: glab auth login, then set gitlab.base_url");
    }
    Ok(())
}

fn run_doctor(args: DoctorArgs) -> Result<(), Error> {
    let now = match args.now {
        Some(value) => crate::models::Timestamp::parse(value).map_err(Error::Validation)?,
        None => crate::models::Timestamp::parse(chrono::Utc::now().to_rfc3339())
            .map_err(Error::Validation)?,
    };
    let token_env_set = std::env::var("GITLAB_TOKEN")
        .map(|value| !value.trim().is_empty())
        .unwrap_or(false);
    let mut checks = crate::doctor::diagnose(Path::new("."), &now, probe_glab(), token_env_set);
    if args.online {
        checks.retain(|check| check.name != "connectivity");
        checks.push(online_gitlab());
    }
    let ok = crate::doctor::all_ok(&checks);
    print!("{}", crate::doctor::render(&checks));
    if ok {
        Ok(())
    } else {
        Err(Error::Config(
            "doctor found a problem. follow the next: lines above".into(),
        ))
    }
}

fn run_mcp_config(args: McpConfigArgs) -> Result<(), Error> {
    let project = config::load_project(Path::new(".")).ok();
    let bind = project
        .as_ref()
        .map(|item| item.config.mcp.bind.clone())
        .unwrap_or_else(|| "127.0.0.1:8080".to_owned());
    let entra = project
        .as_ref()
        .is_some_and(|item| item.config.auth.provider == config::AuthProvider::Entra);
    let body = crate::mcp::render_client_config(&args.client, &bind, entra)?;
    print!("{body}");
    if entra {
        eprintln!(
            "Entra is enabled. The client opens a browser. Do not paste a token. next: depgraph serve-mcp"
        );
    } else {
        eprintln!("next: depgraph serve-mcp --insecure-local --snapshot data/snapshot.json");
    }
    Ok(())
}

fn run_completions(args: CompletionArgs) -> Result<(), Error> {
    let mut command = Cli::command();
    clap_complete::generate(args.shell, &mut command, "depgraph", &mut std::io::stdout());
    Ok(())
}

fn run_runs(args: RunsArgs) -> Result<(), Error> {
    let data = args.data.unwrap_or_else(|| PathBuf::from("data"));
    for key in crate::store::list_runs(&data.join("runs"))? {
        println!("{key}");
    }
    Ok(())
}

fn run_retain(args: RetainArgs) -> Result<(), Error> {
    let data = args.data.unwrap_or_else(|| PathBuf::from("data"));
    let now = match args.now {
        Some(value) => crate::models::Timestamp::parse(value).map_err(Error::Validation)?,
        None => crate::models::Timestamp::parse(chrono::Utc::now().to_rfc3339())
            .map_err(Error::Validation)?,
    };
    let days = config::load_project(Path::new("."))
        .map(|project| project.config.retention_days)
        .unwrap_or(90);
    let removed = crate::store::retain(&data.join("runs"), &now, days)?;
    if removed.is_empty() {
        println!("removed 0 runs");
    } else {
        for key in removed {
            println!("removed {key}");
        }
    }
    Ok(())
}

fn run_timeline(args: TimelineArgs) -> Result<(), Error> {
    let deployments: Vec<crate::timeline::DeploymentFixture> =
        yaml_serde::from_str(&fs::read_to_string(&args.deployments)?)
            .map_err(|err| Error::Config(format!("deployments fixture: {err}")))?;
    let incidents = if let Some(path) = &args.incidents {
        yaml_serde::from_str(&fs::read_to_string(path)?)
            .map_err(|err| Error::Config(format!("incidents fixture: {err}")))?
    } else {
        Vec::new()
    };
    let environments =
        config::parse_environments(&fs::read_to_string("config/environments.yaml")?)?.index()?;
    let events = crate::timeline::correlate(
        &deployments,
        &incidents,
        args.env.into_env(),
        &args.since,
        |name| {
            environments.resolve(name).or(match name {
                "dev" => Some(crate::models::Env::Dev),
                "acc" => Some(crate::models::Env::Acc),
                "prod" => Some(crate::models::Env::Prod),
                _ => None,
            })
        },
    );
    for event in events {
        println!(
            "{}\t{}\t{}\t{}",
            event.at, event.env, event.kind, event.summary
        );
    }
    Ok(())
}

fn run_stats(args: StatsArgs) -> Result<(), Error> {
    let path = args
        .snapshot
        .unwrap_or_else(|| PathBuf::from("data/snapshot.json"));
    let graph: crate::models::GraphSnapshot = serde_json::from_str(&fs::read_to_string(&path)?)
        .map_err(|err| Error::Validation(err.to_string()))?;
    let stats = crate::score::measure(&graph);
    println!("active_edges {}", stats.active_edges);
    for (env, count) in &stats.active_edges_by_env {
        println!("active_edges_{env} {count}");
    }
    println!("query_p95_ms {:.3}", stats.query_p95_ms);
    println!(
        "next: paste these into docs/decisions/ADR-012-graph-database.md when measuring a real snapshot"
    );
    Ok(())
}

fn run_scorecard(args: ScorecardArgs) -> Result<(), Error> {
    let path = args
        .snapshot
        .unwrap_or_else(|| PathBuf::from("data/snapshot.json"));
    let graph: crate::models::GraphSnapshot = serde_json::from_str(&fs::read_to_string(&path)?)
        .map_err(|err| Error::Validation(err.to_string()))?;
    println!("owner\tcomponents\twith_system\twith_telemetry");
    for row in crate::score::scorecard(&graph) {
        println!(
            "{}\t{}\t{}\t{}",
            row.owner, row.components, row.with_system, row.with_telemetry
        );
    }
    println!("note: this is an improvement aid, not a ranking for blame");
    Ok(())
}

fn probe_glab() -> crate::doctor::GlabStatus {
    match std::process::Command::new("glab")
        .args(["auth", "status"])
        .output()
    {
        Err(_) => crate::doctor::GlabStatus::Missing,
        Ok(output) if output.status.success() => crate::doctor::GlabStatus::LoggedIn,
        Ok(_) => crate::doctor::GlabStatus::LoggedOut,
    }
}

fn glab_host() -> Option<String> {
    let output = std::process::Command::new("glab")
        .args(["config", "get", "host"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let host = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if host.is_empty() || host.contains([' ', '@']) {
        None
    } else {
        Some(host)
    }
}

fn online_gitlab() -> crate::doctor::Check {
    let project = match config::load_project(Path::new(".")) {
        Ok(project) => project,
        Err(err) => {
            return crate::doctor::Check {
                name: "connectivity",
                ok: false,
                detail: format!("{err}. next: depgraph init"),
            };
        }
    };
    let url = format!(
        "{}/api/v4/version",
        project.config.gitlab.base_url.trim_end_matches('/')
    );
    let runtime = match tokio_runtime() {
        Ok(runtime) => runtime,
        Err(err) => {
            return crate::doctor::Check {
                name: "connectivity",
                ok: false,
                detail: format!("{err}"),
            };
        }
    };
    let result = runtime.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|err| err.to_string())?;
        let response = client
            .get(&url)
            .send()
            .await
            .map_err(|_| "could not reach GitLab".to_owned())?;
        Ok::<_, String>(response.status())
    });
    match result {
        Ok(status) if status.is_success() => crate::doctor::Check {
            name: "connectivity",
            ok: true,
            detail: "GitLab /api/v4/version responded".to_owned(),
        },
        Ok(status) => crate::doctor::Check {
            name: "connectivity",
            ok: false,
            detail: format!(
                "GitLab returned {status}. next: check gitlab.base_url and glab auth login"
            ),
        },
        Err(err) => crate::doctor::Check {
            name: "connectivity",
            ok: false,
            detail: format!("{err}. next: check gitlab.base_url and the network"),
        },
    }
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
        from_run: args.from_run,
        to_run: args.to_run,
        date: args.date,
        runs: args.runs,
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
