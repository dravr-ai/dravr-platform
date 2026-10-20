// ABOUTME: Keeps the server suites in one test binary, apart from those that set environment variables
// ABOUTME: A new top-level file is another ~100 MB binary; an env write inside `it` races every other suite
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Each top-level file in `tests/` links its own copy of the server. 793 of
//! them came to 84 GiB of binaries; the `it` binary holds the same suites in
//! one. Only a suite that writes the process environment stays separate,
//! because the environment is shared by every test in a binary.

use std::fs;
use std::path::{Path, PathBuf};

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests")
}

fn suites(dir: &Path, support: &str) -> Vec<(String, String)> {
    let mut found: Vec<(String, String)> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .filter(|path| path.file_name().is_some_and(|name| name != support))
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            // A scan enforcing a rule across the test tree: no lint can say
            // which binary a file belongs in, so the sources are the data.
            (name, fs::read_to_string(&path).unwrap())
        })
        .collect();
    found.sort();
    found
}

/// The environment writers, spelled in halves so this file does not match itself.
const ENV_WRITERS: [&str; 2] = [concat!("set", "_var"), concat!("remove", "_var")];

fn writes_environment(source: &str) -> bool {
    ENV_WRITERS.iter().any(|writer| source.contains(writer))
}

#[test]
fn a_top_level_suite_is_one_that_writes_the_environment() {
    let separate = suites(&tests_dir(), "common.rs");
    assert!(!separate.is_empty(), "no top-level suites: scan is stale");
    let movable: Vec<&str> = separate
        .iter()
        .filter(|(_, source)| !writes_environment(source))
        .map(|(name, _)| name.as_str())
        .collect();
    assert!(
        movable.is_empty(),
        "these suites leave the environment alone, so they belong in tests/it/ \
         (a module line in it/main.rs) rather than a binary of their own: {movable:?}"
    );
}

#[test]
fn no_suite_in_the_shared_binary_writes_the_environment() {
    let shared = suites(&tests_dir().join("it"), "main.rs");
    assert!(shared.len() > 1, "no it/ modules: scan is stale");
    let writers: Vec<&str> = shared
        .iter()
        .filter(|(_, source)| writes_environment(source))
        .map(|(name, _)| name.as_str())
        .collect();
    assert!(
        writers.is_empty(),
        "these it/ modules write the process environment, which every suite in the \
         binary shares; move each to tests/ as its own binary: {writers:?}"
    );
}
