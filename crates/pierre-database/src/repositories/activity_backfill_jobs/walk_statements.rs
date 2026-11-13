// ABOUTME: The statements that move a history walk down its window: a step's progress, its end, its hand-off
// ABOUTME: And the reopen of its API phase a grant owes, or a purge leaves, never under a step still holding the lease

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

/// Owes a walk its API phase through the grant now live beside its session
/// ([`HAND_OFF_HISTORY_WALK_SQL`]'s row), from the one state `$9` names
/// ([`super::ReopenFrom`]): a walk whose session ended alone (walked, no API
/// phase on it), or one a purge left owed a reopen (`$11`, the purged phase)
/// and still is. Only once no step holds its lease at `$10`: a
/// step of a purged phase may still be paging the grant, and a second step
/// beside it would page it twice. Its rows start over at zero, its end
/// having been told; its retry is due at once, whatever a failed step before
/// the purge left; its `created_at_ms` is kept, so the resume sweep takes it
/// on its next pass.
pub(crate) const REOPEN_HISTORY_WALK_SQL: &str = "UPDATE activity_backfill_jobs \
     SET walk_backend = $5, before_ts = $6, months_done = $7, fetch_limit = $8, \
         rows_landed = 0, after_ts = NULL, secs_per_unit = NULL, outcome = NULL, \
         attempts = 0, leased_until_ms = 0, lease_token = NULL, next_attempt_ms = 0 \
     WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND kind = $4 \
       AND outcome = $9 AND (walk_backend IS NULL OR outcome = $11) \
       AND leased_until_ms <= $10";

/// Keeps a walk a purge left owed a reopen (`$6`) as walked (`$5`), its
/// session walked alone, once no step holds its lease at `$7`: no grant is
/// live to reopen the phase through, and one connected later reopens it.
pub(crate) const SETTLE_PURGED_HISTORY_WALK_SQL: &str = "UPDATE activity_backfill_jobs \
     SET outcome = $5, walk_backend = NULL, leased_until_ms = 0, lease_token = NULL \
     WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND kind = $4 \
       AND outcome = $6 AND leased_until_ms <= $7";

/// The walks (`$1`) a purge left owed a reopen (`$2`) that no step holds at
/// `$3`, oldest first, at most `$4`: across every tenant, as the resume
/// sweep's claim reads.
pub(crate) const PURGED_HISTORY_WALKS_SQL: &str = "SELECT user_id, tenant_id, provider \
     FROM activity_backfill_jobs \
     WHERE kind = $1 AND outcome = $2 AND leased_until_ms <= $3 \
     ORDER BY created_at_ms ASC \
     LIMIT $4";
