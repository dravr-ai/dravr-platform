// ABOUTME: The /mcp seam of an admin's support session: which connection asks, whether a session is live, the user's context
// ABOUTME: The listing and the call both read it, so a live session lists and serves the user and nothing else

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! An admin's support session through their assistant
//! ([`pierre_tool_runtime::support_session`], carnet#1511), as `/mcp` serves
//! it.
//!
//! The auth hook resolves the admin's own context; the dispatcher asks here
//! whether a session started through this very connection is live, and if
//! so lists and dispatches as the user. The two session tools are the one
//! exception: they run as the admin, so the session can always be ended.

use dravr_tronc::mcp::tool::ToolContext;
use pierre_core::errors::AppResult;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_tool_runtime::context::AuthMethod;
use pierre_tool_runtime::support_session::{
    self, grant_for_user, LiveSupport, END_TOOL, START_TOOL,
};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

/// The connection a `/mcp` call came through, as a support session binds it:
/// the OAuth client its token was issued to, or the admin's API keys.
pub(super) fn binding(ctx: &ToolContext) -> Option<String> {
    let api_key = ctx.auth_method.as_deref() == Some(AuthMethod::ApiKey.as_str());
    support_session::binding_of(api_key, ctx.client_id.as_deref())
}

/// The support session live for the admin `ctx` names on this connection.
///
/// # Errors
///
/// The repository error from the lookup: the caller refuses rather than
/// serve as the admin a connection whose session may be live.
pub(super) async fn live(
    resources: &ServerContext,
    ctx: &ToolContext,
) -> AppResult<Option<LiveSupport>> {
    let Some(admin) = ctx
        .user_id
        .as_deref()
        .and_then(|id| Uuid::parse_str(id).ok())
    else {
        return Ok(None);
    };
    let grant = OAuthScope::parse_granted(&ctx.scopes.join(" "));
    support_session::live_support(
        &resources.common.repos,
        admin,
        &grant,
        binding(ctx).as_deref(),
        Transport::McpHttp,
    )
    .await
}

/// Whether `tool_name` runs as the admin even while a session is live: the
/// two session tools, so a session can always be ended or replaced.
pub(super) fn runs_as_admin(tool_name: &str) -> bool {
    tool_name == START_TOOL || tool_name == END_TOOL
}

/// `ctx` acting as the user `live` names: their id and tenant, no admin
/// role, and the admin's grant without `admin`. The client and the auth
/// method stay the admin's connection's, so the call is still recorded as
/// that client's.
pub(super) fn as_user(ctx: &ToolContext, live: &LiveSupport) -> ToolContext {
    let granted = OAuthScope::parse_granted(&ctx.scopes.join(" "));
    let mut user = ctx.clone();
    user.user_id = Some(live.target().to_string());
    user.tenant_id = Some(live.tenant.to_string());
    user.is_admin = false;
    user.scopes = grant_for_user(&granted)
        .iter()
        .map(|scope| scope.as_str().to_owned())
        .collect();
    user
}
