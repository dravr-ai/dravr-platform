// ABOUTME: Endurance Phase 5 — workout library cornerstone TOML loading + slug lookup
// ABOUTME: Locks in the six compiled-in cornerstones and the require_cornerstone error path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use pierre_mcp_server::services::workout_library::{
    cornerstone_by_slug, cornerstone_templates, require_cornerstone, CORNERSTONE_SLUGS,
};

#[test]
fn six_cornerstones_load_from_compiled_in_toml() {
    let templates = cornerstone_templates();
    assert_eq!(
        templates.len(),
        6,
        "must load all six compiled-in cornerstones"
    );
    let slugs: Vec<&str> = templates.iter().map(|t| t.slug.as_str()).collect();
    for expected in CORNERSTONE_SLUGS {
        assert!(
            slugs.contains(expected),
            "cornerstone {expected} missing from loaded library"
        );
    }
}

#[test]
fn each_cornerstone_resolves_by_slug() {
    for slug in CORNERSTONE_SLUGS {
        let template = cornerstone_by_slug(slug)
            .unwrap_or_else(|| panic!("cornerstone {slug} should resolve"));
        assert_eq!(template.slug.as_str(), *slug);
        assert!(
            !template.structure.is_empty(),
            "cornerstone {slug} must define at least one workout step"
        );
        assert!(
            template.duration_minutes > 0,
            "cornerstone {slug} must have a non-zero duration_minutes"
        );
        assert!(template.is_compiled_in);
    }
}

#[test]
fn require_cornerstone_errors_for_unknown_slug() {
    let err = require_cornerstone("not_a_real_template").expect_err("unknown slug must fail");
    let msg = format!("{err}");
    assert!(
        msg.contains("Endurance cornerstone"),
        "error must mention Endurance cornerstone, got: {msg}"
    );
}

#[test]
fn cornerstone_slug_set_is_alphabetically_distinct() {
    use std::collections::HashSet;
    let unique: HashSet<&&str> = CORNERSTONE_SLUGS.iter().collect();
    assert_eq!(
        unique.len(),
        CORNERSTONE_SLUGS.len(),
        "slugs must be unique"
    );
}
