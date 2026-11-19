// ABOUTME: The one list of inbound date strings every parsing path is asked about
// ABOUTME: Shared by the JSON, strict-parser and SQLite decode goldens so no path skips a row
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The shared inbound input list (carnet#1620, gate 7).
//!
//! Inputs only. Each path keeps its own verdict column next to the code that
//! produces it, and asserts that its column names exactly these inputs in this
//! order, so a row added here turns every path red until it has a verdict:
//!
//! - JSON into an instant and the strict parser, in `pierre-core`
//!   (`date_inbound_golden_test`);
//! - `SQLite` text into an instant, in `pierre-database`
//!   (`date_decode_golden_sqlite_test`), which includes this file by
//!   path because one crate's tests cannot import another's.

/// Every inbound shape, in the order the verdict tables list them.
pub(super) const INBOUND_INPUTS: [&str; 54] = [
    "2026-11-17T10:00:05Z",
    "2026-11-17T10:00:05+00:00",
    "2026-11-17T05:00:05-05:00",
    "2026-11-17T15:30:05+05:30",
    "2026-11-17T10:00:05-00:00",
    "2026-11-17T10:00:05+0000",
    "2026-11-17T05:00:05-0500",
    "2026-11-17T10:00:05+00",
    "2026-11-17T10:00:05 +00:00",
    "2026-11-17t10:00:05z",
    "2026-11-17T10:00:05z",
    "2026-11-17t10:00:05Z",
    "2026-11-17 10:00:05Z",
    "2026-11-17 10:00:05+00:00",
    "2026-11-17  10:00:05Z",
    "2026-11-17_10:00:05Z",
    "2026-11-17 10:00:05 UTC",
    "2026-11-17T10:00:05 UTC",
    "2026-11-17T10:00:05UTC",
    "2026-11-17T10:00:05 utc",
    "2026-11-17T10:00:05 GMT",
    "2026-11-17T05:00:05 EST",
    "2026-1-7T9:0:5Z",
    "2026-11-17T9:00:05Z",
    "2026-11-17T10:00:05",
    "2026-11-17 10:00:05",
    "2026-11-17T10:00Z",
    "2026-11-17",
    "2026-11-17Z",
    "",
    "2026-11-17T10:00:05.1Z",
    "2026-11-17T10:00:05.12Z",
    "2026-11-17T10:00:05.123Z",
    "2026-11-17T10:00:05.1234Z",
    "2026-11-17T10:00:05.12345Z",
    "2026-11-17T10:00:05.123456Z",
    "2026-11-17T10:00:05.1234567Z",
    "2026-11-17T10:00:05.12345678Z",
    "2026-11-17T10:00:05.123456789Z",
    "2026-11-17T10:00:05.1234567891Z",
    "2026-11-17T10:00:05.100+00:00",
    "2026-11-17T10:00:05,123Z",
    "2026-11-17T10:00:05.Z",
    "2016-12-31T23:59:60Z",
    "2026-11-17T10:00:60Z",
    "2026-11-17T24:00:00Z",
    "2026-02-30T10:00:05Z",
    "2025-02-29T10:00:05Z",
    "2026-13-01T10:00:05Z",
    " 2026-11-17T10:00:05Z",
    "2026-11-17T10:00:05Z ",
    "20261117T100005Z",
    "Tue, 17 Nov 2026 10:00:05 GMT",
    "1794909605",
];
