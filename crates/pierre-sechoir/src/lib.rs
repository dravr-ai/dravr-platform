// ABOUTME: Structural duplicate finder for the platform and its satellite repositories
// ABOUTME: Reports near-identical Rust functions and files copied between repos, cross-repo first
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! `sechoir` (French for a dryer) follows unclebob's
//! [dryer](https://github.com/unclebob/dryer): normalize each function so
//! local names and literals vanish while called functions, methods, macros and
//! operators stay, fingerprint every subtree, and report pairs whose
//! fingerprint sets overlap past a threshold.
//!
//! Two things are added for a fleet of repositories. Every root carries a
//! label, so a copy between two repositories is reported as such. Files are
//! also compared line by line, because a copied scaffold is many functions
//! each too small to report alone.

mod extract;
mod files;
mod join;
mod normalize;
mod report;
mod similarity;

use std::path::{Path, PathBuf};
use std::{fs, io};

pub use report::{CopiedFile, FileSite, FunctionSite, Report, SimilarFunctions};
pub use similarity::{ParseSimilarityError, Similarity};

use extract::{Collector, MinimumSize, Unit};
use files::{CopyRule, SourceLines};

/// A repository to scan, under the label the report names it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    /// Short name shown in the report, such as `riviere`.
    pub label: String,
    /// Directory scanned recursively.
    pub path: PathBuf,
}

/// Thresholds for what counts as a copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Options {
    /// Least Jaccard similarity for two functions to be reported.
    pub threshold: Similarity,
    /// Functions spanning fewer lines are not compared.
    pub min_lines: usize,
    /// Functions with fewer normalized nodes are not compared.
    pub min_nodes: usize,
    /// Least share of the smaller file two files must share.
    pub file_overlap: Similarity,
    /// Least number of shared lines for two files to be reported.
    pub min_shared_lines: usize,
}

impl Default for Options {
    /// dryer's default threshold; size floors tuned on the dravr fleet so a
    /// report holds copies worth extracting rather than look-alike accessors.
    fn default() -> Self {
        Self {
            threshold: Similarity::ratio(82, 100),
            min_lines: 5,
            min_nodes: 40,
            file_overlap: Similarity::ratio(1, 2),
            min_shared_lines: 15,
        }
    }
}

/// Why a scan could not complete.
#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    /// A root or a file under it could not be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// The directory or file that failed.
        path: PathBuf,
        /// The underlying I/O error.
        source: io::Error,
    },
}

/// Scan every root and compare what was found.
///
/// # Errors
///
/// [`ScanError::Read`] when a root or one of its files cannot be read. A file
/// that reads but does not parse is listed in [`Report::unparsed_files`].
pub fn scan(roots: &[Root], options: &Options) -> Result<Report, ScanError> {
    let minimum = MinimumSize {
        lines: options.min_lines,
        nodes: options.min_nodes,
    };
    let mut report = Report::default();
    let mut units = Vec::new();
    let mut sources = Vec::new();
    for root in roots {
        let paths = extract::rust_files(&root.path).map_err(|source| ScanError::Read {
            path: root.path.clone(),
            source,
        })?;
        report.files_scanned += paths.len();
        for path in paths {
            let text = fs::read_to_string(&path).map_err(|source| ScanError::Read {
                path: path.clone(),
                source,
            })?;
            let file = relative(&path, &root.path);
            sources.push(SourceLines::from_source(&root.label, &file, &text));
            match syn::parse_file(&text) {
                Ok(ast) => Collector {
                    repo: &root.label,
                    file: &file,
                    minimum,
                    units: &mut units,
                }
                .items(&ast.items),
                Err(_) => report.unparsed_files.push(format!("{}:{file}", root.label)),
            }
        }
    }
    report.functions_compared = units.len();
    report.similar_functions = similar_functions(&units, options.threshold);
    let rule = CopyRule {
        min_overlap: options.file_overlap,
        min_shared_lines: options.min_shared_lines,
    };
    report.copied_files = copied_files(&sources, rule);
    Ok(report)
}

fn relative(path: &Path, root: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned()
}

fn similar_functions(units: &[Unit], threshold: Similarity) -> Vec<SimilarFunctions> {
    let mut found: Vec<SimilarFunctions> = join::similar_pairs(units, threshold)
        .into_iter()
        .map(|pair| {
            let (left, right) = (&units[pair.left], &units[pair.right]);
            SimilarFunctions {
                score: pair.score,
                nodes: left.shape.nodes.min(right.shape.nodes),
                left: function_site(left),
                right: function_site(right),
            }
        })
        .collect();
    found.sort_by(|a, b| {
        b.crosses_repos()
            .cmp(&a.crosses_repos())
            .then(b.score.cmp(&a.score))
            .then(b.nodes.cmp(&a.nodes))
            .then_with(|| {
                (&a.left.repo, &a.left.file, a.left.start_line).cmp(&(
                    &b.left.repo,
                    &b.left.file,
                    b.left.start_line,
                ))
            })
    });
    found
}

fn function_site(unit: &Unit) -> FunctionSite {
    FunctionSite {
        repo: unit.repo.clone(),
        file: unit.file.clone(),
        name: unit.name.clone(),
        start_line: unit.start_line,
        end_line: unit.end_line,
    }
}

fn copied_files(sources: &[SourceLines], rule: CopyRule) -> Vec<CopiedFile> {
    let site = |index: usize| FileSite {
        repo: sources[index].repo.clone(),
        file: sources[index].file.clone(),
        lines: sources[index].lines.len(),
    };
    let mut found: Vec<CopiedFile> = files::copied_files(sources, rule)
        .into_iter()
        .map(|pair| CopiedFile {
            overlap: pair.overlap,
            shared_lines: pair.shared,
            left: site(pair.left),
            right: site(pair.right),
        })
        .collect();
    found.sort_by(|a, b| {
        b.shared_lines
            .cmp(&a.shared_lines)
            .then(b.overlap.cmp(&a.overlap))
            .then_with(|| (&a.left.repo, &a.left.file).cmp(&(&b.left.repo, &b.left.file)))
    });
    found
}
