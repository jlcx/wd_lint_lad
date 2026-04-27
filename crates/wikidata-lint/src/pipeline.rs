use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;
use std::io::{BufRead, Write};
use std::sync::Arc;
use std::thread;

use crossbeam_channel::{Receiver, Sender, bounded};
use wd_core::{Issue, entity::Entity};

use crate::checks::streaming::{self, StreamingCandidate, StreamingState};
use crate::checks::{self, CheckCtx, EnabledChecks};
use crate::matchers::CompiledRules;
use crate::output::OutputFormat;

pub struct ScannerConfig {
    pub compiled: Arc<CompiledRules>,
    pub enabled: Arc<EnabledChecks>,
    pub threads: usize,
    pub progress_interval: Option<u64>,
    pub verbose: bool,
}

#[derive(Debug)]
struct Batch {
    idx: u64,
    issues: Vec<Issue>,
    streaming: Option<StreamingCandidate>,
}

impl PartialEq for Batch {
    fn eq(&self, other: &Self) -> bool {
        self.idx == other.idx
    }
}
impl Eq for Batch {}
impl Ord for Batch {
    fn cmp(&self, other: &Self) -> Ordering {
        self.idx.cmp(&other.idx)
    }
}
impl PartialOrd for Batch {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub fn run<R, W>(
    config: ScannerConfig,
    input: R,
    output: W,
    format: OutputFormat,
) -> anyhow::Result<()>
where
    R: BufRead + Send,
    W: Write + Send,
{
    if !matches!(format, OutputFormat::Jsonl) {
        anyhow::bail!("output format {format:?} is not yet implemented (jsonl only for now)");
    }

    let n_workers = config.threads.max(1);
    let cap = (n_workers * 4).max(8);
    let (work_tx, work_rx) = bounded::<(u64, Vec<u8>)>(cap);
    let (out_tx, out_rx) = bounded::<Batch>(cap);

    thread::scope(|s| -> anyhow::Result<()> {
        let reader_handle = {
            let work_tx = work_tx.clone();
            s.spawn(move || reader_loop(input, work_tx))
        };
        drop(work_tx);

        let mut worker_handles = Vec::with_capacity(n_workers);
        for _ in 0..n_workers {
            let work_rx = work_rx.clone();
            let out_tx = out_tx.clone();
            let compiled = config.compiled.clone();
            let enabled = config.enabled.clone();
            let verbose = config.verbose;
            worker_handles.push(s.spawn(move || worker_loop(work_rx, out_tx, compiled, enabled, verbose)));
        }
        drop(work_rx);
        drop(out_tx);

        let writer_handle = s.spawn(move || writer_loop(out_rx, output, config.progress_interval));

        let reader_result = reader_handle.join().expect("reader panicked");
        for h in worker_handles {
            h.join().expect("worker panicked");
        }
        let writer_result = writer_handle.join().expect("writer panicked");

        reader_result?;
        writer_result?;
        Ok(())
    })
}

fn reader_loop<R: BufRead>(
    mut input: R,
    work_tx: Sender<(u64, Vec<u8>)>,
) -> anyhow::Result<()> {
    const INITIAL_LINE_CAP: usize = 8 * 1024;
    let mut buf: Vec<u8> = Vec::with_capacity(INITIAL_LINE_CAP);
    let mut idx: u64 = 0;
    loop {
        buf.clear();
        let n = input.read_until(b'\n', &mut buf)?;
        if n == 0 {
            break;
        }
        // Trim trailing \r\n.
        while matches!(buf.last(), Some(b'\n' | b'\r')) {
            buf.pop();
        }
        // Frame-line filter: ignore '[', ']', or all-whitespace.
        let body = trim_ascii_whitespace(&buf);
        if body.is_empty() || body == b"[" || body == b"]" {
            continue;
        }
        // Strip a trailing ',' (not preceded by whitespace, since we stripped \r\n).
        if buf.last() == Some(&b',') {
            buf.pop();
        }
        // Move ownership to the worker; allocate a fresh buffer for the next line.
        let bytes = std::mem::replace(&mut buf, Vec::with_capacity(INITIAL_LINE_CAP));
        if work_tx.send((idx, bytes)).is_err() {
            break;
        }
        idx = idx.wrapping_add(1);
    }
    Ok(())
}

fn trim_ascii_whitespace(b: &[u8]) -> &[u8] {
    let start = b.iter().position(|&c| !c.is_ascii_whitespace()).unwrap_or(b.len());
    let end = b
        .iter()
        .rposition(|&c| !c.is_ascii_whitespace())
        .map(|i| i + 1)
        .unwrap_or(0);
    if start >= end {
        &[]
    } else {
        &b[start..end]
    }
}

fn worker_loop(
    work_rx: Receiver<(u64, Vec<u8>)>,
    out_tx: Sender<Batch>,
    compiled: Arc<CompiledRules>,
    enabled: Arc<EnabledChecks>,
    verbose: bool,
) {
    let aliases_enabled = enabled.contains("aliases.long");
    let descriptions_long_enabled = enabled.contains("descriptions.long");
    let ctx = CheckCtx {
        compiled: compiled.as_ref(),
        enabled: enabled.as_ref(),
    };
    while let Ok((idx, bytes)) = work_rx.recv() {
        let mut issues = Vec::new();
        let streaming = match serde_json::from_slice::<Entity>(&bytes) {
            Ok(entity) => {
                checks::run_all(&entity, &ctx, &mut issues);
                streaming::build_candidate(
                    &entity,
                    ctx.compiled,
                    aliases_enabled,
                    descriptions_long_enabled,
                )
            }
            Err(e) => {
                if verbose {
                    eprintln!("parse error at line index {idx}: {e}");
                }
                None
            }
        };
        if out_tx.send(Batch { idx, issues, streaming }).is_err() {
            break;
        }
    }
}

fn writer_loop<W: Write>(
    rx: Receiver<Batch>,
    mut output: W,
    progress_interval: Option<u64>,
) -> anyhow::Result<()> {
    let mut buffer: BinaryHeap<Reverse<Batch>> = BinaryHeap::new();
    let mut next: u64 = 0;
    let mut entity_count: u64 = 0;
    let mut state = StreamingState::default();
    let mut streaming_buf: Vec<Issue> = Vec::new();

    while let Ok(batch) = rx.recv() {
        buffer.push(Reverse(batch));
        while let Some(Reverse(top)) = buffer.peek() {
            if top.idx != next {
                break;
            }
            let Reverse(batch) = buffer.pop().unwrap();
            for issue in &batch.issues {
                emit_jsonl(&mut output, issue)?;
            }
            if let Some(candidate) = &batch.streaming {
                streaming_buf.clear();
                streaming::apply(candidate, &mut state, &mut streaming_buf);
                for issue in &streaming_buf {
                    emit_jsonl(&mut output, issue)?;
                }
            }
            next = next.wrapping_add(1);
            entity_count = entity_count.wrapping_add(1);
            if let Some(every) = progress_interval
                && every > 0
                && entity_count % every == 0
            {
                eprintln!("progress: {entity_count} entities");
            }
        }
    }
    output.flush()?;
    Ok(())
}

fn emit_jsonl<W: Write>(out: &mut W, issue: &Issue) -> anyhow::Result<()> {
    serde_json::to_writer(&mut *out, issue)?;
    out.write_all(b"\n")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use wd_core::Rules;

    fn rules_with_max(max: usize) -> Rules {
        let json = format!(
            r#"{{
                "nationalities_lower": [],
                "misspellings": {{}},
                "bad_starts_descriptions": [],
                "marketing_imperatives": [],
                "promotional_substrings": [],
                "promotional_exempt_substrings": [],
                "trademark_chars": [],
                "html_entity_substrings": [],
                "multi_sentence_markers": [],
                "obituary_markers": [],
                "thresholds": {{ "description_max_len": {max}, "descgust_score_threshold": 4 }}
            }}"#
        );
        serde_json::from_str(&json).unwrap()
    }

    /// Default helper for the existing too-long tests: enables only
    /// `description.too_long` so streaming checks don't add records.
    fn run_pipeline(dump: &str, threads: usize) -> String {
        run_with(rules_with_max(20), &["description.too_long"], dump, threads)
    }

    #[test]
    fn detects_too_long_description_preserves_input_order() {
        let dump = "[\n\
{\"id\":\"Q1\",\"descriptions\":{\"en\":{\"language\":\"en\",\"value\":\"short\"}}},\n\
{\"id\":\"Q2\",\"descriptions\":{\"en\":{\"language\":\"en\",\"value\":\"this description well exceeds the configured limit\"}}},\n\
{\"id\":\"Q3\",\"descriptions\":{\"en-gb\":{\"language\":\"en-gb\",\"value\":\"another way too long description for the threshold\"}}}\n\
]\n";
        let out = run_pipeline(dump, 4);
        let lines: Vec<_> = out.lines().collect();
        assert_eq!(lines.len(), 2, "got: {out}");
        assert!(lines[0].contains("\"qid\":\"Q2\""));
        assert!(lines[0].contains("\"check\":\"description.too_long\""));
        assert!(lines[0].contains("\"suggestion\":null"));
        assert!(lines[1].contains("\"qid\":\"Q3\""));
        assert!(lines[1].contains("\"lang\":\"en-gb\""));
    }

    #[test]
    fn skips_framing_lines() {
        let dump = "[\n]\n";
        let out = run_pipeline(dump, 1);
        assert!(out.is_empty());
    }

    #[test]
    fn skips_non_english_descriptions() {
        let dump = "[\n\
{\"id\":\"Q1\",\"descriptions\":{\"de\":{\"language\":\"de\",\"value\":\"this would trigger if it were english\"}}}\n\
]\n";
        let out = run_pipeline(dump, 1);
        assert!(out.is_empty(), "got: {out}");
    }

    fn run_with(rules: Rules, enabled_ids: &[&str], dump: &str, threads: usize) -> String {
        let compiled = crate::matchers::CompiledRules::compile(&rules).unwrap();
        let enabled = EnabledChecks::from_list(enabled_ids).unwrap();
        let config = ScannerConfig {
            compiled: Arc::new(compiled),
            enabled: Arc::new(enabled),
            threads,
            progress_interval: None,
            verbose: false,
        };
        let mut output: Vec<u8> = Vec::new();
        run(config, Cursor::new(dump.as_bytes()), &mut output, OutputFormat::Jsonl).unwrap();
        String::from_utf8(output).unwrap()
    }

    fn rules_for_streaming() -> Rules {
        // Generous threshold so description.too_long never fires.
        serde_json::from_str(
            r#"{
                "nationalities_lower": [],
                "misspellings": {},
                "bad_starts_descriptions": [],
                "marketing_imperatives": [],
                "promotional_substrings": [],
                "promotional_exempt_substrings": [],
                "trademark_chars": [],
                "html_entity_substrings": [],
                "multi_sentence_markers": [],
                "obituary_markers": [],
                "thresholds": {"description_max_len": 1000, "descgust_score_threshold": 100}
            }"#,
        )
        .unwrap()
    }

    #[test]
    fn descriptions_long_emits_strictly_increasing_max() {
        let dump = "[\n\
{\"id\":\"Q1\",\"descriptions\":{\"en\":{\"language\":\"en\",\"value\":\"short\"}}},\n\
{\"id\":\"Q2\",\"descriptions\":{\"en\":{\"language\":\"en\",\"value\":\"a longer description\"}}},\n\
{\"id\":\"Q3\",\"descriptions\":{\"en\":{\"language\":\"en\",\"value\":\"x\"}}},\n\
{\"id\":\"Q4\",\"descriptions\":{\"en\":{\"language\":\"en\",\"value\":\"this is the longest of them all\"}}}\n\
]\n";
        let out = run_with(rules_for_streaming(), &["descriptions.long"], dump, 4);
        let lines: Vec<_> = out.lines().collect();
        assert_eq!(lines.len(), 3, "got: {out}");
        assert!(lines[0].contains("\"qid\":\"Q1\"") && lines[0].contains("\"new_max_len\":5"));
        assert!(lines[1].contains("\"qid\":\"Q2\"") && lines[1].contains("\"new_max_len\":20"));
        assert!(lines[2].contains("\"qid\":\"Q4\"") && lines[2].contains("\"new_max_len\":31"));
    }

    #[test]
    fn aliases_long_respects_p31_skip_and_skip_qids() {
        let mut rules = rules_for_streaming();
        rules.excluded_p31_for_long_aliases = ["Q5".to_string()].into_iter().collect();
        rules
            .skip_qids
            .insert("long_aliases".into(), ["Q3".to_string()].into_iter().collect());
        let dump = "[\n\
{\"id\":\"Q1\",\"claims\":{\"P31\":[{\"mainsnak\":{\"datavalue\":{\"value\":{\"id\":\"Q42\"}}}}]},\"aliases\":{\"en\":[{\"language\":\"en\",\"value\":\"hello\"}]}},\n\
{\"id\":\"Q2\",\"claims\":{\"P31\":[{\"mainsnak\":{\"datavalue\":{\"value\":{\"id\":\"Q5\"}}}}]},\"aliases\":{\"en\":[{\"language\":\"en\",\"value\":\"this is much longer and would otherwise win\"}]}},\n\
{\"id\":\"Q3\",\"claims\":{\"P31\":[{\"mainsnak\":{\"datavalue\":{\"value\":{\"id\":\"Q42\"}}}}]},\"aliases\":{\"en\":[{\"language\":\"en\",\"value\":\"this should also be skipped via skip_qids\"}]}},\n\
{\"id\":\"Q4\",\"aliases\":{\"en\":[{\"language\":\"en\",\"value\":\"no p31 ineligible\"}]}},\n\
{\"id\":\"Q6\",\"claims\":{\"P31\":[{\"mainsnak\":{\"datavalue\":{\"value\":{\"id\":\"Q42\"}}}}]},\"aliases\":{\"en\":[{\"language\":\"en\",\"value\":\"longer alias here for sure\"}]}}\n\
]\n";
        let out = run_with(rules, &["aliases.long"], dump, 4);
        let lines: Vec<_> = out.lines().collect();
        // Only Q1 (5) and Q6 (26) are eligible AND set new highs.
        assert_eq!(lines.len(), 2, "got: {out}");
        assert!(lines[0].contains("\"qid\":\"Q1\"") && lines[0].contains("\"check\":\"aliases.long\""));
        assert!(lines[0].contains("\"field\":\"alias\""));
        assert!(lines[0].contains("\"new_max_len\":5"));
        assert!(lines[1].contains("\"qid\":\"Q6\"") && lines[1].contains("\"new_max_len\":26"));
    }

    #[test]
    fn tolerates_unparseable_lines() {
        let dump = "[\n\
not valid json,\n\
{\"id\":\"Q2\",\"descriptions\":{\"en\":{\"language\":\"en\",\"value\":\"this description well exceeds the configured limit\"}}}\n\
]\n";
        let out = run_pipeline(dump, 2);
        let lines: Vec<_> = out.lines().collect();
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("\"qid\":\"Q2\""));
    }
}
