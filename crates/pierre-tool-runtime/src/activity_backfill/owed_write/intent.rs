// ABOUTME: A walk step records its write before it lands, and clears the record once its check after the write was read
// ABOUTME: The resume sweep settles a record left behind: a purge of the key since takes the page back, none drops it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The durable record of a walk step's write (registre#1555).
//!
//! Before its write a step records which activities it writes under which
//! key, at which sync stamp, and from when the purge attestations answer for
//! it ([`record_intent`]). Once its check after the write was read the step
//! settled the write itself and clears the record ([`clear_intent`]). When
//! that check stays unreadable through its retries, the record stays, and
//! the resume sweep settles it once the attestations read again
//! ([`settle_write_intents`]), a grace after the step recorded it so it never
//! races the step's own check: a purge of the key that deleted activities,
//! attested since the step's first check and not before it, takes the page
//! back as the step would have ([`KeyPurge`]); with none the record goes
//! and the page stays. Either way only the rows still carrying the step's
//! write go ([`LiveActivityStore::discard_written`]): a copy another writer
//! stored since, at a later stamp, stays.
//!
//! The sweep cannot read the counts the step's write returned, so after a
//! retention expiry it takes back what the step added without the check
//! the step makes of them ([`super::inserted_every_one`]); the stamp is what
//! spares a ride another writer stored after the write.
//!
//! [`LiveActivityStore::discard_written`]: pierre_services::live_activities::LiveActivityStore::discard_written

use std::sync::Arc;

use chrono::{DateTime, Utc};
use pierre_core::models::Activity;
use pierre_database::repositories::{ActivityWriteIntent, ActivityWriteIntentRepository};
use tracing::{error, info, warn};
use uuid::Uuid;

use super::{purge_of, reached_ids, take_back, unanswered, Attested, Held, KeyPurge, Reach};
use crate::activity_backfill::ActivityBackfillJob;
use crate::runtime::ToolRuntime;

/// How long after a step recorded its write the sweep leaves the record to
/// the step: far past every retry of the step's check after its write.
pub(super) const INTENT_GRACE_MS: i64 = 120_000;

/// Records one sweep pass settles at most.
const SWEEP_BATCH: i64 = 20;

/// How long the sweep waits before it tries a record again after a failed
/// attempt, doubled per attempt up to [`MAX_RETRY_SHIFT`] doublings.
const RETRY_WAIT_MS: i64 = 60_000;

/// The most doublings of [`RETRY_WAIT_MS`]: about an hour.
const MAX_RETRY_SHIFT: i64 = 6;

/// Failed attempts after which the sweep gives a record up: a store that
/// refused to take the page back this often is not refusing for a moment.
const MAX_INTENT_ATTEMPTS: i64 = 12;

/// The step's record of the write it is about to make: `activities` under
/// its key at the stamp `written_at`, what of them the store `held` before,
/// and the attestations `attested` before its first check. `Err` (logged)
/// when it could not be recorded: the step then writes nothing.
pub(super) async fn record_intent(
    job: &ActivityBackfillJob,
    activities: &[Activity],
    held: &Held,
    written_at: DateTime<Utc>,
    attested: Attested<'_>,
) -> Result<String, ()> {
    let now_ms = Utc::now().timestamp_millis();
    let intent = ActivityWriteIntent {
        id: Uuid::new_v4().to_string(),
        tenant_id: job.tenant_id,
        user_id: job.user_id,
        provider: job.provider_name.clone(),
        kind: job.kind.as_str().to_owned(),
        page_ids: reached_ids(activities, held, Reach::Page),
        added_ids: reached_ids(activities, held, Reach::Added),
        written_at_us: written_at.timestamp_micros(),
        purges_since_ms: attested.since.timestamp_millis(),
        attested_before: attested.before.to_vec(),
        created_at_ms: now_ms,
        due_at_ms: now_ms.saturating_add(INTENT_GRACE_MS),
        attempts: 0,
    };
    job.resources
        .repos()
        .activity_write_intents
        .record_write_intent(&intent)
        .await
        .map(|()| intent.id)
        .map_err(|e| {
            warn!(user_id = %job.user_id, provider = %job.provider_name, kind = %job.kind, error = %e, "Walk step: its write could not be recorded before it lands; nothing it read is written");
        })
}

/// The step settled its write: its record goes. A record that could not be
/// cleared is left to the sweep, which settles it again from the
/// attestations.
pub(super) async fn clear_intent(job: &ActivityBackfillJob, id: &str) {
    if let Err(e) = job
        .resources
        .repos()
        .activity_write_intents
        .clear_write_intent(id)
        .await
    {
        warn!(user_id = %job.user_id, provider = %job.provider_name, kind = %job.kind, error = %e, "Walk step: its settled write's record could not be cleared; the resume sweep settles it");
    }
}

/// Settle the steps' records due at `now_ms`, earliest first: each one whose
/// step's check after its write stayed unreadable. A record whose
/// attestations cannot be read, or whose page could not all be taken back,
/// is tried again later.
pub(crate) async fn settle_write_intents(resources: &Arc<dyn ToolRuntime>, now_ms: i64) {
    let intents = &resources.repos().activity_write_intents;
    let due = match intents.due_write_intents(now_ms, SWEEP_BATCH).await {
        Ok(due) => due,
        Err(e) => {
            warn!(error = %e, "Walk write sweep: the steps' records of their writes could not be read");
            return;
        }
    };
    for intent in due {
        let settled = settle_intent(resources, &intent).await.is_ok();
        if settled || intent.attempts.saturating_add(1) >= MAX_INTENT_ATTEMPTS {
            if !settled {
                error!(user_id = %intent.user_id, provider = %intent.provider, kind = %intent.kind, attempts = intent.attempts.saturating_add(1), "Walk write sweep: a step's write could not be settled after every attempt; its page stays");
            }
            clear_settled(intents.as_ref(), &intent).await;
        } else {
            postpone(intents.as_ref(), &intent, now_ms).await;
        }
    }
}

/// The record of a write the sweep settled, or gave up on, goes. One that
/// could not be cleared is settled again by the next pass.
async fn clear_settled(intents: &dyn ActivityWriteIntentRepository, intent: &ActivityWriteIntent) {
    if let Err(e) = intents.clear_write_intent(&intent.id).await {
        warn!(user_id = %intent.user_id, provider = %intent.provider, error = %e, "Walk write sweep: a settled write's record could not be cleared; the next pass settles it again");
    }
}

/// Count a failed attempt on `intent` and hold it a while past `now_ms`,
/// twice as long per attempt, up to about an hour.
async fn postpone(
    intents: &dyn ActivityWriteIntentRepository,
    intent: &ActivityWriteIntent,
    now_ms: i64,
) {
    let wait = RETRY_WAIT_MS << intent.attempts.clamp(0, MAX_RETRY_SHIFT);
    if let Err(e) = intents
        .postpone_write_intent(&intent.id, now_ms.saturating_add(wait))
        .await
    {
        warn!(user_id = %intent.user_id, provider = %intent.provider, error = %e, "Walk write sweep: a failed attempt could not be counted");
    }
}

/// Settle one step's record: read the purges of its key attested since its
/// first check, beyond those read before it, and take back what they
/// reached of its page. `Err` (logged) when the attestations could not be
/// read or a row could not be taken back.
async fn settle_intent(
    resources: &Arc<dyn ToolRuntime>,
    intent: &ActivityWriteIntent,
) -> Result<(), ()> {
    let (Some(since), Some(written_at)) = (
        DateTime::from_timestamp_millis(intent.purges_since_ms),
        DateTime::from_timestamp_micros(intent.written_at_us),
    ) else {
        warn!(user_id = %intent.user_id, provider = %intent.provider, "Walk write sweep: a step's record holds a time out of range; it is dropped");
        return Ok(());
    };
    let landed_since = resources
        .repos()
        .provider_data
        .purge_reasons_since(intent.user_id, &intent.tenant_id, &intent.provider, since)
        .await
        .map_err(|e| {
            warn!(user_id = %intent.user_id, provider = %intent.provider, error = %e, "Walk write sweep: the purges of a step's key could not be read");
        })?;
    let Some(purge) = purge_of(&unanswered(landed_since, &intent.attested_before)) else {
        info!(user_id = %intent.user_id, provider = %intent.provider, kind = %intent.kind, "Walk write sweep: no purge of the key reached a step's write; its page stays");
        return Ok(());
    };
    let reached = match purge {
        KeyPurge::Whole => &intent.page_ids,
        KeyPurge::Expired => &intent.added_ids,
    };
    let taken = take_back(
        resources,
        intent.user_id,
        &intent.tenant_id,
        &intent.provider,
        reached,
        written_at,
    )
    .await;
    if taken {
        info!(user_id = %intent.user_id, provider = %intent.provider, kind = %intent.kind, ?purge, "Walk write sweep: a purge of the key reached a step's write; its rows are taken back");
        Ok(())
    } else {
        Err(())
    }
}
