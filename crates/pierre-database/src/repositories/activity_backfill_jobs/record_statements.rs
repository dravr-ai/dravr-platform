// ABOUTME: The statements that record an ask in the job ledger: a new job, a live read's newer ask, its rebind
// ABOUTME: One row per (user, provider, kind); an owed row is never replaced, an ended one only from its own tenant

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// A conflict on `(user_id, provider, kind)` changes nothing while a job is
/// owed (`outcome IS NULL`): a second ask is the same job, and
/// `rows_affected` tells the caller so. A row that only records an ended job
/// is replaced whole by the new one, but only by an ask in the row's own
/// tenant: an ask from another tenant never overwrites what this tenant's
/// card reads.
pub(crate) const RECORD_BACKFILL_JOB_SQL: &str = "INSERT INTO activity_backfill_jobs \
     (id, tenant_id, user_id, provider, after_ts, before_ts, fetch_limit, \
      conversation_id, created_at_ms, leased_until_ms, attempts, lease_token, kind, \
      months_total, months_done, rows_landed, secs_per_unit, turn_id, asked_message_id) \
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19) \
     ON CONFLICT (user_id, provider, kind) DO UPDATE SET \
       id = excluded.id, tenant_id = excluded.tenant_id, after_ts = excluded.after_ts, \
       before_ts = excluded.before_ts, fetch_limit = excluded.fetch_limit, \
       conversation_id = excluded.conversation_id, created_at_ms = excluded.created_at_ms, \
       leased_until_ms = excluded.leased_until_ms, attempts = excluded.attempts, \
       lease_token = excluded.lease_token, months_total = excluded.months_total, \
       months_done = excluded.months_done, rows_landed = excluded.rows_landed, \
       secs_per_unit = excluded.secs_per_unit, turn_id = excluded.turn_id, \
       asked_message_id = excluded.asked_message_id, walk_backend = NULL, outcome = NULL, \
       next_attempt_ms = 0 \
     WHERE activity_backfill_jobs.outcome IS NOT NULL \
       AND activity_backfill_jobs.tenant_id = excluded.tenant_id";

/// A live read asked again while one is owed answers the newest ask: the
/// conflict moves only where its follow-up goes, never its window or its
/// lease, and only in the row's own tenant. One statement, so a run that
/// finishes the owed row meanwhile either serializes before it (the ask
/// inserts a new row) or after it (the rebound row is finished): the ask is
/// never lost between two statements. `RETURNING id` tells the two apart —
/// the ask's own id when it inserted, the owed row's when it rebound,
/// nothing when another tenant's row stands, or when the ask has no
/// conversation: a read nobody waits on (a stale head's top-up) never takes
/// the follow-up away from an athlete who does. No guard on `outcome`:
/// nothing ends a live read incomplete, so an owed row is the only one there
/// is.
pub(crate) const RECORD_LIVE_READ_SQL: &str = "INSERT INTO activity_backfill_jobs \
     (id, tenant_id, user_id, provider, after_ts, before_ts, fetch_limit, \
      conversation_id, created_at_ms, leased_until_ms, attempts, lease_token, kind, \
      months_total, months_done, rows_landed, secs_per_unit, turn_id, asked_message_id) \
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19) \
     ON CONFLICT (user_id, provider, kind) DO UPDATE SET \
       conversation_id = excluded.conversation_id, turn_id = excluded.turn_id, \
       asked_message_id = excluded.asked_message_id \
     WHERE activity_backfill_jobs.tenant_id = excluded.tenant_id \
       AND excluded.conversation_id IS NOT NULL \
     RETURNING id";

/// Rebinds the live read owed for the pair to a newer ask, without ever
/// recording one: the ask arrived while this instance runs the read, and a
/// row it inserted would be a second read nobody runs until its lease lapsed.
/// Same rules as [`RECORD_LIVE_READ_SQL`]'s conflict: the row's own tenant,
/// and only an ask with a conversation.
pub(crate) const REBIND_LIVE_READ_SQL: &str = "UPDATE activity_backfill_jobs \
     SET conversation_id = $5, turn_id = $6, asked_message_id = $7 \
     WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 AND kind = $4 \
       AND $5 IS NOT NULL";
