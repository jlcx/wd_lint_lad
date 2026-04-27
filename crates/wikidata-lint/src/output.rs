#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
#[clap(rename_all = "lowercase")]
pub enum OutputFormat {
    Jsonl,
    Csv,
    Tsv,
}
