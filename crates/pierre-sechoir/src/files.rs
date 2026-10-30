// ABOUTME: Finds whole source files copied from one repository into another
// ABOUTME: Compares the sets of meaningful lines; per-function matching misses a copied file of small functions
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A copied scaffold (a `main.rs`, an `auth.rs`) is many small functions, each
//! under the size floor of the function comparison. Comparing files line by
//! line catches it. Lines that appear in many files (imports, derives) say
//! nothing about copying and are ignored.

use std::collections::{HashMap, HashSet};

use crate::similarity::Similarity;

/// A line found in more files than this is boilerplate, not evidence of a copy.
const BOILERPLATE_FILE_COUNT: usize = 40;

/// Lines that carry no content of their own.
const STRUCTURAL_LINES: &[&str] = &["}", "};", "});", "),", "{", "})", ");"];

/// One file's meaningful lines.
#[derive(Debug)]
pub(crate) struct SourceLines {
    pub(crate) repo: String,
    pub(crate) file: String,
    pub(crate) lines: HashSet<String>,
}

impl SourceLines {
    /// Trimmed, non-comment lines longer than a few characters.
    pub(crate) fn from_source(repo: &str, file: &str, source: &str) -> Self {
        let lines = source
            .lines()
            .map(str::trim)
            .filter(|line| line.len() > 3 && !line.starts_with("//"))
            .filter(|line| !STRUCTURAL_LINES.contains(line))
            .map(str::to_owned)
            .collect();
        Self {
            repo: repo.to_owned(),
            file: file.to_owned(),
            lines,
        }
    }
}

/// Two files in different repositories sharing many lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FilePair {
    pub(crate) left: usize,
    pub(crate) right: usize,
    pub(crate) shared: usize,
    /// Shared lines as a share of the smaller file.
    pub(crate) overlap: Similarity,
}

/// What it takes for two files to count as a copy.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CopyRule {
    pub(crate) min_overlap: Similarity,
    pub(crate) min_shared_lines: usize,
}

/// Cross-repository file pairs that meet `rule`.
pub(crate) fn copied_files(files: &[SourceLines], rule: CopyRule) -> Vec<FilePair> {
    let mut holders: HashMap<&str, Vec<usize>> = HashMap::new();
    for (index, file) in files.iter().enumerate() {
        if file.lines.len() >= rule.min_shared_lines {
            for line in &file.lines {
                holders.entry(line.as_str()).or_default().push(index);
            }
        }
    }
    let mut shared: HashMap<(usize, usize), usize> = HashMap::new();
    for group in holders
        .values()
        .filter(|g| g.len() <= BOILERPLATE_FILE_COUNT)
    {
        for (n, &a) in group.iter().enumerate() {
            for &b in &group[n + 1..] {
                if files[a].repo != files[b].repo {
                    *shared.entry((a, b)).or_default() += 1;
                }
            }
        }
    }
    shared
        .into_iter()
        .filter_map(|((left, right), count)| {
            let smaller = files[left].lines.len().min(files[right].lines.len());
            let qualifies =
                count >= rule.min_shared_lines && rule.min_overlap.admits(count, smaller);
            qualifies.then(|| FilePair {
                left,
                right,
                shared: count,
                overlap: Similarity::ratio(count, smaller),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbered(prefix: &str, count: usize) -> String {
        use std::fmt::Write;
        (0..count).fold(String::new(), |mut out, n| {
            let _ = writeln!(out, "let {prefix}_{n} = compute({n});");
            out
        })
    }

    const RULE: CopyRule = CopyRule {
        min_overlap: Similarity::ONE,
        min_shared_lines: 5,
    };

    #[test]
    fn skips_comments_braces_and_short_lines() {
        let source = "// note\nfn run() {\n}\n  x;\n    call_something();\n";
        let lines = SourceLines::from_source("r", "f.rs", source);
        let mut kept: Vec<&str> = lines.lines.iter().map(String::as_str).collect();
        kept.sort_unstable();
        assert_eq!(
            kept,
            ["call_something();", "fn run() {"],
            "only lines with content are kept"
        );
    }

    #[test]
    fn reports_a_file_copied_across_repositories() {
        let body = numbered("value", 10);
        let files = [
            SourceLines::from_source("a", "src/x.rs", &body),
            SourceLines::from_source("b", "src/y.rs", &body),
            SourceLines::from_source("a", "src/z.rs", &body),
        ];
        let pairs = copied_files(&files, RULE);
        let mut found: Vec<(usize, usize)> = pairs.iter().map(|p| (p.left, p.right)).collect();
        found.sort_unstable();
        assert_eq!(
            found,
            [(0, 1), (1, 2)],
            "copies are reported across repositories only"
        );
        assert!(
            pairs.iter().all(|p| p.shared == 10),
            "all ten lines are shared"
        );
    }

    #[test]
    fn small_overlap_is_not_a_copy() {
        let files = [
            SourceLines::from_source("a", "x.rs", &(numbered("left", 10) + &numbered("same", 4))),
            SourceLines::from_source("b", "y.rs", &(numbered("right", 10) + &numbered("same", 4))),
        ];
        let rule = CopyRule {
            min_overlap: "0.5".parse().unwrap(),
            min_shared_lines: 3,
        };
        assert!(
            copied_files(&files, rule).is_empty(),
            "4 of 14 lines is under half"
        );
    }

    #[test]
    fn boilerplate_lines_are_not_evidence() {
        let shared_import = "use serde::Serialize;\n";
        let files: Vec<SourceLines> = (0..=BOILERPLATE_FILE_COUNT)
            .map(|n| {
                let source = format!("{shared_import}{}", numbered(&format!("own{n}"), 6));
                SourceLines::from_source(&format!("repo{n}"), "lib.rs", &source)
            })
            .collect();
        let rule = CopyRule {
            min_overlap: "0.1".parse().unwrap(),
            min_shared_lines: 1,
        };
        assert!(
            copied_files(&files, rule).is_empty(),
            "a line in every file proves nothing"
        );
    }
}
