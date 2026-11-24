// ABOUTME: Thin wrapper over the canonical pierre_runtime_context::resolve_tenant helper, plus the shared conversation wire shape
// ABOUTME: Chat routes use Required — no user-id fallback; errors if user has no tenant. Also `details.reason` refusals, a coach's external write's among them, and the threads an outside client is not served
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_auth::auth::AuthResult;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::{ConversationRecord, TenantId};
use pierre_database::repositories::SubjectThreads;
use pierre_providers::ai_scope;
use pierre_runtime_context::{resolve_tenant, tenant::require, TenantMode};
use pierre_services::athlete_display_name::fetch_user_display_name;
use pierre_services::coached_athlete::CoachedAthleteRefusal;
use pierre_tool_runtime::implementations::plan_scope;
use serde_json::json;
use uuid::Uuid;

use super::dto::ConversationResponse;
use crate::mcp::resources::ServerContext;

/// A refusal whose code a client branches on, carried as `details.reason`.
pub(super) fn refusal(code: ErrorCode, reason: &str, message: &str) -> AppError {
    with_reason(AppError::new(code, message), reason)
}

/// A coach writes a coached athlete's `object` — their "plan", their
/// "thresholds" — only from Dravr's own apps and channels. An API-key, MCP or
/// OAuth client acting for the coach is outside the agreement the athlete
/// made with their coach inside Dravr, and the tools refuse it the same
/// write, in the same words (carnet#1533).
pub(super) fn refuse_external_coach_write(auth: &AuthResult, object: &str) -> Result<(), AppError> {
    if auth.acts_for_outside_client() {
        Err(plan_scope::external_write_error(object))
    } else {
        Ok(())
    }
}

/// Whether the request is served to an outside client: an outside
/// credential (`AuthResult::acts_for_outside_client`), or a request the transport layer
/// declared outside Dravr's own surfaces (an API key). A request no layer
/// declared is read by its credential, as every route reads one
/// (`ai_scope::result_exposure`: an undeclared route is not external).
pub(super) fn served_outside(auth: &AuthResult) -> bool {
    ai_scope::served_outside(auth.acts_for_outside_client())
}

/// Which coach threads about one athlete the list served to `auth` holds: an
/// outside client is listed none about a member who does not share their
/// training with the coach (EFVP 9.4c).
pub(super) fn listed_subjects(auth: &AuthResult) -> SubjectThreads {
    if served_outside(auth) {
        SubjectThreads::SharedOnly
    } else {
        SubjectThreads::All
    }
}

/// The answer a conversation the caller cannot read gets: the same 404 a
/// conversation that does not exist gets.
pub(super) fn conversation_not_found() -> AppError {
    AppError::not_found("Conversation not found")
}

/// Refuse an outside client a coach's thread about a member who does not
/// share their training with the coach (a member who left the group shares
/// nothing): the thread is titled with the member's name and is about them
/// throughout. Answered as a conversation that does not exist, so the client
/// learns neither the name nor that such a member exists — as the tools
/// answer it "no match" (EFVP 2026-11-17, decision 9.4c). A no-op on Dravr's
/// own surfaces and for any other conversation.
///
/// # Errors
///
/// The 404 of a conversation that does not exist, and the repository error
/// when the membership cannot be read.
pub(super) async fn refuse_unshared_subject(
    resources: &ServerContext,
    auth: &AuthResult,
    conv: &ConversationRecord,
) -> Result<(), AppError> {
    let Some(subject) = conv.subject() else {
        return Ok(());
    };
    if !served_outside(auth) {
        return Ok(());
    }
    let member = resources
        .common
        .repos
        .groups
        .get_member(&subject.group_id.to_string(), subject.athlete_user_id)
        .await?;
    if member.is_some_and(|member| member.coach_sharing_consent) {
        Ok(())
    } else {
        Err(conversation_not_found())
    }
}

/// [`refuse_unshared_subject`] for a route that holds only the
/// conversation's id: read for an outside client only. A conversation the
/// caller does not take part in is left to the route's own 404.
///
/// # Errors
///
/// As [`refuse_unshared_subject`], and the repository error when the
/// conversation cannot be read.
pub(crate) async fn refuse_unshared_subject_by_id(
    resources: &ServerContext,
    auth: &AuthResult,
    conversation_id: &str,
    tenant_id: TenantId,
) -> Result<(), AppError> {
    if !served_outside(auth) {
        return Ok(());
    }
    let conv = resources
        .common
        .repos
        .chat
        .get_conversation(conversation_id, &auth.user_id.to_string(), tenant_id)
        .await?;
    match conv {
        Some(conv) => refuse_unshared_subject(resources, auth, &conv).await,
        None => Ok(()),
    }
}

/// A coaching gate's refusal as the error `auth` is answered with. An outside
/// client is never told that a member stopped sharing, nor their name: it is
/// answered as for an athlete who is not a member (EFVP 9.4c).
pub(super) fn coaching_refusal(auth: &AuthResult, refused: CoachedAthleteRefusal) -> AppError {
    match refused {
        CoachedAthleteRefusal::CoachSharingRevoked { .. } if served_outside(auth) => {
            CoachedAthleteRefusal::NotAMember.into_error()
        }
        refused => refused.into_error(),
    }
}

/// `error` with the `details.reason` a client branches on. A
/// `PermissionDenied` refusal is built with `AppError::new` at its call site,
/// so `scripts/ci/check-permission-denied-messages.sh` reads its sentence.
pub(super) fn with_reason(mut error: AppError, reason: &str) -> AppError {
    error.details = Some(Box::new(json!({ "reason": reason })));
    error
}

/// The wire shape of a conversation that may be a coach's thread bound to one
/// athlete: that athlete's roster name is resolved alongside, so the three
/// `subject_*` fields go out together. One user read for such a thread, none
/// for any other.
pub(super) async fn conversation_response(
    resources: &ServerContext,
    conv: ConversationRecord,
) -> ConversationResponse {
    let subject_display_name = match conv.subject_user_id {
        Some(athlete) => Some(fetch_user_display_name(&resources.common.repos, athlete).await),
        None => None,
    };
    ConversationResponse::new(conv, subject_display_name)
}

/// Resolve the caller's tenant id via the canonical resolver. Returns an
/// auth error if the user has no tenants — never fabricates an id from
/// the user uuid.
///
/// # Errors
///
/// Returns the resolver's error, and an auth error for a user with no
/// tenant.
pub(crate) async fn get_tenant_id(
    auth: &AuthResult,
    resources: &Arc<ServerContext>,
) -> Result<TenantId, AppError> {
    require(resolve_tenant(resources, auth, TenantMode::Required).await?)
}

/// Reject group-scoped access unless the group admits the caller: a live
/// member of an active group, or its human coach
/// ([`CoachingGroupRepository::admits_member_or_coach`](pierre_database::repositories::CoachingGroupRepository::admits_member_or_coach)).
/// Otherwise the caller would be handed peer content and fitness data they
/// have no relationship to. The coach oversees the group through its agent
/// persona, with each member's data still gated by their own
/// `coach_sharing_consent` (a fellow member's read by `peer_sharing_consent`).
/// Gates both conversation attachment and the
/// room-transcript read.
pub(super) async fn verify_group_membership(
    resources: &Arc<ServerContext>,
    group_id: &str,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Result<(), AppError> {
    if resources
        .common
        .repos
        .groups
        .admits_member_or_coach(group_id, user_id, tenant_id)
        .await?
    {
        return Ok(());
    }
    Err(AppError::new(
        ErrorCode::PermissionDenied,
        "Cannot attach conversation to a group you don't belong to",
    ))
}

#[cfg(test)]
mod tests {
    use pierre_auth::auth::{AuthMethod as Credential, AuthResult};
    use pierre_auth::oauth2_server::first_party::WEB_CLIENT_ID;
    use pierre_core::permissions::scopes::OAuthScope;
    use serde_json::json;
    use uuid::Uuid;

    use super::refuse_external_coach_write;

    /// What a coach writes of a coached athlete from a chat route.
    const OBJECTS: [&str; 2] = ["plan", "thresholds"];

    /// A JWT's authentication naming `client_id`, holding every scope.
    fn jwt(client_id: Option<&str>) -> AuthResult {
        AuthResult {
            user_id: Uuid::new_v4(),
            auth_method: Credential::JwtToken {
                tier: "starter".to_owned(),
                impersonation: None,
            },
            active_tenant_id: None,
            scopes: OAuthScope::self_grant(),
            client_id: client_id.map(str::to_owned),
            grant_id: None,
            client_grant_id: None,
            ceiling: None,
        }
    }

    /// The middleware refuses an application's token before these routes
    /// run; were one to reach them, the coach's write to the plan (publish,
    /// discard) or to a threshold is still refused.
    #[test]
    fn an_applications_token_is_refused_the_coachs_write() {
        for object in OBJECTS {
            let refused = refuse_external_coach_write(&jwt(Some("mcp_client_test")), object)
                .expect_err("an application writes nothing of a coached athlete's");
            assert_eq!(
                refused.http_status(),
                403,
                "bug: refused as forbidden, not sent to sign in again"
            );
            assert_eq!(
                refused.details.as_deref().map(|details| &details["reason"]),
                Some(&json!("external_transport")),
                "bug: refused for the transport, in the tools' own words"
            );
            assert!(
                refused.message.contains(object),
                "bug: the refusal names the athlete's {object}: {}",
                refused.message
            );
        }
    }

    #[test]
    fn the_coachs_own_session_writes() {
        for client_id in [Some(WEB_CLIENT_ID), None] {
            for object in OBJECTS {
                assert!(
                    refuse_external_coach_write(&jwt(client_id), object).is_ok(),
                    "bug: the coach's own app session ({client_id:?}) is refused the {object} write"
                );
            }
        }
    }
}
