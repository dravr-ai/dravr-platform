// ABOUTME: Walks a repository for Rust sources and lifts out the functions worth comparing
// ABOUTME: Skips test code, build output and vendored trees; keeps free fns, methods and trait defaults
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::path::{Path, PathBuf};
use std::{fs, io};

use proc_macro2::Span;
use syn::spanned::Spanned;
use syn::{Attribute, Block, ImplItem, Item, TraitItem, Type};

use crate::normalize::{self, Shape};

/// Directories never scanned: build output, dependencies, test-only trees and
/// the git-ignored clones of shared config.
const SKIPPED_DIRS: &[&str] = &[
    "target",
    "node_modules",
    ".git",
    "tests",
    "benches",
    "examples",
    "vendor",
    "fixtures",
    ".build",
    ".savoir",
];

/// One function body, normalized and placed.
#[derive(Debug)]
pub(crate) struct Unit {
    pub(crate) repo: String,
    pub(crate) file: String,
    pub(crate) name: String,
    pub(crate) start_line: usize,
    pub(crate) end_line: usize,
    pub(crate) shape: Shape,
}

/// Every `.rs` file under `root` outside the skipped directories, sorted so a
/// report is stable from run to run.
pub(crate) fn rust_files(root: &Path) -> io::Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                if !SKIPPED_DIRS.contains(&name.to_string_lossy().as_ref()) {
                    pending.push(path);
                }
            } else if is_production_source(&name.to_string_lossy()) {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

fn is_production_source(file_name: &str) -> bool {
    Path::new(file_name)
        .extension()
        .is_some_and(|ext| ext == "rs")
        && file_name != "tests.rs"
        && !file_name.ends_with("_tests.rs")
        && !file_name.ends_with("_test.rs")
}

/// Size floor below which a function is too small to report as a copy.
#[derive(Debug, Clone, Copy)]
pub(crate) struct MinimumSize {
    pub(crate) lines: usize,
    pub(crate) nodes: usize,
}

/// Collects units from one parsed file.
#[derive(Debug)]
pub(crate) struct Collector<'out> {
    pub(crate) repo: &'out str,
    pub(crate) file: &'out str,
    pub(crate) minimum: MinimumSize,
    pub(crate) units: &'out mut Vec<Unit>,
}

impl Collector<'_> {
    pub(crate) fn items(&mut self, items: &[Item]) {
        for item in items {
            match item {
                Item::Fn(f) if !is_test_only(&f.attrs) => {
                    self.add(f.sig.ident.to_string(), f.span(), &f.block);
                }
                Item::Impl(block) if !is_test_only(&block.attrs) => {
                    let owner = type_name(&block.self_ty);
                    for member in &block.items {
                        if let ImplItem::Fn(f) = member {
                            if !is_test_only(&f.attrs) {
                                self.add(format!("{owner}::{}", f.sig.ident), f.span(), &f.block);
                            }
                        }
                    }
                }
                Item::Trait(t) => self.trait_defaults(&t.ident.to_string(), &t.items),
                Item::Mod(m) if !is_test_only(&m.attrs) && m.ident != "tests" => {
                    if let Some((_, inner)) = &m.content {
                        self.items(inner);
                    }
                }
                _ => {}
            }
        }
    }

    fn trait_defaults(&mut self, owner: &str, members: &[TraitItem]) {
        for member in members {
            if let TraitItem::Fn(f) = member {
                if let Some(body) = &f.default {
                    self.add(format!("{owner}::{}", f.sig.ident), f.span(), body);
                }
            }
        }
    }

    fn add(&mut self, name: String, span: Span, body: &Block) {
        let (start_line, end_line) = (span.start().line, span.end().line);
        if end_line + 1 < start_line + self.minimum.lines {
            return;
        }
        let shape = normalize::shape_of(body);
        if shape.nodes < self.minimum.nodes {
            return;
        }
        self.units.push(Unit {
            repo: self.repo.to_owned(),
            file: self.file.to_owned(),
            name,
            start_line,
            end_line,
            shape,
        });
    }
}

fn type_name(ty: &Type) -> String {
    match ty {
        Type::Path(p) => p
            .path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default(),
        _ => "_".to_owned(),
    }
}

/// `#[test]`, `#[tokio::test]`, `#[bench]` and `#[cfg(test)]` mark code that
/// never ships; copies inside tests are not this report's concern.
fn is_test_only(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let path = attr.path();
        let last = path
            .segments
            .last()
            .map(|s| s.ident.to_string())
            .unwrap_or_default();
        if last == "test" || last == "bench" {
            return true;
        }
        path.is_ident("cfg")
            && attr
                .meta
                .require_list()
                .is_ok_and(|list| list.tokens.to_string().trim() == "test")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn units_of(code: &str, minimum: MinimumSize) -> Vec<Unit> {
        let file = syn::parse_file(code).unwrap();
        let mut units = Vec::new();
        let mut collector = Collector {
            repo: "r",
            file: "f.rs",
            minimum,
            units: &mut units,
        };
        collector.items(&file.items);
        units
    }

    const ANY_SIZE: MinimumSize = MinimumSize { lines: 0, nodes: 0 };

    #[test]
    fn collects_free_functions_methods_and_trait_defaults() {
        let code = r"
            fn free() { a(); }
            impl Store { fn save(&self) { b(); } }
            trait Named { fn name(&self) -> String { c() } fn required(&self); }
            impl Named for Store { fn required(&self) {} }
            mod inner { fn nested() { d(); } }
        ";
        let names: Vec<String> = units_of(code, ANY_SIZE)
            .into_iter()
            .map(|u| u.name)
            .collect();
        assert_eq!(
            names,
            [
                "free",
                "Store::save",
                "Named::name",
                "Store::required",
                "nested"
            ],
            "every body is a unit"
        );
    }

    #[test]
    fn skips_test_code() {
        let code = r"
            #[test] fn a_test() { a(); }
            #[tokio::test] async fn an_async_test() { a(); }
            #[cfg(test)] mod checks { fn helper() { a(); } }
            mod tests { fn helper() { a(); } }
            fn shipped() { a(); }
        ";
        let names: Vec<String> = units_of(code, ANY_SIZE)
            .into_iter()
            .map(|u| u.name)
            .collect();
        assert_eq!(names, ["shipped"], "only shipped code is compared");
    }

    #[test]
    fn records_line_spans_and_applies_the_size_floor() {
        let code = "fn one() { a(); }\n\nfn three()\n{\n    a();\n}\n";
        let units = units_of(code, MinimumSize { lines: 3, nodes: 0 });
        assert_eq!(units.len(), 1, "the one-line function is under the floor");
        assert_eq!(
            (units[0].start_line, units[0].end_line),
            (3, 6),
            "spans are 1-based lines"
        );
    }

    #[test]
    fn production_source_excludes_test_files() {
        assert!(is_production_source("lib.rs"), "a plain module is scanned");
        for name in ["tests.rs", "store_tests.rs", "store_test.rs", "notes.md"] {
            assert!(!is_production_source(name), "{name} is not production Rust");
        }
    }
}
