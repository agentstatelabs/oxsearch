//! `oxsearch`: export an index to NDJSON, import NDJSON into an index, and
//! transfer documents between indices or clusters.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use oxsearch_cli::{
    ExportOptions, ImportOptions, Input, Output, Report, Stopped, TransferOptions, connect, export,
    import, redacted, transfer,
};
use serde_json::Value;

/// Export, import and transfer Elasticsearch documents.
///
/// Credentials come from the environment, never from arguments:
/// OXSEARCH_API_KEY, or OXSEARCH_USERNAME and OXSEARCH_PASSWORD; for a
/// transfer's destination, the same names with OXSEARCH_TO_. Progress goes
/// to standard error, and through the `metrics` facade to an OpenTelemetry
/// collector when OTEL_EXPORTER_OTLP_ENDPOINT is set.
#[derive(Parser)]
#[command(name = "oxsearch", version, about, long_about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write an index's documents to NDJSON: one record per line, with
    /// each document's index, id and routing, so an import puts it back.
    Export(ExportArgs),
    /// Write NDJSON into an index: records as an export writes them, or
    /// plain documents.
    Import(ImportArgs),
    /// Copy documents to another index or cluster, with their ids and
    /// routing.
    Transfer(TransferArgs),
}

#[derive(Args)]
struct Cluster {
    /// The cluster's URL. Its value from the environment isn't shown,
    /// since a URL can hold a password.
    #[arg(
        long,
        env = "OXSEARCH_URL",
        hide_env_values = true,
        default_value = "http://localhost:9200"
    )]
    url: String,
}

#[derive(Args)]
struct Reading {
    /// Only the documents this query matches, as JSON.
    #[arg(long, value_parser = json)]
    query: Option<Value>,
    /// Slices of the point in time read at once.
    #[arg(long, default_value_t = 1)]
    slices: u32,
    /// Documents in each page read.
    #[arg(long, default_value_t = 1000)]
    page_size: i32,
    /// How long the point in time stays open between pages.
    #[arg(long, default_value = "1m")]
    keep_alive: String,
}

#[derive(Args)]
struct Writing {
    /// Documents in each bulk request.
    #[arg(long, default_value_t = 1000)]
    batch_size: usize,
    /// Bulk requests in flight at once.
    #[arg(long, default_value_t = 2)]
    concurrency: usize,
}

#[derive(Args)]
struct Keeping {
    /// Keep one document in every N, from the first.
    #[arg(long, default_value_t = 1, value_name = "N")]
    every: u64,
    /// Keep at most N documents.
    #[arg(long, value_name = "N")]
    limit: Option<u64>,
}

#[derive(Args)]
struct ExportArgs {
    #[command(flatten)]
    cluster: Cluster,
    /// The index, alias or pattern to export.
    #[arg(long)]
    index: String,
    /// The file to write, which mustn't exist yet; `-` is standard output.
    #[arg(short, long, default_value = "-")]
    output: PathBuf,
    /// Compress with gzip; a file name ending in `.gz` is compressed anyway.
    #[arg(long)]
    gzip: bool,
    /// Write the documents themselves, without their ids and routing.
    #[arg(long)]
    plain: bool,
    #[command(flatten)]
    reading: Reading,
    #[command(flatten)]
    keeping: Keeping,
}

#[derive(Args)]
struct ImportArgs {
    #[command(flatten)]
    cluster: Cluster,
    /// The index to write. Without it, each record goes back to its own.
    #[arg(long)]
    index: Option<String>,
    /// The file to read, plain or gzip; `-` is standard input.
    #[arg(short, long, default_value = "-")]
    input: PathBuf,
    /// The lines are documents themselves, not records.
    #[arg(long)]
    plain: bool,
    /// For plain documents, the field that holds each one's id.
    #[arg(long, requires = "plain")]
    id_field: Option<String>,
    #[command(flatten)]
    writing: Writing,
    #[command(flatten)]
    keeping: Keeping,
}

#[derive(Args)]
struct TransferArgs {
    #[command(flatten)]
    cluster: Cluster,
    /// The index, alias or pattern to copy.
    #[arg(long)]
    index: String,
    /// The destination cluster's URL, if it's another cluster.
    #[arg(long, env = "OXSEARCH_TO_URL", hide_env_values = true)]
    to_url: Option<String>,
    /// The index to write. Without it, each document goes to an index of
    /// the name it was read from, in the other cluster.
    #[arg(long)]
    to_index: Option<String>,
    #[command(flatten)]
    reading: Reading,
    #[command(flatten)]
    writing: Writing,
    #[command(flatten)]
    keeping: Keeping,
}

fn json(text: &str) -> Result<Value, String> {
    serde_json::from_str(text).map_err(|e| e.to_string())
}

fn stdio(path: PathBuf) -> Option<PathBuf> {
    (path.as_os_str() != "-").then_some(path)
}

async fn run(command: Command) -> Result<Report, Stopped> {
    let usage = |message: String| Stopped {
        error: oxsearch_cli::Error::Usage(message),
        report: Report::default(),
    };
    let client = |url: &str, prefix: &str| {
        connect(url, prefix).map_err(|error| Stopped {
            error,
            report: Report::default(),
        })
    };
    match command {
        Command::Export(args) => {
            let mut options = ExportOptions::new(args.index);
            options.query = args.reading.query;
            options.slices = args.reading.slices;
            options.page_size = args.reading.page_size;
            options.keep_alive = args.reading.keep_alive;
            options.every = args.keeping.every;
            options.limit = args.keeping.limit;
            options.plain = args.plain;
            let output = match stdio(args.output) {
                Some(path) => Output::file(path),
                None => Output::stdout(),
            };
            let output = if args.gzip { output.gzip(true) } else { output };
            tracing::info!(
                url = redacted(&args.cluster.url),
                index = options.index,
                "exporting"
            );
            export(&client(&args.cluster.url, "OXSEARCH_")?, &options, &output).await
        }
        Command::Import(args) => {
            let mut options = ImportOptions::new();
            options.index = args.index;
            options.plain = args.plain;
            options.id_field = args.id_field;
            options.batch_size = args.writing.batch_size;
            options.concurrency = args.writing.concurrency;
            options.every = args.keeping.every;
            options.limit = args.keeping.limit;
            let input = match stdio(args.input) {
                Some(path) => Input::file(path),
                None => Input::stdin(),
            };
            tracing::info!(url = redacted(&args.cluster.url), "importing");
            import(&client(&args.cluster.url, "OXSEARCH_")?, &options, &input).await
        }
        Command::Transfer(args) => {
            let same_cluster = args.to_url.is_none();
            if same_cluster && args.to_index.as_ref().is_none_or(|to| *to == args.index) {
                return Err(usage(
                    "a transfer within one cluster needs another index: set --to-index".to_owned(),
                ));
            }
            let mut options = TransferOptions::new(args.index);
            options.to_index = args.to_index;
            options.query = args.reading.query;
            options.slices = args.reading.slices;
            options.page_size = args.reading.page_size;
            options.keep_alive = args.reading.keep_alive;
            options.batch_size = args.writing.batch_size;
            options.concurrency = args.writing.concurrency;
            options.every = args.keeping.every;
            options.limit = args.keeping.limit;
            let from = client(&args.cluster.url, "OXSEARCH_")?;
            match &args.to_url {
                Some(url) => {
                    tracing::info!(
                        from = redacted(&args.cluster.url),
                        to = redacted(url),
                        "transferring"
                    );
                    transfer(&from, &client(url, "OXSEARCH_TO_")?, &options).await
                }
                None => transfer(&from, &from, &options).await,
            }
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let telemetry = match oxtel::init() {
        Ok(guard) => guard,
        Err(e) => {
            eprintln!("oxsearch: telemetry: {e}");
            return ExitCode::FAILURE;
        }
    };
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("oxsearch: {e}");
            return ExitCode::FAILURE;
        }
    };
    let outcome = runtime.block_on(run(cli.command));
    drop(runtime);
    drop(telemetry);
    match outcome {
        Ok(report) => {
            eprintln!("oxsearch: {report}");
            if report.is_ok() {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(stopped) => {
            eprintln!("oxsearch: {}", stopped.report);
            eprintln!("oxsearch: stopped: {}", stopped.error);
            ExitCode::FAILURE
        }
    }
}
