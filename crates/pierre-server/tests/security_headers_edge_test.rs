// ABOUTME: Pins nginx as the single source of browser security response headers
// ABOUTME: and asserts no Rust source in the workspace serves a competing header set
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Browser security response headers are set once, at the nginx edge.
//!
//! `docker/images/frontend/security-headers.conf` carries the whole set and
//! `nginx.conf` includes it at server level and again inside every location
//! that declares an `add_header` of its own (nginx drops inherited
//! `add_header` directives the moment a location declares one). The API
//! service runs behind that nginx with internal-only ingress.
//!
//! These tests guard both halves of that arrangement: the edge really sets the
//! headers, and no Rust crate re-grows a second, unserved copy of them.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::str_to_string
)]

use std::fs;
use std::path::{Path, PathBuf};

/// Header names the edge is responsible for.
const EDGE_HEADERS: &[&str] = &[
    "X-Content-Type-Options",
    "X-Frame-Options",
    "Referrer-Policy",
    "Strict-Transport-Security",
    "Content-Security-Policy",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root resolves from the crate manifest directory")
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("{} is readable: {e}", path.display()))
}

#[test]
fn edge_snippet_sets_every_security_header() {
    let snippet = read("docker/images/frontend/security-headers.conf");

    for header in EDGE_HEADERS {
        assert!(
            snippet.contains(&format!("add_header {header} ")),
            "security-headers.conf must set {header}"
        );
    }

    assert!(
        snippet.contains(r#"add_header X-Content-Type-Options "nosniff" always;"#),
        "X-Content-Type-Options must be nosniff, always"
    );
    assert!(
        snippet.contains(r#"add_header X-Frame-Options "DENY" always;"#),
        "X-Frame-Options must be DENY, always"
    );
    assert!(
        snippet.contains(r#"add_header Referrer-Policy "strict-origin-when-cross-origin" always;"#),
        "Referrer-Policy must be strict-origin-when-cross-origin, always"
    );
    assert!(
        snippet.contains("max-age=31536000; includeSubDomains"),
        "HSTS must be one year and cover subdomains"
    );
    assert!(
        snippet.contains("frame-ancestors 'none';"),
        "CSP must forbid framing"
    );
    assert!(
        snippet.contains("default-src 'self';"),
        "CSP must default to same-origin"
    );

    let directives: Vec<&str> = snippet
        .lines()
        .filter(|line| line.trim_start().starts_with("add_header "))
        .collect();
    assert_eq!(
        directives.len(),
        EDGE_HEADERS.len(),
        "every add_header in the snippet is one of the {} edge headers",
        EDGE_HEADERS.len()
    );
    for line in directives {
        // `always` is what makes nginx emit the header on error responses too.
        assert!(
            line.trim_end().ends_with("always;"),
            "add_header must carry `always` so error responses keep it: {line}"
        );
    }
}

#[test]
fn every_nginx_location_with_its_own_add_header_reincludes_the_snippet() {
    let conf = read("docker/images/frontend/nginx.conf");
    let include = "include /etc/nginx/security-headers.conf;";

    assert!(
        conf.contains(include),
        "nginx.conf must include the security header snippet"
    );

    // Split on `location ` so the first chunk is the server-level preamble.
    let mut chunks = conf.split("\n        location ");
    let server_level = chunks.next().unwrap_or_default();
    assert!(
        server_level.contains(include),
        "the snippet must be included at server level so locations without an \
         add_header of their own inherit it"
    );

    let mut checked = 0_usize;
    for chunk in chunks {
        let name = chunk.lines().next().unwrap_or_default().trim().to_owned();
        let declares_own_header = chunk
            .lines()
            .any(|line| line.trim_start().starts_with("add_header "));
        if declares_own_header {
            checked += 1;
            assert!(
                chunk.contains(include),
                "location `{name}` declares its own add_header, so it must \
                 re-include the security header snippet or nginx drops the \
                 inherited ones"
            );
        }
    }
    assert!(
        checked >= 4,
        "expected several locations with their own add_header, found {checked}"
    );
}

#[test]
fn no_rust_source_serves_a_competing_security_header_set() {
    let mut sources = Vec::new();
    let crates_dir = repo_root().join("crates");
    let crate_dirs = fs::read_dir(&crates_dir).expect("crates/ is readable");
    for crate_dir in crate_dirs.flatten() {
        collect_rust_sources(&crate_dir.path().join("src"), &mut sources);
    }
    assert!(
        sources.len() > 100,
        "the walk must reach the whole workspace, found {} files",
        sources.len()
    );

    let mut hits = Vec::new();
    for path in &sources {
        let Ok(body) = fs::read_to_string(path) else {
            continue;
        };
        for (index, line) in body.lines().enumerate() {
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            for header in EDGE_HEADERS {
                if line.contains(header) {
                    hits.push(format!("{}:{}: {header}", path.display(), index + 1));
                }
            }
        }
    }

    assert!(
        hits.is_empty(),
        "nginx is the single source of browser security headers; these Rust \
         sources declare their own copy:\n{}",
        hits.join("\n")
    );
}

/// Collect every `.rs` file under one crate's `src` tree.
fn collect_rust_sources(dir: &Path, sources: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rust_sources(&path, sources);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            sources.push(path);
        }
    }
}
