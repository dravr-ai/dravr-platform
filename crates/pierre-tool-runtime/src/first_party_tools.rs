// ABOUTME: Tools that only Dravr's own surfaces list and run: left out of an external caller's listing, refused if called
// ABOUTME: One predicate over the tool's declared TransportPolicy, shared by tools/list, the A2A card and the dispatch chokepoint

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tools kept on Dravr's own surfaces.
//!
//! Some tools only make sense inside Dravr's own apps and channels, and are not
//! offered to an MCP client or an A2A agent at all:
//!
//! - the coach family reads other athletes' data for the coach who leads their
//!   group, under consents those athletes gave inside Dravr;
//! - agent store and agent CRUD manage the personas of Dravr's own chat;
//! - memory and self-steering (`remember_fact`, `agent_note_add`,
//!   `agent_followup_schedule`, the commitments, `forget_playbook`) write what
//!   Dravr's own chat later reads as the athlete's record, which an assistant's
//!   text does not get to become.
//!
//! A tool declares this as a property of itself
//! ([`RuntimeTool::transport_policy`](crate::security::RuntimeTool::transport_policy)),
//! with the same [`TransportPolicy`] a provider declares for its data, and the
//! same [`Transport::is_first_party`] decides. There is no list of names: a
//! tool added later is kept off external transports by declaring it.
//!
//! Listing and execution ask the same question, the way
//! [`crate::coach_seat`] does for the coach's seat: an external listing leaves
//! these tools out ([`retain_served_over`]), a chat turn's catalogue leaves
//! them out by the turn's transport
//! ([`offered_schemas`](crate::chat_catalogue::offered_schemas)), and the
//! executor refuses one if an external caller names it anyway ([`refusal`]).

use pierre_core::transport::{Transport, TransportPolicy};
use pierre_mcp_schema::ToolSchema;
use serde_json::json;

use crate::protocol::types::UniversalResponse;
use crate::registry::ToolRegistry;

/// `error_code` of the refusal a first-party-only tool returns to an external
/// caller.
pub const FIRST_PARTY_SURFACE_ERROR_CODE: &str = "first_party_surface";

/// `reason` of that refusal: the same reason the coach's calendar write gives
/// over an external transport.
const EXTERNAL_TRANSPORT_REASON: &str = "external_transport";

/// Whether `tool_name` is withheld from a call served over `transport`.
///
/// `None` is external, as everywhere a transport is read: an entry point that
/// forgets to declare one withholds the tool instead of serving it. An unknown
/// tool is not withheld here: the executor already refuses a name the registry
/// does not resolve.
#[must_use]
pub fn withholds(registry: &ToolRegistry, tool_name: &str, transport: Option<Transport>) -> bool {
    registry
        .transport_policy(tool_name)
        .is_some_and(TransportPolicy::is_first_party_only)
        && !transport.is_some_and(Transport::is_first_party)
}

/// Drop from `schemas` every tool withheld over `transport`.
///
/// For a listing an entry point serves: `tools/list` on `/mcp`, the A2A
/// extended card.
pub fn retain_served_over(
    registry: &ToolRegistry,
    schemas: &mut Vec<ToolSchema>,
    transport: Transport,
) {
    schemas.retain(|schema| !withholds(registry, &schema.name, Some(transport)));
}

/// The in-band refusal for `tool_name` when `transport` withholds it.
///
/// In band, like the coach seat's: the caller adapts to "not over this
/// interface" the way it adapts to a tenant-disabled tool. Nothing the tool
/// would have read is looked up before it.
#[must_use]
pub fn refusal(
    registry: &ToolRegistry,
    tool_name: &str,
    transport: Option<Transport>,
) -> Option<UniversalResponse> {
    withholds(registry, tool_name, transport).then(|| UniversalResponse {
        success: false,
        result: Some(json!({
            "error_code": FIRST_PARTY_SURFACE_ERROR_CODE,
            "reason": EXTERNAL_TRANSPORT_REASON,
        })),
        error: Some(refusal_message(tool_name)),
        metadata: None,
    })
}

/// What the caller reads when it names a tool kept on Dravr's own surfaces.
fn refusal_message(tool_name: &str) -> String {
    format!("'{tool_name}' is not available over this interface: it runs in Dravr's own app or channels.")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every transport variant, first-party and external alike.
    const ALL: [Transport; 7] = [
        Transport::WebApp,
        Transport::MobileApp,
        Transport::Messaging,
        Transport::PlatformJob,
        Transport::McpHttp,
        Transport::A2a,
        Transport::ApiKey,
    ];

    #[test]
    fn the_refusal_names_the_tool_and_where_it_runs() {
        let message = refusal_message("get_roster_overview");
        assert!(
            message.starts_with("'get_roster_overview' is not available over this interface"),
            "bug: the refusal must name the tool first: {message}"
        );
        assert!(
            message.contains("Dravr's own app or channels"),
            "bug: the refusal must say where the tool runs: {message}"
        );
    }

    #[test]
    fn an_unknown_tool_is_never_withheld_here() {
        let registry = ToolRegistry::new();
        for transport in ALL {
            assert!(
                !withholds(&registry, "no_such_tool", Some(transport)),
                "bug: an unknown tool is the executor's refusal, not this one ({transport:?})"
            );
        }
        assert!(
            !withholds(&registry, "no_such_tool", None),
            "bug: an unknown tool is the executor's refusal, not this one (undeclared transport)"
        );
    }

    #[cfg(all(feature = "tools-groups", feature = "tools-memory"))]
    mod with_registered_tools {
        use std::sync::Arc;

        use super::super::*;
        use super::ALL;
        use crate::implementations::groups::create_group_tools;
        use crate::implementations::memory::create_memory_tools;

        fn registry() -> ToolRegistry {
            let mut registry = ToolRegistry::new();
            for tool in create_group_tools() {
                registry.register_with_category(Arc::from(tool), "groups");
            }
            for tool in create_memory_tools() {
                registry.register_with_category(Arc::from(tool), "memory");
            }
            registry
        }

        fn group_tool_names(registry: &ToolRegistry) -> Vec<&str> {
            registry.tools_in_category("groups")
        }

        #[test]
        fn every_coach_tool_runs_on_dravrs_own_surfaces_only() {
            let registry = registry();
            let coach_tools = group_tool_names(&registry);
            assert_eq!(coach_tools.len(), 5, "the coach family: {coach_tools:?}");
            for name in coach_tools {
                for transport in ALL {
                    assert_eq!(
                        withholds(&registry, name, Some(transport)),
                        !transport.is_first_party(),
                        "{name} over {transport:?}"
                    );
                }
                assert!(
                    withholds(&registry, name, None),
                    "an undeclared transport is external"
                );
            }
        }

        #[test]
        fn a_tool_on_every_transport_is_never_withheld() {
            let registry = registry();
            let name = "recall_user_memory";
            for transport in ALL {
                assert!(
                    !withholds(&registry, name, Some(transport)),
                    "bug: {name} is served on every transport, yet {transport:?} withholds it"
                );
            }
            assert!(
                !withholds(&registry, name, None),
                "bug: {name} is served on every transport, yet an undeclared one withholds it"
            );
        }

        #[test]
        fn an_external_listing_loses_the_withheld_tools_and_keeps_the_rest() {
            let registry = registry();
            let mut schemas = registry.all_schemas();
            let before = schemas.len();
            retain_served_over(&registry, &mut schemas, Transport::McpHttp);
            assert_eq!(
                before - schemas.len(),
                8,
                "bug: an external listing must drop the five coach tools and the three \
                 memory writes"
            );
            let kept: Vec<&str> = schemas.iter().map(|schema| schema.name.as_str()).collect();
            assert_eq!(
                kept,
                ["recall_user_memory"],
                "bug: an external listing must keep every tool served on every transport"
            );

            let mut in_app = registry.all_schemas();
            retain_served_over(&registry, &mut in_app, Transport::WebApp);
            assert_eq!(
                in_app.len(),
                before,
                "bug: an in-app listing must keep every tool"
            );
        }

        #[test]
        fn an_external_call_to_a_coach_tool_is_refused_in_band() {
            let registry = registry();
            let refused = refusal(
                &registry,
                "get_group_member_activities",
                Some(Transport::A2a),
            )
            .expect("an external call to a coach tool is refused");
            assert!(!refused.success, "bug: a refusal must not report success");
            let result = refused.result.expect("the refusal carries its code");
            assert_eq!(
                result["error_code"], FIRST_PARTY_SURFACE_ERROR_CODE,
                "bug: the refusal must carry its code: {result}"
            );
            assert_eq!(
                result["reason"], "external_transport",
                "bug: the refusal must carry its reason: {result}"
            );
            assert!(
                refusal(
                    &registry,
                    "get_group_member_activities",
                    Some(Transport::Messaging)
                )
                .is_none(),
                "bug: a linked messaging channel is one of Dravr's own surfaces"
            );
        }
    }
}
