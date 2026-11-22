// ABOUTME: A2A GetExtendedAgentCard — the public card plus the live registry's tools as skills, listed per caller
// ABOUTME: Handler on A2AServer dispatched by the parent protocol module's JSON-RPC router

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::transport::Transport;
use pierre_tool_runtime::deployment_tools;
use pierre_tool_runtime::support_session::END_TOOL;
use pierre_tool_runtime::tool_access::{self, Caller, CallerRole};
use serde_json::to_value;
use tracing::error;

use super::principal::AuthPrincipal;
use super::A2AServer;
use crate::agent_card::{AgentCard, AgentSkill};
use crate::{A2ARequest, A2AResponse};

impl A2AServer {
    /// Handle `GetExtendedAgentCard`: the public card enriched with the
    /// live tool registry exposed as additional skills. This is the
    /// authenticated, dynamic variant of the discovery card (the public
    /// card's skills are the curated static subset).
    ///
    /// Listed per caller, by the one access predicate the call path refuses
    /// with (carnet#1511): the agent's grant, the A2A transport and the live
    /// role of the account the principal acts for, which for a
    /// client-credentials client is its registering user (Q17). A static
    /// skill that names a tool the predicate withholds goes too.
    pub(super) async fn handle_extended_card(
        &self,
        request: A2ARequest,
        principal: AuthPrincipal,
    ) -> A2AResponse {
        let Some(resources) = &self.resources else {
            return Self::server_not_configured_error(request.id);
        };

        let mut card = AgentCard::new(resources.ctx.oauth2_server_config());
        let registry = resources.tool_runtime.tool_registry();
        // A live support session lists the user's skills, and the one that
        // ends it (carnet#1511).
        let acting = match Self::acting(resources.ctx.as_ref(), &principal).await {
            Ok(acting) => acting,
            Err(reason) => return Self::a2a_error(-32000, reason, request.id),
        };
        let caller = Caller {
            role: CallerRole::resolve(resources.ctx.repos(), acting.user_id).await,
            grant: &acting.scopes,
            transport: Some(Transport::A2a),
            support_session: acting.support.is_some(),
        };
        card.skills.retain(|skill| {
            registry
                .get(&skill.id)
                .is_none_or(|tool| tool_access::withheld(tool.as_ref(), &caller).is_none())
        });
        let public_skill_ids: Vec<String> =
            card.skills.iter().map(|skill| skill.id.clone()).collect(); // Safe: String ownership for dedup set
        let mut schemas = registry.all_schemas();
        tool_access::retain_offered(registry, &mut schemas, &caller);
        if acting.support.is_some() {
            schemas.extend(registry.list_schemas_by_name_set(&[END_TOOL.to_owned()]));
        }
        deployment_tools::retain_answerable(
            registry,
            &mut schemas,
            resources.tool_runtime.config(),
        );
        for schema in schemas {
            if public_skill_ids.contains(&schema.name) {
                continue;
            }
            card.skills.push(AgentSkill {
                id: schema.name.clone(), // Safe: String ownership for skill id
                name: schema.name,
                description: schema.description,
                tags: vec!["tool".into()],
                examples: Vec::new(),
                input_modes: Vec::new(),
                output_modes: Vec::new(),
            });
        }

        match to_value(&card) {
            Ok(value) => Self::ok(value, request.id),
            Err(e) => {
                error!("Failed to serialize A2A extended agent card: {e}");
                Self::a2a_error(-32603, "Failed to serialize extended card", request.id)
            }
        }
    }
}
