use std::path::PathBuf;

use clap::Parser;

use crate::output::OutputFormat;

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Stream a Wikidata JSON dump and emit issues found in labels, descriptions, and aliases."
)]
pub struct Cli {
    /// Path to the rules JSON file.
    #[arg(long)]
    pub rules: PathBuf,

    /// Comma-separated check IDs to enable. Default: all known checks.
    #[arg(long, value_delimiter = ',')]
    pub checks: Option<Vec<String>>,

    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Jsonl)]
    pub format: OutputFormat,

    /// Output path (default: stdout).
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// Emit a progress line to stderr every --progress-interval entities.
    #[arg(long)]
    pub progress: bool,

    /// Entities per progress line.
    #[arg(long, default_value_t = 1_000_000)]
    pub progress_interval: u64,

    /// Number of parsing threads. Default: available parallelism.
    #[arg(long)]
    pub threads: Option<usize>,

    /// Log non-fatal events (e.g. parse errors) to stderr.
    #[arg(long, short = 'v')]
    pub verbose: bool,
}
