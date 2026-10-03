// ABOUTME: Which system agent answers when nobody picked one — for a person's own thread, and for a group
// ABOUTME: Coach-facing agents never answer athletes; a coach who does not train gets the roster agent

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Default agents.
//!
//! Every surface that binds an agent without anyone choosing one — signup's
//! starter selection, the DM forge and per-turn rebind, a channel group's
//! bootstrap, `/group create` — answers from the tenant's system agents. Two
//! audiences read those answers:
//!
//! - **A person's own thread.** A coach who does not train gets the
//!   [`ROSTER_AGENT_HANDLE`] agent: their thread runs their practice. Everyone
//!   else gets the first athlete-facing agent.
//! - **A group.** The athletes talk to it, so it is never coach-facing
//!   ([`Agent::is_coach_facing`]), whoever created the group.
//!
//! The system list is ordered newest first, so "the first" agent is whatever
//! was seeded last; skipping coach-facing ones is what keeps a newly seeded
//! coach tool from becoming every athlete's default.

use pierre_core::models::agents::Agent;

/// Catalogue handle of the agent a coach who does not train holds for their
/// own thread (dravr-contremaitre `prompts/agents/coaching/roster-agent`).
///
/// LIMITATION(registre#748): `ROSTER_AGENT_HANDLE`'s agent cannot review the coach's athletes from
/// the coach's own thread; no tool reads a coach's roster outside each athlete's group thread.
pub const ROSTER_AGENT_HANDLE: &str = "roster-agent";

/// The first agent an athlete may be answered by.
#[must_use]
pub fn first_athlete_facing(system: &[Agent]) -> Option<&Agent> {
    system.iter().find(|agent| !agent.is_coach_facing())
}

/// The roster agent, when this tenant has it.
#[must_use]
pub fn roster_agent(system: &[Agent]) -> Option<&Agent> {
    system
        .iter()
        .find(|agent| agent.handle.as_deref() == Some(ROSTER_AGENT_HANDLE))
}

/// The agent a person's own thread falls back to when they selected none.
///
/// A coach who does not train gets the roster agent and never an athlete's:
/// with no roster agent seeded yet, the thread runs on the house prompt
/// rather than read them as an athlete.
#[must_use]
pub fn own_thread_default(system: &[Agent], coach_only: bool) -> Option<String> {
    let agent = if coach_only {
        roster_agent(system)
    } else {
        first_athlete_facing(system)
    };
    agent.map(|agent| agent.id.to_string())
}

/// The agent a group answers with: `preferred` unless it is coach-facing,
/// else the first athlete-facing system agent.
///
/// `preferred` is whatever the creating surface would have used — the
/// creator's thread agent or their own selection. A person's own copy of a
/// coach-facing agent (a Store install keeps its tags) is refused like the
/// system one.
#[must_use]
pub fn group_default(system: &[Agent], preferred: Option<&Agent>) -> Option<String> {
    preferred
        .filter(|agent| !agent.is_coach_facing())
        .or_else(|| first_athlete_facing(system))
        .map(|agent| agent.id.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn agent(id: &str, handle: &str, tags: &[&str]) -> Agent {
        serde_json::from_value(json!({
            "id": id,
            "user_id": "00000000-0000-0000-0000-000000000001",
            "tenant_id": "00000000-0000-0000-0000-000000000002",
            "title": handle,
            "description": null,
            "system_prompt": "Test.",
            "category": "training",
            "tags": tags,
            "token_count": 0,
            "created_at": "2026-10-03T00:00:00Z",
            "updated_at": "2026-10-03T00:00:00Z",
            "handle": handle,
        }))
        .unwrap()
    }

    const ROSTER: &str = "00000000-0000-0000-0000-00000000000a";
    const TAPER: &str = "00000000-0000-0000-0000-00000000000b";
    const ENDURANCE: &str = "00000000-0000-0000-0000-00000000000c";

    /// Newest first, as `list_system_agents` returns them: both coach tools
    /// were seeded after the athlete agent.
    fn catalogue() -> Vec<Agent> {
        vec![
            agent(ROSTER, ROSTER_AGENT_HANDLE, &["coach-tool"]),
            agent(TAPER, "taper-builder-agent", &["coach-tool"]),
            agent(ENDURANCE, "endurance-agent", &["endurance"]),
        ]
    }

    #[test]
    fn an_athlete_never_defaults_to_a_newer_coach_tool() {
        assert_eq!(
            own_thread_default(&catalogue(), false).as_deref(),
            Some(ENDURANCE)
        );
        assert_eq!(
            first_athlete_facing(&catalogue()).map(|a| a.id.to_string()),
            Some(ENDURANCE.to_owned())
        );
    }

    #[test]
    fn a_coach_who_does_not_train_defaults_to_the_roster_agent() {
        assert_eq!(
            own_thread_default(&catalogue(), true).as_deref(),
            Some(ROSTER)
        );
    }

    #[test]
    fn a_coach_who_does_not_train_is_never_read_as_an_athlete() {
        let without_roster: Vec<Agent> = catalogue()
            .into_iter()
            .filter(|a| a.id.to_string() != ROSTER)
            .collect();
        assert_eq!(own_thread_default(&without_roster, true), None);
    }

    #[test]
    fn a_group_never_answers_with_a_coach_facing_agent() {
        let system = catalogue();
        assert_eq!(
            group_default(&system, Some(&system[0])).as_deref(),
            Some(ENDURANCE)
        );
        let installed_copy = agent(
            "00000000-0000-0000-0000-0000000000fe",
            ROSTER_AGENT_HANDLE,
            &["coach-tool"],
        );
        assert_eq!(
            group_default(&system, Some(&installed_copy)).as_deref(),
            Some(ENDURANCE)
        );
        assert_eq!(
            group_default(&catalogue(), None).as_deref(),
            Some(ENDURANCE)
        );
    }

    #[test]
    fn a_group_keeps_an_athlete_facing_or_personal_choice() {
        let system = catalogue();
        assert_eq!(
            group_default(&system, Some(&system[2])).as_deref(),
            Some(ENDURANCE)
        );
        let personal = "00000000-0000-0000-0000-0000000000ff";
        let own = agent(personal, "my-agent", &["endurance"]);
        assert_eq!(
            group_default(&system, Some(&own)).as_deref(),
            Some(personal)
        );
    }
}
