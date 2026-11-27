// ABOUTME: The system agents' statements: create, list, the catalogue-wide listing, read, update and delete, and use totals
// ABOUTME: Split from agents.rs so it stays under the size ceiling; agents.rs re-exports every one

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// Create a system agent.
pub(crate) const INSERT_SYSTEM_AGENT_SQL: &str = r"
    INSERT INTO agents (
        id, user_id, tenant_id, title, description, system_prompt,
        category, tags, sample_prompts, token_count,
        created_at, updated_at, is_system, visibility, prerequisites,
        forked_from, max_tool_iterations, temperature, startup_query, data_requirements
    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11, TRUE, $12,
              NULL, NULL, NULL, NULL, NULL, NULL)
";

/// A tenant's system agents, newest first.
pub(crate) const LIST_SYSTEM_AGENTS_SQL: &str = concat!(
    "SELECT ",
    agent_columns!(""),
    " FROM agents WHERE tenant_id = $1 AND is_system = TRUE ORDER BY created_at DESC"
);

/// Each agent's uses summed over the members of tenant `$1`, or over every
/// user when it binds `NULL`. Grouped on the member rather than on the agent's
/// tenant: system agents sit in the seed tenant while every tenant's members
/// use them.
pub(crate) const AGENT_USE_TOTALS_SQL: &str = r"
    SELECT ca.agent_id AS agent_id, CAST(SUM(ca.use_count) AS BIGINT) AS uses
    FROM agent_assignments ca
    WHERE $1 IS NULL
       OR EXISTS (SELECT 1 FROM tenant_users tu
                  WHERE tu.user_id = ca.user_id AND tu.tenant_id = $1)
    GROUP BY ca.agent_id
";

/// Every system agent, newest first, whatever tenant holds the row.
///
/// The documented catalogue-wide exception to tenant scoping: system agents
/// are pinned to the seed tenant but exposed to every tenant through the
/// catalogue (`AGENT_REACHABLE_SQL`), so a brand-new tenant, which holds no
/// agent rows of its own, still has a catalogue to pick a default from.
pub(crate) const LIST_CATALOGUE_SYSTEM_AGENTS_SQL: &str = concat!(
    "SELECT ",
    agent_columns!(""),
    " FROM agents WHERE is_system = TRUE ORDER BY created_at DESC"
);

/// One system agent of the tenant.
pub(crate) const GET_SYSTEM_AGENT_SQL: &str = concat!(
    "SELECT ",
    agent_columns!(""),
    " FROM agents WHERE id = $1 AND tenant_id = $2 AND is_system = TRUE"
);

/// Update a system agent's content.
pub(crate) const UPDATE_SYSTEM_AGENT_SQL: &str = r"
    UPDATE agents SET title = $1, description = $2, system_prompt = $3,
        category = $4, tags = $5, sample_prompts = $6, token_count = $7, updated_at = $8
    WHERE id = $9 AND tenant_id = $10 AND is_system = TRUE
";

/// Delete a system agent.
pub(crate) const DELETE_SYSTEM_AGENT_SQL: &str =
    "DELETE FROM agents WHERE id = $1 AND tenant_id = $2 AND is_system = TRUE";
