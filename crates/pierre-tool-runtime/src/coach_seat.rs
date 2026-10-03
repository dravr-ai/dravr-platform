// ABOUTME: Which tools a group's coach may call in their group — the one predicate every tool surface shares
// ABOUTME: The coach's own data is never read there: data tools are withheld unless they reach the group's athletes

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The coach's seat, as the tool layer sees it.
//!
//! In a group the sender coaches, the subject is the group's athletes, never
//! the coach (carnet#741). Prompt assembly leaves the coach's own context out;
//! this module covers the tools the model calls itself (carnet#742). Every
//! chat-callable tool runs as the caller, so on the seat a `get_activities`
//! with no athlete named would read the coach's own rides.
//!
//! The rule is a property of the tool, not a list of names: a tool that reads
//! or writes data is withheld unless its category reaches the group's
//! athletes or the coach's catalogue. A self-data tool added later is withheld
//! without anyone remembering to list it.
//!
//! Advertisement and execution ask the same question, the way
//! [`crate::implementations::guided_flow`] does for guided flows: the declared
//! tool list leaves these tools out, and the executor refuses them if the
//! model calls one anyway.

use dravr_tronc::mcp::tool::ToolCapabilities;
use serde_json::json;

use crate::protocol::types::UniversalResponse;
use crate::registry::ToolRegistry;

/// Whose data a turn's tools run against.
///
/// An enum rather than a flag because each value names a seat: the subject is
/// reading their own data, the coach is reading the group's athletes'.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TurnSeat {
    /// The caller is the turn's subject: every tool runs on their own data.
    #[default]
    Subject,
    /// The caller coaches the turn's group: their own data tools are withheld.
    Coach,
}

/// Categories whose data tools stay callable on the coach's seat: `groups`
/// reaches a named athlete through their own consent gate, and `store` is the
/// coach's agent catalogue, not anyone's training.
const COACH_SEAT_CATEGORIES: &[&str] = &["groups", "store"];

/// `error_code` of the refusal a withheld tool returns.
pub const COACH_SEAT_ERROR_CODE: &str = "coach_seat_withheld";

/// Whether `tool_name` is withheld while the sender holds the coach's seat.
///
/// An unknown tool is not withheld here: the executor already refuses a name
/// the registry does not resolve.
#[must_use]
pub fn is_withheld_on_coach_seat(registry: &ToolRegistry, tool_name: &str) -> bool {
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

/// What the model reads when it calls a withheld tool anyway.
#[must_use]
fn coach_seat_message(tool_name: &str) -> String {
    format!(
        "'{tool_name}' is not available here: you are talking to this group's coach, and \
         their own data is never read in the group. For an athlete in the group, call \
         get_group_member_activities with that athlete's name. If the coach asks about their \
         own training, tell them to ask in their own conversation with Dravr, or in a group \
         where they train as an athlete."
    )
}

/// The in-band refusal for a withheld tool: an ordinary refusal the model
/// adapts to, not a security event.
#[must_use]
pub(crate) fn coach_seat_response(tool_name: &str) -> UniversalResponse {
    UniversalResponse {
        success: false,
        result: Some(json!({
            "error_code": COACH_SEAT_ERROR_CODE,
            "reason": "coach_seat",
        })),
        error: Some(coach_seat_message(tool_name)),
        metadata: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refusal_points_at_the_athlete_tool_and_the_coachs_own_thread() {
        let message = coach_seat_message("get_activities");
        assert!(message.contains("get_group_member_activities"));
        assert!(message.contains("their own conversation with Dravr"));
        let response = coach_seat_response("get_activities");
        assert!(!response.success);
        assert_eq!(
            response.result.unwrap()["error_code"],
            COACH_SEAT_ERROR_CODE
        );
    }
}
