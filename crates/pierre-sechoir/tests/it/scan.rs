// ABOUTME: Scans two temporary repositories and checks what pierre-sechoir reports across them
// ABOUTME: Covers renamed copies, copied files, test-code exclusion, unparsed files and read errors
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::fmt::Write;
use std::fs;
use std::path::Path;

use pierre_sechoir::{Options, Root, ScanError};
use tempfile::TempDir;

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

    let report = pierre_sechoir::scan(
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

    let report = pierre_sechoir::scan(
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

    let report = pierre_sechoir::scan(&[root("alpha", &alpha)], &Options::default()).unwrap();

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
