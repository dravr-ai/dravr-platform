// ABOUTME: The statements of a job row's life past its record: renew, finish, end, release, take over, reap
// ABOUTME: And the reads of its lease, its progress and its state; both backends' impls name them unqualified

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use super::known_backfill_job_kinds;

/// A renewal applies only while this runner's claim holds the row.
pub(crate) const RENEW_BACKFILL_JOB_LEASE_SQL: &str =
    "UPDATE activity_backfill_jobs SET leased_until_ms = $1 \
     WHERE id = $2 AND lease_token = $3";

pub(crate) const FINISH_BACKFILL_JOB_SQL: &str =
    "DELETE FROM activity_backfill_jobs WHERE id = $1 AND lease_token = $2";

pub(crate) const FINISH_LIVE_READ_SQL: &str = "DELETE FROM activity_backfill_jobs \
     WHERE id = $1 AND lease_token = $2 \
     RETURNING conversation_id, turn_id, asked_message_id";

pub(crate) const REAP_EXHAUSTED_BACKFILL_JOBS_SQL: &str = concat!(
    "DELETE FROM activity_backfill_jobs \
     WHERE attempts >= $1 AND leased_until_ms <= $2 AND outcome IS NULL \
       AND kind IN ",
    known_backfill_job_kinds!(),
    " RETURNING id, tenant_id, user_id, provider, after_ts, before_ts, fetch_limit, \
               conversation_id, created_at_ms, leased_until_ms, attempts, lease_token, \
               kind, months_total, months_done, rows_landed, secs_per_unit, turn_id, \
               asked_message_id, walk_backend, walk_sport"
);

/// Keeps the row, released, with the outcome it ended on — only while it is
/// owed: a walk a purge already ended keeps the outcome the purge gave it.
pub(crate) const END_BACKFILL_JOB_SQL: &str = "UPDATE activity_backfill_jobs \
     SET outcome = $3, leased_until_ms = 0, lease_token = NULL \
     WHERE id = $1 AND lease_token = $2 AND outcome IS NULL";

/// Hands a claim back unrun, with the attempt it counted.
pub(crate) const RELEASE_BACKFILL_JOB_CLAIM_SQL: &str = "UPDATE activity_backfill_jobs \
     SET leased_until_ms = 0, attempts = CASE WHEN attempts > 0 THEN attempts - 1 ELSE 0 END, \
         lease_token = NULL \
     WHERE id = $1 AND lease_token = $2 AND outcome IS NULL";

/// Gives a failed attempt's lease back, keeping its attempt; the sweep's next is due at `$3`.
pub(crate) const RELEASE_BACKFILL_JOB_LEASE_SQL: &str = "UPDATE activity_backfill_jobs \
     SET leased_until_ms = 0, lease_token = NULL, next_attempt_ms = $3 \
     WHERE id = $1 AND lease_token = $2 AND outcome IS NULL";

/// Leases an owed live read no attempt holds to a new ask, in one
/// statement: a resume sweep claiming the same row at the same instant
/// either lands first (the row is leased, and this matches nothing) or
/// after (and finds it leased). Same tenant rule as [`super::RECORD_LIVE_READ_SQL`].
pub(crate) const TAKE_OVER_IDLE_LIVE_READ_SQL: &str = "UPDATE activity_backfill_jobs \
     SET after_ts = $5, before_ts = $6, fetch_limit = $7, \
         conversation_id = COALESCE($8, conversation_id), \
         turn_id = CASE WHEN $8 IS NULL THEN turn_id ELSE $9 END, \
         asked_message_id = CASE WHEN $8 IS NULL THEN asked_message_id ELSE $10 END, \
         leased_until_ms = $11, lease_token = $12, attempts = attempts + 1 \
     WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND kind = $4 \
       AND outcome IS NULL AND leased_until_ms <= $13 \
     RETURNING id, attempts";

pub(crate) const BACKFILL_JOB_LEASE_SQL: &str =
    "SELECT leased_until_ms FROM activity_backfill_jobs \
     WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND kind = $4 AND outcome IS NULL";

pub(crate) const JOB_PROGRESS_SQL: &str =
    "SELECT months_total, months_done, rows_landed, secs_per_unit FROM activity_backfill_jobs \
     WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND kind = $4";

pub(crate) const BACKFILL_JOB_STATE_SQL: &str = "SELECT outcome FROM activity_backfill_jobs \
     WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND kind = $4";
