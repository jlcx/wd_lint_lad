mod checks;
mod cli;
mod matchers;
mod output;
mod pipeline;

use std::fs::File;
use std::io::{self, BufReader, BufWriter};
use std::process::ExitCode;
use std::sync::Arc;

use clap::Parser;

use crate::checks::EnabledChecks;
use crate::cli::Cli;
use crate::matchers::CompiledRules;
use crate::pipeline::ScannerConfig;

const EXIT_BAD_ARGS: u8 = 2;
const EXIT_IO: u8 = 3;

fn main() -> ExitCode {
    let cli = Cli::parse();

    let rules = match wd_core::load_rules(&cli.rules) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(EXIT_BAD_ARGS);
        }
    };

    let compiled = match CompiledRules::compile(&rules) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: failed to compile rules: {e}");
            return ExitCode::from(EXIT_BAD_ARGS);
        }
    };

    let enabled = match cli.checks.as_deref() {
        None => EnabledChecks::all(),
        Some(items) => match EnabledChecks::from_list(items) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("error: {e}");
                return ExitCode::from(EXIT_BAD_ARGS);
            }
        },
    };

    let threads = cli.threads.unwrap_or_else(|| {
        std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1)
    });

    let config = ScannerConfig {
        compiled: Arc::new(compiled),
        enabled: Arc::new(enabled),
        threads,
        progress_interval: cli.progress.then_some(cli.progress_interval),
        verbose: cli.verbose,
    };

    let input = BufReader::new(io::stdin());

    let result: anyhow::Result<()> = match cli.output.as_deref() {
        None => pipeline::run(config, input, BufWriter::new(io::stdout()), cli.format),
        Some(path) => match File::create(path) {
            Ok(f) => pipeline::run(config, input, BufWriter::new(f), cli.format),
            Err(e) => Err(anyhow::anyhow!(
                "failed to open output {}: {e}",
                path.display()
            )),
        },
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::from(EXIT_IO)
        }
    }
}
