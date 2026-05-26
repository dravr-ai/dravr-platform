// ABOUTME: Unit coverage for the coach drift classifier consumed by `pierre-cli check-drift coaches`
// ABOUTME: Drives every DriftOutcome branch without touching a database
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// Background: 2026-05-25 audit revealed seed-coaches Cloud Run job had been
// exit-1 for 3.5 weeks with no alert. The drift sub-command + its scheduled
// Cloud Run Job (see infra/environments/dev/drift_check.tf) is the daily
// gate that surfaces the next contremaitre/DB mismatch. These tests pin
// the pure classifier so the daily ERROR/WARN/clean ladder doesn't
// regress silently.

#![allow(clippy::unwrap_used, clippy::expect_used, missing_docs)]

use pierre_coach_parser::drift::{classify_drift, DbCoachRow, DriftOutcome};

fn row(source: &str, hash: Option<&str>) -> DbCoachRow {
    DbCoachRow {
        source: source.to_owned(),
        content_hash: hash.map(str::to_owned),
    }
}

#[test]
fn in_sync_when_hashes_match_and_source_is_contremaitre() {
    let db = row("contremaitre", Some("abc123"));
    assert_eq!(classify_drift("abc123", Some(&db)), DriftOutcome::InSync);
}

#[test]
fn hash_drift_when_contremaitre_source_but_hashes_differ() {
    let db = row("contremaitre", Some("old_hash"));
    assert_eq!(
        classify_drift("new_hash", Some(&db)),
        DriftOutcome::HashDrift {
            expected: "new_hash".to_owned(),
            actual: Some("old_hash".to_owned()),
        }
    );
}

#[test]
fn hash_drift_when_contremaitre_source_but_db_hash_is_null() {
    let db = row("contremaitre", None);
    assert_eq!(
        classify_drift("expected_hash", Some(&db)),
        DriftOutcome::HashDrift {
            expected: "expected_hash".to_owned(),
            actual: None,
        }
    );
}

#[test]
fn source_mismatch_when_db_source_is_seed() {
    let db = row("seed", Some("abc123"));
    assert_eq!(
        classify_drift("abc123", Some(&db)),
        DriftOutcome::SourceMismatch {
            actual_source: "seed".to_owned(),
        }
    );
}

#[test]
fn source_mismatch_when_db_source_is_custom_even_if_hashes_match() {
    // Custom coaches are user-owned: hash equality is coincidental, not
    // a signal that contremaitre is authoritative for them.
    let db = row("custom", Some("abc123"));
    assert_eq!(
        classify_drift("abc123", Some(&db)),
        DriftOutcome::SourceMismatch {
            actual_source: "custom".to_owned(),
        }
    );
}

#[test]
fn missing_when_db_has_no_row() {
    assert_eq!(classify_drift("any_hash", None), DriftOutcome::Missing);
}
