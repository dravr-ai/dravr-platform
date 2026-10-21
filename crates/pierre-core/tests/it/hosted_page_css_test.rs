// ABOUTME: Pins the shared Boreal sheet the hosted pages embed and the helper that places it
// ABOUTME: Both schemes must be in the compiled-in sheet, and a request value never expands into it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(missing_docs, clippy::expect_used)]

use pierre_core::html::{with_hosted_page_css, HOSTED_PAGE_CSS};

#[test]
fn the_sheet_carries_both_schemes_from_the_boreal_tokens() {
    // Light is the default; dark overrides it under the media query.
    let dark_at = HOSTED_PAGE_CSS
        .find("@media (prefers-color-scheme: dark) {")
        .expect("the sheet has a dark scheme");
    let (light, dark) = HOSTED_PAGE_CSS.split_at(dark_at);

    assert!(light.contains("  color-scheme: light dark;"));
    // Sage-forest #255f4d on white #ffffff in light.
    assert!(light.contains("--color-primary: 37 95 77;"));
    assert!(light.contains("--color-card: 255 255 255;"));
    // Mint #a3d0be on surface-container-high #272b27 in dark.
    assert!(dark.contains("--color-primary: 163 208 190;"));
    assert!(dark.contains("--color-card: 39 43 39;"));
    // The notice ink is bound to its tint in each scheme: #664c16 / #d8bc81.
    assert!(light.contains("--color-on-warning-container: 102 76 22;"));
    assert!(dark.contains("--color-on-warning-container: 216 188 129;"));
}

#[test]
fn the_sheet_holds_no_placeholder_braces_and_no_retired_palette() {
    assert!(!HOSTED_PAGE_CSS.contains("{{"));
    assert!(!HOSTED_PAGE_CSS.contains("}}"));
    assert!(!HOSTED_PAGE_CSS.to_ascii_lowercase().contains("#7c3aed"));
    assert!(!HOSTED_PAGE_CSS.contains("--pierre-"));
    assert!(!HOSTED_PAGE_CSS.contains("linear-gradient"));
}

#[test]
fn the_helper_places_the_sheet_and_leaves_the_rest_of_the_page_alone() {
    let page = with_hosted_page_css(
        "<head><style>{{HOSTED_PAGE_CSS}}</style></head><body>{{NAME}}</body>",
    );

    assert!(page.starts_with("<head><style>/* ABOUTME: The Boreal stylesheet"));
    assert!(page.ends_with("</style></head><body>{{NAME}}</body>"));
    assert_eq!(
        page.len(),
        HOSTED_PAGE_CSS.len() + "<head><style></style></head><body>{{NAME}}</body>".len()
    );
}

#[test]
fn a_value_substituted_after_the_sheet_is_never_expanded_into_it() {
    // A renderer places the sheet first, then its request values: a value that
    // spells the placeholder stays text.
    let page = with_hosted_page_css("<style>{{HOSTED_PAGE_CSS}}</style><p>{{MESSAGE}}</p>")
        .replace("{{MESSAGE}}", "{{HOSTED_PAGE_CSS}}");

    assert_eq!(page.matches("color-scheme: light dark;").count(), 1);
    assert!(page.ends_with("<p>{{HOSTED_PAGE_CSS}}</p>"));
}
