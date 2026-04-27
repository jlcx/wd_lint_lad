use std::path::PathBuf;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    version,
    about = "Convert wikidata-lint JSONL into a QuickStatements CSV batch."
)]
pub struct Cli {
    /// Path to the rules JSON file (same one the scanner used).
    #[arg(long)]
    pub rules: PathBuf,

    /// Directory to write per-column CSV files into. Each
    /// `(field, language)` combination produces one file
    /// (`Den.csv`, `Den-gb.csv`, `Lfr.csv`, `Aen.csv`, ...) with a
    /// dense `qid,<column>` shape. Created if it does not exist.
    #[arg(long)]
    pub output_dir: PathBuf,

    /// Comma-separated check IDs to enable (default: all fixable).
    #[arg(long, value_delimiter = ',')]
    pub enable: Option<Vec<String>>,

    /// Comma-separated check IDs to disable.
    #[arg(long, value_delimiter = ',')]
    pub disable: Option<Vec<String>>,

    /// Where to write detection-only and skipped records (default: discard with stderr count).
    #[arg(long)]
    pub unfixable: Option<PathBuf>,
}
