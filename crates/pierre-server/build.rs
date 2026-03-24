// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: Cargo build script that automatically builds the TypeScript SDK
// ABOUTME: Ensures sdk/dist/ exists before running tests that depend on it

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
//! cd sdk && npm install && npm run build
//! ```
//!
//! This build script automates that process, ensuring the SDK is always available.
//!
//! ## When SDK is Built
//!
//! The SDK is built automatically when:
//! - `sdk/dist/cli.js` doesn't exist (first time setup)
//! - SDK source files change (`sdk/src/`, `sdk/package.json`, `sdk/tsconfig.json`)
//!
//! ## CI/CD Integration
//!
//! CI workflows (`.github/workflows/ci.yml`) explicitly build the SDK before tests.
//! This build script serves as a safety net for local development.

use std::env;
use std::path::Path;
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

    // Tell Cargo to re-run this script if SDK source files change
    println!("cargo:rerun-if-changed={}", sdk_path.join("src").display());
    println!(
        "cargo:rerun-if-changed={}",
        sdk_path.join("package.json").display()
    );
    println!(
        "cargo:rerun-if-changed={}",
        sdk_path.join("tsconfig.json").display()
    );
    let dist_path = sdk_path.join("dist");
    let cli_js = dist_path.join("cli.js");

    // Only build SDK if it doesn't exist or if we're in development
    // In CI, the SDK should already be built by the workflow
    let should_build = !cli_js.exists() || env::var("CARGO_FEATURE_DEV").is_ok();

    if should_build && sdk_path.exists() {
        println!("cargo:warning=Building TypeScript SDK (required for E2E tests)...");

        // Check if npm is available
        let npm_check = if cfg!(target_os = "windows") {
            Command::new("cmd")
                .args(["/C", "npm", "--version"])
                .output()
        } else {
            Command::new("npm").arg("--version").output()
        };

        if npm_check.is_err() {
            println!("cargo:warning=npm not found - skipping SDK build");
            println!("cargo:warning=Install Node.js to enable SDK-dependent tests");
            return;
        }

        // Install dependencies if node_modules doesn't exist
        let node_modules = sdk_path.join("node_modules");
        if !node_modules.exists() {
            println!("cargo:warning=Installing SDK dependencies...");
            let install_status = if cfg!(target_os = "windows") {
                Command::new("cmd")
                    .args(["/C", "npm", "install"])
                    .current_dir(&sdk_path)
                    .status()
            } else {
                Command::new("npm")
                    .arg("install")
                    .current_dir(&sdk_path)
                    .status()
            };

            match install_status {
                Ok(status) if status.success() => {
                    println!("cargo:warning=SDK dependencies installed successfully");
                }
                Ok(status) => {
                    println!(
                        "cargo:warning=SDK dependency installation failed with exit code: {status}"
                    );
                    println!("cargo:warning=Some tests may fail without SDK");
                    return;
                }
                Err(e) => {
                    println!("cargo:warning=Failed to run npm install: {e}");
                    println!("cargo:warning=Some tests may fail without SDK");
                    return;
                }
            }
        }

        // Build the SDK
        println!("cargo:warning=Compiling TypeScript SDK...");
        let build_status = if cfg!(target_os = "windows") {
            Command::new("cmd")
                .args(["/C", "npm", "run", "build"])
                .current_dir(&sdk_path)
                .status()
        } else {
            Command::new("npm")
                .args(["run", "build"])
                .current_dir(&sdk_path)
                .status()
        };

        match build_status {
            Ok(status) if status.success() => {
                println!("cargo:warning=SDK built successfully: sdk/dist/cli.js");
            }
            Ok(status) => {
                println!("cargo:warning=SDK build failed with exit code: {status}");
                println!("cargo:warning=Some E2E tests may fail");
            }
            Err(e) => {
                println!("cargo:warning=Failed to run npm build: {e}");
                println!("cargo:warning=Some E2E tests may fail");
            }
        }
    } else if cli_js.exists() {
        println!("cargo:warning=SDK already built (sdk/dist/cli.js exists)");
    } else {
        println!("cargo:warning=SDK directory not found - skipping SDK build");
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
