// ABOUTME: The wellness repositories' deletes: one night, recovery day or snapshot by id, softly or for good
// ABOUTME: And the hard deletes of a provider's nights overlapping a window and its days in a date span; shared by both engines

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ============================================================================
// sleep_sessions
// ============================================================================

/// Soft-delete one session: stamps `deleted_at` once, never twice.
pub(crate) const SOFT_DELETE_SLEEP_SESSION_SQL: &str = "UPDATE sleep_sessions SET deleted_at = $1 \
                 WHERE id = $2 AND tenant_id = $3 AND deleted_at IS NULL";

/// Hard-delete one session under its tenant.
pub(crate) const DELETE_SLEEP_SESSION_SQL: &str =
    "DELETE FROM sleep_sessions WHERE id = $1 AND tenant_id = $2";

/// Hard-delete every night a user holds under one provider that overlaps the
/// window `[$4, $5)`: it starts before the window ends and ends after it
/// starts. A night touching the window only at an edge is kept.
pub(crate) const DELETE_SLEEP_SESSIONS_OVERLAPPING_SQL: &str = "DELETE FROM sleep_sessions \
                 WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 \
                 AND start_time < $5 AND end_time > $4";

// ============================================================================
// recovery_metrics
// ============================================================================

/// Soft-delete one recovery row: stamps `deleted_at` once, never twice.
pub(crate) const SOFT_DELETE_RECOVERY_METRIC_SQL: &str =
    "UPDATE recovery_metrics SET deleted_at = $1 \
                 WHERE id = $2 AND tenant_id = $3 AND deleted_at IS NULL";

/// Hard-delete one recovery row under its tenant.
pub(crate) const DELETE_RECOVERY_METRIC_SQL: &str =
    "DELETE FROM recovery_metrics WHERE id = $1 AND tenant_id = $2";

/// Hard-delete every recovery day a user holds under one provider dated
/// `[$4, $5]`, both ends included.
pub(crate) const DELETE_RECOVERY_METRICS_DATED_SQL: &str = "DELETE FROM recovery_metrics \
                 WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 \
                 AND date >= $4 AND date <= $5";

// ============================================================================
// health_snapshots
// ============================================================================

/// Soft-delete one snapshot: stamps `deleted_at` once, never twice.
pub(crate) const SOFT_DELETE_HEALTH_SNAPSHOT_SQL: &str =
    "UPDATE health_snapshots SET deleted_at = $1 \
                 WHERE id = $2 AND tenant_id = $3 AND deleted_at IS NULL";

/// Hard-delete one snapshot under its tenant.
pub(crate) const DELETE_HEALTH_SNAPSHOT_SQL: &str =
    "DELETE FROM health_snapshots WHERE id = $1 AND tenant_id = $2";

/// Hard-delete every body day a user holds under one provider dated
/// `[$4, $5]`, both ends included.
pub(crate) const DELETE_HEALTH_SNAPSHOTS_DATED_SQL: &str = "DELETE FROM health_snapshots \
                 WHERE user_id = $1 AND tenant_id = $2 AND provider = $3 \
                 AND date >= $4 AND date <= $5";
