// ABOUTME: The dravr-contremaitre source tree this build compiled against, found without invoking cargo
// ABOUTME: Shared by the suites that read contremaitre catalogue files (persona contracts, agent frontmatter)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, dead_code)]

use std::env;
use std::fs;
use std::path::PathBuf;

/// The dravr-contremaitre tree the build compiled against.
///
/// `DRAVR_CONTREMAITRE_ROOT` when set: a local `[patch]` onto an unpushed
/// contremaitre tree leaves `Cargo.lock` without a rev. Otherwise the locked
/// rev comes from the workspace `Cargo.lock` and the tree is the rev's
/// 7-char prefix under `$CARGO_HOME/git/checkouts`, the same resolution
/// `scripts/ci/check-contremaitre-sync.sh` performs shell-side. Resolved
/// WITHOUT invoking cargo: a `cargo metadata` subprocess inside a running
/// test blocks on the package-cache lock whenever another cargo process is
/// alive — which in CI is always. The checkout is present wherever this
/// compiles, because building the workspace required fetching it.
pub(crate) fn contremaitre_tree() -> PathBuf {
    if let Ok(root) = env::var("DRAVR_CONTREMAITRE_ROOT") {
        return PathBuf::from(root);
    }
    let lock =
        fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock"))
            .expect("read workspace Cargo.lock");
    let rev = lock
        .lines()
        .find(|l| l.contains("dravr-ai/dravr-contremaitre") && l.contains('#'))
        .and_then(|l| l.rsplit('#').next())
        .map(|r| r.trim_end_matches('"').trim().to_owned())
        .expect("a dravr-contremaitre rev in Cargo.lock, or DRAVR_CONTREMAITRE_ROOT");
    let cargo_home = env::var("CARGO_HOME").map_or_else(
        |_| {
            PathBuf::from(env::var("HOME").expect("HOME set when CARGO_HOME is not")).join(".cargo")
        },
        PathBuf::from,
    );
    let checkouts = cargo_home.join("git/checkouts");
    fs::read_dir(&checkouts)
        .unwrap_or_else(|e| panic!("read {}: {e}", checkouts.display()))
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("dravr-contremaitre-"))
        })
        .expect("a dravr-contremaitre checkout")
        .join(&rev[..7])
}
