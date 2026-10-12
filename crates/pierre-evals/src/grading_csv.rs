// ABOUTME: Minimal RFC 4180 CSV writing and reading for blind grading sheets
// ABOUTME: Quotes what needs quoting, reads quoted fields, doubled quotes, embedded breaks and an Excel BOM
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Grading sheet CSV
//!
//! Graders fill the blind sheet in a spreadsheet and hand back a CSV, so the
//! format has to survive what spreadsheets write: a UTF-8 byte-order mark in
//! front of the header, CRLF line ends, and claim text that carries commas,
//! quotes and — after a grader's edit — line breaks inside a note.

use std::borrow::Cow;
use std::mem::take;

use pierre_core::errors::{AppError, AppResult};

/// The byte-order mark Excel writes in front of a UTF-8 CSV.
const BOM: char = '\u{feff}';

/// One CSV record: the fields joined by commas, each quoted when it carries a
/// comma, a quote or a line break, with its quotes doubled. Ends in CRLF, the
/// RFC 4180 record terminator.
#[must_use]
pub fn record(fields: &[&str]) -> String {
    let mut out = fields
        .iter()
        .map(|field| {
            if field.contains([',', '"', '\r', '\n']) {
                format!("\"{}\"", field.replace('"', "\"\""))
            } else {
                (*field).to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    out.push_str("\r\n");
    out
}

/// The characters a spreadsheet reads as the start of a formula when a cell
/// opens with one.
const FORMULA_LEADS: [char; 6] = ['=', '+', '-', '@', '\t', '\r'];

/// Free text as a spreadsheet must show it: with a leading apostrophe, the
/// spreadsheet's own text marker, when it opens with a formula character.
///
/// A claim is a sentence of an agent's reply, and a markdown reply opens lines
/// with a "- " bullet or a "+12%". Opened bare, the grader's spreadsheet reads
/// either as a formula: an error cell where the claim should be, or whatever a
/// reply smuggled in evaluated on the grader's machine.
#[must_use]
pub fn spreadsheet_text(text: &str) -> Cow<'_, str> {
    if text.starts_with(FORMULA_LEADS) {
        Cow::Owned(format!("'{text}"))
    } else {
        Cow::Borrowed(text)
    }
}

/// Parse CSV text into records of fields.
///
/// Blank lines are skipped. A field opened by a quote runs to its closing
/// quote, across line breaks; a quote inside an unquoted field is kept as
/// text, the way spreadsheets read one.
///
/// # Errors
///
/// An invalid-input error when a quoted field is never closed: the file was
/// cut short, and reading on would shift every later field.
pub fn parse(text: &str) -> AppResult<Vec<Vec<String>>> {
    let text = text.strip_prefix(BOM).unwrap_or(text);
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();

    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    field.push('"');
                    chars.next();
                } else {
                    quoted = false;
                }
            } else {
                field.push(c);
            }
            continue;
        }
        match c {
            ',' => current.push(take(&mut field)),
            '\r' | '\n' => {
                if c == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                current.push(take(&mut field));
                end_record(&mut records, take(&mut current));
            }
            '"' if field.is_empty() => quoted = true,
            other => field.push(other),
        }
    }
    if quoted {
        return Err(AppError::invalid_input(format!(
            "CSV ends inside a quoted field (record {})",
            records.len() + 1
        )));
    }
    if !field.is_empty() || !current.is_empty() {
        current.push(field);
        end_record(&mut records, current);
    }
    Ok(records)
}

/// Keep a finished record unless it is a blank line.
fn end_record(records: &mut Vec<Vec<String>>, record: Vec<String>) {
    let blank = record.len() == 1 && record.first().is_some_and(String::is_empty);
    if !blank {
        records.push(record);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_field_is_written_bare_and_a_special_one_quoted() {
        assert_eq!(record(&["C001", "fr", "plain"]), "C001,fr,plain\r\n");
        assert_eq!(
            record(&["Tu as couru 21,4 km", "He said \"go\""]),
            "\"Tu as couru 21,4 km\",\"He said \"\"go\"\"\"\r\n"
        );
    }

    #[test]
    fn what_is_written_reads_back_unchanged() {
        let fields = [
            "C007",
            "Your HRV was 85 ms, a \"great\" sign.",
            "line one\nline two",
            "",
        ];
        let text = format!("{}{}", record(&["a", "b", "c", "d"]), record(&fields));
        let parsed = parse(&text).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[1], fields);
    }

    #[test]
    fn a_spreadsheet_export_parses() {
        // Excel: BOM, CRLF, a trailing empty column, no final line break.
        let text = "\u{feff}item_id,grade,note\r\nC001,true,\r\nC002,false,\"wrong, by 9 km\"";
        let parsed = parse(text).unwrap();
        assert_eq!(parsed[0], ["item_id", "grade", "note"]);
        assert_eq!(parsed[1], ["C001", "true", ""]);
        assert_eq!(parsed[2], ["C002", "false", "wrong, by 9 km"]);
    }

    #[test]
    fn text_opening_like_a_formula_is_marked_as_text() {
        for (text, cell) in [
            ("- You ran 42 km this week.", "'- You ran 42 km this week."),
            ("+12% on last week.", "'+12% on last week."),
            ("=HYPERLINK(\"x\")", "'=HYPERLINK(\"x\")"),
            ("@SUM(A1)", "'@SUM(A1)"),
        ] {
            assert_eq!(spreadsheet_text(text), cell);
        }
        // Only the opening character counts.
        assert_eq!(
            spreadsheet_text("You ran 42 km - a big week."),
            "You ran 42 km - a big week."
        );
    }

    #[test]
    fn blank_lines_are_skipped() {
        let parsed = parse("a,b\n\n\nc,d\n").unwrap();
        assert_eq!(parsed, vec![vec!["a", "b"], vec!["c", "d"]]);
    }

    #[test]
    fn an_unclosed_quote_is_an_error_not_a_shifted_row() {
        let err = parse("item_id,note\nC001,\"never closed\n").unwrap_err();
        assert!(err.to_string().contains("quoted field"), "{err}");
    }
}
