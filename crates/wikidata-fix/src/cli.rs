use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Convert wikidata-lint JSONL into a QuickStatements v2 / CSV batch."
)]
pub struct Cli {
    /// Path to the rules JSON file (same one the scanner used).
    #[arg(long)]
    pub rules: PathBuf,

    /// Comma-separated check IDs to enable (default: all fixable).
    #[arg(long, value_delimiter = ',')]
    pub enable: Option<Vec<String>>,

    /// Comma-separated check IDs to disable.
    #[arg(long, value_delimiter = ',')]
    pub disable: Option<Vec<String>>,

    /// Where to write detection-only and skipped records (default: discard with stderr count).
    #[arg(long)]
    pub unfixable: Option<PathBuf>,

    /// Add a trailing `notes` column listing contributing check IDs.
    #[arg(long)]
    pub annotate: bool,
}
