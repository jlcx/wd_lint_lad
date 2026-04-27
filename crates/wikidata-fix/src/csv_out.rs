//! Per-column CSV emit.
//!
//! Real-world QuickStatements interprets empty cells in `Lxx`/`Dxx`/
//! `Axx` columns as "set this field to empty", which destructively
//! blanks fields we never intended to touch. To avoid that, the fixer
//! emits **one CSV per `(field, lang)` column**, each with header
//! `qid,<column>` and rows only for items that have a fix for that
//! column. No empty cells anywhere in the output.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::BufWriter;
use std::path::Path;

use crate::coalesce::{CellOut, ProcessResult};

pub fn write_split(dir: &Path, result: &ProcessResult) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    // Group by column so we open each file once. Cells within a
    // column are appended in their original order, which preserves
    // the first-seen-qid ordering from stage 1.
    let mut by_column: BTreeMap<&str, Vec<&CellOut>> = BTreeMap::new();
    for cell in &result.cells {
        by_column.entry(cell.column.as_str()).or_default().push(cell);
    }
    for (column, cells) in by_column {
        let path = dir.join(format!("{column}.csv"));
        let writer = BufWriter::new(File::create(&path)?);
        let mut csv_writer = csv::Writer::from_writer(writer);
        csv_writer.write_record(["qid", column])?;
        for cell in cells {
            csv_writer.write_record([cell.qid.as_str(), cell.value.as_str()])?;
        }
        csv_writer.flush()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn cell(qid: &str, column: &str, value: &str) -> CellOut {
        CellOut {
            qid: qid.into(),
            column: column.into(),
            value: value.into(),
        }
    }

    fn read_dir_sorted(dir: &Path) -> Vec<(String, String)> {
        let mut entries: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap())
            .map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                let body = fs::read_to_string(e.path()).unwrap();
                (name, body)
            })
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        entries
    }

    #[test]
    fn one_file_per_column_dense_rows() {
        let dir = tempfile::tempdir().unwrap();
        let result = ProcessResult {
            cells: vec![
                cell("Q01", "Den", "thing"),
                cell("Q02", "Den-gb", "British thing"),
                cell("Q03", "Den", "Palestinian writer"),
            ],
            unfixable: vec![],
            suppressed_count: 0,
        };
        write_split(dir.path(), &result).unwrap();
        let files = read_dir_sorted(dir.path());
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].0, "Den-gb.csv");
        assert_eq!(files[0].1, "qid,Den-gb\nQ02,British thing\n");
        assert_eq!(files[1].0, "Den.csv");
        assert_eq!(files[1].1, "qid,Den\nQ01,thing\nQ03,Palestinian writer\n");
    }

    #[test]
    fn rfc4180_quoting_for_special_chars() {
        let dir = tempfile::tempdir().unwrap();
        let result = ProcessResult {
            cells: vec![cell("Q01", "Den", "has, a comma")],
            unfixable: vec![],
            suppressed_count: 0,
        };
        write_split(dir.path(), &result).unwrap();
        let files = read_dir_sorted(dir.path());
        assert_eq!(files.len(), 1);
        assert!(files[0].1.contains("\"has, a comma\""));
    }

    #[test]
    fn no_cells_emits_no_files() {
        let dir = tempfile::tempdir().unwrap();
        let result = ProcessResult {
            cells: vec![],
            unfixable: vec![],
            suppressed_count: 0,
        };
        write_split(dir.path(), &result).unwrap();
        assert_eq!(read_dir_sorted(dir.path()).len(), 0);
    }

    #[test]
    fn cells_within_a_column_preserve_input_order() {
        let dir = tempfile::tempdir().unwrap();
        let result = ProcessResult {
            cells: vec![
                cell("Q03", "Den", "third"),
                cell("Q01", "Den", "first"),
                cell("Q02", "Den", "second"),
            ],
            unfixable: vec![],
            suppressed_count: 0,
        };
        write_split(dir.path(), &result).unwrap();
        let files = read_dir_sorted(dir.path());
        assert_eq!(
            files[0].1,
            "qid,Den\nQ03,third\nQ01,first\nQ02,second\n"
        );
    }
}
