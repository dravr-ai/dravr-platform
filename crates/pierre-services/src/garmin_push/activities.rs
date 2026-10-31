// ABOUTME: Garmin activity pushes into the activity cache — summaries, details' laps and samples, long activities' FIT files
// ABOUTME: Every push records the fetch at its receipt time, so the provider's freshness is its last push

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Activity pushes.
//!
//! Where these write is the contract `GarminProvider` reads from
//! (`pierre_providers::garmin_provider`): the summary is one cached row under
//! `garmin`, its laps go beside it through `store_activity_detail`, its
//! samples to `activity_streams`, and every push records the fetch at its
//! receipt time.
//!
//! `activityDetails` leaves out activities longer than 24 h, so their samples
//! come from the activity's FIT file, which Garmin offers once through a
//! callback URL. The file is fetched only for such an activity: the length
//! is read from its cached summary, so a file whose summary has not arrived
//! yet waits for it, within the URL's 24 hours. A fetched file is kept from
//! its download until its samples and laps are stored, so a retry after a
//! failed write decodes the kept copy instead of asking Garmin for a file it
//! will not serve twice.
//!
//! A summary, its samples and its laps are each written only when no later
//! push of them has landed: a batch retried after a newer one leaves the
//! newer values standing.
//!
//! The activity's power is derived from its samples (`with_power`), so the
//! row carries it whichever push brings them: the details' samples, a long
//! activity's file, or — for a summary that arrives without either, as a
//! re-send after them does — the samples already stored. Samples that land
//! after a newer summary (a retried details batch, a file) amend the standing
//! row in place (`amend_activity`) rather than going through the receipt
//! guard that would refuse them, so a ride never keeps its samples and loses
//! its power.

use std::slice;

use pierre_core::constants::oauth_providers::GARMIN;
use pierre_core::errors::AppResult;

use dravr_barrage::parse::fit::{FitActivityFile, Position};
use pierre_core::models::{Activity, Lap, TimeSeriesData};
use pierre_database::repositories::{ActivityDetail, ActivityStreamsWrite, StreamOrigin};
use pierre_providers::fit_decode::decode_bounded;
use pierre_providers::upload_provider::file_sha256;
use serde_json::Value;
use tracing::{debug, info, warn};

use super::mapping::{
    activity_from_summary, laps_from_details, streams_from_samples, with_power, DETAILS_MAX_SECONDS,
};
use super::wire::{ActivityDetailsRecord, ActivityFileRecord, ActivitySummary};
use super::{records_as, ActivityFile, BatchOutcome, OwnerBatch, Retry};

/// Record that the owner's Garmin activities were brought up to date by
/// the push received at the batch's receipt time, unless a later push
/// already did: a batch retried after a newer one must not move the
/// provider's freshness back.
async fn record_push(batch: &OwnerBatch<'_>) -> AppResult<()> {
    let cache = &batch.repos.activity_cache;
    let latest = cache
        .latest_activity_sync(batch.owner.user_id, &batch.owner.tenant_id, GARMIN)
        .await?;
    if latest.is_some_and(|latest| latest >= batch.received_at) {
        return Ok(());
    }
    cache
        .record_activity_fetch(
            batch.owner.user_id,
            &batch.owner.tenant_id,
            GARMIN,
            batch.received_at,
        )
        .await
}

/// Upsert `activity` as the owner's Garmin row, synced at the receipt,
/// unless a later push of it already landed. Returns whether it was written.
async fn upsert(batch: &OwnerBatch<'_>, activity: &Activity) -> AppResult<bool> {
    let written = batch
        .repos
        .activity_cache
        .upsert_activities_synced_at(
            batch.owner.user_id,
            &batch.owner.tenant_id,
            GARMIN,
            slice::from_ref(activity),
            batch.received_at,
        )
        .await?;
    Ok(written > 0)
}

/// Store an activity's samples in the one sample store, as pushed, received
/// at the batch's receipt, unless a later push of them already landed.
/// `source_ref` is the SHA-256 of the file they were decoded from, `None`
/// for samples pushed as JSON.
async fn store_streams(
    batch: &OwnerBatch<'_>,
    activity_id: &str,
    streams: &TimeSeriesData,
    source_ref: Option<&str>,
) -> AppResult<()> {
    batch
        .repos
        .activity_streams
        .upsert_activity_streams(&ActivityStreamsWrite {
            tenant_id: &batch.owner.tenant_id,
            user_id: batch.owner.user_id,
            provider: GARMIN,
            activity_id,
            origin: StreamOrigin::Push,
            source_ref,
            streams,
            synced_at: batch.received_at,
        })
        .await
        .map(|_| ())
}

/// The samples stored for the owner's activity `activity_id`, if any.
async fn stored_streams(
    batch: &OwnerBatch<'_>,
    activity_id: &str,
) -> AppResult<Option<TimeSeriesData>> {
    batch
        .repos
        .activity_streams
        .get_activity_streams(
            &batch.owner.tenant_id,
            batch.owner.user_id,
            GARMIN,
            activity_id,
        )
        .await
}

/// `activities`: each summary upserts its cached row, with the power of the
/// samples already stored for it.
pub(super) async fn write_summaries(
    batch: &OwnerBatch<'_>,
    records: Vec<Value>,
) -> AppResult<BatchOutcome> {
    let mut outcome = BatchOutcome::default();
    let summaries: Vec<ActivitySummary> = records_as(records, &mut outcome);
    for summary in &summaries {
        match activity_from_summary(None, summary) {
            Ok(activity) => {
                let activity = match stored_streams(batch, activity.id()).await? {
                    Some(streams) => with_power(activity, &streams)?,
                    None => activity,
                };
                if upsert(batch, &activity).await? {
                    outcome.stored += 1;
                } else {
                    // A later push of this activity already landed.
                    outcome.skipped += 1;
                }
            }
            Err(reason) => {
                debug!(reason = reason.as_str(), "Garmin activity summary not kept");
                outcome.skipped += 1;
            }
        }
    }
    record_push(batch).await?;
    Ok(outcome)
}

/// `activityDetails`: the summary upserts the cached row first (the laps
/// need it), then the laps go beside it and the samples to the streams.
pub(super) async fn write_details(
    batch: &OwnerBatch<'_>,
    records: Vec<Value>,
) -> AppResult<BatchOutcome> {
    let mut outcome = BatchOutcome::default();
    let details: Vec<ActivityDetailsRecord> = records_as(records, &mut outcome);
    for record in &details {
        let activity_id = record
            .activity_id
            .as_deref()
            .or(record.summary.activity_id.as_deref());
        let activity = match activity_from_summary(activity_id, &record.summary) {
            Ok(activity) => activity,
            Err(reason) => {
                debug!(reason = reason.as_str(), "Garmin activity details not kept");
                outcome.skipped += 1;
                continue;
            }
        };
        let start = activity.start_date().timestamp();
        let duration = i64::try_from(activity.duration_seconds()).unwrap_or(i64::MAX);
        let streams = streams_from_samples(start, &record.samples);
        let activity = match &streams {
            Some(streams) => with_power(activity, streams)?,
            None => activity,
        };
        // A later summary of the activity may already stand; the laps and
        // samples below are still this push's, and the samples keep their own
        // receipt order, so the standing summary takes their power in place.
        // The record counts as stored for what it brought.
        if !upsert(batch, &activity).await? {
            if let Some(streams) = &streams {
                amend_power(batch, activity.id(), streams).await?;
            }
        }
        if let Some(laps) = laps_from_details(start, duration, &record.laps, &record.samples) {
            store_laps(batch, activity.id(), laps).await?;
        }
        if let Some(streams) = &streams {
            store_streams(batch, activity.id(), streams, None).await?;
        }
        outcome.stored += 1;
    }
    record_push(batch).await?;
    Ok(outcome)
}

/// Store an activity's laps beside its cached row; they stand until a later
/// push replaces them.
async fn store_laps(batch: &OwnerBatch<'_>, activity_id: &str, laps: Vec<Lap>) -> AppResult<()> {
    let detail = ActivityDetail {
        splits: None,
        laps: Some(laps),
    };
    let stored = batch
        .repos
        .activity_cache
        .store_activity_detail(
            batch.owner.user_id,
            &batch.owner.tenant_id,
            GARMIN,
            activity_id,
            &detail,
            None,
        )
        .await?;
    if !stored {
        warn!(
            user_id = %batch.owner.user_id,
            "Garmin laps had no cached activity to stand beside; not stored"
        );
    }
    Ok(())
}

/// What to do with one activity file.
enum FilePlan {
    /// Fetch it: its activity is longer than the details cover.
    Fetch,
    /// Store it from the copy kept when an earlier run downloaded it.
    Kept(Vec<u8>),
    /// Nothing: the details cover it, or its samples and laps are stored.
    Skip,
    /// Its summary has not arrived, so its length is unknown yet.
    Wait,
}

/// Whether the owner's activity `activity_id` needs its file fetched.
async fn plan_file(batch: &OwnerBatch<'_>, activity_id: &str) -> AppResult<FilePlan> {
    let Some(activity) = batch
        .repos
        .activity_cache
        .get_cached_activity(
            batch.owner.user_id,
            &batch.owner.tenant_id,
            GARMIN,
            activity_id,
        )
        .await?
    else {
        return Ok(FilePlan::Wait);
    };
    if activity.duration_seconds() <= DETAILS_MAX_SECONDS {
        return Ok(FilePlan::Skip);
    }
    // A kept copy means a download whose samples or laps did not land yet:
    // the copy is dropped only once both are stored.
    if let Some(bytes) = batch
        .repos
        .pushed_data
        .kept_file(
            &batch.owner.tenant_id,
            batch.owner.user_id,
            GARMIN,
            activity_id,
        )
        .await?
    {
        return Ok(FilePlan::Kept(bytes));
    }
    Ok(if stored_streams(batch, activity_id).await?.is_some() {
        FilePlan::Skip
    } else {
        FilePlan::Fetch
    })
}

/// `activityFiles`: the file of an activity longer than 24 h is fetched once,
/// kept until its samples and laps are stored, and stored; every other file
/// is left unfetched.
pub(super) async fn write_files(
    batch: &OwnerBatch<'_>,
    records: Vec<Value>,
) -> AppResult<BatchOutcome> {
    let mut outcome = BatchOutcome::default();
    let files: Vec<ActivityFileRecord> = records_as(records, &mut outcome);
    for file in &files {
        let (Some(activity_id), Some(url)) =
            (file.activity_id.as_deref(), file.callback_url.as_deref())
        else {
            outcome.unreadable += 1;
            continue;
        };
        if file.manual == Some(true)
            || !file
                .file_type
                .as_deref()
                .is_some_and(|kind| kind.eq_ignore_ascii_case("FIT"))
        {
            outcome.skipped += 1;
            continue;
        }
        match plan_file(batch, activity_id).await? {
            FilePlan::Skip => outcome.skipped += 1,
            FilePlan::Kept(bytes) => {
                store_file(batch, activity_id, bytes).await?;
                outcome.stored += 1;
            }
            FilePlan::Wait => {
                outcome.absorb(BatchOutcome {
                    retry: Some(Retry::Wait(
                        "an activity file arrived before its activity summary".to_owned(),
                    )),
                    ..BatchOutcome::default()
                });
            }
            FilePlan::Fetch => match batch.host.fetch_activity_file(batch.owner, url).await? {
                ActivityFile::Bytes(bytes) => {
                    // Garmin serves the file once: keep it before anything
                    // else can fail, so a retry decodes this copy.
                    batch
                        .repos
                        .pushed_data
                        .keep_file(
                            &batch.owner.tenant_id,
                            batch.owner.user_id,
                            GARMIN,
                            activity_id,
                            &bytes,
                            batch.received_at,
                        )
                        .await?;
                    store_file(batch, activity_id, bytes).await?;
                    outcome.stored += 1;
                }
                ActivityFile::Gone => {
                    info!(
                        user_id = %batch.owner.user_id,
                        "Garmin activity file is no longer offered; its samples stay unstored"
                    );
                    outcome.skipped += 1;
                }
                ActivityFile::TooLarge => {
                    warn!(
                        user_id = %batch.owner.user_id,
                        "Garmin activity file is larger than any workout's file; not stored"
                    );
                    outcome.skipped += 1;
                }
                ActivityFile::RateLimited(wait) => {
                    outcome.absorb(BatchOutcome {
                        retry: Some(Retry::Later(format!(
                            "the activity file download budget is spent for {}s",
                            wait.as_secs()
                        ))),
                        ..BatchOutcome::default()
                    });
                }
            },
        }
    }
    Ok(outcome)
}

/// Decode a fetched FIT file and store its samples and laps for the
/// owner's activity `activity_id`. A file that is not a readable activity is
/// logged and dropped: fetching it again would answer the same bytes, if it
/// answered at all.
async fn store_file(batch: &OwnerBatch<'_>, activity_id: &str, bytes: Vec<u8>) -> AppResult<()> {
    let id = activity_id.to_owned();
    let sha = file_sha256(&bytes);
    // CPU-bound and memory-heavy: decoded on the blocking pool under the
    // process-wide decode bound every FIT decode shares.
    let decoded = decode_bounded(move || {
        FitActivityFile::parse(&bytes).map(|file| {
            let laps = file
                .activities(|_| id.clone(), GARMIN)
                .first()
                .and_then(|activity| activity.laps().cloned());
            (file.time_series(Position::Read), laps)
        })
    })
    .await?;
    let drop_kept = || {
        batch.repos.pushed_data.drop_file(
            &batch.owner.tenant_id,
            batch.owner.user_id,
            GARMIN,
            activity_id,
        )
    };
    let (streams, laps) = match decoded {
        Ok(decoded) => decoded,
        Err(e) => {
            warn!(
                user_id = %batch.owner.user_id,
                error = %e,
                "Garmin activity file is not a readable activity; dropped"
            );
            return drop_kept().await;
        }
    };
    store_streams(batch, activity_id, &streams, Some(&sha)).await?;
    if let Some(laps) = laps {
        store_laps(batch, activity_id, laps).await?;
    }
    // Its summary came first (a file waits for it) and carried no power.
    amend_power(batch, activity_id, &streams).await?;
    // All landed: the kept copy has served its purpose.
    drop_kept().await
}

/// Give the owner's standing cached activity `activity_id` the power of
/// `streams`, leaving its receipt as it is: its own push carried no samples
/// (a summary never does), so the power is the samples' to add whenever they
/// land.
async fn amend_power(
    batch: &OwnerBatch<'_>,
    activity_id: &str,
    streams: &TimeSeriesData,
) -> AppResult<()> {
    let Some(activity) = batch
        .repos
        .activity_cache
        .get_cached_activity(
            batch.owner.user_id,
            &batch.owner.tenant_id,
            GARMIN,
            activity_id,
        )
        .await?
    else {
        return Ok(());
    };
    let amended = with_power(activity, streams)?;
    batch
        .repos
        .activity_cache
        .amend_activity(
            batch.owner.user_id,
            &batch.owner.tenant_id,
            GARMIN,
            &amended,
        )
        .await
        .map(|_| ())
}
