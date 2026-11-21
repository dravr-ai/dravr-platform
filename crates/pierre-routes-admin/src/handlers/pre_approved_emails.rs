// ABOUTME: Admin route handlers for the standing allow-list of pre-approved email addresses
// ABOUTME: List, allow (approving a pending account on the spot, optionally inviting) and remove
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Extension, Json,
};
use serde_json::{json, to_value, Value};
use tracing::info;

use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::errors::AppResult;
use pierre_middleware::redaction::mask_email;
use pierre_services::admin_ops::AccountCaller;
use pierre_services::pre_approval;

use super::admin_scope::require_platform_reach;
use super::api_keys::json_response;
use super::invitations::{self, send_invite_if_eligible};
use super::types::{AdminResponse, AllowEmailRequest};
use super::users::{announce_approval, deny_without_manage_users};
use crate::audit::AuditDetail;
use crate::context::AdminApiContext;

/// `GET /admin/pre-approved-emails` — the standing allow-list.
///
/// A caller bound to one tenant is refused (403): the list names every
/// address the platform lets in, whatever tenant its account is or will be
/// in ([`require_platform_reach`]).
pub(crate) async fn handle_list_pre_approved_emails(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_without_manage_users(&admin_token) {
        return Ok(denied.into_response());
    }
    require_platform_reach(&admin_token, "the allow-list is", "read it")?;

    let entries = pre_approval::list(&context.repos).await?;

    info!(
        count = entries.len(),
        token_id = %admin_token.token_id,
        "Pre-approved emails listed"
    );

    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("{} pre-approved email(s)", entries.len()),
            data: to_value(json!({
                "emails": entries,
                "total": entries.len(),
            }))
            .ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}

/// `POST /admin/pre-approved-emails` — pre-approve one address.
///
/// The address needs no account: that is what a standing allow is for. When an
/// account does exist and is pending, the allow approves it now, and the user
/// is notified through the same announcement path as `POST /admin/approve-user`.
///
/// A caller bound to one tenant is refused (403) before anything is written
/// ([`require_platform_reach`]).
pub(crate) async fn handle_allow_email(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Json(request): Json<AllowEmailRequest>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_without_manage_users(&admin_token) {
        return Ok(denied.into_response());
    }
    require_platform_reach(&admin_token, "a pre-approval is", "allow one")?;

    let ctx = context.as_ref();
    // Refused before anything is written: a link answered to the caller is
    // super-admin only (see the invitations module).
    let delivery = match invitations::delivery(ctx, &request, &admin_token) {
        Ok(delivery) => delivery,
        Err(refused) => return Ok(*refused),
    };
    // An allow approves a pending account on the spot, so it takes the rank
    // approving one by id does: a token without the flag approves no admin
    // (`pre_approval::allow` reads the account and refuses before anything
    // else). The allow is recorded against the admin behind the token —
    // the approving admin of a device login, the admin of a console
    // session. A service token names no person, so `allowed_by` stays NULL
    // rather than attributing its allow to some arbitrary admin account.
    let caller = AccountCaller::of_token(&admin_token);
    let result =
        pre_approval::allow(&ctx.repos, &caller, &request.email, request.note.as_deref()).await?;

    if let Some(approved) = result.approved_user.as_ref() {
        announce_approval(
            ctx,
            &approved.id.to_string(),
            approved.id,
            &approved.email,
            approved.display_name.as_deref(),
            caller.name(),
        )
        .await;
    }

    let invite =
        send_invite_if_eligible(ctx, &result, request.send_invite, &delivery, &admin_token).await?;
    let invited = invite.invited();
    let recipient = mask_email(&result.email);

    info!(
        %recipient,
        outcome = ?result.outcome,
        send_invite = request.send_invite,
        invited,
        token_id = %admin_token.token_id,
        "Pre-approval allow recorded"
    );

    // The address is in the body, which the audit recorder never reads.
    let audit = AuditDetail::new(
        "pre_approval.allow",
        format!("Pre-approved {}", result.email),
    )
    .target(json!({
        "email": result.email,
        "outcome": format!("{:?}", result.outcome),
        "invited": invited,
    }));
    Ok(audit.attach(
        json_response(
            AdminResponse {
                success: true,
                message: format!("{}{}", result.message(), invite.sentence()),
                data: Some(allow_data(&result, &invite)),
            },
            StatusCode::OK,
        )
        .into_response(),
    ))
}

/// The allow's answer body; `invite_url` appears only when the link was asked
/// for and issued.
fn allow_data(result: &pre_approval::AllowResult, invite: &invitations::InviteReport) -> Value {
    let mut data = json!({
        "email": result.email,
        "outcome": result.outcome,
        "approved_user_id": result.approved_user.as_ref().map(|u| u.id.to_string()),
        "invited": invite.invited(),
        "invitation_expires_at": invite.expires_at().map(|t| t.to_rfc3339()),
    });
    if let (Some(url), Some(fields)) = (invite.invite_url(), data.as_object_mut()) {
        fields.insert("invite_url".to_owned(), Value::String(url.to_owned()));
    }
    data
}

/// `DELETE /admin/pre-approved-emails/{email}` — drop a standing allow, and
/// revoke the invitation link sent with it.
///
/// An account that already registered against the address keeps whatever
/// status it holds; removing the allow only stops a *future* registration from
/// skipping the queue, or an outstanding invitation from creating one.
///
/// A caller bound to one tenant is refused (403), the allow left standing
/// ([`require_platform_reach`]).
pub(crate) async fn handle_disallow_email(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(email): Path<String>,
) -> AppResult<impl IntoResponse> {
    if let Some(denied) = deny_without_manage_users(&admin_token) {
        return Ok(denied.into_response());
    }
    require_platform_reach(&admin_token, "a pre-approval is", "remove one")?;

    let result = pre_approval::disallow(&context.repos, &email).await?;

    info!(
        removed = result.removed,
        token_id = %admin_token.token_id,
        "Pre-approval removal processed"
    );

    // Said plainly: removing a pre-approval never deletes an account, and an
    // admin once read this action as a user delete (carnet#1391).
    let audit = AuditDetail::new(
        "pre_approval.remove",
        format!(
            "Removed the pre-approval of {} (no account deleted)",
            result.email
        ),
    )
    .target(json!({
        "email": result.email,
        "removed": result.removed,
        "account_status": result.account_status,
    }));
    Ok(audit.attach(
        json_response(
            AdminResponse {
                success: true,
                message: result.message(),
                data: to_value(json!({
                    "email": result.email,
                    "removed": result.removed,
                    "invitation_revoked": result.invitation_revoked,
                    "account_status": result.account_status,
                }))
                .ok(),
            },
            StatusCode::OK,
        )
        .into_response(),
    ))
}
