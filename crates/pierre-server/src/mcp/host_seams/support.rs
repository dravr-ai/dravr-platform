// ABOUTME: The /mcp seam of an admin's support session: which connection asks, whether a session is live, the user's context
// ABOUTME: The listing and the call both read it, so a live session lists and serves the user and nothing else

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! An admin's support session through their assistant
//! ([`pierre_tool_runtime::support_session`], carnet#1511), as `/mcp` serves
//! it.
//!
//! The auth hook resolves the admin's own context and records the
//! credential it admitted ([`pierre_middleware::rate_limiting::request_credential`]:
//! the engine's context has no slot for it); the dispatcher asks here
//! whether a session started with that very credential is live, and if so
//! lists and dispatches as the user. The two session tools are the one
//! exception: they run as the admin, so the session can always be ended.

use dravr_barrage::Tier;
use dravr_tronc::mcp::tool::ToolContext;
use pierre_auth::oauth2_server::grant_ceiling::GrantCeiling;
use pierre_core::errors::AppResult;
use pierre_core::permissions::impersonation::CallerCredential;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_middleware::rate_limiting;
use pierre_tool_runtime::support_session::{
    self, grant_for_user, LiveSupport, END_TOOL, START_TOOL,
};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;

/// The credential a `/mcp` call came through, as the auth hook recorded it
/// for this request: its kind and client, and what a support session binds
/// to (the API key, or the OAuth grant its token was minted under). `None`
/// outside an HTTP request, which starts and serves no session.
pub(super) fn credential() -> Option<CallerCredential> {
    rate_limiting::request_credential()
}

/// The ceiling of the consent grant a `/mcp` call's credential was admitted
/// under, as the auth hook recorded it for this request (plan slice 3);
/// `None` under a plain grant, for every other credential and outside an
/// HTTP request.
pub(super) fn ceiling() -> Option<GrantCeiling> {
    rate_limiting::request_ceiling()
}

/// The MCP client extension a host declares its Host Profile with, per
/// request (owl PDPP §6.2 carrier B).
const HOST_PROFILE_EXTENSION: &str = "app.dravr.vault/host-profile";

/// [`ceiling`] for the call `ctx` serves, with the Host Profile tier its own
/// request declares: a step-up child's T2 opens only while each call's claim
/// still says T2 (owl §6.1, the stricter carrier wins).
pub(super) fn call_ceiling(ctx: &ToolContext) -> Option<GrantCeiling> {
    ceiling().map(|ceiling| ceiling.with_request_host_tier(request_host_tier(ctx)))
}

/// The tier `ctx`'s request declares in
/// `_meta["io.modelcontextprotocol/clientCapabilities"].extensions["app.dravr.vault/host-profile"]`
/// as `{"tier_max": "t2", "version": …}`; `None` when it declares none or a
/// value this server does not read, which opens nothing above T1.
fn request_host_tier(ctx: &ToolContext) -> Option<Tier> {
    let declared = ctx
        .client_capabilities
        .as_ref()?
        .get("extensions")?
        .get(HOST_PROFILE_EXTENSION)?
        .get("tier_max")?
        .as_str()?;
    match declared.to_ascii_uppercase().as_str() {
        "T0" => Some(Tier::T0),
        "T1" => Some(Tier::T1),
        "T2" => Some(Tier::T2),
        _ => None,
    }
}

/// The support session live for the admin `ctx` names on the credential
/// `credential` binds to ([`credential`], read once per call by the caller).
///
/// # Errors
///
/// The repository error from the lookup: the caller refuses rather than
/// serve as the admin a connection whose session may be live.
pub(super) async fn live(
    resources: &ServerContext,
    ctx: &ToolContext,
    credential: Option<&CallerCredential>,
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
        credential,
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn declaring(capabilities: serde_json::Value) -> ToolContext {
        ToolContext::default().with_client_capabilities(capabilities)
    }

    #[test]
    fn a_requests_host_profile_claim_is_read_and_anything_else_opens_nothing() {
        let claim = |value| json!({"extensions": {HOST_PROFILE_EXTENSION: value}});
        assert_eq!(
            request_host_tier(&declaring(claim(json!({"tier_max": "t2", "version": 1})))),
            Some(Tier::T2),
            "bug: a T2 claim was not read"
        );
        assert_eq!(
            request_host_tier(&declaring(claim(json!({"tier_max": "t3"})))),
            None,
            "bug: a T3 claim read as a tier"
        );
        assert_eq!(
            request_host_tier(&declaring(claim(json!({})))),
            None,
            "bug: a claim with no tier read as one"
        );
        assert_eq!(
            request_host_tier(&declaring(json!({}))),
            None,
            "bug: no claim read as one"
        );
        assert_eq!(
            request_host_tier(&ToolContext::default()),
            None,
            "bug: legacy"
        );
    }
}
