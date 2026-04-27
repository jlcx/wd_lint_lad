use std::path::{Path, PathBuf};

use assert_cmd::Command;

const FIXABLE_CHECKS: &str = "description.misspelled,\
description.starts_with_lowercase_nationality,\
description.contains_lowercase_nationality,\
description.contains_html_entity,\
description.contains_double_space,\
description.space_before_comma,\
description.contains_trademark,\
description.ends_with_punctuation,\
description.starts_with_label,\
description.composite";

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
}

fn run_scanner(rules: &Path, dump: Vec<u8>, checks: Option<&str>) -> Vec<u8> {
    let mut cmd = Command::cargo_bin("wikidata-lint").unwrap();
    cmd.arg("--rules").arg(rules);
    if let Some(c) = checks {
        cmd.arg("--checks").arg(c);
    }
    let output = cmd.write_stdin(dump).output().unwrap();
    assert!(
        output.status.success(),
        "scanner exited {:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}

fn run_fixer(rules: &Path, jsonl: Vec<u8>, output_dir: &Path, unfixable_path: &Path) {
    let output = Command::cargo_bin("wikidata-fix")
        .unwrap()
        .arg("--rules")
        .arg(rules)
        .arg("--output-dir")
        .arg(output_dir)
        .arg("--unfixable")
        .arg(unfixable_path)
        .write_stdin(jsonl)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "fixer exited {:?}: {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Concatenate the per-column CSV files in `dir`, sorted by filename,
/// with `===== <name> =====` separators, into a single deterministic
/// string for snapshotting.
fn read_split_csvs(dir: &Path) -> String {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap())
        .map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let body = std::fs::read_to_string(e.path()).unwrap();
            (name, body)
        })
        .collect();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut out = String::new();
    for (name, body) in entries {
        out.push_str("===== ");
        out.push_str(&name);
        out.push_str(" =====\n");
        out.push_str(&body);
    }
    out
}

#[test]
fn scanner_full_run_against_fixture() {
    let dir = fixtures_dir();
    let dump = std::fs::read(dir.join("dump.json")).unwrap();
    let rules = dir.join("rules.json");
    let stdout = run_scanner(&rules, dump, None);
    let s = String::from_utf8(stdout).unwrap();
    insta::assert_snapshot!("scanner_all_checks", s);
}

#[test]
fn roundtrip_filtered_to_fixable_produces_clean_csv() {
    let dir = fixtures_dir();
    let dump = std::fs::read(dir.join("dump.json")).unwrap();
    let rules = dir.join("rules.json");

    let scanner_out = run_scanner(&rules, dump, Some(FIXABLE_CHECKS));

    let output_dir = tempfile::tempdir().unwrap();
    let unfixable = tempfile::NamedTempFile::new().unwrap();
    run_fixer(&rules, scanner_out, output_dir.path(), unfixable.path());

    let csv_str = read_split_csvs(output_dir.path());
    insta::assert_snapshot!("roundtrip_filtered_csv", csv_str);

    let unfixable_str = std::fs::read_to_string(unfixable.path()).unwrap();
    insta::assert_snapshot!("roundtrip_filtered_unfixable", unfixable_str);
}

#[test]
fn roundtrip_full_scanner_routes_mixed_groups_to_unfixable() {
    let dir = fixtures_dir();
    let dump = std::fs::read(dir.join("dump.json")).unwrap();
    let rules = dir.join("rules.json");

    let scanner_out = run_scanner(&rules, dump, None);

    let output_dir = tempfile::tempdir().unwrap();
    let unfixable = tempfile::NamedTempFile::new().unwrap();
    run_fixer(&rules, scanner_out, output_dir.path(), unfixable.path());

    let csv_str = read_split_csvs(output_dir.path());
    insta::assert_snapshot!("roundtrip_full_csv", csv_str);

    let unfixable_str = std::fs::read_to_string(unfixable.path()).unwrap();
    insta::assert_snapshot!("roundtrip_full_unfixable", unfixable_str);
}
