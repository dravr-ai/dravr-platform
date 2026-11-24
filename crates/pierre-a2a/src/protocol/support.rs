// ABOUTME: The A2A seam of an admin's support session: whether one is live for this agent, and who the call acts as
// ABOUTME: The extended card and tool execution both read it, so a live session lists and serves the user alone

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! An admin's support session through their agent
//! ([`pierre_tool_runtime::support_session`], carnet#1511), as A2A serves
//! it: bound to the OAuth grant the agent's user token was minted under
//! (its refresh family), so another connection of the same application
//! acts as the admin. A client-credentials agent never holds `admin`, so it
//! never has one.

use pierre_core::models::TenantId;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_runtime_context::A2ACtx;
use pierre_tool_runtime::support_session::{
    self, grant_for_user, LiveSupport, END_TOOL, START_TOOL,
};
use tracing::error;
use uuid::Uuid;

use super::principal::AuthPrincipal;
use super::A2AServer;

/// Who one A2A call acts as.
pub(super) struct Acting {
    /// The user the call runs for: the principal's, or the user a live
    /// support session acts as.
    pub(super) user_id: Uuid,
    /// The tenant it runs in.
    pub(super) tenant: Option<TenantId>,
    /// The grant it runs under: the principal's, without `admin` in a
    /// support session.
    pub(super) scopes: Vec<OAuthScope>,
    /// The live support session, when the call runs in one.
    pub(super) support: Option<LiveSupport>,
}

impl A2AServer {
    /// The support session live for `principal` on its connection.
    ///
    /// # Errors
    ///
    /// The repository error's text, when the lookup fails: the caller
    /// refuses rather than act as the admin while a session may be live.
    pub(super) async fn live_support(
        ctx: &dyn A2ACtx,
        principal: &AuthPrincipal,
    ) -> Result<Option<LiveSupport>, String> {
        support_session::live_support(
            ctx.repos(),
            principal.user_id,
            &principal.scopes,
            Some(&principal.credential),
            Transport::A2a,
        )
        .await
        .map_err(|e| {
            error!(error = %e, "A2A: the support-session lookup failed");
            "Whether this agent is in a support session could not be read; please retry".to_owned()
        })
    }

    /// Whether a support session is live for `principal`, reading a failed
    /// lookup as live: a conversation is never run as the admin while one
    /// may be.
    pub(super) async fn support_session_live(ctx: &dyn A2ACtx, principal: &AuthPrincipal) -> bool {
        Self::live_support(ctx, principal)
            .await
            .map_or(true, |live| live.is_some())
    }

    /// Who a call of `tool_name` acts as: [`Self::acting`], except for the
    /// two session tools, which run as the admin.
    ///
    /// # Errors
    ///
    /// As [`Self::live_support`].
    pub(super) async fn acting_for(
        ctx: &dyn A2ACtx,
        principal: &AuthPrincipal,
        tool_name: &str,
    ) -> Result<Acting, String> {
        if tool_name == START_TOOL || tool_name == END_TOOL {
            return Ok(Acting::principal(principal));
        }
        Self::acting(ctx, principal).await
    }

    /// Who `principal` acts as: the user of a live support session, else
    /// itself.
    ///
    /// # Errors
    ///
    /// As [`Self::live_support`].
    pub(super) async fn acting(
        ctx: &dyn A2ACtx,
        principal: &AuthPrincipal,
    ) -> Result<Acting, String> {
        Ok(match Self::live_support(ctx, principal).await? {
            Some(live) => Acting {
                user_id: live.target(),
                tenant: Some(live.tenant),
                scopes: grant_for_user(&principal.scopes),
                support: Some(live),
            },
            None => Acting::principal(principal),
        })
    }
}

impl Acting {
    /// `principal` acting as itself.
    fn principal(principal: &AuthPrincipal) -> Self {
        Self {
            user_id: principal.user_id,
            tenant: principal.active_tenant_id.map(TenantId::from_uuid),
            scopes: principal.scopes.clone(),
            support: None,
        }
    }
}
