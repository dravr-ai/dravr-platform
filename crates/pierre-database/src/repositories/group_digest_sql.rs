// ABOUTME: Shared statements for a group's weekly digest lease — claim the week, then finish it
// ABOUTME: Split from coaching_groups.rs so it stays under the size ceiling; the repository body there runs them

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// A missing row inserts (nobody has claimed the week); an existing one takes
/// the new lease only when the week was never finished and the old lease has
/// lapsed. Both drivers report zero rows changed when the WHERE fails — that
/// zero is the losing claim.
pub(crate) const CLAIM_GROUP_DIGEST_SQL: &str = r"INSERT INTO group_digest_deliveries
              (tenant_id, group_id, week_key, leased_until_ms, finished_at_ms)
              VALUES ($1, $2, $3, $4, 0)
              ON CONFLICT(tenant_id, group_id, week_key) DO UPDATE
              SET leased_until_ms = excluded.leased_until_ms
              WHERE group_digest_deliveries.finished_at_ms = 0
                AND group_digest_deliveries.leased_until_ms <= $5";

/// Close one group's week: stamp it finished and release the lease.
pub(crate) const FINISH_GROUP_DIGEST_SQL: &str = r"UPDATE group_digest_deliveries
              SET finished_at_ms = $1, leased_until_ms = 0
              WHERE tenant_id = $2 AND group_id = $3 AND week_key = $4";
