// ABOUTME: Repository test for messaging_resumable_turns — record at ingress, claim in order, lease, release, finish, reap
// ABOUTME: Runs on whichever backend DATABASE_URL names, so the SQLite and PostgreSQL statements are both driven
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The concurrency contract behind a turn that is a row before it is a run
//! (registre#126).
//!
//! Every turn becomes one row; whichever runner claims first takes the lease
//! and the attempt bump in a single statement; nobody else can take it until
//! the lease ends; a younger turn waits behind an older one in the same
//! conversation; a finished turn leaves nothing behind. Each of those is a
//! property two instances could violate together, so each is asserted on the
//! statements themselves rather than through the dispatcher.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use anyhow::Result;
use pierre_core::models::TenantId;
use pierre_database::backends::factory::Database;
use pierre_database::database::generate_encryption_key;
use pierre_database::database::test_utils::create_test_db_with_key;
use pierre_database::repositories::{ResumableTurnClaim, ResumableTurnRow, TurnClaim, TurnLease};
use uuid::Uuid;

/// The cap the dispatcher applies; the repository takes it as a parameter so
/// the claim semantics at the boundary are what this file pins.
const MAX_ATTEMPTS: i64 = 2;
/// One lease, in the tests' millisecond clock.
const LEASE_MS: i64 = 90_000;

async fn fresh_db() -> Result<Database> {
    let key = generate_encryption_key().to_vec();
    Ok(create_test_db_with_key(key).await?)
}

fn row(
    tenant: TenantId,
    conversation: &str,
    inbound_id: &str,
    created_at_ms: i64,
) -> ResumableTurnRow {
    ResumableTurnRow {
        id: Uuid::new_v4().to_string(),
        tenant_id: tenant.to_string(),
        channel_tenant_id: tenant.to_string(),
        user_tenant_id: tenant.to_string(),
        session_id: "session-1".to_owned(),
        conversation: conversation.to_owned(),
        user_id: Uuid::new_v4().to_string(),
        channel: "telegram".to_owned(),
        sender_id: "tg-sender".to_owned(),
        conversation_id: Some("chat-42".to_owned()),
        channel_message_id: inbound_id.to_owned(),
        thread_id: None,
        text_content: "peux-tu sortir le NP de ma dernière course?".to_owned(),
        is_group_chat: false,
        locale: "fr".to_owned(),
        turn_id: Uuid::new_v4().to_string(),
        placeholder_message_id: None,
        attempts: 0,
        enqueue_seq: 0,
        created_at_ms,
    }
}

fn lease(leased_by: &str, now_ms: i64) -> TurnLease<'_> {
    TurnLease {
        leased_by,
        now_ms,
        lease_until_ms: now_ms + LEASE_MS,
        max_attempts: MAX_ATTEMPTS,
    }
}

/// A sweep claim at `now_ms` that takes never-leased rows recorded before
/// `queued_older_than_ms`.
fn sweep(leased_by: &str, now_ms: i64, queued_older_than_ms: i64) -> ResumableTurnClaim<'_> {
    ResumableTurnClaim {
        lease: lease(leased_by, now_ms),
        queued_older_than_ms,
        limit: 10,
    }
}

#[tokio::test]
async fn a_turn_is_recorded_once_per_inbound_message() -> Result<()> {
    let db = fresh_db().await?;
    let repo = &db.repositories().resumable_turns;
    let tenant = TenantId::generate();

    let first = row(tenant, "conv-a", "wamid.001", 1_000);
    assert!(
        repo.record_resumable_turn(&first).await?,
        "first record inserts"
    );

    // Same tenant, channel and inbound id from a webhook redelivery: the row
    // already on file wins, and the caller learns nothing new was written.
    let duplicate = row(tenant, "conv-a", "wamid.001", 2_000);
    assert!(
        !repo.record_resumable_turn(&duplicate).await?,
        "a second record of the same inbound message must not insert"
    );

    // Another tenant's message with the same channel id is a different turn.
    let other_tenant = row(TenantId::generate(), "conv-a", "wamid.001", 3_000);
    assert!(repo.record_resumable_turn(&other_tenant).await?);

    let TurnClaim::Claimed(stored) = repo
        .claim_resumable_turn(tenant, &first.id, &lease("inst-a", 10_000))
        .await?
    else {
        panic!("the recorded row is claimable by id");
    };
    assert_eq!(stored.id, first.id);
    assert_eq!(stored.created_at_ms, 1_000);
    assert_eq!(stored.text_content, first.text_content);
    assert_eq!(stored.enqueue_seq, 0);
    assert_eq!(stored.attempts, 1, "the claim is the first run's start");
    Ok(())
}

#[tokio::test]
async fn a_claim_by_id_is_refused_while_the_lease_is_live_and_free_once_it_ends() -> Result<()> {
    let db = fresh_db().await?;
    let repo = &db.repositories().resumable_turns;
    let tenant = TenantId::generate();
    let turn = row(tenant, "conv-a", "wamid.lease", 1_000);
    repo.record_resumable_turn(&turn).await?;

    assert!(matches!(
        repo.claim_resumable_turn(tenant, &turn.id, &lease("inst-a", 10_000))
            .await?,
        TurnClaim::Claimed(_)
    ));
    assert_eq!(
        repo.claim_resumable_turn(tenant, &turn.id, &lease("inst-b", 10_000 + LEASE_MS - 1))
            .await?,
        TurnClaim::Blocked,
        "the lease holds until it ends"
    );

    // The holder died: past the lease end the row is free, and the next
    // claim counts as another run.
    let TurnClaim::Claimed(reclaimed) = repo
        .claim_resumable_turn(tenant, &turn.id, &lease("inst-b", 10_000 + LEASE_MS + 1))
        .await?
    else {
        panic!("an expired lease is free");
    };
    assert_eq!(reclaimed.attempts, 2);

    assert_eq!(
        repo.claim_resumable_turn(TenantId::generate(), &turn.id, &lease("inst-c", 500_000))
            .await?,
        TurnClaim::Missing,
        "another tenant cannot see the row"
    );
    Ok(())
}

#[tokio::test]
async fn a_younger_turn_waits_behind_its_older_sibling_in_the_conversation() -> Result<()> {
    let db = fresh_db().await?;
    let repo = &db.repositories().resumable_turns;
    let tenant = TenantId::generate();
    let older = row(tenant, "conv-a", "wamid.older", 1_000);
    let younger = row(tenant, "conv-a", "wamid.younger", 2_000);
    let elsewhere = row(tenant, "conv-b", "wamid.elsewhere", 3_000);
    for r in [&older, &younger, &elsewhere] {
        repo.record_resumable_turn(r).await?;
    }

    assert_eq!(
        repo.claim_resumable_turn(tenant, &younger.id, &lease("inst-a", 10_000))
            .await?,
        TurnClaim::Blocked,
        "the younger turn is refused while the older one is on file, even unleased"
    );
    assert!(
        matches!(
            repo.claim_resumable_turn(tenant, &elsewhere.id, &lease("inst-a", 10_000))
                .await?,
            TurnClaim::Claimed(_)
        ),
        "another conversation is not held up"
    );
    assert!(matches!(
        repo.claim_resumable_turn(tenant, &older.id, &lease("inst-a", 10_000))
            .await?,
        TurnClaim::Claimed(_)
    ));

    // Once the older turn is finished, the younger one runs.
    assert!(repo.finish_resumable_turn(tenant, &older.id).await?);
    assert!(matches!(
        repo.claim_resumable_turn(tenant, &younger.id, &lease("inst-a", 10_001))
            .await?,
        TurnClaim::Claimed(_)
    ));
    Ok(())
}

#[tokio::test]
async fn the_sweep_takes_expired_leases_and_old_unleased_rows_oldest_first() -> Result<()> {
    let db = fresh_db().await?;
    let repo = &db.repositories().resumable_turns;
    let tenant = TenantId::generate();
    let fresh = row(tenant, "conv-f", "wamid.fresh", 9_900); // recorded moments ago
    let old_unleased = row(tenant, "conv-o", "wamid.old", 1_000); // recorded, never started
    let expired = row(tenant, "conv-e", "wamid.expired", 5_000); // leased, holder died
    let live = row(tenant, "conv-l", "wamid.live", 2_000); // leased, holder alive
    for r in [&fresh, &old_unleased, &expired, &live] {
        repo.record_resumable_turn(r).await?;
    }
    assert!(matches!(
        repo.claim_resumable_turn(tenant, &expired.id, &lease("dead", 6_000))
            .await?,
        TurnClaim::Claimed(_)
    ));
    assert!(matches!(
        repo.claim_resumable_turn(tenant, &live.id, &lease("alive", 9_000))
            .await?,
        TurnClaim::Claimed(_)
    ));

    let now = 6_000 + LEASE_MS + 10; // the dead holder's lease has just ended
    let listed = repo
        .list_stale_resumable_turns(&sweep("reader", now, now - 90_000))
        .await?;
    assert_eq!(
        listed.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
        vec![old_unleased.id.as_str(), expired.id.as_str()],
        "the listing sees exactly what a claim would take, and takes nothing"
    );
    assert_eq!(listed[0].attempts, 0, "a listing leaves the counts alone");

    let taken = repo
        .claim_resumable_turns(&sweep("sweeper", now, now - 90_000))
        .await?;
    let ids: Vec<&str> = taken.iter().map(|r| r.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![old_unleased.id.as_str(), expired.id.as_str()],
        "the old unleased row and the expired lease, oldest first; not the fresh row, not the live lease"
    );
    assert_eq!(taken[0].attempts, 1, "first run of the never-started row");
    assert_eq!(
        taken[1].attempts, 2,
        "second run of the row whose holder died"
    );

    let again = repo
        .claim_resumable_turns(&sweep("sweeper", now + 1, now - 90_000))
        .await?;
    assert!(again.is_empty(), "everything claimable is leased now");
    assert!(
        repo.list_stale_resumable_turns(&sweep("reader", now + 1, now - 90_000))
            .await?
            .is_empty(),
        "and the listing agrees"
    );
    Ok(())
}

#[tokio::test]
async fn a_row_at_the_cap_is_claimed_once_more_and_past_it_never_again() -> Result<()> {
    let db = fresh_db().await?;
    let repo = &db.repositories().resumable_turns;
    let tenant = TenantId::generate();

    let mut at_cap = row(tenant, "conv-c", "wamid.at-cap", 1_000);
    at_cap.attempts = MAX_ATTEMPTS;
    repo.record_resumable_turn(&at_cap).await?;

    // At the cap: claimed one last time, so the runner can close the
    // athlete's placeholder with the notice instead of running the LLM.
    let TurnClaim::Claimed(last) = repo
        .claim_resumable_turn(tenant, &at_cap.id, &lease("inst-a", 10_000))
        .await?
    else {
        panic!("a row at the cap is claimable once more");
    };
    assert_eq!(last.attempts, MAX_ATTEMPTS + 1);

    // Past the cap: nothing claims it, by id or by sweep, however old.
    assert_eq!(
        repo.claim_resumable_turn(tenant, &at_cap.id, &lease("inst-b", 500_000))
            .await?,
        TurnClaim::Exhausted
    );
    assert!(repo
        .claim_resumable_turns(&sweep("sweeper", 500_000, 400_000))
        .await?
        .is_empty());

    // And it no longer blocks a younger sibling.
    let younger = row(tenant, "conv-c", "wamid.younger", 2_000);
    repo.record_resumable_turn(&younger).await?;
    assert!(
        matches!(
            repo.claim_resumable_turn(tenant, &younger.id, &lease("inst-b", 500_000))
                .await?,
            TurnClaim::Claimed(_)
        ),
        "an exhausted sibling holds nobody up"
    );

    // The reaper drops it once its last lease has ended.
    assert_eq!(
        repo.reap_exhausted_turns(10_000 + LEASE_MS - 1, MAX_ATTEMPTS)
            .await?,
        0
    );
    assert_eq!(
        repo.reap_exhausted_turns(10_000 + LEASE_MS + 1, MAX_ATTEMPTS)
            .await?,
        1
    );
    assert_eq!(
        repo.claim_resumable_turn(tenant, &at_cap.id, &lease("inst-b", 600_000))
            .await?,
        TurnClaim::Missing
    );
    Ok(())
}

#[tokio::test]
async fn a_lease_is_renewed_by_its_holder_only() -> Result<()> {
    let db = fresh_db().await?;
    let repo = &db.repositories().resumable_turns;
    let tenant = TenantId::generate();
    let turn = row(tenant, "conv-r", "wamid.renew", 1_000);
    repo.record_resumable_turn(&turn).await?;
    assert!(matches!(
        repo.claim_resumable_turn(tenant, &turn.id, &lease("inst-a", 10_000))
            .await?,
        TurnClaim::Claimed(_)
    ));

    assert!(
        repo.renew_resumable_turn_lease(tenant, &turn.id, "inst-a", 10_000 + 3 * LEASE_MS)
            .await?,
        "the holder renews"
    );
    assert!(
        !repo
            .renew_resumable_turn_lease(tenant, &turn.id, "inst-b", 900_000)
            .await?,
        "another instance cannot extend a lease it does not hold"
    );
    assert_eq!(
        repo.claim_resumable_turn(tenant, &turn.id, &lease("inst-b", 10_000 + 2 * LEASE_MS))
            .await?,
        TurnClaim::Blocked,
        "the renewed lease holds past where the original would have ended"
    );
    Ok(())
}

#[tokio::test]
async fn a_released_row_is_claimable_at_once() -> Result<()> {
    let db = fresh_db().await?;
    let repo = &db.repositories().resumable_turns;
    let tenant = TenantId::generate();
    let turn = row(tenant, "conv-x", "wamid.release", 1_000);
    repo.record_resumable_turn(&turn).await?;
    assert!(matches!(
        repo.claim_resumable_turn(tenant, &turn.id, &lease("inst-a", 10_000))
            .await?,
        TurnClaim::Claimed(_)
    ));

    // Drained with an attempt left: the holder hands the row back.
    repo.release_resumable_turn(tenant, &turn.id, 10_500)
        .await?;

    let TurnClaim::Claimed(retaken) = repo
        .claim_resumable_turn(tenant, &turn.id, &lease("inst-b", 10_501))
        .await?
    else {
        panic!("a released row does not wait for the lease to expire");
    };
    assert_eq!(retaken.attempts, 2, "the hand-back keeps the run count");

    // Released again, the sweep takes it without any grace: it has been
    // leased before, so it is not a fresh row about to be started.
    repo.release_resumable_turn(tenant, &turn.id, 10_600)
        .await?;
    let taken = repo
        .claim_resumable_turns(&sweep("sweeper", 10_601, 0))
        .await?;
    assert_eq!(taken.len(), 1);
    assert_eq!(taken[0].id, turn.id);
    Ok(())
}

#[tokio::test]
async fn placeholder_and_enqueue_count_are_kept_on_the_row() -> Result<()> {
    let db = fresh_db().await?;
    let repo = &db.repositories().resumable_turns;
    let tenant = TenantId::generate();
    let turn = row(tenant, "conv-p", "wamid.placeholder", 1_000);
    repo.record_resumable_turn(&turn).await?;

    repo.set_resumable_turn_placeholder(tenant, &turn.id, "4242")
        .await?;
    assert_eq!(
        repo.bump_resumable_turn_enqueue(tenant, &turn.id).await?,
        Some(1)
    );
    assert_eq!(
        repo.bump_resumable_turn_enqueue(tenant, &turn.id).await?,
        Some(2)
    );
    assert_eq!(
        repo.bump_resumable_turn_enqueue(TenantId::generate(), &turn.id)
            .await?,
        None,
        "another tenant counts nothing"
    );

    let TurnClaim::Claimed(seen) = repo
        .claim_resumable_turn(tenant, &turn.id, &lease("inst-a", 10_000))
        .await?
    else {
        panic!("claimable");
    };
    assert_eq!(seen.placeholder_message_id.as_deref(), Some("4242"));
    assert_eq!(seen.enqueue_seq, 2);
    Ok(())
}

#[tokio::test]
async fn finishing_a_turn_deletes_it_under_its_own_tenant_only() -> Result<()> {
    let db = fresh_db().await?;
    let repo = &db.repositories().resumable_turns;
    let tenant = TenantId::generate();
    let turn = row(tenant, "conv-d", "wamid.finish", 1_000);
    repo.record_resumable_turn(&turn).await?;

    assert!(
        !repo
            .finish_resumable_turn(TenantId::generate(), &turn.id)
            .await?,
        "another tenant cannot finish this turn"
    );
    assert!(repo.finish_resumable_turn(tenant, &turn.id).await?);
    assert!(
        !repo.finish_resumable_turn(tenant, &turn.id).await?,
        "finishing twice reports the second as a no-op"
    );
    assert_eq!(
        repo.claim_resumable_turn(tenant, &turn.id, &lease("inst-a", 10_000))
            .await?,
        TurnClaim::Missing,
        "a finished turn can never be run again"
    );
    Ok(())
}
