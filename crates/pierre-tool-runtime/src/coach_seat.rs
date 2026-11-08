// ABOUTME: Which tools a coach may call where they coach — their group, or their thread about one athlete
// ABOUTME: One predicate every tool surface shares: the coach's own data tools in a group, their own reads and writes in a thread

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
//! - **A coach's thread about one athlete** (carnet#1070, carnet#1387): every
//!   turn there is about that athlete. A `remember_fact` would file "her FTP
//!   is 250" as the coach's own FTP, and a note, follow-up, commitment, goal
//!   or physiology write would land in the coach's dossier and reach every
//!   other conversation that reads it; a `recall_user_memory` would answer a
//!   question about her with the coach's own memory. A tool that writes data
//!   is withheld unless it takes the `athlete` argument, which there defaults
//!   to the thread's athlete. A tool that reads data is withheld unless it
//!   takes that argument or declares a data class
//!   ([`DataClass`](crate::read_subject::DataClass)): a training read reads
//!   the thread's athlete, and a wellness read too under their wellness grant
//!   ([`crate::read_subject`]); a tool that reads no person's data answers the
//!   same whoever asks. The `groups` tools take
//!   neither — in a thread their room resolves as private, and each would
//!   scan every group the coach coaches — so they stay in the room only.
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
use crate::read_subject::{self, ATHLETE_ARGUMENT, WELLNESS_GRANT_WHERE};
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
    /// a tool that reads or writes data acts on that athlete or is withheld,
    /// so the coach's own data never answers a question about them and
    /// nothing said about them is filed on the coach's own account.
    AthleteThread,
}

impl TurnSeat {
    /// Whether the caller is the turn's subject — the one seat on which their
    /// own data is what the turn is about.
    ///
    /// Off it, the caller is a coach (of the room, or in their thread about one
    /// athlete): none of their activities is read for the model on the
    /// platform's initiative, and what the turn says is not filed as theirs —
    /// save the answers to a guided walk they run on themselves in their room,
    /// which the chat pipeline's turn standing lets through.
    #[must_use]
    pub const fn sender_is_subject(self) -> bool {
        matches!(self, Self::Subject)
    }

    /// Whether `tool_name` is withheld on this seat.
    ///
    /// An unknown tool is not withheld here: the executor already refuses a
    /// name the registry does not resolve.
    #[must_use]
    pub fn withholds(self, registry: &ToolRegistry, tool_name: &str) -> bool {
        match self {
            Self::Subject => false,
            Self::Coach => is_withheld_on_coach_seat(registry, tool_name),
            Self::AthleteThread => {
                writes_callers_own_data(registry, tool_name)
                    || reads_callers_own_data(registry, tool_name)
            }
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
            Self::AthleteThread if writes_callers_own_data(registry, tool_name) => (
                ATHLETE_THREAD_ERROR_CODE,
                "athlete_thread",
                athlete_thread_message(tool_name),
            ),
            Self::AthleteThread => (
                ATHLETE_THREAD_ERROR_CODE,
                "athlete_thread_read",
                athlete_thread_read_message(tool_name),
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

/// `error_code` of the refusal a tool withheld on the coach's seat returns.
pub const COACH_SEAT_ERROR_CODE: &str = "coach_seat_withheld";

/// `error_code` of the refusal a tool withheld in a coach's thread about one
/// athlete returns.
pub const ATHLETE_THREAD_ERROR_CODE: &str = "athlete_thread_withheld";

/// Whether `tool_name` is withheld while the sender holds the coach's seat.
///
/// The plan tools are among them: an athlete's plan is never posted in the
/// group. `get_group_member_plan_status`, a `groups` tool, stays callable and
/// says whether the athlete has a published plan and where their coach reads
/// it (carnet#1244).
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

/// Whether `tool_name` reads the caller's own data: it reads data, declares
/// no data class, and takes no [`ATHLETE_ARGUMENT`] to read anyone else.
fn reads_callers_own_data(registry: &ToolRegistry, tool_name: &str) -> bool {
    let Some(tool) = registry.get(tool_name) else {
        return false;
    };
    tool.capabilities().contains(ToolCapabilities::READS_DATA)
        && registry.data_class(tool_name).is_none()
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
         get_group_member_activities with that athlete's name for their sessions, or \
         get_group_member_plan_status for whether they have a published plan and where their \
         coach reads it — the plan itself is never posted in the group. If the coach asks \
         about their own training, tell them to ask in their own conversation with Dravr, or \
         in a group where they train as an athlete."
    )
}

/// What the model reads when it calls a read withheld in a coach's thread
/// about one athlete.
fn athlete_thread_read_message(tool_name: &str) -> String {
    let wellness = read_subject::wellness_data();
    format!(
        "'{tool_name}' is not available here: in this thread about one athlete it would read the \
         coach's own data, never the athlete's. The athlete's training reads here — activities, \
         the daily load history, trends, goals, training physiology and the analyses of them — \
         act on the athlete with `athlete` left out. Their wellness ({wellness}) reads here \
         only once they share it with the coach, which only they can do, \
         {WELLNESS_GRANT_WHERE}. Their memory is never readable here; \
         say so. If the coach wants their own data, tell them to ask in their own conversation \
         with Dravr."
    )
}

/// What the model reads when it calls a write withheld in a coach's thread
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
    fn the_coach_seat_refusal_points_a_plan_request_at_the_plan_status() {
        let message = coach_seat_message("get_training_plan");
        assert!(
            message.contains("get_group_member_plan_status"),
            "the refusal names the plan-status tool a coach seat keeps: {message}"
        );
        assert!(
            message.contains("the plan itself is never posted in the group"),
            "the refusal says the plan stays out of the room: {message}"
        );
    }

    #[test]
    fn the_athlete_thread_refusal_says_nothing_is_saved_to_the_coach() {
        let message = athlete_thread_message("remember_fact");
        assert!(message.starts_with("'remember_fact' is not available here"));
        assert!(message.contains("nothing said in it is saved to the coach's own account"));
        assert!(message.contains("their own conversation with Dravr"));
    }

    #[test]
    fn the_athlete_thread_read_refusal_says_whose_data_it_would_read() {
        let message = athlete_thread_read_message("recall_user_memory");
        assert!(
            message.starts_with("'recall_user_memory' is not available here"),
            "bug: the refusal names the tool it withholds: {message}"
        );
        assert!(
            message.contains("the coach's own data, never the athlete's"),
            "bug: the refusal says whose data the read would return: {message}"
        );
        assert!(
            message.contains("Their memory is never readable here; say so"),
            "bug: the refusal tells the model to say so rather than estimate: {message}"
        );
        assert!(
            message.contains(
                "Their wellness (sleep, HRV, resting heart rate, weight, readiness and daily \
                 notes) reads here only once they share it with the coach"
            ) && message.contains("Group info → What your human coach can do"),
            "bug: the refusal says the wellness takes the athlete's grant, and where: {message}"
        );
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

    #[cfg(feature = "tools-groups")]
    mod with_the_plan_status {
        use std::sync::Arc;

        use super::super::*;
        use crate::implementations::group_member_plan::GetGroupMemberPlanStatusTool;
        use crate::implementations::training_plans::create_training_plan_tools;

        #[test]
        fn the_coach_seat_keeps_the_plan_status_and_withholds_the_plan() {
            let mut registry = ToolRegistry::new();
            registry.register_with_category(Arc::new(GetGroupMemberPlanStatusTool), "groups");
            for tool in create_training_plan_tools() {
                registry.register_with_category(Arc::from(tool), "memory");
            }
            assert!(
                !TurnSeat::Coach.withholds(&registry, "get_group_member_plan_status"),
                "a groups tool stays callable on the coach seat"
            );
            assert!(
                TurnSeat::Coach.withholds(&registry, "get_training_plan"),
                "the plan itself stays withheld on the coach seat"
            );
        }
    }

    #[cfg(feature = "tools-memory")]
    mod with_registered_tools {
        use std::sync::Arc;

        use super::super::*;
        use crate::implementations::memory::create_memory_tools;
        use crate::implementations::training_plans::create_training_plan_tools;
        use crate::read_subject::DataClass;

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
            let own_write = "remember_fact";
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

        #[test]
        fn the_athlete_thread_keeps_the_plan_tools_and_withholds_the_coachs_own_reads() {
            let registry = registry();
            // The plan tools take `athlete`, which defaults to the thread's
            // athlete.
            for kept in ["save_training_plan", "get_training_plan"] {
                assert!(
                    !TurnSeat::AthleteThread.withholds(&registry, kept),
                    "{kept}"
                );
                assert!(TurnSeat::AthleteThread.refusal(&registry, kept).is_none());
            }
            // Recall reads only the coach's own memory: it would answer a
            // question about the athlete with the coach's facts.
            let refusal = TurnSeat::AthleteThread
                .refusal(&registry, "recall_user_memory")
                .expect("the coach's own read is withheld in the thread");
            assert_eq!(
                refusal.result.unwrap()["reason"],
                "athlete_thread_read",
                "a read is refused with the read's own wording"
            );
        }

        #[test]
        fn a_declared_data_class_keeps_a_read_in_the_athlete_thread() {
            let mut registry = registry();
            assert!(
                registry.declare_data_class("recall_user_memory", DataClass::NoSubject),
                "the tool is registered"
            );
            assert!(
                !TurnSeat::AthleteThread.withholds(&registry, "recall_user_memory"),
                "a read that declares its class answers the same whoever asks"
            );
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
