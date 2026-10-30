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

pub use report::Report;
pub use similarity::{ParseSimilarityError, Similarity};

use extract::{Collector, MinimumSize, Unit};
use files::{CopyRule, SourceLines};
use report::{CopiedFile, FileSite, FunctionSite, SimilarFunctions};

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
    /// A root holds no Rust source, so nothing in it was compared: a wrong
    /// path or an empty clone, never a clean result.
    #[error("{label} ({path}) holds no Rust source to compare")]
    NoSources {
        /// The root's label.
        label: String,
        /// The directory that was scanned.
        path: PathBuf,
    },
}

/// Scan every root and compare what was found.
///
/// # Errors
///
/// [`ScanError::Read`] when a root or one of its files cannot be read, and
/// [`ScanError::NoSources`] when a root holds no Rust source. A file that reads
/// but does not parse is listed in the report as not parsed.
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
        if paths.is_empty() {
            return Err(ScanError::NoSources {
                label: root.label.clone(),
                path: root.path.clone(),
            });
        }
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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use tempfile::TempDir;

    use super::*;

    /// A function long enough to clear the default size floor.
    fn loader(name: &str, field: &str) -> String {
        format!(
            "pub fn {name}(path: &str) -> Result<Vec<String>, String> {{
    let text = fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for line in text.lines() {{
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {{
            continue;
        }}
        let parts: Vec<&str> = trimmed.split(',').collect();
        if parts.len() < 3 {{
            return Err(format!(\"bad line {{}}\", line));
        }}
        out.push(parts[2].to_uppercase());
        out.push(parts[1].{field}().to_string());
    }}
    Ok(out)
}}
"
        )
    }

    fn write(root: &Path, relative: &str, text: &str) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    fn root(label: &str, dir: &TempDir) -> Root {
        Root {
            label: label.to_owned(),
            path: dir.path().to_path_buf(),
        }
    }

    #[test]
    fn reports_a_renamed_copy_across_repositories() {
        let (alpha, beta) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        write(alpha.path(), "src/load.rs", &loader("load_rows", "trim"));
        write(
            beta.path(),
            "src/import.rs",
            &loader("read_entries", "trim"),
        );
        write(beta.path(), "src/other.rs", &loader("different", "len"));

        let report = scan(
            &[root("alpha", &alpha), root("beta", &beta)],
            &Options::default(),
        )
        .unwrap();

        assert_eq!(report.files_scanned, 3, "every source file is read");
        assert_eq!(
            report.functions_compared, 3,
            "all three loaders clear the size floor"
        );
        let first = &report.similar_functions[0];
        assert!(
            first.crosses_repos(),
            "the cross-repository copy is listed first"
        );
        assert_eq!(
            first.score.to_string(),
            "1.000",
            "renaming the function changes nothing"
        );
        let mut names = [first.left.name.as_str(), first.right.name.as_str()];
        names.sort_unstable();
        assert_eq!(
            names,
            ["load_rows", "read_entries"],
            "the two copies are named"
        );
        assert!(
            report
                .similar_functions
                .iter()
                .all(|p| p.left.name != "different" || p.score < first.score),
            "calling .len() instead of .trim() lowers the score"
        );
    }

    #[test]
    fn reports_a_copied_file_and_ignores_test_code() {
        use std::fmt::Write;

        let (alpha, beta) = (TempDir::new().unwrap(), TempDir::new().unwrap());
        // Twenty distinct settings plus a loader: a scaffold file, well past the
        // fifteen shared lines a copy needs.
        let settings = (0..20).fold(String::new(), |mut out, n| {
            let _ = writeln!(out, "pub const SETTING_{n}: &str = \"value-{n}\";");
            out
        });
        let scaffold = settings + &loader("serve", "trim");
        write(alpha.path(), "src/main.rs", &scaffold);
        write(beta.path(), "src/main.rs", &scaffold);
        write(
            alpha.path(),
            "tests/it/copy.rs",
            &loader("in_a_test", "trim"),
        );
        write(
            beta.path(),
            "src/checks.rs",
            &format!(
                "#[cfg(test)]\nmod checks {{\n{}}}\n",
                loader("also_test", "trim")
            ),
        );

        let report = scan(
            &[root("alpha", &alpha), root("beta", &beta)],
            &Options::default(),
        )
        .unwrap();

        // checks.rs repeats the loader's lines too, so it is a line-level copy as
        // well; what matters here is that the identical main.rs pair is found.
        let copy = report
            .copied_files
            .iter()
            .find(|c| c.left.file == "src/main.rs" && c.right.file == "src/main.rs")
            .expect("main.rs is reported as copied between the repositories");
        assert_ne!(
            copy.left.repo, copy.right.repo,
            "the copy spans two repositories"
        );
        assert_eq!(copy.overlap.to_string(), "1.000", "the files are identical");
        let names: Vec<&str> = report
            .similar_functions
            .iter()
            .flat_map(|p| [p.left.name.as_str(), p.right.name.as_str()])
            .collect();
        assert!(
            !names.contains(&"in_a_test") && !names.contains(&"also_test"),
            "test code is not compared: {names:?}"
        );
    }

    #[test]
    fn lists_files_that_do_not_parse() {
        let alpha = TempDir::new().unwrap();
        write(alpha.path(), "src/lib.rs", &loader("load_rows", "trim"));
        write(alpha.path(), "src/broken.rs", "fn half_written( {");

        let report = scan(&[root("alpha", &alpha)], &Options::default()).unwrap();

        assert_eq!(
            report.unparsed_files,
            ["alpha:src/broken.rs"],
            "the broken file is named, not skipped silently"
        );
        assert_eq!(
            report.functions_compared, 1,
            "the parseable file is still compared"
        );
    }
}
