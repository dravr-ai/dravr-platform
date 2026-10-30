// ABOUTME: Scans temporary repositories through pierre-sechoir's public API, as its binary does
// ABOUTME: Covers the markdown naming a cross-repo copy and the errors that refuse a scan of nothing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fs;
use std::path::Path;

use pierre_sechoir::{Options, Root, ScanError};
use tempfile::TempDir;

/// A function long enough to clear the default size floor.
fn loader(name: &str) -> String {
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
        out.push(parts[1].trim().to_string());
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
fn the_markdown_names_a_cross_repository_copy() {
    let (alpha, beta) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    write(alpha.path(), "src/load.rs", &loader("load_rows"));
    write(beta.path(), "src/import.rs", &loader("read_entries"));

    let report = pierre_sechoir::scan(
        &[root("alpha", &alpha), root("beta", &beta)],
        &Options::default(),
    )
    .unwrap();

    assert_eq!(
        report.functions_compared, 2,
        "both loaders clear the size floor"
    );
    let markdown = report.to_markdown();
    assert!(
        markdown.starts_with("**2 files, 2 functions compared.**"),
        "the summary counts what was read: {markdown}"
    );
    assert!(
        markdown.contains("| alpha ↔ beta |"),
        "the two repositories are named as sharing code: {markdown}"
    );
    assert!(
        markdown.contains("| 1.000 |") && markdown.contains("`load_rows`"),
        "the renamed copy is listed at full score: {markdown}"
    );
}

#[test]
fn a_missing_root_is_an_error() {
    let missing = Root {
        label: "gone".to_owned(),
        path: "/nonexistent/sechoir-root".into(),
    };
    let error = pierre_sechoir::scan(&[missing], &Options::default()).unwrap_err();
    assert!(
        matches!(error, ScanError::Read { .. }),
        "a root that cannot be read fails the scan: {error}"
    );
}

#[test]
fn a_root_without_rust_source_is_an_error() {
    let (alpha, empty) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    write(alpha.path(), "src/lib.rs", &loader("load_rows"));
    write(empty.path(), "README.md", "no code here");

    let error = pierre_sechoir::scan(
        &[root("alpha", &alpha), root("empty", &empty)],
        &Options::default(),
    )
    .unwrap_err();
    assert!(
        matches!(&error, ScanError::NoSources { label, .. } if label == "empty"),
        "one empty root fails the whole scan rather than reporting a partial fleet: {error}"
    );
}
