// ABOUTME: Cargo build script that automatically builds the TypeScript SDK
// ABOUTME: Ensures sdk/dist/ exists before running tests that depend on it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Automatic SDK Builder
//!
//! This build script ensures the TypeScript SDK in `sdk/` is automatically
//! built before running Cargo commands that need it (like `cargo test` or `cargo build`).
//!
//! ## Why This Exists
//!
//! Some E2E tests (e.g., `test_concurrent_multitenant_get_activities`) require
//! `sdk/dist/cli.js` to exist. Since `sdk/dist/` is gitignored (build artifacts
//! shouldn't be committed), local developers would need to manually run:
//! ```bash
//! cd sdk && bun install && bun run build
//! ```
//!
//! This build script automates that process, ensuring the SDK is always available.
//!
//! ## When SDK is Built
//!
//! The SDK is built automatically when:
//! - `sdk/dist/cli.js` doesn't exist (first time setup)
//! - SDK source files change (`sdk/src/`, `sdk/package.json`, `sdk/tsconfig.json`,
//!   `sdk/bun.lock`): `sdk/dist/.source-digest` records the sources the bundle
//!   was built from, and a different digest rebuilds it
//!
//! ## CI/CD Integration
//!
//! CI workflows (`.github/workflows/ci.yml`) explicitly build the SDK before tests.
//! This build script serves as a safety net for local development.

use std::collections::hash_map::DefaultHasher;
use std::env;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    // Embed git commit SHA for deploy notifications and health checks
    embed_git_commit_sha();

    // The SDK lives at the workspace root, two levels above this crate
    let Some(workspace_root) = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
    else {
        println!("cargo:warning=Cannot determine workspace root - skipping SDK build");
        return;
    };

    let sdk_path = workspace_root.join("sdk");
    if !sdk_path.exists() {
        println!("cargo:warning=SDK directory not found - skipping SDK build");
        return;
    }

    // Tell Cargo to re-run this script if SDK source files change
    for input in SDK_INPUTS {
        println!("cargo:rerun-if-changed={}", sdk_path.join(input).display());
    }
    let dist_path = sdk_path.join("dist");
    let stamp = dist_path.join(SOURCE_DIGEST_STAMP);
    let digest = sdk_source_digest(&sdk_path);

    // Build when the bundle is missing, when the sources it was built from are
    // not the sources on disk now, or in development. A bundle that merely
    // exists proved nothing: once built, it was never rebuilt, and an SDK
    // change ran its E2E tests against the old bundle.
    let built_from = fs::read_to_string(&stamp).ok();
    let current = dist_path.join("cli.js").exists() && built_from.as_deref() == Some(&*digest);
    if current && env::var("CARGO_FEATURE_DEV").is_err() {
        return;
    }

    println!("cargo:warning=Building TypeScript SDK (required for E2E tests)...");
    // bun, never npm: a second package manager writes a second lockfile beside
    // bun.lock and the two drift.
    if Command::new("bun").arg("--version").output().is_err() {
        println!("cargo:warning=bun not found - skipping SDK build");
        println!("cargo:warning=Install bun to enable SDK-dependent tests");
        return;
    }
    if !sdk_path.join("node_modules").exists()
        && !run_bun(
            &sdk_path,
            &["install", "--frozen-lockfile"],
            "install SDK dependencies",
        )
    {
        return;
    }
    if run_bun(&sdk_path, &["run", "build"], "build the SDK") {
        // The stamp records what this bundle was built from; a failed write
        // only costs a rebuild next time.
        let _ = fs::write(&stamp, &digest);
        println!("cargo:warning=SDK built successfully: sdk/dist/cli.js");
    }
}

/// The SDK files the bundle is built from, relative to `sdk/`.
const SDK_INPUTS: [&str; 4] = ["src", "package.json", "tsconfig.json", "bun.lock"];

/// The file in `sdk/dist/` that names the sources the bundle was built from.
const SOURCE_DIGEST_STAMP: &str = ".source-digest";

/// Run `bun <args>` in the SDK, reporting a failure as a build warning.
fn run_bun(sdk_path: &Path, args: &[&str], what: &str) -> bool {
    match Command::new("bun")
        .args(args)
        .current_dir(sdk_path)
        .status()
    {
        Ok(status) if status.success() => true,
        Ok(status) => {
            println!("cargo:warning=Failed to {what} (exit {status}); some E2E tests may fail");
            false
        }
        Err(e) => {
            println!("cargo:warning=Failed to {what}: {e}; some E2E tests may fail");
            false
        }
    }
}

/// A digest of every SDK input's path and content, in path order.
///
/// Compared only with a digest this same script wrote, so the hasher need not
/// be stable across toolchains: a new one costs a rebuild, nothing more.
fn sdk_source_digest(sdk_path: &Path) -> String {
    let mut files = Vec::new();
    for input in SDK_INPUTS {
        collect_files(&sdk_path.join(input), &mut files);
    }
    files.sort();
    let mut hasher = DefaultHasher::new();
    for file in files {
        file.hash(&mut hasher);
        fs::read(&file).unwrap_or_default().hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

fn collect_files(path: &Path, out: &mut Vec<PathBuf>) {
    if path.is_dir() {
        for entry in fs::read_dir(path).into_iter().flatten().flatten() {
            collect_files(&entry.path(), out);
        }
    } else if path.is_file() {
        out.push(path.to_path_buf());
    }
}

/// Embed the git commit SHA as a compile-time environment variable
///
/// Sets `GIT_COMMIT_SHA` so Rust code can access it via `env!("GIT_COMMIT_SHA")`.
/// Falls back to "unknown" if git is unavailable (e.g., in Docker builds without .git).
fn embed_git_commit_sha() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or_else(|| Path::new("."));

    // Re-run when HEAD changes (new commits, branch switches)
    let git_head = workspace_root.join(".git/HEAD");
    if git_head.exists() {
        println!("cargo:rerun-if-changed={}", git_head.display());
    }
    let git_refs = workspace_root.join(".git/refs");
    if git_refs.exists() {
        println!("cargo:rerun-if-changed={}", git_refs.display());
    }

    let sha = Command::new("git")
        .args(["rev-parse", "--short", "HEAD"])
        .current_dir(workspace_root)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map_or_else(|| "unknown".to_owned(), |s| s.trim().to_owned());

    println!("cargo:rustc-env=GIT_COMMIT_SHA={sha}");
}
