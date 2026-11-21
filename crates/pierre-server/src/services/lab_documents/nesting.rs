// ABOUTME: The nesting guard in front of the PDF reader: objects nested, forms drawn or page parents chained past a bound are refused
// ABOUTME: The parser and pdf-extract recurse through all three, and a stack overflow aborts the process; these checks never recurse

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
//!
//! Once lopdf has loaded the file, two more chains pdf-extract follows by
//! recursion are bounded by [`MAX_PDF_INDIRECTION`] ([`check_document`]):
//!
//! - the form `XObject`s a page draws: pdf-extract draws a form by recursing
//!   into its content, so a form that draws itself (a few hundred bytes)
//!   overflows the stack;
//! - a page's `/Parent` chain, which pdf-extract climbs for an inherited
//!   resource or media box: a page that is its own parent never returns.
//!
//! pdf-extract also draws a form anew at every `Do`, so the draws a
//! document asks for, and the form content they decode, are bounded too
//! ([`MAX_PDF_FORM_DRAWS`], [`MAX_PDF_DRAWN_BYTES`]): a few forms each drawing
//! the next twice would otherwise never return.

use std::collections::{HashMap, HashSet};
use std::io::Read;

use flate2::read::ZlibDecoder;
use pdf_extract::content::Content;
use pdf_extract::{Dictionary, Document, Object, ObjectId, Stream as PdfStream};

/// The deepest a PDF's arrays and dictionaries may nest. Real reports nest a
/// handful of levels; lopdf overflows a worker's stack in the thousands.
pub(super) const MAX_PDF_NESTING: usize = 64;

/// The most one object stream is inflated to for the scan: past it the file
/// is refused, as a decompression bomb would be.
const MAX_INFLATED_OBJECT_STREAM: u64 = 16 * 1024 * 1024;

/// The deepest a page's form `XObject`s may draw one another, and the longest
/// a page's `/Parent` chain may run. Real reports draw a form or two and sit
/// a few levels down their page tree.
pub(super) const MAX_PDF_INDIRECTION: usize = 32;

/// The most form draws a document's pages may make. pdf-extract draws a form
/// anew at every `Do`, the forms it draws included, so thirty forms each
/// drawing the next twice ask it for a billion draws.
pub(super) const MAX_PDF_FORM_DRAWS: u64 = 4096;

/// The most form content a document's form draws may decode between them.
pub(super) const MAX_PDF_DRAWN_BYTES: u64 = 64 * 1024 * 1024;

/// Why a PDF is refused before it is parsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NestingRefusal {
    /// Its arrays or dictionaries nest deeper than [`MAX_PDF_NESTING`].
    TooDeep,
    /// An object stream could not be read for the scan: a filter other than
    /// `FlateDecode`, data that does not inflate, or data past the bound.
    UncheckedObjectStream,
    /// Its forms draw one another, or its pages' parents chain, in a cycle
    /// or deeper than [`MAX_PDF_INDIRECTION`].
    TooIndirect,
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

/// Refuse a loaded `document` when a page's forms draw one another, or its
/// `/Parent` chain runs, in a cycle or past [`MAX_PDF_INDIRECTION`], or when
/// its pages draw more forms than [`MAX_PDF_FORM_DRAWS`] or decode more form
/// content than [`MAX_PDF_DRAWN_BYTES`] doing it: pdf-extract follows both
/// chains by recursion, and draws each form anew at every `Do`.
///
/// # Errors
///
/// [`NestingRefusal::TooIndirect`].
pub(super) fn check_document(document: &Document) -> Result<(), NestingRefusal> {
    let empty = Dictionary::new();
    let mut total = Cost::default();
    for page_id in document.get_pages().into_values() {
        let resources = page_resources(document, page_id)?;
        let content = document.get_page_content(page_id).unwrap_or_default();
        let resources = resources.unwrap_or((&empty, page_id));
        total = total.then(drawing_cost(document, &content, resources)?);
        if total.too_much() {
            return Err(NestingRefusal::TooIndirect);
        }
    }
    Ok(())
}

/// `object`, or the object it refers to.
fn resolved<'doc>(document: &'doc Document, object: &'doc Object) -> Option<&'doc Object> {
    match object {
        Object::Reference(id) => document.get_object(*id).ok(),
        direct => Some(direct),
    }
}

/// The dictionary `dictionary` holds under `key`, resolved.
fn dictionary_at<'doc>(
    document: &'doc Document,
    dictionary: &'doc Dictionary,
    key: &[u8],
) -> Option<&'doc Dictionary> {
    let object = dictionary.get(key).ok()?;
    resolved(document, object)?.as_dict().ok()
}

/// The resources a page is drawn with, as pdf-extract finds them (its own,
/// else the nearest ancestor's), with the object that holds them; the whole
/// `/Parent` chain is climbed, since pdf-extract climbs it for other keys.
fn page_resources(
    document: &Document,
    page_id: ObjectId,
) -> Result<Option<(&Dictionary, ObjectId)>, NestingRefusal> {
    let mut found = None;
    let mut node = Some(page_id);
    let mut climbed = 0usize;
    while let Some(id) = node {
        climbed += 1;
        if climbed > MAX_PDF_INDIRECTION {
            return Err(NestingRefusal::TooIndirect);
        }
        let Ok(dictionary) = document.get_dictionary(id) else {
            break;
        };
        if found.is_none() {
            found = dictionary_at(document, dictionary, b"Resources").map(|held| (held, id));
        }
        node = dictionary
            .get(b"Parent")
            .and_then(Object::as_reference)
            .ok();
    }
    Ok(found)
}

/// A form `XObject` drawn under the resources of the object holding them: the
/// form itself, or the page or form it inherits them from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Drawn {
    form: ObjectId,
    resources: ObjectId,
}

/// A form a content draws, with what drawing it reads.
#[derive(Clone, Copy)]
struct Draw<'doc> {
    drawn: Drawn,
    form: &'doc PdfStream,
    resources: &'doc Dictionary,
}

/// A form's content, as pdf-extract reads it.
fn form_content(form: &PdfStream) -> Vec<u8> {
    if form.filters().is_ok() {
        form.decompressed_content()
            .unwrap_or_else(|_| form.content.clone())
    } else {
        form.content.clone()
    }
}

/// The forms `content` draws (`Do`) under `resources`, held by the object
/// `owner`. A content that does not decode draws nothing: pdf-extract
/// refuses it.
fn forms_drawn<'doc>(
    document: &'doc Document,
    content: &[u8],
    (resources, owner): (&'doc Dictionary, ObjectId),
) -> Vec<Draw<'doc>> {
    let Ok(content) = Content::decode(content) else {
        return Vec::new();
    };
    let Some(xobjects) = dictionary_at(document, resources, b"XObject") else {
        return Vec::new();
    };
    content
        .operations
        .iter()
        .filter(|operation| operation.operator == "Do")
        .filter_map(|operation| {
            let name = operation.operands.first()?.as_name().ok()?;
            let form_id = xobjects.get(name).ok()?.as_reference().ok()?;
            let form = document.get_object(form_id).ok()?.as_stream().ok()?;
            let (resources, owner) = dictionary_at(document, &form.dict, b"Resources")
                .map_or((resources, owner), |own| (own, form_id));
            Some(Draw {
                drawn: Drawn {
                    form: form_id,
                    resources: owner,
                },
                form,
                resources,
            })
        })
        .collect()
}

/// What drawing a form costs pdf-extract, the forms it draws included: it
/// draws every `Do` anew, decoding the form's content each time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Cost {
    /// The deepest chain of forms, this one counted.
    depth: usize,
    /// How many forms are drawn, this one counted.
    draws: u64,
    /// How many bytes of form content those draws decode.
    bytes: u64,
}

impl Cost {
    /// This cost and `other`'s, drawn one after the other.
    fn then(self, other: Self) -> Self {
        Self {
            depth: self.depth.max(other.depth),
            draws: self.draws.saturating_add(other.draws),
            bytes: self.bytes.saturating_add(other.bytes),
        }
    }

    /// Whether it is past what a document may cost.
    const fn too_much(self) -> bool {
        self.draws > MAX_PDF_FORM_DRAWS || self.bytes > MAX_PDF_DRAWN_BYTES
    }
}

/// A form on the walk's path, with what it draws and what drawing it costs
/// so far.
struct Frame<'doc> {
    drawn: Option<Drawn>,
    draws: Vec<Draw<'doc>>,
    next: usize,
    /// Its own content's length, once.
    own_bytes: u64,
    /// What the forms it draws cost.
    beneath: Cost,
}

impl<'doc> Frame<'doc> {
    fn new(
        document: &'doc Document,
        drawn: Option<Drawn>,
        content: &[u8],
        resources: (&'doc Dictionary, ObjectId),
    ) -> Self {
        Self {
            drawn,
            draws: forms_drawn(document, content, resources),
            next: 0,
            own_bytes: u64::try_from(content.len()).unwrap_or(u64::MAX),
            beneath: Cost::default(),
        }
    }
}

/// What a page whose `content` draws forms costs to draw, refusing forms
/// drawn in a cycle or deeper than [`MAX_PDF_INDIRECTION`]; walked depth
/// first without recursion, each form under each resources decoded once and
/// its cost remembered.
///
/// # Errors
///
/// [`NestingRefusal::TooIndirect`].
fn drawing_cost<'doc>(
    document: &'doc Document,
    content: &[u8],
    resources: (&'doc Dictionary, ObjectId),
) -> Result<Cost, NestingRefusal> {
    let mut costs: HashMap<Drawn, Cost> = HashMap::new();
    let mut on_path: HashSet<Drawn> = HashSet::new();
    let mut path = vec![Frame::new(document, None, content, resources)];
    while let Some(top) = path.last_mut() {
        if let Some(draw) = top.draws.get(top.next).copied() {
            top.next += 1;
            // The forms on the path above this one: every frame but the page.
            let above = path.len() - 1;
            if let Some(&cost) = costs.get(&draw.drawn) {
                if above + cost.depth > MAX_PDF_INDIRECTION {
                    return Err(NestingRefusal::TooIndirect);
                }
                if let Some(top) = path.last_mut() {
                    top.beneath = top.beneath.then(cost);
                    if top.beneath.too_much() {
                        return Err(NestingRefusal::TooIndirect);
                    }
                }
                continue;
            }
            if !on_path.insert(draw.drawn) || above + 1 > MAX_PDF_INDIRECTION {
                return Err(NestingRefusal::TooIndirect);
            }
            let drawn_content = form_content(draw.form);
            path.push(Frame::new(
                document,
                Some(draw.drawn),
                &drawn_content,
                (draw.resources, draw.drawn.resources),
            ));
            continue;
        }
        let Some(done) = path.pop() else { break };
        let Some(drawn) = done.drawn else {
            // The page itself: what its forms cost.
            return Ok(done.beneath);
        };
        on_path.remove(&drawn);
        let cost = Cost {
            depth: done.beneath.depth + 1,
            draws: done.beneath.draws.saturating_add(1),
            bytes: done.beneath.bytes.saturating_add(done.own_bytes),
        };
        if cost.too_much() {
            return Err(NestingRefusal::TooIndirect);
        }
        costs.insert(drawn, cost);
        if let Some(parent) = path.last_mut() {
            parent.beneath = parent.beneath.then(cost);
        }
    }
    Ok(Cost::default())
}

#[cfg(test)]
pub(super) mod tests {
    use std::io::Write;

    use flate2::write::ZlibEncoder;
    use flate2::Compression;
    use pdf_extract::content::Operation;

    use super::*;

    /// A content drawing the form named `X{index}`.
    fn draws(index: usize) -> Vec<u8> {
        Content {
            operations: vec![Operation::new(
                "Do",
                vec![Object::Name(format!("X{index}").into_bytes())],
            )],
        }
        .encode()
        .unwrap()
    }

    /// One page drawing a chain of `forms` forms, each drawing the next; the
    /// last draws form `back_to` when one is named. The page is its own
    /// parent when `own_parent` holds.
    pub(in crate::services::lab_documents) fn document(
        forms: usize,
        back_to: Option<usize>,
        own_parent: bool,
    ) -> Document {
        fanned_document(forms, 1, (back_to, own_parent))
    }

    /// [`document`], each form drawing the next `fan` times.
    fn fanned_document(
        forms: usize,
        fan: usize,
        (back_to, own_parent): (Option<usize>, bool),
    ) -> Document {
        let mut document = Document::with_version("1.5");
        let ids: Vec<ObjectId> = (0..forms).map(|_| document.new_object_id()).collect();
        let mut xobjects = Dictionary::new();
        for (index, id) in ids.iter().enumerate() {
            xobjects.set(format!("X{index}"), *id);
        }
        for (index, id) in ids.iter().enumerate() {
            let content = if index + 1 < forms {
                draws(index + 1).repeat(fan)
            } else {
                back_to.map_or_else(Vec::new, draws)
            };
            let mut form = Dictionary::new();
            form.set("Type", "XObject");
            form.set("Subtype", "Form");
            document
                .objects
                .insert(*id, Object::Stream(PdfStream::new(form, content)));
        }
        let mut resources = Dictionary::new();
        resources.set("XObject", xobjects);
        let resources_id = document.add_object(resources);
        let page_content = if forms > 0 { draws(0) } else { Vec::new() };
        let content_id = document.add_object(PdfStream::new(Dictionary::new(), page_content));
        let tree_id = document.new_object_id();
        let page_id = document.new_object_id();
        let mut page = Dictionary::new();
        page.set("Type", "Page");
        page.set("Parent", if own_parent { page_id } else { tree_id });
        page.set("Contents", content_id);
        page.set("Resources", resources_id);
        if !own_parent {
            // A self-parented page has none, so pdf-extract climbs for one.
            page.set("MediaBox", vec![0.into(), 0.into(), 200.into(), 200.into()]);
        }
        document.objects.insert(page_id, Object::Dictionary(page));
        let mut pages = Dictionary::new();
        pages.set("Type", "Pages");
        pages.set("Kids", vec![Object::from(page_id)]);
        pages.set("Count", 1);
        document.objects.insert(tree_id, Object::Dictionary(pages));
        let mut catalog = Dictionary::new();
        catalog.set("Type", "Catalog");
        catalog.set("Pages", tree_id);
        let catalog_id = document.add_object(catalog);
        document.trailer.set("Root", catalog_id);
        document
    }

    #[test]
    fn a_form_that_draws_itself_is_refused() {
        assert_eq!(
            check_document(&document(1, Some(0), false)),
            Err(NestingRefusal::TooIndirect),
            "bug: a form drawing itself, which recurses pdf-extract off its stack, was let through"
        );
        assert_eq!(
            check_document(&document(3, Some(1), false)),
            Err(NestingRefusal::TooIndirect),
            "bug: a cycle through three forms was let through"
        );
    }

    #[test]
    fn forms_drawn_to_the_bound_pass_and_one_more_is_refused() {
        assert_eq!(
            check_document(&document(MAX_PDF_INDIRECTION, None, false)),
            Ok(()),
            "bug: forms drawn to the bound itself refused"
        );
        assert_eq!(
            check_document(&document(MAX_PDF_INDIRECTION + 1, None, false)),
            Err(NestingRefusal::TooIndirect),
            "bug: forms drawn past the bound let through"
        );
    }

    #[test]
    fn forms_drawn_past_the_draw_bound_are_refused() {
        // Twelve forms each drawing the next twice: 4095 draws, the bound
        // less one; thirteen: 8191, which a billion-draw file also trips.
        assert_eq!(
            check_document(&fanned_document(12, 2, (None, false))),
            Ok(()),
            "bug: draws within the bound refused"
        );
        assert_eq!(
            check_document(&fanned_document(13, 2, (None, false))),
            Err(NestingRefusal::TooIndirect),
            "bug: draws past the bound let through"
        );
        assert_eq!(
            check_document(&fanned_document(30, 2, (None, false))),
            Err(NestingRefusal::TooIndirect),
            "bug: a billion draws let through"
        );
    }

    #[test]
    fn a_page_that_is_its_own_parent_is_refused() {
        assert_eq!(
            check_document(&document(0, None, true)),
            Err(NestingRefusal::TooIndirect),
            "bug: a page whose parent chain never ends, where pdf-extract never returns, was let through"
        );
        assert_eq!(
            check_document(&document(0, None, false)),
            Ok(()),
            "bug: a plain page refused"
        );
    }

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
