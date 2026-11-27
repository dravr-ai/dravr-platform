// ABOUTME: The insert of a user's agent, plain and if-absent under a chosen id
// ABOUTME: Run by impl_agents_repository's create bodies; one statement text shared by both
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

/// The insert of a user's agent, shared by [`INSERT_AGENT_SQL`] and
/// [`INSERT_AGENT_IF_ABSENT_SQL`].
macro_rules! insert_agent_sql {
    () => {
        r"
    INSERT INTO agents (
        id, user_id, tenant_id, title, description, system_prompt,
        category, tags, sample_prompts, token_count,
        created_at, updated_at, is_system, visibility, prerequisites,
        forked_from, max_tool_iterations, temperature, startup_query, data_requirements,
        purpose, when_to_use, instructions, example_inputs, example_outputs, success_criteria,
        content_hash
    ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $11, FALSE, $12, NULL, NULL, $13, NULL,
              $14, $15, $16, $17, $18, $19, $20, $21, $22)
"
    };
}

/// Create a user's agent.
pub(crate) const INSERT_AGENT_SQL: &str = insert_agent_sql!();

/// Create a user's agent under an id the caller chose, unless a row holds
/// that id already: of two writers racing on one id, one inserts and the
/// other writes nothing.
pub(crate) const INSERT_AGENT_IF_ABSENT_SQL: &str =
    concat!(insert_agent_sql!(), " ON CONFLICT (id) DO NOTHING");
