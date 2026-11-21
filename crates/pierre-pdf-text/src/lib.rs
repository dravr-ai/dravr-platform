// ABOUTME: Reads an uploaded PDF's text page by page, the work the server hands its `pierre-pdf-text` child process
// ABOUTME: The PDF is untrusted: the child is bounded in time and memory, and answers one JSON outcome on stdout

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A PDF's text, read in a child process
//!
//! An athlete's uploaded lab report is untrusted input to a PDF parser
//! (`lopdf`, under `pdf-extract`). The server never parses one in its own
//! process: it spawns `pierre-pdf-text`, a binary of `pierre_mcp_server`
//! built on this crate, hands it the bytes on stdin and reads one
//! [`Outcome`] back as JSON on stdout. A crash, a stack overflow, a panic,
//! a hang (the server's wall-clock timeout) or an allocation past
//! [`MEMORY_LIMIT_BYTES`] (the child's capped allocator) ends the child, and
//! the server answers it as a refusal; the server itself never runs the
//! parser's code, so no PDF can take it down.
//!
//! Before the text is read the loaded document is checked ([`check_document`])
//! for the chains `pdf-extract` follows by recursion with no bound: forms
//! that draw one another, and a page's `/Parent` chain. That check is cheap
//! and turns the shapes it knows into a named refusal rather than a crash.
//!
//! The output is one JSON object, so a stdout cut short by the server's cap
//! fails to parse and is refused, never read as a shorter document.

use std::io::{Read, Write};
use std::process::ExitCode;

use pdf_extract::Document;
use serde::{Deserialize, Serialize};

mod document_check;

pub use document_check::{check_document, TooIndirect};

/// The largest PDF the child reads: the server refuses a larger body before
/// it spawns one.
pub const MAX_PDF_BYTES: usize = 10 * 1024 * 1024;

/// The most pages a PDF is read for.
pub const MAX_PAGES: usize = 200;

/// The most text a PDF is read for, every page together.
pub const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;

/// The most the child may allocate. A Cloud Run instance has 2 GiB and serves
/// one request at a time; a real lab report is read in a few megabytes.
pub const MEMORY_LIMIT_BYTES: usize = 512 * 1024 * 1024;

/// What every PDF starts with.
pub const PDF_MAGIC: &[u8] = b"%PDF-";

/// What the child answers on stdout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum Outcome {
    /// The text of each page, in order.
    Pages {
        /// One entry per page.
        pages: Vec<String>,
    },
    /// The PDF is refused, for `reason`.
    Refused {
        /// Why.
        reason: Refusal,
    },
}

/// Why the child refused a PDF.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Refusal {
    /// It is past [`MAX_PDF_BYTES`].
    TooLarge,
    /// It is not a PDF the parser reads: no `%PDF-` header, a structure that
    /// does not load, an encryption the empty password does not open, or
    /// text that does not extract.
    Unreadable,
    /// Its forms draw one another, or its pages' parents chain, in a cycle
    /// or past the bounds [`check_document`] holds.
    TooIndirect,
    /// It has more than [`MAX_PAGES`] pages.
    TooManyPages,
    /// It holds more than [`MAX_TEXT_BYTES`] of text.
    TooMuchText,
}

impl From<Refusal> for Outcome {
    fn from(reason: Refusal) -> Self {
        Self::Refused { reason }
    }
}

/// The text of the PDF `bytes`, page by page, or why it is refused.
#[must_use]
pub fn read_pages(bytes: &[u8]) -> Outcome {
    if bytes.len() > MAX_PDF_BYTES {
        return Refusal::TooLarge.into();
    }
    if !bytes.starts_with(PDF_MAGIC) {
        return Refusal::Unreadable.into();
    }
    let Ok(mut document) = Document::load_mem(bytes) else {
        return Refusal::Unreadable.into();
    };
    // Checked as pdf-extract reads it: an encrypted file under the empty
    // password, so the forms checked are the ones it draws.
    if document.is_encrypted() && document.decrypt("").is_err() {
        return Refusal::Unreadable.into();
    }
    if check_document(&document).is_err() {
        return Refusal::TooIndirect.into();
    }
    let Ok(pages) = pdf_extract::extract_text_from_mem_by_pages(bytes) else {
        return Refusal::Unreadable.into();
    };
    if pages.len() > MAX_PAGES {
        return Refusal::TooManyPages.into();
    }
    if pages.iter().map(String::len).sum::<usize>() > MAX_TEXT_BYTES {
        return Refusal::TooMuchText.into();
    }
    Outcome::Pages { pages }
}

/// The child's work: the PDF on `input`, its [`Outcome`] as JSON on `output`.
///
/// Fails only when the outcome cannot be written; the server reads a failing
/// child as a refusal.
pub fn run(input: impl Read, mut output: impl Write) -> ExitCode {
    let mut bytes = Vec::new();
    // One byte past the bound tells a body at the bound from one past it.
    let limit = u64::try_from(MAX_PDF_BYTES).unwrap_or(u64::MAX).saturating_add(1);
    if input.take(limit).read_to_end(&mut bytes).is_err() {
        return ExitCode::FAILURE;
    }
    let outcome = read_pages(&bytes);
    if serde_json::to_writer(&mut output, &outcome).is_err() || output.flush().is_err() {
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_that_is_not_a_pdf_is_unreadable() {
        assert_eq!(
            read_pages(b"PK\x03\x04 a zip"),
            Outcome::from(Refusal::Unreadable),
            "bug: a non-PDF body was read"
        );
    }

    #[test]
    fn a_truncated_pdf_is_unreadable_not_a_panic() {
        assert_eq!(
            read_pages(b"%PDF-1.7\n1 0 obj <<"),
            Outcome::from(Refusal::Unreadable),
            "bug: a broken PDF was read"
        );
    }

    #[test]
    fn a_body_past_the_bound_is_refused_unread() {
        let mut body = PDF_MAGIC.to_vec();
        body.resize(MAX_PDF_BYTES + 1, b' ');
        assert_eq!(
            read_pages(&body),
            Outcome::from(Refusal::TooLarge),
            "bug: a body past the bound was parsed"
        );
    }

    #[test]
    fn a_form_that_draws_itself_is_refused_before_the_text_is_read() {
        let mut bytes = Vec::new();
        document_check::tests::document(1, Some(0), false)
            .save_to(&mut bytes)
            .unwrap();
        assert_eq!(
            read_pages(&bytes),
            Outcome::from(Refusal::TooIndirect),
            "bug: a self-drawing form reached pdf-extract"
        );
    }

    #[test]
    fn the_outcome_is_one_json_object_a_cut_short_copy_of_which_does_not_parse() {
        let mut output = Vec::new();
        let code = run(&b"not a pdf"[..], &mut output);
        assert_eq!(code, ExitCode::SUCCESS, "bug: the child failed to answer");
        let outcome: Outcome = serde_json::from_slice(&output).unwrap();
        assert_eq!(outcome, Outcome::from(Refusal::Unreadable));
        let pages = serde_json::to_vec(&Outcome::Pages {
            pages: vec!["page one".to_owned(), "page two".to_owned()],
        })
        .unwrap();
        assert!(
            serde_json::from_slice::<Outcome>(&pages[..pages.len() - 3]).is_err(),
            "bug: a truncated answer parsed as a shorter document"
        );
    }
}
