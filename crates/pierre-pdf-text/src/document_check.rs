// ABOUTME: The cheap check before pdf-extract reads a loaded PDF: forms drawn or page parents chained past a bound are refused
// ABOUTME: pdf-extract follows both by recursion with no bound; this walk never recurses

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # How far a PDF's drawing and page tree reach
//!
//! Once lopdf has loaded a file, two chains pdf-extract follows by recursion
//! are bounded by [`MAX_PDF_INDIRECTION`] ([`check_document`]):
//!
//! - the form `XObject`s a page draws: pdf-extract draws a form by recursing
//!   into its content (`process_stream` at every `Do`), so a form that draws
//!   itself (a few hundred bytes) overflows the stack;
//! - a page's `/Parent` chain, which pdf-extract climbs for an inherited
//!   resource or media box: a page that is its own parent never returns.
//!
//! pdf-extract also draws a form anew at every `Do`, so the draws a
//! document asks for, and the form content they decode, are bounded too
//! ([`MAX_PDF_FORM_DRAWS`], [`MAX_PDF_DRAWN_BYTES`]): a few forms each drawing
//! the next twice would otherwise never return.
//!
//! Nesting inside one object needs no check here: lopdf 0.42 stops parsing an
//! array or dictionary past 100 levels (`MAX_NESTING_DEPTH`). This check is
//! not the boundary either: whatever it misses ends the child process, never
//! the server.

use std::collections::{HashMap, HashSet};

use pdf_extract::content::Content;
use pdf_extract::{Dictionary, Document, Object, ObjectId, Stream as PdfStream};

/// The deepest a page's form `XObject`s may draw one another, and the longest
/// a page's `/Parent` chain may run. Real reports draw a form or two and sit
/// a few levels down their page tree.
pub(crate) const MAX_PDF_INDIRECTION: usize = 32;

/// The most form draws a document's pages may make. pdf-extract draws a form
/// anew at every `Do`, the forms it draws included, so thirty forms each
/// drawing the next twice ask it for a billion draws.
pub(crate) const MAX_PDF_FORM_DRAWS: u64 = 4096;

/// The most form content a document's form draws may decode between them.
pub(crate) const MAX_PDF_DRAWN_BYTES: u64 = 64 * 1024 * 1024;

/// A document [`check_document`] refuses: its forms draw one another, or its
/// pages' parents chain, in a cycle or past the bounds above.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooIndirect;

/// Refuse a loaded `document` whose drawing or page tree pdf-extract cannot
/// finish.
///
/// That is when a page's forms draw one another, or its
/// `/Parent` chain runs, in a cycle or past [`MAX_PDF_INDIRECTION`], or when
/// its pages draw more forms than [`MAX_PDF_FORM_DRAWS`] or decode more form
/// content than [`MAX_PDF_DRAWN_BYTES`] doing it: pdf-extract follows both
/// chains by recursion, and draws each form anew at every `Do`.
///
/// # Errors
///
/// [`TooIndirect`].
pub fn check_document(document: &Document) -> Result<(), TooIndirect> {
    let empty = Dictionary::new();
    let mut total = Cost::default();
    for page_id in document.get_pages().into_values() {
        let resources = page_resources(document, page_id)?;
        let content = document.get_page_content(page_id).unwrap_or_default();
        let resources = resources.unwrap_or((&empty, page_id));
        total = total.then(drawing_cost(document, &content, resources)?);
        if total.too_much() {
            return Err(TooIndirect);
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
) -> Result<Option<(&Dictionary, ObjectId)>, TooIndirect> {
    let mut found = None;
    let mut node = Some(page_id);
    let mut climbed = 0usize;
    while let Some(id) = node {
        climbed += 1;
        if climbed > MAX_PDF_INDIRECTION {
            return Err(TooIndirect);
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
/// [`TooIndirect`].
fn drawing_cost<'doc>(
    document: &'doc Document,
    content: &[u8],
    resources: (&'doc Dictionary, ObjectId),
) -> Result<Cost, TooIndirect> {
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
                    return Err(TooIndirect);
                }
                if let Some(top) = path.last_mut() {
                    top.beneath = top.beneath.then(cost);
                    if top.beneath.too_much() {
                        return Err(TooIndirect);
                    }
                }
                continue;
            }
            if !on_path.insert(draw.drawn) || above + 1 > MAX_PDF_INDIRECTION {
                return Err(TooIndirect);
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
            return Err(TooIndirect);
        }
        costs.insert(drawn, cost);
        if let Some(parent) = path.last_mut() {
            parent.beneath = parent.beneath.then(cost);
        }
    }
    Ok(Cost::default())
}

#[cfg(test)]
pub(crate) mod tests {
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
    pub(crate) fn document(
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
            Err(TooIndirect),
            "bug: a form drawing itself, which recurses pdf-extract off its stack, was let through"
        );
        assert_eq!(
            check_document(&document(3, Some(1), false)),
            Err(TooIndirect),
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
            Err(TooIndirect),
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
            Err(TooIndirect),
            "bug: draws past the bound let through"
        );
        assert_eq!(
            check_document(&fanned_document(30, 2, (None, false))),
            Err(TooIndirect),
            "bug: a billion draws let through"
        );
    }

    #[test]
    fn a_page_that_is_its_own_parent_is_refused() {
        assert_eq!(
            check_document(&document(0, None, true)),
            Err(TooIndirect),
            "bug: a page whose parent chain never ends, where pdf-extract never returns, was let through"
        );
        assert_eq!(
            check_document(&document(0, None, false)),
            Ok(()),
            "bug: a plain page refused"
        );
    }
}
