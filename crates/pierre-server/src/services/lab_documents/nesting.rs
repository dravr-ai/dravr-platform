// ABOUTME: The nesting guard in front of the PDF parser: arrays and dictionaries nested past a bound are refused unread
// ABOUTME: lopdf recurses into a dictionary with no depth limit, and a stack overflow aborts the process; this scan never recurses

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # How deep a PDF nests
//!
//! lopdf (under `pdf-extract`) parses a dictionary by recursing into each
//! value, with no depth limit: a file of a few kilobytes nesting `<< /A`
//! twenty thousand times overflows the thread's stack, and a stack overflow
//! aborts the whole process — every turn running on the instance with it —
//! where no `catch_unwind` reaches. So the bytes are scanned first, without
//! recursion, and a file nesting deeper than [`MAX_PDF_NESTING`] is refused
//! before the parser sees it.
//!
//! The scan reads what the parser would parse as objects:
//!
//! - the file's own bytes, skipping each stream's data (a content stream or an
//!   image is not parsed into objects here, and its binary bytes would count
//!   brackets that are not there), literal strings `( … )`, hex strings
//!   `< … >` and comments;
//! - the data of each object stream (`/Type /ObjStm`), where a PDF 1.5 file
//!   keeps its objects compressed: inflated when it is `/FlateDecode`, as it
//!   is read when unfiltered, and refused when it names any other filter,
//!   since what it holds could not be checked.

use std::io::Read;

use flate2::read::ZlibDecoder;

/// The deepest a PDF's arrays and dictionaries may nest. Real reports nest a
/// handful of levels; lopdf overflows a worker's stack in the thousands.
pub(super) const MAX_PDF_NESTING: usize = 64;

/// The most one object stream is inflated to for the scan: past it the file
/// is refused, as a decompression bomb would be.
const MAX_INFLATED_OBJECT_STREAM: u64 = 16 * 1024 * 1024;

/// Why a PDF is refused before it is parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NestingRefusal {
    /// Its arrays or dictionaries nest deeper than [`MAX_PDF_NESTING`].
    TooDeep,
    /// An object stream could not be read for the scan: a filter other than
    /// `FlateDecode`, data that does not inflate, or data past the bound.
    UncheckedObjectStream,
}

/// Refuse `bytes` when the objects it holds nest deeper than
/// [`MAX_PDF_NESTING`], or when one of its object streams cannot be checked.
///
/// # Errors
///
/// The [`NestingRefusal`] that applies.
pub(super) fn check_nesting(bytes: &[u8]) -> Result<(), NestingRefusal> {
    let mut scan = Scan::new(bytes);
    while let Some(stream) = scan.next_stream()? {
        if is_object_stream(stream.dictionary) {
            let objects = object_stream_data(stream.dictionary, stream.data)?;
            Scan::new(&objects).finish()?;
        }
    }
    Ok(())
}

/// One stream the scan passed: its dictionary's bytes and its data.
struct Stream<'pdf> {
    dictionary: &'pdf [u8],
    data: &'pdf [u8],
}

/// A scan of one run of PDF syntax, tracking the nesting depth.
struct Scan<'pdf> {
    bytes: &'pdf [u8],
    at: usize,
    depth: usize,
    /// Where the dictionary of the stream about to begin starts: the last
    /// `<<` opened at depth zero.
    dictionary_start: usize,
}

impl<'pdf> Scan<'pdf> {
    const fn new(bytes: &'pdf [u8]) -> Self {
        Self {
            bytes,
            at: 0,
            depth: 0,
            dictionary_start: 0,
        }
    }

    /// Scan to the end, refusing a nesting past the bound.
    fn finish(mut self) -> Result<(), NestingRefusal> {
        while self.next_stream()?.is_some() {}
        Ok(())
    }

    fn open(&mut self) -> Result<(), NestingRefusal> {
        self.depth += 1;
        if self.depth > MAX_PDF_NESTING {
            return Err(NestingRefusal::TooDeep);
        }
        Ok(())
    }

    fn close(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn peek(&self, offset: usize) -> Option<u8> {
        self.bytes.get(self.at + offset).copied()
    }

    /// Scan on to the next stream's data and pass over it, returning it;
    /// `None` at the end of the bytes.
    fn next_stream(&mut self) -> Result<Option<Stream<'pdf>>, NestingRefusal> {
        while let Some(byte) = self.peek(0) {
            match byte {
                b'%' => self.skip_comment(),
                b'(' => self.skip_literal_string(),
                b'[' => {
                    self.open()?;
                    self.at += 1;
                }
                b']' => {
                    self.close();
                    self.at += 1;
                }
                b'<' if self.peek(1) == Some(b'<') => {
                    if self.depth == 0 {
                        self.dictionary_start = self.at;
                    }
                    self.open()?;
                    self.at += 2;
                }
                b'>' if self.peek(1) == Some(b'>') => {
                    self.close();
                    self.at += 2;
                }
                b'<' => self.skip_hex_string(),
                b's' if self.depth == 0 && self.at_keyword(b"stream") => {
                    let dictionary = &self.bytes[self.dictionary_start..self.at];
                    return Ok(Some(self.pass_stream(dictionary)));
                }
                _ => self.at += 1,
            }
        }
        Ok(None)
    }

    /// Whether `keyword` starts here as a whole token.
    fn at_keyword(&self, keyword: &[u8]) -> bool {
        let before_is_delimiter = self.at == 0
            || self
                .bytes
                .get(self.at - 1)
                .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b'>');
        let after = self.bytes.get(self.at + keyword.len()).copied();
        before_is_delimiter
            && self.bytes[self.at..].starts_with(keyword)
            && after.is_some_and(|byte| byte == b'\r' || byte == b'\n')
    }

    /// Pass over a stream's data, from its `stream` keyword to `endstream`.
    fn pass_stream(&mut self, dictionary: &'pdf [u8]) -> Stream<'pdf> {
        let mut start = self.at + b"stream".len();
        if self.bytes.get(start) == Some(&b'\r') {
            start += 1;
        }
        if self.bytes.get(start) == Some(&b'\n') {
            start += 1;
        }
        let end =
            find(&self.bytes[start..], b"endstream").map_or(self.bytes.len(), |at| start + at);
        self.at = (end + b"endstream".len()).min(self.bytes.len());
        Stream {
            dictionary,
            data: &self.bytes[start..end],
        }
    }

    fn skip_comment(&mut self) {
        while let Some(byte) = self.peek(0) {
            self.at += 1;
            if byte == b'\n' || byte == b'\r' {
                return;
            }
        }
    }

    /// Pass over a literal string, its balanced parentheses and escapes.
    fn skip_literal_string(&mut self) {
        let mut open = 0usize;
        while let Some(byte) = self.peek(0) {
            self.at += 1;
            match byte {
                b'\\' => self.at += 1,
                b'(' => open += 1,
                b')' => {
                    open = open.saturating_sub(1);
                    if open == 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }

    fn skip_hex_string(&mut self) {
        while let Some(byte) = self.peek(0) {
            self.at += 1;
            if byte == b'>' {
                return;
            }
        }
    }
}

/// The first place `needle` occurs in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Whether a stream's dictionary names it an object stream.
fn is_object_stream(dictionary: &[u8]) -> bool {
    find(dictionary, b"/ObjStm").is_some()
}

/// An object stream's objects, inflated when it is `FlateDecode`.
fn object_stream_data(dictionary: &[u8], data: &[u8]) -> Result<Vec<u8>, NestingRefusal> {
    if find(dictionary, b"/Filter").is_none() {
        return Ok(data.to_vec());
    }
    let flate_only = find(dictionary, b"/FlateDecode").is_some()
        && [
            b"/LZWDecode".as_slice(),
            b"/ASCII85Decode",
            b"/ASCIIHexDecode",
            b"/RunLengthDecode",
        ]
        .iter()
        .all(|other| find(dictionary, other).is_none());
    if !flate_only {
        return Err(NestingRefusal::UncheckedObjectStream);
    }
    let mut inflated = Vec::new();
    ZlibDecoder::new(data)
        .take(MAX_INFLATED_OBJECT_STREAM + 1)
        .read_to_end(&mut inflated)
        .map_err(|_| NestingRefusal::UncheckedObjectStream)?;
    if u64::try_from(inflated.len()).unwrap_or(u64::MAX) > MAX_INFLATED_OBJECT_STREAM {
        return Err(NestingRefusal::UncheckedObjectStream);
    }
    Ok(inflated)
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use flate2::write::ZlibEncoder;
    use flate2::Compression;

    use super::*;

    fn nested_dictionary(levels: usize) -> Vec<u8> {
        let mut pdf = b"%PDF-1.4\n1 0 obj\n<< /X ".to_vec();
        pdf.extend(b"<< /A ".repeat(levels));
        pdf.push(b'1');
        pdf.extend(b" >>".repeat(levels));
        pdf.extend(b" >>\nendobj\n%%EOF\n");
        pdf
    }

    #[test]
    fn a_report_nesting_a_few_levels_passes() {
        assert_eq!(
            check_nesting(&nested_dictionary(5)),
            Ok(()),
            "bug: a shallow file refused"
        );
    }

    #[test]
    fn a_dictionary_nested_past_the_bound_is_refused() {
        assert_eq!(
            check_nesting(&nested_dictionary(20_000)),
            Err(NestingRefusal::TooDeep),
            "bug: the nesting that overflows lopdf's stack was let through"
        );
        assert_eq!(
            // The outer `<< /X` is one level: the bound itself, inside it.
            check_nesting(&nested_dictionary(MAX_PDF_NESTING - 1)),
            Ok(()),
            "bug: the bound itself refused"
        );
    }

    #[test]
    fn brackets_in_strings_comments_and_stream_data_are_not_nesting() {
        let mut pdf =
            b"%PDF-1.4\n% [[[[ <<<<\n1 0 obj\n<< /T ([[[[ <<<< \\) (((x))) /H <5B5B5B> >>\n"
                .to_vec();
        pdf.extend(b"<< /Length 300 >>\nstream\n");
        pdf.extend(b"[".repeat(300));
        pdf.extend(b"\nendstream\nendobj\n%%EOF\n");
        assert_eq!(
            check_nesting(&pdf),
            Ok(()),
            "bug: syntax outside objects counted as nesting"
        );
    }

    #[test]
    fn a_deep_nesting_hidden_in_a_compressed_object_stream_is_refused() {
        let objects = [
            b"<< /A ".repeat(20_000),
            b"1".to_vec(),
            b" >>".repeat(20_000),
        ]
        .concat();
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&objects).unwrap();
        let data = encoder.finish().unwrap();
        let mut pdf = format!(
            "%PDF-1.5\n1 0 obj\n<< /Type /ObjStm /N 1 /First 4 /Filter /FlateDecode /Length {} >>\nstream\n",
            data.len()
        )
        .into_bytes();
        pdf.extend(&data);
        pdf.extend(b"\nendstream\nendobj\n%%EOF\n");
        assert_eq!(
            check_nesting(&pdf),
            Err(NestingRefusal::TooDeep),
            "bug: the object stream was not scanned"
        );
    }

    #[test]
    fn an_object_stream_under_an_unchecked_filter_is_refused() {
        let pdf = b"%PDF-1.5\n1 0 obj\n<< /Type /ObjStm /Filter /LZWDecode /Length 3 >>\nstream\nabc\nendstream\nendobj\n";
        assert_eq!(
            check_nesting(pdf),
            Err(NestingRefusal::UncheckedObjectStream),
            "bug: an unchecked object stream"
        );
    }
}
