// ABOUTME: Which tools a coach may call where they coach — their group, or their thread about one athlete
// ABOUTME: One predicate every tool surface shares: the coach's own data tools in a group, their own-account writes in a thread

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The coach's seats, as the tool layer sees them.
//!
//! Every chat-callable tool runs as the caller. Two turns are not about the
//! caller, though, and there a tool the model calls on the caller's own
//! account reads or files the wrong person's data:
//!
//! - **A group the sender coaches** (carnet#741, carnet#742): the subject is
//!   the group's athletes, so a `get_activities` with no athlete named would
//!   read the coach's own rides. A tool that reads or writes data is withheld
//!   unless its category reaches the group's athletes or the coach's
//!   catalogue.
//! - **A coach's thread about one athlete** (carnet#1070): every turn there is
//!   about that athlete, so a `remember_fact` would file "her FTP is 250" as
//!   the coach's own FTP, and a note, follow-up, commitment, goal or
//!   physiology write would land in the coach's dossier and reach every other
//!   conversation that reads it. A tool that writes data is withheld unless it
//!   takes the `athlete` argument, which there defaults to the thread's
//!   athlete. Reads of the coach's own data stay callable, and the thread's
//!   prompt says whose they are.
//!
//! Each rule is a property of the tool, not a list of names: a self-data tool
//! added later is withheld without anyone remembering to list it.
//!
//! Advertisement and execution ask the same question, the way
//! [`crate::implementations::guided_flow`] does for guided flows: the declared
//! tool list leaves these tools out, and the executor refuses them if the
//! model calls one anyway.

use dravr_tronc::mcp::tool::ToolCapabilities;
use serde_json::{json, Value};

use crate::protocol::types::UniversalResponse;
use crate::registry::ToolRegistry;

/// Whose data a turn's tools run against.
///
/// An enum rather than a flag because each value names a seat: the subject is
/// reading their own data, the coach is reading the group's athletes', or
/// working on one athlete's plan in their thread about them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TurnSeat {
    /// The caller is the turn's subject: every tool runs on their own data.
    #[default]
    Subject,
    /// The caller coaches the turn's group: their own data tools are withheld.
    Coach,
    /// The caller is a group's coach in their thread about one athlete of it:
    /// a tool that writes data acts on that athlete or is withheld, so nothing
    /// said about them is filed on the coach's own account.
    AthleteThread,
}

impl TurnSeat {
    /// Whether `tool_name` is withheld on this seat.
    ///
    /// An unknown tool is not withheld here: the executor already refuses a
    /// name the registry does not resolve.
    #[must_use]
    pub fn withholds(self, registry: &ToolRegistry, tool_name: &str) -> bool {
        match self {
            Self::Subject => false,
            Self::Coach => is_withheld_on_coach_seat(registry, tool_name),
            Self::AthleteThread => writes_callers_own_data(registry, tool_name),
        }
    }

    /// The in-band refusal for `tool_name` when this seat withholds it: an
    /// ordinary refusal the model adapts to, not a security event.
    #[must_use]
    pub(crate) fn refusal(
        self,
        registry: &ToolRegistry,
        tool_name: &str,
    ) -> Option<UniversalResponse> {
        if !self.withholds(registry, tool_name) {
            return None;
        }
        let (error_code, reason, message) = match self {
            Self::Subject => return None,
            Self::Coach => (
                COACH_SEAT_ERROR_CODE,
                "coach_seat",
                coach_seat_message(tool_name),
            ),
            Self::AthleteThread => (
                ATHLETE_THREAD_ERROR_CODE,
                "athlete_thread",
                athlete_thread_message(tool_name),
            ),
        };
        Some(UniversalResponse {
            success: false,
            result: Some(json!({
                "error_code": error_code,
                "reason": reason,
            })),
            error: Some(message),
            metadata: None,
        })
    }
}

/// Categories whose data tools stay callable on the coach's seat: `groups`
/// reaches a named athlete through their own consent gate, and `store` is the
/// coach's agent catalogue, not anyone's training.
const COACH_SEAT_CATEGORIES: &[&str] = &["groups", "store"];

/// The argument a tool takes when it can act on an athlete other than the
/// caller. A coach names a coached athlete with it, and in their thread about
/// one athlete it defaults to that athlete (the plan and calendar scopes).
const ATHLETE_ARGUMENT: &str = "athlete";

/// `error_code` of the refusal a tool withheld on the coach's seat returns.
pub const COACH_SEAT_ERROR_CODE: &str = "coach_seat_withheld";

/// `error_code` of the refusal a tool withheld in a coach's thread about one
/// athlete returns.
pub const ATHLETE_THREAD_ERROR_CODE: &str = "athlete_thread_withheld";

/// Whether `tool_name` is withheld while the sender holds the coach's seat.
fn is_withheld_on_coach_seat(registry: &ToolRegistry, tool_name: &str) -> bool {
    let Some(tool) = registry.get(tool_name) else {
        return false;
    };
    let touches_data = tool
        .capabilities()
        .intersects(ToolCapabilities::READS_DATA | ToolCapabilities::WRITES_DATA);
    touches_data
        && !registry
            .category_for_tool(tool_name)
            .is_some_and(|category| COACH_SEAT_CATEGORIES.contains(&category))
}

/// Whether `tool_name` writes on the caller's own account: it writes data and
/// takes no [`ATHLETE_ARGUMENT`] to act on anyone else.
fn writes_callers_own_data(registry: &ToolRegistry, tool_name: &str) -> bool {
    let Some(tool) = registry.get(tool_name) else {
        return false;
    };
    tool.capabilities().contains(ToolCapabilities::WRITES_DATA)
        && !takes_athlete_argument(&tool.definition().input_schema)
}

fn takes_athlete_argument(input_schema: &Value) -> bool {
    input_schema
        .get("properties")
        .and_then(|properties| properties.get(ATHLETE_ARGUMENT))
        .is_some()
}

/// What the model reads when it calls a tool withheld on the coach's seat.
fn coach_seat_message(tool_name: &str) -> String {
    format!(
        "'{tool_name}' is not available here: you are talking to this group's coach, and \
         their own data is never read in the group. For an athlete in the group, call \
         get_group_member_activities with that athlete's name. If the coach asks about their \
         own training, tell them to ask in their own conversation with Dravr, or in a group \
         where they train as an athlete."
    )
}

/// What the model reads when it calls a tool withheld in a coach's thread
/// about one athlete.
fn athlete_thread_message(tool_name: &str) -> String {
    format!(
        "'{tool_name}' is not available here: this thread is the coach's workspace for one \
         athlete's plan, and nothing said in it is saved to the coach's own account — not as \
         a fact, note, follow-up, commitment, goal or setting. The athlete's plan and calendar \
         tools act on the athlete. If the coach wants to change their own data, tell them to \
         do it in their own conversation with Dravr."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_coach_seat_refusal_points_at_the_athlete_tool_and_the_coachs_own_thread() {
        let message = coach_seat_message("get_activities");
        assert!(message.contains("get_group_member_activities"));
        assert!(message.contains("their own conversation with Dravr"));
    }

    #[test]
    fn the_athlete_thread_refusal_says_nothing_is_saved_to_the_coach() {
        let message = athlete_thread_message("remember_fact");
        assert!(message.starts_with("'remember_fact' is not available here"));
        assert!(message.contains("nothing said in it is saved to the coach's own account"));
        assert!(message.contains("their own conversation with Dravr"));
    }

    #[test]
    fn only_a_property_named_athlete_counts_as_acting_on_someone_else() {
        assert!(takes_athlete_argument(&json!({
            "type": "object",
            "properties": { "athlete": { "type": "string" }, "weeks": { "type": "array" } },
        })));
        assert!(!takes_athlete_argument(&json!({
            "type": "object",
            "properties": { "athlete_notes": { "type": "string" } },
        })));
        assert!(!takes_athlete_argument(&json!({ "type": "object" })));
    }

    #[cfg(feature = "tools-memory")]
    mod with_registered_tools {
        use std::sync::Arc;

        use super::super::*;
        use crate::implementations::memory::create_memory_tools;
        use crate::implementations::training_plans::create_training_plan_tools;

        fn registry() -> ToolRegistry {
            let mut registry = ToolRegistry::new();
            for tool in create_memory_tools()
                .into_iter()
                .chain(create_training_plan_tools())
            {
                registry.register_with_category(Arc::from(tool), "memory");
            }
            registry
        }

        #[test]
        fn the_athlete_thread_withholds_every_write_on_the_coachs_own_account() {
            let registry = registry();
            for own_write in ["remember_fact", "agent_note_add", "agent_followup_schedule"] {
                assert!(
                    TurnSeat::AthleteThread.withholds(&registry, own_write),
                    "{own_write} files on the caller's own account"
                );
                let refusal = TurnSeat::AthleteThread
                    .refusal(&registry, own_write)
                    .expect("a withheld tool is refused");
                assert!(!refusal.success);
                assert_eq!(
                    refusal.result.unwrap()["error_code"],
                    ATHLETE_THREAD_ERROR_CODE
                );
            }
        }

        #[test]
        fn the_athlete_thread_keeps_the_plan_tools_and_the_coachs_own_reads() {
            let registry = registry();
            // The plan tools take `athlete`, which defaults to the thread's
            // athlete; recall only reads the coach's own memory.
            for kept in [
                "save_training_plan",
                "get_training_plan",
                "recall_user_memory",
            ] {
                assert!(
                    !TurnSeat::AthleteThread.withholds(&registry, kept),
                    "{kept}"
                );
                assert!(TurnSeat::AthleteThread.refusal(&registry, kept).is_none());
            }
        }

        #[test]
        fn the_subject_seat_withholds_nothing_and_the_coach_seat_every_data_tool() {
            let registry = registry();
            for name in ["remember_fact", "save_training_plan", "recall_user_memory"] {
                assert!(!TurnSeat::Subject.withholds(&registry, name), "{name}");
                assert!(TurnSeat::Subject.refusal(&registry, name).is_none());
                assert!(TurnSeat::Coach.withholds(&registry, name), "{name}");
                assert_eq!(
                    TurnSeat::Coach
                        .refusal(&registry, name)
                        .unwrap()
                        .result
                        .unwrap()["error_code"],
                    COACH_SEAT_ERROR_CODE
                );
            }
        }
    }
}
