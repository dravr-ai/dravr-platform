// ABOUTME: The statements that keep a history walk's wellness copy resumable and remember its first fill
// ABOUTME: A session-read cap pauses the copy with no attempt spent; a completed first fill is kept across re-asks (carnet#1661)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// The day a paused wellness copy resumes from, as the row holds it.
pub(crate) const WELLNESS_RESUME_DAY_SQL: &str = "SELECT wellness_resume_day \
     FROM activity_backfill_jobs \
     WHERE id = $1 AND lease_token = $2 AND tenant_id = $3";

/// Pauses a walk's wellness copy at `$4` until `$5` (unix ms): the row is
/// released and the attempt its claim counted is returned, as
/// [`super::RELEASE_BACKFILL_JOB_CLAIM_SQL`] does, so a cap that pauses the
/// copy never spends the walk's attempts.
pub(crate) const PAUSE_WELLNESS_COPY_SQL: &str = "UPDATE activity_backfill_jobs \
     SET wellness_resume_day = $4, leased_until_ms = 0, \
         attempts = CASE WHEN attempts > 0 THEN attempts - 1 ELSE 0 END, \
         lease_token = NULL, next_attempt_ms = $5 \
     WHERE id = $1 AND lease_token = $2 AND tenant_id = $3 AND outcome IS NULL";

/// When the walk's first wellness fill completed (unix ms), `NULL` while it
/// has not.
pub(crate) const WELLNESS_FILLED_SQL: &str = "SELECT wellness_filled_at_ms \
     FROM activity_backfill_jobs \
     WHERE id = $1 AND lease_token = $2 AND tenant_id = $3";

/// Records that the walk's first wellness fill completed at `$4` (unix ms),
/// keeping the first time it did. Neither a new ask of the row
/// ([`super::RECORD_BACKFILL_JOB_SQL`]) nor the walk's end clears it.
pub(crate) const MARK_WELLNESS_FILLED_SQL: &str = "UPDATE activity_backfill_jobs \
     SET wellness_filled_at_ms = COALESCE(wellness_filled_at_ms, $4) \
     WHERE id = $1 AND lease_token = $2 AND tenant_id = $3";
