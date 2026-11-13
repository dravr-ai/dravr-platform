// ABOUTME: The wellness repositories' read-only lookups: data sources, origins, owning tenants
// ABOUTME: Shared by the SQLite and PostgreSQL engines

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ============================================================================
// data_sources
// ============================================================================

/// The columns every data-source read decodes, in the order
/// [`data_source_from_row`] reads them.
macro_rules! data_source_columns {
    () => {
        "id, user_id, provider, device_model, software_version, source, device_type, original_source_name"
    };
}

/// Every data source a user has under a tenant, newest first.
pub(crate) const LIST_DATA_SOURCES_SQL: &str = concat!(
    "
            SELECT ",
    data_source_columns!(),
    "
            FROM data_sources
            WHERE user_id = $1 AND tenant_id = $2
            ORDER BY created_at DESC
            "
);

// ============================================================================
// origin and tenant lookups
// ============================================================================

/// The distinct origins of the live sessions starting inside a window.
pub(crate) const GET_SLEEP_ORIGINS_SQL: &str = "
            SELECT DISTINCT provider, COALESCE(data_source_id, '') AS data_source_id
            FROM sleep_sessions
            WHERE user_id = $1 AND tenant_id = $2 AND start_time >= $3 AND start_time <= $4
              AND deleted_at IS NULL
            ";

/// The tenant a session was stored under, for the enforme adapter that only
/// holds the provider id.
pub(crate) const FIND_SLEEP_SESSION_TENANT_SQL: &str =
    "SELECT tenant_id FROM sleep_sessions WHERE id = $1";

/// The distinct origins of the live recovery readings dated inside a window.
pub(crate) const GET_RECOVERY_ORIGINS_SQL: &str = "
            SELECT DISTINCT provider, COALESCE(data_source_id, '') AS data_source_id
            FROM recovery_metrics
            WHERE user_id = $1 AND tenant_id = $2 AND date >= $3 AND date <= $4
              AND deleted_at IS NULL
            ";

/// The tenant a recovery row was stored under.
pub(crate) const FIND_RECOVERY_METRIC_TENANT_SQL: &str =
    "SELECT tenant_id FROM recovery_metrics WHERE id = $1";

/// The tenant a snapshot was stored under.
pub(crate) const FIND_HEALTH_SNAPSHOT_TENANT_SQL: &str =
    "SELECT tenant_id FROM health_snapshots WHERE id = $1";
