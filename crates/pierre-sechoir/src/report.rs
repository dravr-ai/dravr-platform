// ABOUTME: The findings of a scan and their markdown rendering
// ABOUTME: Cross-repository copies come first: they are the ones a shared crate can remove
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::similarity::Similarity;

/// Where a function lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FunctionSite {
    /// Label of the repository it was found in.
    pub repo: String,
    /// Path relative to that repository's scanned root.
    pub file: String,
    /// `function`, `Type::method` or `Trait::method`.
    pub name: String,
    /// First line, 1-based.
    pub start_line: usize,
    /// Last line, inclusive.
    pub end_line: usize,
}

/// Two functions with near-identical structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimilarFunctions {
    /// Jaccard similarity of their fingerprint sets.
    pub score: Similarity,
    /// Node count of the smaller body: how much code the copy spans.
    pub nodes: usize,
    /// One of the two functions.
    pub left: FunctionSite,
    /// The other.
    pub right: FunctionSite,
}

impl SimilarFunctions {
    /// True when the two functions live in different repositories.
    #[must_use]
    pub fn crosses_repos(&self) -> bool {
        self.left.repo != self.right.repo
    }
}

/// A file in one repository.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileSite {
    /// Label of the repository.
    pub repo: String,
    /// Path relative to the scanned root.
    pub file: String,
    /// Meaningful lines in the file.
    pub lines: usize,
}

/// Two files in different repositories sharing most of their lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CopiedFile {
    /// Shared lines as a share of the smaller file.
    pub overlap: Similarity,
    /// Meaningful lines the two files share.
    pub shared_lines: usize,
    /// One of the two files.
    pub left: FileSite,
    /// The other.
    pub right: FileSite,
}

/// Everything one scan found.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Report {
    /// Rust files read, across every root.
    pub files_scanned: usize,
    /// Functions above the size floor, the population that was compared.
    pub functions_compared: usize,
    /// Files `syn` could not parse, so none of their functions were compared.
    pub unparsed_files: Vec<String>,
    /// Function pairs: cross-repository first, then by score and size.
    pub similar_functions: Vec<SimilarFunctions>,
    /// File pairs across repositories, most shared lines first.
    pub copied_files: Vec<CopiedFile>,
}

impl Report {
    /// The report as markdown, fit for a terminal or an issue body.
    #[must_use]
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        let cross = self
            .similar_functions
            .iter()
            .filter(|p| p.crosses_repos())
            .count();
        let _ = writeln!(
            out,
            "**{} files, {} functions compared.** {} copied files across repos; {} similar \
             function pairs, {cross} of them across repos.\n",
            self.files_scanned,
            self.functions_compared,
            self.copied_files.len(),
            self.similar_functions.len(),
        );
        self.write_repo_pairs(&mut out);
        self.write_copied_files(&mut out);
        self.write_functions(&mut out);
        if !self.unparsed_files.is_empty() {
            let _ = writeln!(out, "\n### Not parsed ({})\n", self.unparsed_files.len());
            for file in &self.unparsed_files {
                let _ = writeln!(out, "- `{file}`");
            }
        }
        out
    }

    /// Which pairs of repositories share the most code, the shape of the work.
    fn write_repo_pairs(&self, out: &mut String) {
        let mut totals: BTreeMap<(String, String), (usize, usize)> = BTreeMap::new();
        for copy in &self.copied_files {
            totals
                .entry(repo_pair(&copy.left.repo, &copy.right.repo))
                .or_default()
                .0 += copy.shared_lines;
        }
        for pair in self.similar_functions.iter().filter(|p| p.crosses_repos()) {
            totals
                .entry(repo_pair(&pair.left.repo, &pair.right.repo))
                .or_default()
                .1 += 1;
        }
        if totals.is_empty() {
            return;
        }
        let mut rows: Vec<_> = totals.into_iter().collect();
        rows.sort_by(|a, b| {
            (b.1 .0 + b.1 .1)
                .cmp(&(a.1 .0 + a.1 .1))
                .then_with(|| a.0.cmp(&b.0))
        });
        out.push_str("### Repos sharing code\n\n| Repos | Lines in copied files | Similar functions |\n|---|---:|---:|\n");
        for ((a, b), (lines, functions)) in rows {
            let _ = writeln!(out, "| {a} ↔ {b} | {lines} | {functions} |");
        }
    }

    fn write_copied_files(&self, out: &mut String) {
        if self.copied_files.is_empty() {
            return;
        }
        out.push_str(
            "\n### Copied files\n\n| Overlap | Shared lines | File | File |\n|---:|---:|---|---|\n",
        );
        for copy in &self.copied_files {
            let _ = writeln!(
                out,
                "| {} | {} | {}:`{}` ({}) | {}:`{}` ({}) |",
                copy.overlap,
                copy.shared_lines,
                copy.left.repo,
                copy.left.file,
                copy.left.lines,
                copy.right.repo,
                copy.right.file,
                copy.right.lines,
            );
        }
    }

    fn write_functions(&self, out: &mut String) {
        if self.similar_functions.is_empty() {
            return;
        }
        out.push_str("\n### Similar functions\n\n| Score | Nodes | Function | Function |\n|---:|---:|---|---|\n");
        for pair in &self.similar_functions {
            let _ = writeln!(
                out,
                "| {} | {} | {} | {} |",
                pair.score,
                pair.nodes,
                function_cell(&pair.left),
                function_cell(&pair.right)
            );
        }
    }
}

fn repo_pair(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_owned(), b.to_owned())
    } else {
        (b.to_owned(), a.to_owned())
    }
}

fn function_cell(site: &FunctionSite) -> String {
    format!(
        "{}:`{}:{}-{}` `{}`",
        site.repo, site.file, site.start_line, site.end_line, site.name
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn site(repo: &str, name: &str) -> FunctionSite {
        FunctionSite {
            repo: repo.to_owned(),
            file: "src/lib.rs".to_owned(),
            name: name.to_owned(),
            start_line: 3,
            end_line: 9,
        }
    }

    #[test]
    fn markdown_lists_repo_pairs_files_and_functions() {
        let report = Report {
            files_scanned: 4,
            functions_compared: 10,
            unparsed_files: vec!["b:src/odd.rs".to_owned()],
            similar_functions: vec![SimilarFunctions {
                score: Similarity::ONE,
                nodes: 50,
                left: site("b", "load"),
                right: site("a", "load"),
            }],
            copied_files: vec![CopiedFile {
                overlap: "0.9".parse().unwrap(),
                shared_lines: 30,
                left: FileSite {
                    repo: "a".to_owned(),
                    file: "src/main.rs".to_owned(),
                    lines: 33,
                },
                right: FileSite {
                    repo: "b".to_owned(),
                    file: "src/main.rs".to_owned(),
                    lines: 34,
                },
            }],
        };
        let markdown = report.to_markdown();
        assert!(
            markdown.starts_with("**4 files, 10 functions compared.** 1 copied files"),
            "summary first"
        );
        assert!(
            markdown.contains("| a ↔ b | 30 | 1 |"),
            "repo pairs are ordered and totalled: {markdown}"
        );
        assert!(
            markdown.contains("| 0.900 | 30 | a:`src/main.rs` (33) | b:`src/main.rs` (34) |"),
            "{markdown}"
        );
        assert!(
            markdown
                .contains("| 1.000 | 50 | b:`src/lib.rs:3-9` `load` | a:`src/lib.rs:3-9` `load` |"),
            "{markdown}"
        );
        assert!(
            markdown.contains("### Not parsed (1)\n\n- `b:src/odd.rs`"),
            "unparsed files are listed"
        );
    }

    #[test]
    fn an_empty_report_is_only_the_summary() {
        let markdown = Report::default().to_markdown();
        assert_eq!(
            markdown.lines().filter(|l| !l.is_empty()).count(),
            1,
            "nothing to list: {markdown}"
        );
    }
}
