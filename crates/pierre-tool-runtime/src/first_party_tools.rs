// ABOUTME: Tools that Dravr's own surfaces list and run, kept from external callers but an admin holding admin
// ABOUTME: The transport arm of tool_access: why each is kept there, and the in-band refusal an external caller reads

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Tools kept on Dravr's own surfaces.
//!
//! Some tools only make sense inside Dravr's own apps and channels, and are not
//! offered to an MCP client or an A2A agent, except to an admin whose
//! connection holds `admin` (carnet#1511, Q14 and Q25):
//!
//! - the coaching-platform roster tools (`get_provider_roster`,
//!   `propose_athlete_link`) read a coach's platform roster, people who are
//!   not Dravr members included, and notify a member;
//! - agent store, agent CRUD and the agent reads (`list_agents`, `get_agent`,
//!   `search_agents`, `get_active_agent`, `list_hidden_agents`) manage and
//!   describe the personas of Dravr's own chat, which an outside assistant
//!   does not run;
//! - memory and self-steering (`remember_fact`, the commitments,
//!   `forget_playbook`) write what Dravr's own chat later reads as the
//!   athlete's record, which an assistant's text does not get to become;
//!   `list_coaching_playbooks` reads what that chat learned, for that chat;
//! - the configuration reads (`get_configuration_catalog`,
//!   `get_configuration_profiles`, `get_user_configuration`,
//!   `validate_configuration`, `get_fitness_config`, `list_fitness_configs`)
//!   expose the settings Dravr's own analytics run on, not the athlete's
//!   training;
//! - the prescriptive nutrition tools (`calculate_daily_nutrition`,
//!   `get_nutrient_timing`) would answer an external caller with no figures
//!   for any athlete (the nutrition gate's external arm,
//!   `pierre_services::medical_flag`), so they are not offered there at all
//!   (carnet#1565).
//!
//! One tool, `publish_training_plan`, is kept in the app from everyone,
//! admins included ([`ToolReach::AppOnly`]): every effect it has is one an
//! external transport refuses for everyone (carnet#1533).
//!
//! A tool declares this as a property of itself
//! ([`RuntimeTool::reach`](crate::security::RuntimeTool::reach)), and
//! [`Transport::is_first_party`] decides. There is no list of names: a tool
//! added later is kept off external transports by declaring it.
//!
//! Listing and execution ask the same question,
//! [`tool_access::withheld`](crate::tool_access::withheld): an external
//! listing leaves these tools out, a chat turn's catalogue leaves them out by
//! the turn's transport, and the executor answers an external caller that
//! names one anyway with [`refusal`].
//!
//! [`ToolReach::AppOnly`]: crate::security::ToolReach::AppOnly
//! [`Transport::is_first_party`]: pierre_core::transport::Transport::is_first_party

use serde_json::json;

use crate::protocol::types::UniversalResponse;

/// `error_code` of the refusal a tool kept on Dravr's own surfaces returns to
/// an external caller.
pub const FIRST_PARTY_SURFACE_ERROR_CODE: &str = "first_party_surface";

/// `reason` of that refusal: the same reason a coach's write to a coached
/// athlete's plan or calendar gives over an external transport.
pub(crate) const EXTERNAL_TRANSPORT_REASON: &str = "external_transport";

/// The in-band refusal for `tool_name` when the transport arm of
/// [`tool_access::withheld`](crate::tool_access::withheld) withholds it.
///
/// In band, like the coach seat's: the caller adapts to "not over this
/// interface" the way it adapts to a tenant-disabled tool. Nothing the tool
/// would have read is looked up before it.
#[must_use]
pub fn refusal(tool_name: &str) -> UniversalResponse {
    UniversalResponse {
        success: false,
        result: Some(json!({
            "error_code": FIRST_PARTY_SURFACE_ERROR_CODE,
            "reason": EXTERNAL_TRANSPORT_REASON,
        })),
        error: Some(refusal_message(tool_name)),
        metadata: None,
    }
}

/// What the caller reads when it names a tool kept on Dravr's own surfaces.
fn refusal_message(tool_name: &str) -> String {
    format!("'{tool_name}' is not available over this interface: it runs in Dravr's own app or channels.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_refusal_names_the_tool_and_where_it_runs() {
        let message = refusal_message("get_provider_roster");
        assert!(
            message.starts_with("'get_provider_roster' is not available over this interface"),
            "bug: the refusal must name the tool first: {message}"
        );
        assert!(
            message.contains("Dravr's own app or channels"),
            "bug: the refusal must say where the tool runs: {message}"
        );
        let refused = refusal("get_provider_roster");
        assert!(!refused.success, "bug: a refusal must not report success");
        let result = refused.result.expect("the refusal carries its code");
        assert_eq!(result["error_code"], FIRST_PARTY_SURFACE_ERROR_CODE);
        assert_eq!(result["reason"], EXTERNAL_TRANSPORT_REASON);
    }

    #[cfg(all(feature = "tools-groups", feature = "tools-memory"))]
    mod with_registered_tools {
        use std::sync::Arc;

        use pierre_core::permissions::scopes::OAuthScope;
        use pierre_core::transport::Transport;

        use crate::implementations::groups::create_group_tools;
        use crate::implementations::memory::create_memory_tools;
        use crate::registry::ToolRegistry;
        use crate::security::{Audience, ToolReach};
        use crate::tool_access::{retain_offered, Caller, CallerRole};

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

        /// Three of the five group tools serve a human coach over every
        /// transport; the two coaching-platform tools stay first-party.
        #[test]
        fn three_group_tools_serve_a_coach_and_two_stay_first_party() {
            let registry = registry();
            let mut coach: Vec<&str> = Vec::new();
            let mut first_party: Vec<&str> = Vec::new();
            for name in registry.tools_in_category("groups") {
                let tool = registry.get(name).expect("registered");
                if tool.audience() == Audience::Coach {
                    assert_eq!(tool.reach(), ToolReach::AnyTransport, "{name}");
                    coach.push(name);
                } else {
                    assert_eq!(tool.reach(), ToolReach::FirstPartyOnly, "{name}");
                    first_party.push(name);
                }
            }
            coach.sort_unstable();
            first_party.sort_unstable();
            assert_eq!(
                coach,
                [
                    "get_group_member_activities",
                    "get_group_member_plan_status",
                    "get_roster_overview"
                ]
            );
            assert_eq!(first_party, ["get_provider_roster", "propose_athlete_link"]);
        }

        #[test]
        fn an_athletes_external_listing_loses_the_coach_and_first_party_tools() {
            let registry = registry();
            let grant = OAuthScope::self_grant();
            let athlete = Caller {
                role: CallerRole::default(),
                grant: &grant,
                transport: Some(Transport::McpHttp),
            };
            let mut schemas = registry.all_schemas();
            retain_offered(&registry, &mut schemas, &athlete);
            let kept: Vec<&str> = schemas.iter().map(|schema| schema.name.as_str()).collect();
            assert_eq!(kept, ["recall_user_memory"]);

            let coach = Caller {
                role: CallerRole {
                    admin: false,
                    coach: true,
                },
                ..athlete
            };
            let mut schemas = registry.all_schemas();
            retain_offered(&registry, &mut schemas, &coach);
            assert_eq!(
                schemas.len(),
                4,
                "the three coach tools and the memory read"
            );

            let in_app = Caller {
                transport: Some(Transport::WebApp),
                ..athlete
            };
            let mut schemas = registry.all_schemas();
            let before = schemas.len();
            retain_offered(&registry, &mut schemas, &in_app);
            assert_eq!(schemas.len(), before, "an in-app listing keeps every tool");
        }
    }
}
