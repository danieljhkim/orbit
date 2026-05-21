#![allow(missing_docs)]

use super::super::*;

#[test]
fn csv_files_emit_no_column_leaves() {
    let src = "id,name,email\n1,alice,a@x\n2,bob,b@x\n";
    let out = TableExtractor::new(TableFormat::Csv).extract(src);
    assert!(out.leaves.is_empty());
}

#[test]
fn tsv_files_emit_no_column_leaves() {
    let src = "id\tname\temail\n1\talice\ta@x\n";
    let out = TableExtractor::new(TableFormat::Tsv).extract(src);
    assert!(out.leaves.is_empty());
}

#[test]
fn empty_input_yields_zero_leaves() {
    let out = TableExtractor::new(TableFormat::Csv).extract("");
    assert!(out.leaves.is_empty());
}

#[test]
fn whitespace_header_still_produces_no_leaves() {
    let src = "id, name ,,email\n";
    let out = TableExtractor::new(TableFormat::Csv).extract(src);
    assert!(out.leaves.is_empty());
}
