// ABOUTME: Admin handlers for coach-seat invitations — issue, list and revoke the email-bound invite that carries coach access (carnet#1524)
// ABOUTME: Super-admin only, like the coach-access queue the console shows them in; an address with an account gets the direct grant instead

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The console's coach-seat invitations.
//!
//! `GET /admin/coach-seat-invitations`, `POST /admin/coach-seat-invitations`
//! and `DELETE /admin/coach-seat-invitations/{email}`, mounted for the admin
//! token and, under `/api/admin/…`, for the console session.
//!
//! A coach-seat invitation is the decision ADR-018 leaves to a super-admin —
//! who may coach — taken before the coach signs up instead of after a
//! request: the single-use mailed link of every invitation, locked to the
//! address, whose redemption also grants coach access and records the coach
//! role (`pierre_services::invitation::settle_coach_seat`). So every route
//! here is super-admin only, as the coach-access queue's are.
//!
//! Only an address with no account is invited. For one that already has an
//! account the answer says so and names it, so the console offers the direct
//! grant (`POST /admin/users/{id}/manages-roster`) instead of a link that
//! could never be redeemed. Sending again replaces the earlier link, which is
//! how an operator re-sends an expired one.

use std::sync::Arc;

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Extension, Json,
};
use serde::Deserialize;
use serde_json::{json, to_value, Value};
use tracing::info;

use pierre_core::admin::models::ValidatedAdminToken;
use pierre_core::errors::AppResult;
use pierre_core::models::{normalize_email, InvitationKind};
use pierre_middleware::redaction::mask_email;
use pierre_services::invitation::{self, INVITATIONS_PER_ADDRESS_PER_HOUR};

use super::api_keys::json_response;
use super::invitations::{issue_and_deliver, Carrier, InviteReport};
use super::strava_pool::deny_if_not_super_admin;
use super::types::AdminResponse;
use super::user_removal::refusal;
use crate::audit::AuditDetail;
use crate::context::AdminApiContext;

/// Body of `POST /admin/coach-seat-invitations`.
#[derive(Debug, Deserialize)]
pub(crate) struct CoachSeatInviteRequest {
    /// The coach's address; normalized server-side.
    pub email: String,
    /// Answer the invitation's link instead of mailing it, for operator-driven
    /// testing on an environment whose test addresses have no mailbox, as the
    /// pre-approval allow's `return_link` does.
    #[serde(default)]
    pub return_link: bool,
}

/// `GET /admin/coach-seat-invitations` — every coach-seat invitation neither
/// redeemed nor revoked, oldest first, expired ones included so the console
/// can send them again.
pub(crate) async fn handle_list_coach_seat_invitations(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
) -> AppResult<Response> {
    if let Some(denied) = deny_if_not_super_admin(&admin_token) {
        return Ok(denied);
    }
    let invitations = invitation::list_coach_seat_invitations(&context.repos).await?;
    info!(
        count = invitations.len(),
        token_id = %admin_token.token_id,
        "Coach-seat invitations listed"
    );
    Ok(json_response(
        AdminResponse {
            success: true,
            message: format!("{} coach-seat invitation(s)", invitations.len()),
            data: to_value(json!({
                "invitations": invitations,
                "total": invitations.len(),
            }))
            .ok(),
        },
        StatusCode::OK,
    )
    .into_response())
}

/// `POST /admin/coach-seat-invitations` — invite a coach the team recruited,
/// with their seat, or say that the address already has an account.
pub(crate) async fn handle_issue_coach_seat_invitation(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Json(request): Json<CoachSeatInviteRequest>,
) -> AppResult<Response> {
    if let Some(denied) = deny_if_not_super_admin(&admin_token) {
        return Ok(denied);
    }
    let ctx = context.as_ref();
    let email = normalize_email(&request.email);

    if let Some(existing) = account_exists(ctx, &email).await? {
        return Ok(existing);
    }

    let carrier = if request.return_link {
        let Some(frontend_url) = ctx.frontend_url.as_deref() else {
            return Ok(refusal(
                StatusCode::SERVICE_UNAVAILABLE,
                "This server has no frontend URL to build an invitation link on".to_owned(),
                None,
            ));
        };
        Carrier::Operator { frontend_url }
    } else {
        let Some(notifier) = ctx.approval_notifier.as_ref() else {
            return Ok(refusal(
                StatusCode::SERVICE_UNAVAILABLE,
                "No invitation sent: this server has no mailer wired".to_owned(),
                None,
            ));
        };
        Carrier::Notifier(notifier.as_ref())
    };

    let report = issue_and_deliver(
        ctx,
        carrier,
        &email,
        &admin_token,
        InvitationKind::CoachSeat,
    )
    .await?;
    info!(
        recipient = %mask_email(&email),
        invited = report.invited(),
        token_id = %admin_token.token_id,
        "Coach-seat invitation requested"
    );

    match &report {
        InviteReport::Sent { expires_at } | InviteReport::Link { expires_at, .. } => {
            let expires_at = *expires_at;
            let mut data = json!({
                "outcome": "invited",
                "email": email,
                "expires_at": expires_at.to_rfc3339(),
            });
            if let (Some(url), Some(fields)) = (report.invite_url(), data.as_object_mut()) {
                fields.insert("invite_url".to_owned(), Value::String(url.to_owned()));
            }
            let message = if report.invite_url().is_some() {
                format!(
                    "Coach-seat invitation link issued, not mailed: it works once, until {}",
                    expires_at.format("%Y-%m-%d %H:%M UTC")
                )
            } else {
                format!(
                    "Coach-seat invitation sent to {email}: its link works once, until {}",
                    expires_at.format("%Y-%m-%d %H:%M UTC")
                )
            };
            // The address is in the body, which the audit recorder never reads.
            let audit = AuditDetail::new(
                "coach_seat_invitation.issue",
                format!("Invited {email} with a coach seat"),
            )
            .target(json!({ "email": email }));
            Ok(audit.attach(
                json_response(
                    AdminResponse {
                        success: true,
                        message,
                        data: Some(data),
                    },
                    StatusCode::OK,
                )
                .into_response(),
            ))
        }
        // The address registered between the lookup above and the issue.
        InviteReport::AccountExists => {
            Ok(account_exists(ctx, &email).await?.unwrap_or_else(|| {
                refusal(
                    StatusCode::CONFLICT,
                    "This address already has an account".to_owned(),
                    None,
                )
            }))
        }
        InviteReport::BudgetSpent => Ok(refusal(
            StatusCode::TOO_MANY_REQUESTS,
            format!(
                "No invitation sent: {INVITATIONS_PER_ADDRESS_PER_HOUR} already went to this \
                 address in the last hour"
            ),
            None,
        )),
        InviteReport::NoNotifier => Ok(refusal(
            StatusCode::SERVICE_UNAVAILABLE,
            "No invitation sent: this server has no mailer wired".to_owned(),
            None,
        )),
        InviteReport::NotDelivered => Ok(refusal(
            StatusCode::BAD_GATEWAY,
            "No invitation sent: its email could not be delivered, so the link was withdrawn"
                .to_owned(),
            None,
        )),
        InviteReport::NotAsked => Ok(refusal(
            StatusCode::INTERNAL_SERVER_ERROR,
            "No invitation was issued".to_owned(),
            None,
        )),
    }
}

/// The answer for an address that already has an account, naming it and
/// whether it already holds coach access, so the console offers the direct
/// grant; `None` when the address has no account.
///
/// # Errors
///
/// Returns the repository's error when the lookup fails.
async fn account_exists(ctx: &AdminApiContext, email: &str) -> AppResult<Option<Response>> {
    let Some(user) = ctx.repos.users.get_by_email(email).await? else {
        return Ok(None);
    };
    info!(
        recipient = %mask_email(email),
        "Coach-seat invitation not issued: the address already has an account"
    );
    let message = if user.manages_roster {
        "This address already has an account, and it already holds coach access".to_owned()
    } else {
        "This address already has an account: grant it coach access directly instead".to_owned()
    };
    Ok(Some(
        json_response(
            AdminResponse {
                success: true,
                message,
                data: Some(json!({
                    "outcome": "account_exists",
                    "email": user.email,
                    "user_id": user.id.to_string(),
                    "display_name": user.display_name,
                    "manages_roster": user.manages_roster,
                })),
            },
            StatusCode::OK,
        )
        .into_response(),
    ))
}

/// `DELETE /admin/coach-seat-invitations/{email}` — revoke the coach-seat
/// invitation an address holds, so its link stops working.
pub(crate) async fn handle_revoke_coach_seat_invitation(
    State(context): State<Arc<AdminApiContext>>,
    Extension(admin_token): Extension<ValidatedAdminToken>,
    Path(email): Path<String>,
) -> AppResult<Response> {
    if let Some(denied) = deny_if_not_super_admin(&admin_token) {
        return Ok(denied);
    }
    let email = normalize_email(&email);
    let revoked = invitation::revoke_coach_seat(&context.repos, &email).await?;
    info!(
        recipient = %mask_email(&email),
        revoked,
        token_id = %admin_token.token_id,
        "Coach-seat invitation revocation processed"
    );
    let message = if revoked {
        format!("Coach-seat invitation for {email} revoked: its link no longer works")
    } else {
        format!("{email} holds no coach-seat invitation to revoke")
    };
    let audit = AuditDetail::new(
        "coach_seat_invitation.revoke",
        format!("Revoked the coach-seat invitation of {email}"),
    )
    .target(json!({ "email": email, "revoked": revoked }));
    Ok(audit.attach(
        json_response(
            AdminResponse {
                success: true,
                message,
                data: Some(json!({ "email": email, "revoked": revoked })),
            },
            StatusCode::OK,
        )
        .into_response(),
    ))
}
