mod cli;
mod coalesce;
mod csv_out;
mod fixes;

use std::collections::HashSet;
use std::fs::File;
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::process::ExitCode;

use clap::Parser;
use wd_core::Issue;

use crate::cli::Cli;
use crate::coalesce::{ProcessConfig, ProcessResult, UnfixableEntry};
use crate::fixes::{FIXABLE_CHECKS, FixCtx};

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

    let enabled_checks = match resolve_enabled(cli.enable.as_deref(), cli.disable.as_deref()) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(EXIT_BAD_ARGS);
        }
    };

    let fix_ctx = FixCtx {
        nationalities: rules.nationalities_lower.iter().cloned().collect(),
        trademark_chars: rules.trademark_chars.clone(),
    };

    let config = ProcessConfig {
        fix_ctx,
        enabled_checks,
        description_max_len: rules.thresholds.description_max_len,
    };

    let stdin = io::stdin();
    let (issues, parse_failures) = match read_issues(BufReader::new(stdin)) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("error: {e}");
            return ExitCode::from(EXIT_IO);
        }
    };

    let result = coalesce::process(issues, parse_failures, &config);

    let stdout = io::stdout();
    if let Err(e) = csv_out::write(BufWriter::new(stdout), &result, cli.annotate) {
        eprintln!("error: {e}");
        return ExitCode::from(EXIT_IO);
    }

    if let Err(e) = write_unfixable(&result, cli.unfixable.as_deref()) {
        eprintln!("error: {e}");
        return ExitCode::from(EXIT_IO);
    }

    if result.suppressed_count > 0 {
        eprintln!("suppressed {} no-op records", result.suppressed_count);
    }

    ExitCode::SUCCESS
}

fn resolve_enabled(
    enable: Option<&[String]>,
    disable: Option<&[String]>,
) -> anyhow::Result<HashSet<String>> {
    let mut enabled: HashSet<String> = if let Some(list) = enable {
        let mut set = HashSet::new();
        for raw in list {
            let id = raw.trim();
            if id.is_empty() {
                continue;
            }
            if !FIXABLE_CHECKS.contains(&id) {
                anyhow::bail!("--enable: not a fixable check id: {id}");
            }
            set.insert(id.to_string());
        }
        set
    } else {
        FIXABLE_CHECKS.iter().map(|s| (*s).to_string()).collect()
    };
    if let Some(list) = disable {
        for raw in list {
            let id = raw.trim();
            if id.is_empty() {
                continue;
            }
            if !FIXABLE_CHECKS.contains(&id) {
                anyhow::bail!("--disable: not a fixable check id: {id}");
            }
            enabled.remove(id);
        }
    }
    Ok(enabled)
}

fn read_issues<R: BufRead>(reader: R) -> io::Result<(Vec<Issue>, Vec<String>)> {
    let mut issues = Vec::new();
    let mut failures = Vec::new();
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Issue>(&line) {
            Ok(i) => issues.push(i),
            Err(_) => failures.push(line),
        }
    }
    Ok((issues, failures))
}

fn write_unfixable(result: &ProcessResult, path: Option<&std::path::Path>) -> anyhow::Result<()> {
    if result.unfixable.is_empty() {
        if path.is_some() {
            // Still create the file (empty) so callers see a deterministic artifact.
            if let Some(p) = path {
                File::create(p)?;
            }
        }
        return Ok(());
    }
    match path {
        Some(p) => {
            let mut w = BufWriter::new(File::create(p)?);
            for entry in &result.unfixable {
                serde_json::to_writer(&mut w, entry)?;
                w.write_all(b"\n")?;
            }
            w.flush()?;
        }
        None => {
            eprintln!(
                "{} unfixable records discarded (use --unfixable <path> to capture)",
                count_unfixable(&result.unfixable)
            );
        }
    }
    Ok(())
}

fn count_unfixable(entries: &[UnfixableEntry]) -> usize {
    entries.len()
}
