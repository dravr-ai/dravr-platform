// ABOUTME: An uploaded .fit of a completed workout becomes one cached activity per session, filed under 'upload'
// ABOUTME: Decoded, checked against the copies the athlete already holds, the file kept; nothing the file lacks is made up

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Activity file uploads (carnet#818).
//!
//! An athlete uploads the `.fit` file of a completed workout. Each session in
//! it becomes an activity read like a provider's: it is written to the
//! activity cache under the [`UPLOAD`] key, so Home's recent list, the week
//! strip and calendar, the weekly volume and the agent's activity reads all
//! see it, and the file is kept (`uploaded_activity_files`) so the series,
//! laps and route are read from it on demand
//! (`pierre_providers::upload_provider`).
//!
//! A session the athlete already holds is not written twice: the same file
//! uploaded again names the same activity ids, and a workout a provider has
//! already synced is found by the session merger the Home list and the
//! agent's reads merge copies with. A file all of whose sessions are already
//! held is refused as a conflict; the error names the copy that holds the
//! first for the server's log, and the client reads the conflict's code.
//! A workout uploaded first and synced by a provider later is two cached
//! copies, which every reader merges into one session.
//!
//! The athlete can delete an uploaded activity, as Strava and intervals.icu
//! let them ([`delete_uploaded_activity`]): its cached row goes, and the file
//! goes with the last of its sessions. An upload or a deletion is a moment the
//! stored activities change, so either one drops the agent's cached lists and
//! recomputes the training-load rollup the way a provider capture does.

use std::sync::Arc;

use chrono::{Duration, Utc};
use pierre_cache::CacheKey;
use pierre_core::constants::oauth_providers::UPLOAD;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{Activity, TenantId};
use pierre_providers::backend_resolver::user_facing_name;
use pierre_providers::deduplication::{merge_duplicates, DedupConfig};
use pierre_providers::fit_file::FitActivityFile;
use pierre_providers::upload_provider::{
    file_sha256, parse_upload_activity_id, upload_activity_id,
};
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::training_history_compute::rewarm_default_window;
use tracing::{info, warn};
use uuid::Uuid;

/// The largest file accepted: an eight-hour ride recorded every second is a
/// few megabytes, so anything past this is not one workout's file.
pub const MAX_UPLOAD_BYTES: usize = 16 * 1024 * 1024;

/// How far either side of a session the duplicate check reads the cache: the
/// merger pairs copies whose starts lie within an hour or so, and a day
/// either side holds every copy of a workout.
const DUPLICATE_WINDOW_HOURS: i64 = 24;

/// Most cached rows the duplicate check reads around one session.
const DUPLICATE_ROW_LIMIT: i64 = 200;

/// A copy of a workout the athlete already holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldCopy {
    /// The provider the athlete knows the copy from, by its user-facing slug.
    pub provider: String,
    /// That provider's id for it.
    pub id: String,
}

/// What an upload stored.
#[derive(Debug, Clone)]
pub struct UploadOutcome {
    /// The activities the file's new sessions became, in file order.
    pub stored: Vec<Activity>,
    /// The copies already held of the file's other sessions.
    pub already_held: Vec<HeldCopy>,
}

/// Decode `bytes` as a completed workout's `.fit` file and keep every session
/// the athlete does not already hold.
///
/// # Errors
///
/// - [`AppError::invalid_input`] when the upload is empty or larger than
///   [`MAX_UPLOAD_BYTES`], is not a FIT file, is a FIT file of another kind
///   (a course, a workout), or holds no completed session;
/// - [`AppError::already_exists`] when every session in it is already held;
/// - a database error when a read or a write fails.
pub async fn upload_activity_file(
    runtime: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    bytes: &[u8],
) -> AppResult<UploadOutcome> {
    if bytes.is_empty() {
        return Err(AppError::invalid_input("the upload holds no file"));
    }
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(AppError::invalid_input(format!(
            "the file is larger than {} MiB",
            MAX_UPLOAD_BYTES / (1024 * 1024)
        )));
    }
    let file = FitActivityFile::parse(bytes).map_err(|e| {
        AppError::invalid_input(format!("the file is not a completed activity: {e}"))
    })?;
    let sha = file_sha256(bytes);
    let sessions = file.activities(|index| upload_activity_id(&sha, index), UPLOAD);

    let mut stored = Vec::with_capacity(sessions.len());
    let mut already_held = Vec::new();
    for session in sessions {
        match held_copy(runtime, tenant_id, user_id, &session).await? {
            Some(copy) => already_held.push(copy),
            None => stored.push(session),
        }
    }
    if stored.is_empty() {
        let first = already_held.first().map_or_else(String::new, |copy| {
            format!(" ({} activity {})", copy.provider, copy.id)
        });
        return Err(AppError::already_exists(format!(
            "the workout this file records{first}"
        )));
    }

    let repos = runtime.repos();
    repos
        .uploaded_activity_files
        .store_uploaded_file(&tenant_id, user_id, &sha, bytes)
        .await?;
    repos
        .activity_cache
        .upsert_activities_synced_at(user_id, &tenant_id, UPLOAD, &stored, Utc::now())
        .await?;
    after_stored_activities_changed(runtime, tenant_id, user_id).await;
    info!(
        %user_id,
        tenant_id = %tenant_id,
        sessions = stored.len(),
        already_held = already_held.len(),
        bytes = bytes.len(),
        "activity file uploaded"
    );
    Ok(UploadOutcome {
        stored,
        already_held,
    })
}

/// Delete one of the athlete's uploaded activities, and the file it came from
/// once none of that file's activities is left.
///
/// Scoped to the athlete and the tenant: an id they did not upload here —
/// another athlete's, or a provider's activity — is not found.
///
/// # Errors
///
/// - [`AppError::not_found`] when the athlete holds no uploaded activity with
///   that id in this tenant;
/// - a database error when a read or a delete fails.
pub async fn delete_uploaded_activity(
    runtime: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    activity_id: &str,
) -> AppResult<()> {
    let not_found = || AppError::not_found(format!("uploaded activity {activity_id}"));
    let (sha, _) = parse_upload_activity_id(activity_id).ok_or_else(not_found)?;
    let repos = runtime.repos();
    let removed = repos
        .activity_cache
        .delete_cached_activity(user_id, &tenant_id, UPLOAD, activity_id)
        .await?;
    if removed == 0 {
        return Err(not_found());
    }
    let file_deleted = if file_activities_remain(runtime, tenant_id, user_id, sha).await? {
        false
    } else {
        repos
            .uploaded_activity_files
            .delete_uploaded_file(&tenant_id, user_id, sha)
            .await?
    };
    after_stored_activities_changed(runtime, tenant_id, user_id).await;
    info!(
        %user_id,
        tenant_id = %tenant_id,
        activity_id,
        file_deleted,
        "uploaded activity deleted"
    );
    Ok(())
}

/// Whether any other activity the file `sha` became is still cached.
///
/// The file names how many sessions it holds; each one's id is derived from
/// the hash and its index. A file that no longer decodes holds nothing any
/// activity could read, so it keeps none.
async fn file_activities_remain(
    runtime: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    sha: &str,
) -> AppResult<bool> {
    let repos = runtime.repos();
    let Some(bytes) = repos
        .uploaded_activity_files
        .get_uploaded_file(&tenant_id, user_id, sha)
        .await?
    else {
        return Ok(false);
    };
    let Ok(file) = FitActivityFile::parse(&bytes) else {
        warn!(%user_id, file_sha256 = sha, "stored upload no longer decodes; deleting it");
        return Ok(false);
    };
    for index in 0..file.session_count() {
        if repos
            .activity_cache
            .get_cached_activity(user_id, &tenant_id, UPLOAD, &upload_activity_id(sha, index))
            .await?
            .is_some()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

/// The copy of `session` the athlete already holds in this tenant, if any:
/// the same session from an earlier upload of the same file, or a copy of the
/// same workout the session merger pairs it with.
async fn held_copy(
    runtime: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
    session: &Activity,
) -> AppResult<Option<HeldCopy>> {
    let cache = &runtime.repos().activity_cache;
    if cache
        .get_cached_activity(user_id, &tenant_id, UPLOAD, session.id())
        .await?
        .is_some()
    {
        return Ok(Some(HeldCopy {
            provider: UPLOAD.to_owned(),
            id: session.id().to_owned(),
        }));
    }
    let window = Duration::hours(DUPLICATE_WINDOW_HOURS);
    let end = session.start_date()
        + Duration::seconds(i64::try_from(session.duration_seconds()).unwrap_or(i64::MAX));
    let rows = cache
        .get_cached_activity_rows(
            user_id,
            &tenant_id,
            session.start_date() - window,
            end + window,
            DUPLICATE_ROW_LIMIT,
        )
        .await?;
    Ok(paired_copy(
        session,
        rows.into_iter().map(|row| (row.provider, row.activity)),
    ))
}

/// The held copy the session merger pairs `session` with, if any.
fn paired_copy(
    session: &Activity,
    held: impl Iterator<Item = (String, Activity)>,
) -> Option<HeldCopy> {
    let held: Vec<(String, Activity)> = held.collect();
    let mut recordings = vec![session.clone()];
    recordings.extend(held.iter().map(|(_, activity)| activity.clone()));
    let (_, report) = merge_duplicates(recordings, &DedupConfig::from_env());
    let group = report
        .groups
        .iter()
        .find(|group| group.fragment_ids.iter().any(|id| id == session.id()))?;
    group
        .fragment_ids
        .iter()
        .filter(|id| *id != session.id())
        .find_map(|id| held.iter().find(|(_, activity)| activity.id() == id))
        .map(|(provider, activity)| HeldCopy {
            provider: user_facing_name(provider).to_owned(),
            id: activity.id().to_owned(),
        })
}

/// What an upload or a deletion changes beyond the stored rows: the agent's
/// cached lists are dropped and the training-load rollup is recomputed, both
/// best effort — the rows are already written, and a rollup that fails to
/// warm is recomputed on the next capture or ask.
async fn after_stored_activities_changed(
    runtime: &Arc<dyn ToolRuntime>,
    tenant_id: TenantId,
    user_id: Uuid,
) {
    forget_cached_lists(runtime, tenant_id, user_id).await;
    if let Err(e) = rewarm_default_window(runtime, tenant_id, user_id).await {
        warn!(%user_id, error = %e, "training-load rollup not recomputed after an upload change");
    }
}

/// Drop the agent's cached activity lists for the athlete, so the next
/// question about their training sees the new workout rather than a list
/// cached before it. Best effort: a list that stays cached ages out with its
/// TTL.
async fn forget_cached_lists(runtime: &Arc<dyn ToolRuntime>, tenant_id: TenantId, user_id: Uuid) {
    let pattern = CacheKey::user_pattern(tenant_id, user_id, "*");
    if let Err(e) = runtime.cache().invalidate_pattern(&pattern).await {
        warn!(%user_id, error = %e, "cached activity lists not dropped after an upload change");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use pierre_core::models::{ActivityBuilder, SportType};

    fn ride(id: &str, provider: &str, minutes_later: i64, distance: f64) -> Activity {
        let start = Utc
            .with_ymd_and_hms(2026, 10, 1, 7, 0, 0)
            .single()
            .unwrap_or_else(|| panic!("a valid instant"))
            + Duration::minutes(minutes_later);
        ActivityBuilder::new(id, "Ride", SportType::Ride, start, 3_600, provider)
            .distance_meters(distance)
            .build()
    }

    #[test]
    fn a_provider_copy_of_the_same_workout_is_the_held_copy() {
        let upload = ride("file-0", UPLOAD, 0, 30_000.0);
        let strava = ride("123", "strava", 1, 30_100.0);
        let copy = paired_copy(&upload, [("strava".to_owned(), strava)].into_iter());
        assert_eq!(
            copy,
            Some(HeldCopy {
                provider: "strava".to_owned(),
                id: "123".to_owned(),
            })
        );
    }

    #[test]
    fn another_workout_the_same_day_is_not_a_copy() {
        let upload = ride("file-0", UPLOAD, 0, 30_000.0);
        let evening = ride("456", "strava", 600, 12_000.0);
        assert_eq!(
            paired_copy(&upload, [("strava".to_owned(), evening)].into_iter()),
            None
        );
    }

    #[test]
    fn a_mirror_backend_copy_is_named_by_the_provider_it_mirrors() {
        let upload = ride("file-0", UPLOAD, 0, 30_000.0);
        let garmin = ride("g-9", "sciotte_garmin", 0, 30_000.0);
        let copy = paired_copy(&upload, [("sciotte_garmin".to_owned(), garmin)].into_iter());
        assert_eq!(copy.map(|copy| copy.provider), Some("garmin".to_owned()));
    }
}
