// ABOUTME: The fork-on-edit statements: find the user's copy, copy the origin, carry its translations and membership
// ABOUTME: Run by impl_agent_lifecycle's fork body; a child of agents so agent_columns! is in scope for the copy read
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// The user's own copy of an origin in the tenant, oldest first: the copy a
/// fork returns instead of forking twice.
pub(crate) const OWNED_COPY_OF_SQL: &str = concat!(
    "SELECT ",
    agent_columns!(""),
    " FROM agents WHERE user_id = $1 AND tenant_id = $2 AND forked_from = $3
    ORDER BY created_at ASC LIMIT 1"
);

/// Fork an agent the caller can reach into a private agent they own.
///
/// Every content and runtime column is copied verbatim from the origin, so
/// the copy runs exactly as the origin did the moment it was forked; `source`
/// becomes `custom` because the athlete now owns the text (the live prompt
/// registry overlays `contremaitre` agents only). `$1` is the copy id, `$2`
/// the owner, `$3` their tenant, `$4` the call time (both timestamps), `$5`
/// the visibility and `$6` the origin, which must belong to the tenant or be
/// a system agent. `forked_from` points at the origin and `slug` keeps its
/// handle, which a copy carries as a reference.
pub(crate) const FORK_AGENT_SQL: &str = r"
    INSERT INTO agents (
        id, user_id, tenant_id, title, description, system_prompt, category, tags,
        sample_prompts, token_count, created_at, updated_at, is_system, visibility,
        prerequisites, forked_from, slug, purpose, when_to_use, instructions,
        example_inputs, example_outputs, success_criteria, startup_query,
        data_requirements, visuals, max_tool_iterations, temperature, source
    )
    SELECT $1, $2, $3, title, description, system_prompt, category, tags,
        sample_prompts, token_count, $4, $4, FALSE, $5,
        prerequisites, id, slug, purpose, when_to_use, instructions,
        example_inputs, example_outputs, success_criteria, startup_query,
        data_requirements, visuals, max_tool_iterations, temperature, 'custom'
    FROM agents WHERE id = $6 AND (tenant_id = $3 OR is_system = TRUE)
";

/// Carry the origin's translation overlays onto a fork: `$1` the copy, `$2`
/// the origin.
pub(crate) const FORK_TRANSLATIONS_SQL: &str = r"
    INSERT INTO agent_translations (
        agent_id, locale, title, description, purpose, instructions, source_sha,
        tags, sample_prompts, created_at, updated_at
    )
    SELECT $1, locale, title, description, purpose, instructions, source_sha,
        tags, sample_prompts, created_at, updated_at
    FROM agent_translations WHERE agent_id = $2
";

/// Count a fork as an install of its published origin `$2` when the user
/// (`$3`) was not already a member of it: the install count is members plus
/// live copies, and uninstalling the copy later takes one away. A member's
/// fork moves their install rather than adding one. Runs before
/// `MOVE_MEMBERSHIP_SQL`; `$1` is the call time.
pub(crate) const COUNT_FORK_AS_INSTALL_SQL: &str = r"
    UPDATE store_listings SET install_count = install_count + 1, updated_at = $1
    WHERE agent_id = $2 AND publish_status = 'published'
    AND NOT EXISTS (
        SELECT 1 FROM agent_assignments
        WHERE agent_id = $2 AND user_id = $3 AND is_member = TRUE)
";

/// Move the user's row (counters and favourite with it) from the origin `$2`
/// onto their fork `$1`, as a membership.
pub(crate) const MOVE_MEMBERSHIP_SQL: &str = r"
    UPDATE agent_assignments SET agent_id = $1, is_member = TRUE
    WHERE agent_id = $2 AND user_id = $3
";

/// Point the user's selection in the tenant at their fork `$1` when it named
/// the origin `$4`.
pub(crate) const RESELECT_FORK_SQL: &str = r"
    UPDATE tenant_users SET selected_agent_id = $1
    WHERE user_id = $2 AND tenant_id = $3 AND selected_agent_id = $4
";
