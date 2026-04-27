use std::io::Write;

use crate::coalesce::ProcessResult;

pub fn write<W: Write>(output: W, result: &ProcessResult, annotate: bool) -> anyhow::Result<()> {
    let mut writer = csv::Writer::from_writer(output);
    let mut header = result.header.clone();
    if annotate {
        header.push("notes".to_string());
    }
    writer.write_record(&header)?;
    for (i, row) in result.rows.iter().enumerate() {
        if annotate {
            let mut full = row.clone();
            full.push(result.annotations[i].clone());
            writer.write_record(&full)?;
        } else {
            writer.write_record(row)?;
        }
    }
    writer.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_and_one_row_no_annotate() {
        let result = ProcessResult {
            header: vec!["qid".into(), "Den".into()],
            rows: vec![vec!["Q1".into(), "the abandoned ship".into()]],
            annotations: vec![String::new()],
            unfixable: vec![],
            suppressed_count: 0,
        };
        let mut buf: Vec<u8> = Vec::new();
        write(&mut buf, &result, false).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s, "qid,Den\nQ1,the abandoned ship\n");
    }

    #[test]
    fn rfc4180_quoting_for_special_chars() {
        let result = ProcessResult {
            header: vec!["qid".into(), "Den".into()],
            rows: vec![vec!["Q1".into(), "has, a comma".into()]],
            annotations: vec![String::new()],
            unfixable: vec![],
            suppressed_count: 0,
        };
        let mut buf: Vec<u8> = Vec::new();
        write(&mut buf, &result, false).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("\"has, a comma\""));
    }

    #[test]
    fn annotate_appends_notes_column() {
        let result = ProcessResult {
            header: vec!["qid".into(), "Den".into()],
            rows: vec![vec!["Q1".into(), "x".into()]],
            annotations: vec!["description.contains_trademark".into()],
            unfixable: vec![],
            suppressed_count: 0,
        };
        let mut buf: Vec<u8> = Vec::new();
        write(&mut buf, &result, true).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(
            s,
            "qid,Den,notes\nQ1,x,description.contains_trademark\n"
        );
    }
}
