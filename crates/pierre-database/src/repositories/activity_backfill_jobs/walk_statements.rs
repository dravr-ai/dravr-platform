// ABOUTME: The statements that move a history walk down its window: a step's progress, its end, its hand-off
// ABOUTME: And the reopen a grant connected beside a walked session owes, never under a step still holding the lease

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

pub(crate) const HISTORY_WALK_ROW_SQL: &str = "SELECT outcome, walk_backend \
     FROM activity_backfill_jobs \
     WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND kind = $4";

/// Adds a landed step to the progress and releases the row for the next
/// step, below where this one reached.
pub(crate) const RECORD_JOB_PROGRESS_SQL: &str = "UPDATE activity_backfill_jobs \
     SET before_ts = $3, months_done = months_done + $4, rows_landed = rows_landed + $5, \
         secs_per_unit = COALESCE($6, secs_per_unit), after_ts = COALESCE($7, after_ts), \
         months_total = COALESCE($8, months_total), \
         attempts = 0, leased_until_ms = 0, lease_token = NULL \
     WHERE id = $1 AND lease_token = $2 AND outcome IS NULL";

/// Adds a walk's last step to the progress and keeps the row, released, as
/// walked.
pub(crate) const END_HISTORY_WALK_SQL: &str = "UPDATE activity_backfill_jobs \
     SET before_ts = $3, months_done = months_done + $4, rows_landed = rows_landed + $5, \
         secs_per_unit = COALESCE($6, secs_per_unit), after_ts = COALESCE($7, after_ts), \
         months_total = COALESCE($8, months_total), outcome = $9, \
         leased_until_ms = 0, lease_token = NULL \
     WHERE id = $1 AND lease_token = $2 AND outcome IS NULL \
     RETURNING months_total, months_done, rows_landed, secs_per_unit";

/// Moves a running walk to its API phase ([`super::WalkHandOff`]) and releases it
/// for that phase's first step. The floor is learned again from the grant's
/// profile (`after_ts` NULL), and the pace measured anew (`secs_per_unit`
/// NULL): an API page is not a scrape.
pub(crate) const HAND_OFF_HISTORY_WALK_SQL: &str = "UPDATE activity_backfill_jobs \
     SET walk_backend = $3, before_ts = $4, months_done = $5, \
         rows_landed = rows_landed + $6, fetch_limit = $7, after_ts = NULL, \
         secs_per_unit = NULL, attempts = 0, leased_until_ms = 0, lease_token = NULL \
     WHERE id = $1 AND lease_token = $2 AND outcome IS NULL";

/// Owes a walk its session ended alone the API phase
/// ([`HAND_OFF_HISTORY_WALK_SQL`]'s row), only while it stands walked, no
/// API phase ran on it, and no step holds its lease at `$10`: a purge that
/// took the phase from under a running step leaves that step its lease, and
/// the walk is reopened only once the step has stopped, so no two steps
/// ever page the grant at once. Its rows start over at zero, its end having been
/// told; its `created_at_ms` is kept, so the resume sweep takes it on its
/// next pass.
pub(crate) const REOPEN_HISTORY_WALK_SQL: &str = "UPDATE activity_backfill_jobs \
     SET walk_backend = $5, before_ts = $6, months_done = $7, fetch_limit = $8, \
         rows_landed = 0, after_ts = NULL, secs_per_unit = NULL, outcome = NULL, \
         attempts = 0, leased_until_ms = 0, lease_token = NULL \
     WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND kind = $4 \
       AND outcome = $9 AND walk_backend IS NULL AND leased_until_ms <= $10";
