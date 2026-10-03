// ABOUTME: Which agents may answer a group's athletes — never one written for a coach
// ABOUTME: One check shared by every path that sets a group's agent: REST create/update, /group agent, /agent

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::agents::Agent;

/// Refuse `agent` as a group's agent when it is coach-facing
/// ([`Agent::is_coach_facing`]).
///
/// A group's athletes talk to its agent, and a coach-facing agent — the
/// roster agent a coach who does not train runs their practice with, or a
/// coach's plan builder — is written for the coach. Every path that sets a
/// group's agent asks this, whoever is asking.
///
/// # Errors
///
/// Returns [`AppError::invalid_input`] for a coach-facing agent.
pub fn require_athlete_facing(agent: &Agent) -> AppResult<()> {
    if agent.is_coach_facing() {
        return Err(AppError::invalid_input(
            "A coach-facing agent cannot answer a group's athletes",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn agent(tags: &[&str]) -> Agent {
        serde_json::from_value(json!({
            "id": "00000000-0000-0000-0000-00000000000a",
            "user_id": "00000000-0000-0000-0000-000000000001",
            "tenant_id": "00000000-0000-0000-0000-000000000002",
            "title": "Agent",
            "description": null,
            "system_prompt": "Test.",
            "category": "training",
            "tags": tags,
            "token_count": 0,
            "created_at": "2026-10-03T00:00:00Z",
            "updated_at": "2026-10-03T00:00:00Z",
        }))
        .unwrap()
    }

    #[test]
    fn a_coach_facing_agent_never_answers_a_group() {
        let err = require_athlete_facing(&agent(&[Agent::COACH_TOOL_TAG])).unwrap_err();
        assert!(err.to_string().contains("coach-facing"), "{err}");
    }

    #[test]
    fn an_athlete_facing_agent_may() {
        assert!(require_athlete_facing(&agent(&["endurance"])).is_ok());
    }
}
