// ABOUTME: Unit tests for the coach-proposal re-rank prompt builder and response parser
// ABOUTME: Covers JSON extraction tolerance, id validation, dedup, capping, and prompt contents
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tests for `pierre_services::coaches` re-ranking helpers (pure functions: no
//! LLM, no DB) plus the messaging coach-proposal idempotency flag round-trip
//! and the backfill migration, against the database the lane names.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use pierre_core::models::TenantId;
use pierre_database::backends::factory::Database;
use pierre_database::repositories::CreateChannelLinkParams;
use pierre_services::coaches::{
    build_rerank_user_prompt, parse_rerank_response, ProposalCandidate,
};

mod common;

fn candidate(id: &str, title: &str, blurb: &str) -> ProposalCandidate {
    ProposalCandidate {
        id: id.to_owned(),
        title: title.to_owned(),
        category: "training".to_owned(),
        tags: vec!["endurance".to_owned()],
        blurb: blurb.to_owned(),
        match_score: 0.8,
    }
}

fn ids(values: &[&str]) -> HashSet<String> {
    values.iter().map(|v| (*v).to_owned()).collect()
}

#[test]
fn parse_clean_json_array_preserves_order_and_reasons() {
    let valid = ids(&["a", "b", "c"]);
    let content = r#"[
        {"id": "b", "reason": "Fits your cycling base."},
        {"id": "a", "reason": "Good for your running volume."}
    ]"#;

    let selections = parse_rerank_response(content, &valid, 3);

    assert_eq!(selections.len(), 2);
    assert_eq!(selections[0].id, "b");
    assert_eq!(selections[0].reason, "Fits your cycling base.");
    assert_eq!(selections[1].id, "a");
}

#[test]
fn parse_tolerates_markdown_fence_and_prose() {
    let valid = ids(&["x"]);
    let content = "Here are my picks:\n```json\n[{\"id\": \"x\", \"reason\": \"Best match.\"}]\n```\nHope that helps!";

    let selections = parse_rerank_response(content, &valid, 3);

    assert_eq!(selections.len(), 1);
    assert_eq!(selections[0].id, "x");
    assert_eq!(selections[0].reason, "Best match.");
}

#[test]
fn parse_drops_ids_outside_candidate_set() {
    let valid = ids(&["a"]);
    let content = r#"[{"id": "a", "reason": "real"}, {"id": "ghost", "reason": "hallucinated"}]"#;

    let selections = parse_rerank_response(content, &valid, 3);

    assert_eq!(selections.len(), 1);
    assert_eq!(selections[0].id, "a");
}

#[test]
fn parse_dedupes_keeping_first_occurrence() {
    let valid = ids(&["a"]);
    let content = r#"[{"id": "a", "reason": "first"}, {"id": "a", "reason": "second"}]"#;

    let selections = parse_rerank_response(content, &valid, 3);

    assert_eq!(selections.len(), 1);
    assert_eq!(selections[0].reason, "first");
}

#[test]
fn parse_caps_at_max() {
    let valid = ids(&["a", "b", "c", "d"]);
    let content = r#"[
        {"id": "a", "reason": "1"},
        {"id": "b", "reason": "2"},
        {"id": "c", "reason": "3"},
        {"id": "d", "reason": "4"}
    ]"#;

    let selections = parse_rerank_response(content, &valid, 3);

    assert_eq!(selections.len(), 3);
    assert_eq!(selections[2].id, "c");
}

#[test]
fn parse_returns_empty_on_garbage() {
    let valid = ids(&["a"]);

    assert!(parse_rerank_response("not json at all", &valid, 3).is_empty());
    assert!(parse_rerank_response("{\"id\": \"a\"}", &valid, 3).is_empty());
    assert!(parse_rerank_response("", &valid, 3).is_empty());
}

#[test]
fn parse_trims_reason_whitespace() {
    let valid = ids(&["a"]);
    let content = r#"[{"id": "a", "reason": "  padded reason  "}]"#;

    let selections = parse_rerank_response(content, &valid, 3);

    assert_eq!(selections[0].reason, "padded reason");
}

#[test]
fn build_prompt_includes_profile_candidates_and_cap() {
    let candidates = vec![
        candidate(
            "coach-1",
            "Marathon Coach",
            "Use for marathon build phases.",
        ),
        candidate(
            "coach-2",
            "Recovery Coach",
            "Use when overtraining is a risk.",
        ),
    ];

    let prompt =
        build_rerank_user_prompt("Trains primarily run; 40 activities.", &candidates, 3, "en");

    assert!(prompt.contains("Trains primarily run"));
    assert!(prompt.contains("coach-1"));
    assert!(prompt.contains("Marathon Coach"));
    assert!(prompt.contains("Use for marathon build phases."));
    assert!(prompt.contains("coach-2"));
    assert!(prompt.contains("at most 3"));
    // The locale drives the rationale language directive so reasons match the
    // user's conversation language instead of defaulting to English.
    assert!(prompt.contains("Write each \"reason\" in English."));
}

#[test]
fn build_prompt_localizes_reason_directive() {
    let candidates = vec![candidate(
        "coach-1",
        "Marathon Coach",
        "Use for marathon builds.",
    )];

    let fr = build_rerank_user_prompt("Court surtout; 40 activités.", &candidates, 3, "fr");
    assert!(fr.contains("Write each \"reason\" in French."));

    // Unrecognized locales fall back to French, matching the messaging-strings
    // registry's default-locale behavior.
    let other = build_rerank_user_prompt("profile", &candidates, 3, "it");
    assert!(other.contains("Write each \"reason\" in French."));

    let es = build_rerank_user_prompt("perfil", &candidates, 3, "es-MX");
    assert!(es.contains("Write each \"reason\" in Spanish."));
}

/// The messaging auto-send idempotency flag must round-trip on a real link:
/// unset → marked → set, idempotent re-marks, and scoped to the exact
/// `(tenant, channel, channel_user_id)` identity.
#[tokio::test]
async fn coach_proposal_flag_round_trips() {
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let repos = database.repositories();
    let tenants = repos.tenants.list_for_user(user_id).await.unwrap();
    let tenant_id = TenantId::from_uuid(tenants[0].id.as_uuid());

    let channel = "telegram";
    let channel_user_id = "tg-12345";

    repos
        .messaging
        .create_channel_link(&CreateChannelLinkParams {
            id: "link-1",
            tenant_id,
            user_id: &user_id.to_string(),
            channel_type: channel,
            channel_user_id,
            display_name: None,
        })
        .await
        .unwrap();

    // Fresh link: proposal not yet sent.
    assert!(
        !repos
            .messaging
            .coach_proposal_sent(tenant_id, channel, channel_user_id)
            .await
            .unwrap(),
        "a fresh link must report the proposal as not yet sent"
    );

    repos
        .messaging
        .mark_coach_proposal_sent(tenant_id, channel, channel_user_id, &[])
        .await
        .unwrap();

    // Now flagged.
    assert!(
        repos
            .messaging
            .coach_proposal_sent(tenant_id, channel, channel_user_id)
            .await
            .unwrap(),
        "after marking, the proposal must report as sent"
    );

    // Re-marking is idempotent (no error, stays set).
    repos
        .messaging
        .mark_coach_proposal_sent(tenant_id, channel, channel_user_id, &[])
        .await
        .unwrap();
    assert!(repos
        .messaging
        .coach_proposal_sent(tenant_id, channel, channel_user_id)
        .await
        .unwrap());

    // A different channel identity on the same tenant is independent.
    assert!(
        !repos
            .messaging
            .coach_proposal_sent(tenant_id, channel, "tg-other")
            .await
            .unwrap(),
        "the flag must be scoped to the exact channel_user_id"
    );
}

/// The exact backfill statement each backend ships, so the test exercises the
/// migration rather than a paraphrase that could drift from it.
const SQLITE_BACKFILL_MIGRATION_SQL: &str =
    include_str!("../../../migrations/20260608000003_backfill_coach_proposal_predating_links.sql");
#[cfg(feature = "postgresql")]
const POSTGRES_BACKFILL_MIGRATION_SQL: &str = include_str!(
    "../../../migrations_pg/20260608000003_backfill_coach_proposal_predating_links.sql"
);

fn at(rfc3339: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(rfc3339)
        .unwrap()
        .with_timezone(&Utc)
}

/// Write a link's `linked_at` and `coach_proposal_sent_at` directly: the
/// repository stamps `linked_at` with the clock, and the backfill's cutoff is
/// a date in the past. `SQLite` keeps timestamps as RFC 3339 text, the way its
/// backend writes them; `PostgreSQL` binds them natively.
async fn set_link_dates(
    db: &Database,
    id: &str,
    linked_at: DateTime<Utc>,
    sent_at: Option<DateTime<Utc>>,
) {
    const SQL: &str = "UPDATE messaging_channel_links \
                       SET linked_at = $1, coach_proposal_sent_at = $2 WHERE id = $3";
    match db {
        Database::SQLite(d) => {
            sqlx::query(SQL)
                .bind(linked_at.to_rfc3339())
                .bind(sent_at.map(|sent| sent.to_rfc3339()))
                .bind(id)
                .execute(d.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(d) => {
            sqlx::query(SQL)
                .bind(linked_at)
                .bind(sent_at)
                .bind(id)
                .execute(d.pool())
                .await
                .unwrap();
        }
    }
}

/// Run the backfill statement the backend ships.
async fn run_backfill(db: &Database) {
    match db {
        Database::SQLite(d) => {
            sqlx::query(SQLITE_BACKFILL_MIGRATION_SQL)
                .execute(d.pool())
                .await
                .unwrap();
        }
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(d) => {
            sqlx::query(POSTGRES_BACKFILL_MIGRATION_SQL)
                .execute(d.pool())
                .await
                .unwrap();
        }
    }
}

/// Read a single link's `coach_proposal_sent_at` (NULL ⇒ `None`).
async fn proposal_sent_at(db: &Database, id: &str) -> Option<DateTime<Utc>> {
    const SQL: &str = "SELECT coach_proposal_sent_at FROM messaging_channel_links WHERE id = $1";
    match db {
        Database::SQLite(d) => sqlx::query_scalar(SQL)
            .bind(id)
            .fetch_one(d.pool())
            .await
            .unwrap(),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(d) => sqlx::query_scalar(SQL)
            .bind(id)
            .fetch_one(d.pool())
            .await
            .unwrap(),
    }
}

/// The backfill stamps only links that predate the proposal feature
/// (`linked_at < 2026-06-05`) and were not already stamped. Links created on
/// or after the cutoff stay NULL so they receive the proposal normally, and
/// already-stamped links keep their original timestamp.
#[tokio::test]
async fn backfill_stamps_only_links_predating_the_feature() {
    let database = common::create_test_database().await.unwrap();
    let (user_id, _user) = common::create_test_user(&database).await.unwrap();
    let repos = database.repositories();
    let tenants = repos.tenants.list_for_user(user_id).await.unwrap();
    let tenant_id = TenantId::from_uuid(tenants[0].id.as_uuid());
    let user_id = user_id.to_string();

    // (id, linked_at, coach_proposal_sent_at)
    let rows = [
        ("old-unstamped", "2026-06-01T10:00:00Z", None),
        ("new-unstamped", "2026-06-07T10:00:00Z", None),
        ("on-cutoff", "2026-06-05T00:00:00Z", None),
        (
            "old-already-stamped",
            "2026-06-01T10:00:00Z",
            Some("2026-06-06T09:00:00Z"),
        ),
    ];
    for (id, linked_at, sent_at) in rows {
        repos
            .messaging
            .create_channel_link(&CreateChannelLinkParams {
                id,
                tenant_id,
                user_id: &user_id,
                channel_type: "telegram",
                channel_user_id: id,
                display_name: None,
            })
            .await
            .unwrap();
        set_link_dates(&database, id, at(linked_at), sent_at.map(at)).await;
    }

    run_backfill(&database).await;

    // A link that predates the feature and was unstamped is now stamped.
    assert!(
        proposal_sent_at(&database, "old-unstamped").await.is_some(),
        "a link linked before the feature must be backfilled as proposed"
    );
    // A link created after the feature is left NULL: it still gets the proposal.
    assert!(
        proposal_sent_at(&database, "new-unstamped").await.is_none(),
        "a link linked after the feature must keep receiving the proposal"
    );
    // The cutoff is exclusive — a link linked exactly on the feature date is new.
    assert!(
        proposal_sent_at(&database, "on-cutoff").await.is_none(),
        "the cutoff is exclusive; an on-date link must keep receiving the proposal"
    );
    // An already-stamped old link keeps its original timestamp (not re-stamped).
    assert_eq!(
        proposal_sent_at(&database, "old-already-stamped").await,
        Some(at("2026-06-06T09:00:00Z")),
        "an already-stamped link must keep its original timestamp"
    );
}
