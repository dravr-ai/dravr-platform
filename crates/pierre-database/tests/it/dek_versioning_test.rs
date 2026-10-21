// ABOUTME: Tests the DEK version wire-format helpers (tag/split) used by both backends
// ABOUTME: Covers the v{N}: format and legacy un-tagged compatibility; DB roundtrip lives in pierre-server tests
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Unit tests for DEK version tagging helpers (ADR-017 Phase 3).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pierre_database::backends::shared::encryption::{
    split_dek_version, tag_dek_version, LEGACY_DEK_VERSION,
};

#[test]
fn tag_and_split_roundtrip() {
    let tagged = tag_dek_version(3, "QUJDREVG");
    assert_eq!(tagged, "v3:QUJDREVG");
    assert_eq!(split_dek_version(&tagged), (3, "QUJDREVG"));
}

#[test]
fn untagged_payload_parses_as_legacy_v1() {
    // Standard base64 never contains ':', so an un-prefixed payload is legacy v1.
    let payload = "QUJDREVGenz+/w==";
    let (version, parsed) = split_dek_version(payload);
    assert_eq!(version, LEGACY_DEK_VERSION);
    assert_eq!(parsed, payload);
}

#[test]
fn malformed_version_prefix_falls_back_to_legacy() {
    // "vx:" is not a valid numeric version; treat the whole string as legacy payload.
    let (version, parsed) = split_dek_version("vx:notaversion");
    assert_eq!(version, LEGACY_DEK_VERSION);
    assert_eq!(parsed, "vx:notaversion");
}

#[test]
fn empty_version_digits_fall_back_to_legacy() {
    let (version, parsed) = split_dek_version("v:payload");
    assert_eq!(version, LEGACY_DEK_VERSION);
    assert_eq!(parsed, "v:payload");
}
