// ABOUTME: Pins that a claim whose lease lapsed and was re-claimed can no longer write the row
// ABOUTME: Covers all five leased tables, and that the outbound retry queue hands each entry to one claim
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A lease that said only until when a row was held, never by whom, would let
//! a worker that stalled past its lease, while another instance re-claimed
//! the row, still renew it, and its finish delete or overwrite the newer
//! claim's work; an outbound retry queue without a claim would let two
//! instances read the same due entries and send an athlete one message twice.
//!
//! Every claim writes a fresh token and every later write names it. Each
//! test here lets one claim lapse, has a second claim take the row, and then
//! shows the first claim's writes change nothing while the second's land.
//! Time is passed in, so a lapse is a later `now`, never a sleep — except on
//! the outbound queue, whose claim reads the clock and is lapsed with a zero
//! lease instead.

use std::collections::HashSet;

use chrono::{Duration, TimeZone, Utc};
use pierre_core::models::TenantId;
use pierre_database::backends::{factory::Database, EnqueueOutboundParams};
use pierre_database::repositories::{
    ActivityBackfillJobRow, BackfillJobClaim, ExtractionJobClaim, JobLease, MemoryExtractionJobRow,
    WorkerRun,
};
use serde_json::Value;
use uuid::Uuid;

use crate::db_fixtures::{create_test_db, seed_user};

const LEASE_MS: i64 = 5 * 60_000;

/// The lease a claimed job row carries.
fn lease_of(id: &str, token: Option<&String>) -> JobLease {
    JobLease {
        id: id.to_owned(),
        token: token.cloned().expect("a claim writes a token"),
    }
}

#[tokio::test]
async fn a_lapsed_extraction_claim_cannot_renew_or_finish_the_row_another_claim_took() {
    let db = create_test_db().await;
    let ledger = &db.repositories().memory_extraction_jobs;
    let tenant = TenantId::generate();
    let now = Utc::now().timestamp_millis();
    let row = MemoryExtractionJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id: tenant,
        usage_tenant_id: Some(tenant),
        payload: r#"{"user_message":"fenced"}"#.to_owned(),
        created_at_ms: now - 10 * 60_000,
        leased_until_ms: now - 1,
        attempts: 1,
        lease_token: Some(Uuid::new_v4().to_string()),
    };
    ledger.record_extraction_job(&row).await.unwrap();
    let claim_at = |now_ms| ExtractionJobClaim {
        now_ms,
        queued_older_than_ms: 2 * 60_000,
        lease_ms: LEASE_MS,
        max_attempts: 10,
        limit: 10,
    };

    let first = ledger
        .claim_stale_extraction_jobs(claim_at(now))
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    let first = lease_of(&row.id, first[0].lease_token.as_ref());
    // The first claim stalls past its lease; a later sweep takes the row.
    let second = ledger
        .claim_stale_extraction_jobs(claim_at(now + LEASE_MS + 1))
        .await
        .unwrap();
    assert_eq!(second.len(), 1, "the lapsed row is re-claimed");
    let second = lease_of(&row.id, second[0].lease_token.as_ref());
    assert_ne!(first.token, second.token, "each claim writes its own token");

    let recorder = lease_of(&row.id, row.lease_token.as_ref());
    for stale in [&recorder, &first] {
        assert!(
            !ledger
                .renew_extraction_job_lease(stale, now + 60 * 60_000)
                .await
                .unwrap(),
            "a superseded claim cannot renew the row"
        );
        assert!(
            !ledger.finish_extraction_job(stale).await.unwrap(),
            "a superseded claim cannot finish the row"
        );
    }
    assert!(
        ledger
            .claim_stale_extraction_jobs(claim_at(now + LEASE_MS + 2))
            .await
            .unwrap()
            .is_empty(),
        "the second claim's lease still holds the row"
    );
    assert!(
        ledger
            .renew_extraction_job_lease(&second, now + 2 * LEASE_MS + 2)
            .await
            .unwrap(),
        "the holder renews the row the stale writes left intact"
    );
    assert!(ledger.finish_extraction_job(&second).await.unwrap());
}

#[tokio::test]
async fn a_lapsed_backfill_claim_cannot_renew_or_finish_the_row_another_claim_took() {
    let db = create_test_db().await;
    let ledger = &db.repositories().activity_backfill_jobs;
    let now = Utc::now().timestamp_millis();
    let row = ActivityBackfillJobRow {
        id: Uuid::new_v4().to_string(),
        tenant_id: TenantId::generate(),
        user_id: Uuid::new_v4(),
        provider: "strava".to_owned(),
        after_ts: Some(1_700_000_000),
        before_ts: None,
        fetch_limit: Some(200),
        conversation_id: None,
        created_at_ms: now - 10 * 60_000,
        leased_until_ms: now - 1,
        attempts: 1,
        lease_token: Some(Uuid::new_v4().to_string()),
    };
    assert!(ledger.record_backfill_job(&row).await.unwrap());
    let claim_at = |now_ms| BackfillJobClaim {
        now_ms,
        queued_older_than_ms: 2 * 60_000,
        lease_ms: LEASE_MS,
        max_attempts: 10,
        limit: 10,
    };

    let first = ledger
        .claim_stale_backfill_jobs(claim_at(now))
        .await
        .unwrap();
    assert_eq!(first.len(), 1);
    let first = lease_of(&row.id, first[0].lease_token.as_ref());
    let second = ledger
        .claim_stale_backfill_jobs(claim_at(now + LEASE_MS + 1))
        .await
        .unwrap();
    assert_eq!(second.len(), 1, "the lapsed row is re-claimed");
    let second = lease_of(&row.id, second[0].lease_token.as_ref());
    assert_ne!(first.token, second.token, "each claim writes its own token");

    let recorder = lease_of(&row.id, row.lease_token.as_ref());
    for stale in [&recorder, &first] {
        assert!(
            !ledger
                .renew_backfill_job_lease(stale, now + 60 * 60_000)
                .await
                .unwrap(),
            "a superseded claim cannot renew the row"
        );
        assert!(
            !ledger.finish_backfill_job(stale).await.unwrap(),
            "a superseded claim cannot finish the row"
        );
    }
    assert!(
        ledger
            .claim_stale_backfill_jobs(claim_at(now + LEASE_MS + 2))
            .await
            .unwrap()
            .is_empty(),
        "the second claim's lease still holds the row"
    );
    assert!(
        !ledger.record_backfill_job(&row).await.unwrap(),
        "the row is still on file for its (user, provider)"
    );
    assert!(ledger
        .renew_backfill_job_lease(&second, now + 2 * LEASE_MS + 2)
        .await
        .unwrap());
    assert!(ledger.finish_backfill_job(&second).await.unwrap());
}

#[tokio::test]
async fn a_lapsed_worker_tick_cannot_finish_or_defer_the_tick_another_instance_took() {
    let db = create_test_db().await;
    let ledger = &db.repositories().worker_runs;
    let name = format!("fence probe {}", Uuid::new_v4());
    let now = 1_000_000_i64;
    let lease = 1_000_i64;

    let first = ledger
        .claim_worker_run(&name, 0, now, lease)
        .await
        .unwrap()
        .expect("the first claim takes the tick");
    assert!(
        ledger
            .claim_worker_run(&name, 0, now + lease / 2, lease)
            .await
            .unwrap()
            .is_none(),
        "inside its lease the tick is held"
    );
    let second = ledger
        .claim_worker_run(&name, 0, now + lease + 1, lease)
        .await
        .unwrap()
        .expect("past its lease the tick is re-claimed");
    assert_ne!(first, second, "each claim writes its own token");
    let held = WorkerRun {
        last_run_at_ms: 0,
        leased_until_ms: now + 2 * lease + 1,
    };
    assert_eq!(ledger.get_worker_run(&name).await.unwrap(), Some(held));

    ledger
        .finish_worker_run(&name, now + lease + 2, Some(&first))
        .await
        .unwrap();
    ledger
        .defer_worker_run(&name, now + 100 * lease, Some(&first))
        .await
        .unwrap();
    assert_eq!(
        ledger.get_worker_run(&name).await.unwrap(),
        Some(held),
        "the lapsed tick's finish and defer leave the newer claim's row alone"
    );
    assert!(
        ledger
            .claim_worker_run(&name, 0, now + lease + 3, lease)
            .await
            .unwrap()
            .is_none(),
        "the second claim still holds the tick"
    );

    ledger
        .finish_worker_run(&name, now + lease + 4, Some(&second))
        .await
        .unwrap();
    assert_eq!(
        ledger.get_worker_run(&name).await.unwrap(),
        Some(WorkerRun {
            last_run_at_ms: now + lease + 4,
            leased_until_ms: 0,
        }),
        "the holder's finish lands and releases the lease"
    );
}

#[tokio::test]
async fn a_lapsed_once_claim_cannot_mark_or_release_the_post_another_claim_took() {
    let db = create_test_db().await;
    let claims = &db.repositories().platform_post_claims;
    let (user_id, tenant) = seed_user(&db).await;
    let noon = Utc.with_ymd_and_hms(2026, 10, 14, 12, 0, 0).unwrap();
    let lease = Duration::minutes(15);
    let claim = |at| claims.claim_platform_post_once(tenant, user_id, "first_sync", at - lease, at);

    let first = claim(noon)
        .await
        .unwrap()
        .expect("the first claim is taken");
    let second = claim(noon + Duration::minutes(16))
        .await
        .unwrap()
        .expect("past its lease the abandoned claim is retaken");
    assert_ne!(first, second, "each claim writes its own token");

    assert!(
        !claims
            .mark_platform_post_posted(tenant, user_id, "first_sync", &first, noon)
            .await
            .unwrap(),
        "the lapsed claim cannot mark the post the newer claim is making"
    );
    assert!(
        !claims
            .release_platform_post(tenant, user_id, "first_sync", &first)
            .await
            .unwrap(),
        "nor free the newer claim's row"
    );
    assert!(
        claim(noon + Duration::minutes(17)).await.unwrap().is_none(),
        "the newer claim's row stands, inside its lease"
    );

    assert!(claims
        .mark_platform_post_posted(
            tenant,
            user_id,
            "first_sync",
            &second,
            noon + Duration::minutes(18)
        )
        .await
        .unwrap());
    assert!(
        claim(noon + Duration::days(30)).await.unwrap().is_none(),
        "the post the holder marked is never claimed again"
    );
}

#[tokio::test]
async fn a_lapsed_period_claim_cannot_release_or_mark_the_post_the_next_period_took() {
    let db = create_test_db().await;
    let claims = &db.repositories().platform_post_claims;
    let (user_id, tenant) = seed_user(&db).await;
    let monday = Utc.with_ymd_and_hms(2026, 10, 12, 0, 0, 0).unwrap();
    let tuesday = monday + Duration::days(1);
    let claim =
        |since, at| claims.claim_platform_post_since(tenant, user_id, "check_in", since, at);

    let first = claim(monday, monday + Duration::hours(8))
        .await
        .unwrap()
        .expect("Monday's claim is taken");
    let second = claim(tuesday, tuesday + Duration::hours(8))
        .await
        .unwrap()
        .expect("Tuesday's period retakes the row");
    assert_ne!(first, second, "each claim writes its own token");

    assert!(
        !claims
            .release_platform_post(tenant, user_id, "check_in", &first)
            .await
            .unwrap(),
        "Monday's claim cannot free Tuesday's"
    );
    assert!(
        !claims
            .mark_platform_post_posted(tenant, user_id, "check_in", &first, tuesday)
            .await
            .unwrap(),
        "nor mark it posted"
    );
    assert!(
        claim(tuesday, tuesday + Duration::hours(9))
            .await
            .unwrap()
            .is_none(),
        "Tuesday's claim still holds the period"
    );

    assert!(claims
        .release_platform_post(tenant, user_id, "check_in", &second)
        .await
        .unwrap());
    assert!(
        claim(tuesday, tuesday + Duration::hours(10))
            .await
            .unwrap()
            .is_some(),
        "the holder's release reopens the period"
    );
}

/// Queue `count` pending entries for a fresh athlete and return their ids.
async fn enqueue(db: &Database, count: usize) -> HashSet<String> {
    let (user_id, tenant_id) = seed_user(db).await;
    let user_id = user_id.to_string();
    let mut ids = HashSet::new();
    for n in 0..count {
        let id = Uuid::new_v4().to_string();
        db.repositories()
            .messaging
            .enqueue_outbound(&EnqueueOutboundParams {
                id: &id,
                message_id: None,
                tenant_id,
                user_id: Some(&user_id),
                channel_type: "whatsapp",
                payload: &format!(r#"{{"Body":"notice {n}"}}"#),
                expires_at: None,
                reauth: None,
            })
            .await
            .unwrap();
        ids.insert(id);
    }
    ids
}

fn ids_of(entries: &[Value]) -> HashSet<String> {
    entries
        .iter()
        .map(|entry| entry["id"].as_str().unwrap().to_owned())
        .collect()
}

/// The one token a claim's entries share.
fn token_of(entries: &[Value]) -> String {
    let tokens: HashSet<&str> = entries
        .iter()
        .map(|entry| entry["lease_token"].as_str().expect("claimed entry token"))
        .collect();
    assert_eq!(tokens.len(), 1, "one claim, one token: {tokens:?}");
    tokens.into_iter().next().unwrap().to_owned()
}

#[tokio::test]
async fn two_outbound_claims_never_hand_out_the_same_entry() {
    let db = create_test_db().await;
    let messaging = &db.repositories().messaging;
    let queued = enqueue(&db, 3).await;

    let first = messaging.claim_due_outbound(2, 60_000).await.unwrap();
    let second = messaging.claim_due_outbound(10, 60_000).await.unwrap();
    assert_eq!(first.len(), 2, "the first claim takes its limit");
    assert_eq!(second.len(), 1, "the second takes only what is left");
    let (first_ids, second_ids) = (ids_of(&first), ids_of(&second));
    assert!(
        first_ids.is_disjoint(&second_ids),
        "no entry is handed to both claims: {first_ids:?} / {second_ids:?}"
    );
    assert_eq!(
        &first_ids | &second_ids,
        queued,
        "between them the two claims took every due entry"
    );
    assert_ne!(token_of(&first), token_of(&second));
    assert!(
        messaging
            .claim_due_outbound(10, 60_000)
            .await
            .unwrap()
            .is_empty(),
        "every entry is held, so a third claim takes nothing"
    );
}

/// Two instances' retry passes claiming at the same moment still split the
/// queue: whatever each takes, no entry is in both.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_outbound_claims_never_hand_out_the_same_entry() {
    let db = create_test_db().await;
    let messaging = &db.repositories().messaging;
    let queued = enqueue(&db, 6).await;

    let (first, second) = tokio::join!(
        messaging.claim_due_outbound(4, 60_000),
        messaging.claim_due_outbound(4, 60_000)
    );
    let (first_ids, second_ids) = (ids_of(&first.unwrap()), ids_of(&second.unwrap()));
    assert!(
        first_ids.is_disjoint(&second_ids),
        "no entry is handed to both claims: {first_ids:?} / {second_ids:?}"
    );
    assert!(first_ids.is_subset(&queued) && second_ids.is_subset(&queued));
    assert!(
        !first_ids.is_empty() || !second_ids.is_empty(),
        "the due entries were claimed"
    );
}

#[tokio::test]
async fn a_settled_entry_is_done_and_an_unsettled_one_returns_after_its_lease() {
    let db = create_test_db().await;
    let messaging = &db.repositories().messaging;
    let queued = enqueue(&db, 2).await;

    // A zero lease: the claim lapses the moment it is made, as one whose
    // instance died mid-send would.
    let first = messaging.claim_due_outbound(10, 0).await.unwrap();
    assert_eq!(ids_of(&first), queued);
    let first_token = token_of(&first);
    let mut ids = queued.iter();
    let (sent, unsettled) = (ids.next().unwrap(), ids.next().unwrap());
    assert!(
        messaging
            .update_outbound_status(sent, &first_token, "sent", 1, None)
            .await
            .unwrap(),
        "the claim settles the entry it holds"
    );

    let second = messaging.claim_due_outbound(10, 60_000).await.unwrap();
    assert_eq!(
        ids_of(&second),
        HashSet::from([unsettled.clone()]),
        "the unsettled entry is claimable again once its lease lapsed; the sent one never"
    );
    let second_token = token_of(&second);
    assert_ne!(first_token, second_token);

    assert!(
        !messaging
            .update_outbound_status(unsettled, &first_token, "sent", 1, None)
            .await
            .unwrap(),
        "the lapsed claim cannot settle the entry the newer claim holds"
    );
    assert!(
        messaging
            .claim_due_outbound(10, 60_000)
            .await
            .unwrap()
            .is_empty(),
        "the refused write left the newer claim's lease in place"
    );
    assert!(messaging
        .update_outbound_status(unsettled, &second_token, "sent", 1, None)
        .await
        .unwrap());
    assert!(
        !messaging
            .update_outbound_status(unsettled, &second_token, "sent", 2, None)
            .await
            .unwrap(),
        "a settled entry's lease is released, so even its own claim writes it once"
    );
    assert!(
        messaging
            .claim_due_outbound(10, 0)
            .await
            .unwrap()
            .is_empty(),
        "both entries are sent; nothing is due"
    );
}
