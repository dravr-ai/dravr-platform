// ABOUTME: Command line for pierre-sechoir: scan labelled repository roots, print a markdown report
// ABOUTME: Exits non-zero when nothing was compared, so a scan of the wrong paths cannot pass as clean
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Usage:
//! ```bash
//! pierre-sechoir platform=crates riviere=../dravr-riviere cageux=../dravr-cageux
//! pierre-sechoir --threshold 0.7 --min-nodes 25 a=path/a b=path/b
//! ```
//!
//! `scripts/ci/fleet-duplication.sh` clones every satellite named in
//! `satellites.toml` and runs this over all of them.

#![deny(dead_code_pub_in_binary)]

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use pierre_sechoir::{Options, Root, Similarity};

/// Finds near-duplicate Rust functions and copied files across repositories.
#[derive(Debug, Parser)]
#[command(name = "pierre-sechoir", version)]
struct Cli {
    /// Repositories to scan, each as LABEL=PATH.
    #[arg(required = true, value_parser = parse_root)]
    roots: Vec<Root>,

    /// Least similarity for two functions to be reported, 0 to 1.
    #[arg(long, default_value_t = Options::default().threshold)]
    threshold: Similarity,

    /// Functions spanning fewer lines are not compared.
    #[arg(long, default_value_t = Options::default().min_lines)]
    min_lines: usize,

    /// Functions with fewer normalized syntax nodes are not compared.
    #[arg(long, default_value_t = Options::default().min_nodes)]
    min_nodes: usize,

    /// Least share of the smaller file two files must share, 0 to 1.
    #[arg(long, default_value_t = Options::default().file_overlap)]
    file_overlap: Similarity,

    /// Least number of lines two files must share.
    #[arg(long, default_value_t = Options::default().min_shared_lines)]
    min_shared_lines: usize,
}

fn parse_root(text: &str) -> Result<Root, String> {
    let (label, path) = text
        .split_once('=')
        .ok_or_else(|| format!("expected LABEL=PATH, got {text:?}"))?;
    if label.is_empty() || path.is_empty() {
        return Err(format!("expected LABEL=PATH with both parts, got {text:?}"));
    }
    Ok(Root {
        label: label.to_owned(),
        path: PathBuf::from(path),
    })
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let options = Options {
        threshold: cli.threshold,
        min_lines: cli.min_lines,
        min_nodes: cli.min_nodes,
        file_overlap: cli.file_overlap,
        min_shared_lines: cli.min_shared_lines,
    };
    match pierre_sechoir::scan(&cli.roots, &options) {
        Ok(report) if report.functions_compared == 0 => {
            eprintln!("pierre-sechoir: no function was compared; check the roots");
            ExitCode::FAILURE
        }
        Ok(report) => {
            print!("{}", report.to_markdown());
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("pierre-sechoir: {error}");
            ExitCode::FAILURE
        }
    }
}
