// ABOUTME: An admin's support session through their own AI assistant over MCP or A2A: start, find, expire, end
// ABOUTME: The impersonation_sessions row the web console uses, bound to the assistant connection that started it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Support sessions through an admin's assistant (carnet#1511)
//!
//! JF, 2026-11-19: an admin's AI assistant can act as a user. This is the
//! web console's support session, over MCP and A2A, under the same rules:
//!
//! - **Who.** An admin whose connection's grant holds `admin`: their own API
//!   key, or an OAuth connection whose grant holds it, which none does while
//!   `admin` is not delegable (`OAuthScope::is_delegable`). Never of another
//!   admin, never of themselves. The two
//!   tools that start and end it are `ADMIN_ONLY`, so the one access
//!   predicate ([`crate::tool_access`]) refuses them to anyone else.
//! - **Bound to the connection that started it** ([`SUPPORT_BINDING`]): an
//!   OAuth application's client id, or the admin's API keys. A session the
//!   web console started never turns an assistant into the user, and one an
//!   assistant started never changes the console's. One live session per
//!   admin across surfaces: a start ends every earlier one, as on the web.
//! - **Sixty minutes from its start**, the web hour, never renewed. Nothing
//!   fires at the hour; the first lookup after it ends the row and writes
//!   the expiry to the audit trail.
//! - **What the connection is while it is live.** The transport swaps the
//!   caller to the user: their tools, their data, their tenant, the admin's
//!   grant without `admin`. Every call runs inside
//!   [`SUPPORT_SESSION`](crate::context::SUPPORT_SESSION), so what a support
//!   session may not leave behind (a credential, a grant, a consent,
//!   carnet#1634) is refused as on the web, and the export tools are
//!   withheld (data_export.rs: support may see what it needs to help, never
//!   carry the whole record away).
//! - **Audit.** Start, end and expiry are `audit_events` rows naming the
//!   admin, the user, the session and the connection; so is every call made
//!   in the session, written before it runs, and a row that cannot be
//!   written refuses the call (the executor's admin-call audit).
//! - **It ends** with [`END_TOOL`], the web console's end, the hour, a new
//!   start on any surface, the admin losing the admin role, or the user
//!   gaining it.

use chrono::{DateTime, Duration, Utc};
use pierre_auth::auth::ImpersonationGrant;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{TenantId, User};
use pierre_core::permissions::impersonation::{
    self, ImpersonationSession, API_KEY_BINDING, SESSION_MINUTES,
};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_core::transport::Transport;
use pierre_database::repositories::{AuditOutcome, AuditSource, NewAuditEvent};
use pierre_database::RepositoryRegistry;
use serde_json::json;
use tracing::{error, info, warn};
use uuid::Uuid;

/// The tool an admin's assistant starts a support session with.
pub const START_TOOL: &str = "admin_impersonate_start";

/// The tool an admin's assistant ends its support session with: the one call
/// a live session serves as the admin, never as the user.
pub const END_TOOL: &str = "admin_impersonate_end";

/// `audit_events.action` of a support session an assistant started.
pub const STARTED_ACTION: &str = "impersonation.started";

/// `audit_events.action` of a support session an assistant ended.
pub const ENDED_ACTION: &str = "impersonation.ended";

/// `audit_events.action` of a support session ended by its hour.
pub const EXPIRED_ACTION: &str = "impersonation.expired";

tokio::task_local! {
    /// The assistant connection the in-flight external call came through,
    /// as a support session binds it ([`binding_of`]): set by the `/mcp`
    /// dispatcher and the A2A executor around each call. Absent on Dravr's
    /// own surfaces, where [`START_TOOL`] has nothing to bind and refuses.
    pub static SUPPORT_BINDING: Option<String>;
}

/// The binding of an external call.
///
/// The OAuth application's client id the credential was issued to, or
/// [`API_KEY_BINDING`] for an API key. `None` for any other credential (a
/// personal MCP token never holds `admin`).
#[must_use]
pub fn binding_of(api_key: bool, client_id: Option<&str>) -> Option<String> {
    if api_key {
        return Some(API_KEY_BINDING.to_owned());
    }
    client_id.map(str::to_owned)
}

/// The binding the in-flight call declared, if any.
#[must_use]
pub fn current_binding() -> Option<String> {
    SUPPORT_BINDING.try_with(Clone::clone).ok().flatten()
}

/// A support session that is live for the connection asking.
#[derive(Debug, Clone)]
pub struct LiveSupport {
    /// The session row.
    pub session: ImpersonationSession,
    /// The tenant the user is served in: their first, as the web console's
    /// start picks it, never the admin's.
    pub tenant: TenantId,
}

impl LiveSupport {
    /// The session as the support-session refusals read it
    /// ([`crate::context::SUPPORT_SESSION`]).
    #[must_use]
    pub fn grant(&self) -> ImpersonationGrant {
        ImpersonationGrant {
            session_id: self.session.id.clone(),
            impersonator_id: self.session.impersonator_id,
            expires_at: expires_at(&self.session).timestamp(),
        }
    }

    /// The user the session acts as.
    #[must_use]
    pub const fn target(&self) -> Uuid {
        self.session.target_user_id
    }
}

/// When `session` stops serving.
fn expires_at(session: &ImpersonationSession) -> DateTime<Utc> {
    session.started_at + Duration::minutes(SESSION_MINUTES)
}

/// The grant the user is served under: the admin's connection's, without
/// `admin`. The user is never an admin, and nothing of the session acts with
/// the admin's own powers.
#[must_use]
pub fn grant_for_user(grant: &[OAuthScope]) -> Vec<OAuthScope> {
    grant
        .iter()
        .copied()
        .filter(|scope| *scope != OAuthScope::Admin)
        .collect()
}

/// The support session live for `admin` on the connection `binding` names,
/// under `grant`, or `None`.
///
/// `None` without a lookup when the grant does not hold `admin`: a session
/// is never served to a connection that could not have started it. A
/// session past its hour, or whose admin is no longer one, or whose user has
/// become one, is ended here and answers `None`.
///
/// # Errors
///
/// Returns the repository error from a lookup: the caller refuses the
/// request rather than serve it as the admin when the session may be live.
pub async fn live_support(
    repos: &RepositoryRegistry,
    admin: Uuid,
    grant: &[OAuthScope],
    binding: Option<&str>,
    transport: Transport,
) -> AppResult<Option<LiveSupport>> {
    let Some(binding) = binding else {
        return Ok(None);
    };
    if !grant.contains(&OAuthScope::Admin) {
        return Ok(None);
    }
    let Some(session) = repos
        .impersonation
        .get_active_bound_session(admin, binding)
        .await?
    else {
        return Ok(None);
    };
    if Utc::now() >= expires_at(&session) {
        close(repos, &session, EXPIRED_ACTION, transport).await?;
        return Ok(None);
    }
    // SECURITY: Global lookups — the admin and the user act across tenants
    let admin_in_standing = repos
        .users
        .get_global(admin)
        .await?
        .is_some_and(|user| user.is_active_admin());
    let target = repos.users.get_global(session.target_user_id).await?;
    let target_served = target.as_ref().is_some_and(|user| !user.role.is_admin());
    if !admin_in_standing || !target_served {
        warn!(
            impersonator_id = %admin,
            session_id = %session.id,
            "support session through an assistant ended: its admin or its user no longer qualifies"
        );
        close(repos, &session, ENDED_ACTION, transport).await?;
        return Ok(None);
    }
    let Some(tenant) = repos.tenants.first_for_user(session.target_user_id).await? else {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "The user this support session acts as belongs to no tenant",
        ));
    };
    Ok(Some(LiveSupport { session, tenant }))
}

/// Who a support session is asked to act as: an email or an account id.
fn parse_target(user: &str) -> Option<Uuid> {
    Uuid::parse_str(user.trim()).ok()
}

/// What [`start`] answers.
#[derive(Debug, Clone)]
pub struct StartedSupport {
    /// The session row.
    pub session: ImpersonationSession,
    /// The user it acts as.
    pub target: User,
    /// When it stops serving.
    pub expires_at: DateTime<Utc>,
}

/// Start a support session for `admin` on the connection `binding` names,
/// acting as `user` (an email or an account id), for `reason`.
///
/// Every earlier session of the admin ends first, on every surface. The
/// start is written to the audit trail; a row that cannot be written ends
/// the session again and refuses the start.
///
/// # Errors
///
/// `PermissionDenied` when the caller is not an active admin, the user is an
/// admin or the caller, or no binding was declared; `InvalidInput` for an
/// empty reason; `NotFound` for an unknown user; the repository error
/// otherwise.
pub async fn start(
    repos: &RepositoryRegistry,
    admin: Uuid,
    binding: Option<&str>,
    user: &str,
    reason: &str,
    transport: Transport,
) -> AppResult<StartedSupport> {
    let Some(binding) = binding else {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "A support session through an assistant starts from that assistant's own MCP or A2A \
             connection; in Dravr's own app, start it from the admin console",
        ));
    };
    let reason = reason.trim();
    if reason.is_empty() {
        return Err(AppError::invalid_input(
            "Say why you act as this user: the reason goes on the audit trail",
        ));
    }
    // SECURITY: Global lookups — impersonation is an admin's, cross-tenant by design
    let in_standing = repos
        .users
        .get_global(admin)
        .await?
        .is_some_and(|caller| caller.is_active_admin());
    if !in_standing {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Admin privileges required for a support session",
        ));
    }
    let target = match parse_target(user) {
        Some(id) => repos.users.get_global(id).await?,
        None => repos.users.get_by_email(user.trim()).await?,
    }
    .ok_or_else(|| AppError::not_found("No Dravr account has that email or id"))?;
    impersonation::check_target(admin, &target)?;
    if repos.tenants.first_for_user(target.id).await?.is_none() {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "That user belongs to no tenant, so there is nothing to act as",
        ));
    }

    repos.impersonation.end_all_sessions(admin).await?;
    let session = ImpersonationSession::bound_to(
        admin,
        target.id,
        Some(reason.to_owned()),
        binding.to_owned(),
    );
    repos.impersonation.create_session(&session).await?;
    let event = event(&session, STARTED_ACTION, "started", transport);
    if let Err(e) = repos.admin_audit.record(&event).await {
        error!(
            session_id = %session.id,
            "support session refused: its start could not be written to the audit trail: {e}"
        );
        repos.impersonation.end_session(&session.id).await?;
        return Err(AppError::internal(
            "The support session was not started: its record on the audit trail could not be \
             written",
        ));
    }
    info!(
        impersonator_id = %admin,
        target_user_id = %target.id,
        session_id = %session.id,
        bound_client = %binding,
        "Admin started a support session through their assistant"
    );
    Ok(StartedSupport {
        expires_at: expires_at(&session),
        session,
        target,
    })
}

/// End the live support session `admin` started on the connection `binding`
/// names, and write the end to the audit trail. `None` when none is live.
///
/// # Errors
///
/// `PermissionDenied` when no binding was declared; the repository error
/// otherwise.
pub async fn end(
    repos: &RepositoryRegistry,
    admin: Uuid,
    binding: Option<&str>,
    transport: Transport,
) -> AppResult<Option<ImpersonationSession>> {
    let Some(binding) = binding else {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "A support session through an assistant is ended from that assistant's own \
             connection; in Dravr's own app, end it from the admin console",
        ));
    };
    let Some(session) = repos
        .impersonation
        .get_active_bound_session(admin, binding)
        .await?
    else {
        return Ok(None);
    };
    close(repos, &session, ENDED_ACTION, transport).await?;
    Ok(Some(session))
}

/// End `session` and write `action` to the audit trail. The session ends
/// whether or not the row can be written; a row that cannot is logged.
async fn close(
    repos: &RepositoryRegistry,
    session: &ImpersonationSession,
    action: &str,
    transport: Transport,
) -> AppResult<()> {
    repos.impersonation.end_session(&session.id).await?;
    let verb = if action == EXPIRED_ACTION {
        "expired"
    } else {
        "ended"
    };
    if let Err(e) = repos
        .admin_audit
        .record(&event(session, action, verb, transport))
        .await
    {
        error!(
            session_id = %session.id,
            action,
            "support session ended; its audit row could not be written: {e}"
        );
    }
    Ok(())
}

/// The audit row of a support session's start, end or expiry.
fn event(
    session: &ImpersonationSession,
    action: &str,
    verb: &str,
    transport: Transport,
) -> NewAuditEvent {
    let label = Transport::label_of(Some(transport));
    NewAuditEvent {
        action: action.to_owned(),
        summary: format!("Support session through an assistant {verb} over {label}"),
        outcome: AuditOutcome::Success,
        source: AuditSource::ToolCall,
        actor_user_id: Some(session.impersonator_id),
        // The user's tenant, not the admin's: only a view of the whole trail
        // reads it, as every row reaching past the tenant its admin acts in.
        tenant_id: None,
        ip_address: None,
        user_agent: None,
        metadata: json!({
            "target": { "user_id": session.target_user_id },
            "impersonation_session_id": session.id,
            "bound_client": session.bound_client,
            "reason": session.reason,
            "transport": label,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_api_key_binds_as_the_admins_keys_and_an_oauth_token_as_its_client() {
        assert_eq!(
            binding_of(true, None).as_deref(),
            Some(API_KEY_BINDING),
            "bug: an API key binds as the admin's keys"
        );
        assert_eq!(
            binding_of(false, Some("claude-ai")).as_deref(),
            Some("claude-ai"),
            "bug: an OAuth token binds as its client"
        );
        assert_eq!(binding_of(false, None), None, "nothing to bind to");
    }

    #[test]
    fn the_user_is_served_without_admin() {
        let served = grant_for_user(&OAuthScope::self_grant());
        assert!(
            !served.contains(&OAuthScope::Admin),
            "bug: the user is never served admin"
        );
        assert!(
            served.contains(&OAuthScope::FitnessRead),
            "bug: the user keeps fitness read"
        );
    }
}
