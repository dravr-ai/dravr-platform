// ABOUTME: An authorization request's agent-write detail checked against the binding it asks for (owl PDPP §4.5, §5.1, §7.3)
// ABOUTME: DPoP: its own profile grant bound to the agent-write resource; none: a child of the MCP package bound to /mcp or /a2a
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use tracing::warn;

use super::OAuth2AuthorizationServer;
use crate::oauth2_server::authorization_details::{
    ResolvedAgentWrite, ResolvedDetails, SenderConstraint, AGENT_WRITE_SCOPE,
};
use crate::oauth2_server::models::OAuth2Error;

/// The refusal of an agent-write request while the consent screen cannot
/// word one: the catalogue lacks its strings.
pub const AGENT_WRITE_NOT_OFFERED: &str = "agent writes are not offered yet";

impl OAuth2AuthorizationServer {
    /// Offer agent-write grants: an agent-write detail is resolved rather
    /// than refused [`AGENT_WRITE_NOT_OFFERED`].
    #[must_use]
    pub const fn with_agent_write_offered(mut self, offered: bool) -> Self {
        self.agent_write_offered = offered;
        self
    }

    /// Whether agent-write grants are offered here.
    #[must_use]
    pub const fn agent_write_offered(&self) -> bool {
        self.agent_write_offered
    }

    /// `write`, the agent-write detail a request bound to `resource` asks for beside a read
    /// grant or not (`with_read`), checked against its binding.
    ///
    /// - `dpop` (binding A) is its own profile grant: bound to the
    ///   agent-write resource, and the request carries `dpop_jkt`, so its
    ///   code is redeemed with a proof only;
    /// - `none` (binding B) is a child of the MCP package: bound to the
    ///   platform's MCP or A2A resource, never the agent-write one.
    ///
    /// Every other request bound to the agent-write resource is refused, a
    /// plain one (`write` `None`) included.
    ///
    /// # Errors
    /// `invalid_authorization_details` while agent writes are not offered or
    /// for a `dpop` request without `dpop_jkt`; `invalid_target` for a
    /// resource the binding does not allow.
    pub(super) fn bind_agent_write(
        &self,
        write: Option<ResolvedAgentWrite>,
        with_read: bool,
        resource: Option<&str>,
        dpop_jkt: Option<&str>,
    ) -> Result<Option<ResolvedAgentWrite>, OAuth2Error> {
        let to_write_resource = self.is_agent_write_resource(resource);
        let Some(write) = write else {
            if to_write_resource {
                return Err(OAuth2Error::invalid_target(
                    "the agent-write resource is granted only to an agent-write detail bound by DPoP",
                ));
            }
            return Ok(None);
        };
        if !self.agent_write_offered {
            return Err(OAuth2Error::invalid_authorization_details(
                AGENT_WRITE_NOT_OFFERED,
            ));
        }
        match write.sender_constraint() {
            SenderConstraint::Dpop => {
                if with_read || !to_write_resource {
                    warn!("agent-write refused: a DPoP-bound detail not bound to the agent-write resource alone");
                    return Err(OAuth2Error::invalid_target(
                        "an agent-write grant bound by DPoP is for the agent-write resource, requested alone",
                    ));
                }
                if dpop_jkt.is_none() {
                    return Err(OAuth2Error::invalid_authorization_details(
                        "an agent-write grant bound by DPoP needs dpop_jkt on the request",
                    ));
                }
            }
            SenderConstraint::None => {
                if !self.is_platform_resource(resource) {
                    warn!("agent-write refused: sender_constraint none outside the MCP package");
                    return Err(OAuth2Error::invalid_target(
                        "an agent-write grant with sender_constraint none is a child of the MCP or A2A grant; bind it to that resource",
                    ));
                }
            }
        }
        Ok(Some(write))
    }

    /// The scope an authorization asking `details` and `write` carries.
    ///
    /// [`AGENT_WRITE_SCOPE`] for a profile grant (binding A), which reads nothing and opens no
    /// tool; `scope` otherwise, a package child's code naming its parent's.
    pub(super) fn agent_write_scope(
        scope: String,
        details: Option<&ResolvedDetails>,
        write: Option<&ResolvedAgentWrite>,
    ) -> String {
        match (details, write) {
            (None, Some(write)) if write.sender_constraint() == SenderConstraint::Dpop => {
                AGENT_WRITE_SCOPE.to_owned()
            }
            _ => scope,
        }
    }
}
