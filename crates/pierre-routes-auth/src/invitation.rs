// ABOUTME: Axum handler for accepting an invitation — the invitee chooses a password and the account is created active
// ABOUTME: Thin wiring over AuthService::accept_invitation; the token is the authorization and is never logged

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use tracing::{error, field, field::Empty, info, warn, Span};

use pierre_auth::dto::auth::AcceptInvitationRequest;
use pierre_core::errors::AppError;
use pierre_services::auth::AuthService;

use crate::AuthRoutesContext;

/// `POST /api/auth/accept-invitation` — create the account an invitation was
/// mailed for.
///
/// Public: the single-use token from the link is the authorization, as a
/// password-reset token is for `/api/auth/complete-reset`. The account lands
/// active with `approved_by` set to the inviting operator, and the response is
/// the registration response, so the web app shows the same next step.
///
/// # Errors
///
/// `InvalidInput` for a password too weak, before the invitation is touched;
/// one uniform `NotFound` for a link that is malformed, unknown, expired,
/// already used, revoked or locked out; `PermissionDenied` when an account
/// already exists for the invited address.
#[tracing::instrument(
    skip(resources, request),
    fields(route = "accept_invitation", user_id = Empty, tenant_id = Empty, source = Empty, success = Empty)
)]
pub(crate) async fn handle_accept_invitation(
    State(resources): State<AuthRoutesContext>,
    Json(request): Json<AcceptInvitationRequest>,
) -> Result<Response, AppError> {
    let auth_service = AuthService::new(
        resources.auth_manager.clone(),
        resources.jwks_manager.clone(),
        resources.config.clone(),
        resources.data.clone(),
    );

    match auth_service
        .accept_invitation(&request.token, request.password, request.display_name)
        .await
    {
        Ok(response) => {
            Span::current().record("user_id", field::display(&response.user_id));
            Span::current().record("success", true);
            info!(
                target: "notify",
                event = "user.signed_up",
                source = "invite",
                "user account created"
            );
            Ok((StatusCode::CREATED, Json(response)).into_response())
        }
        Err(e) => {
            Span::current().record("success", false);
            if e.is_server_fault() {
                error!(error = %e, "Invitation acceptance failed");
            } else {
                warn!(error = %e, "Invitation acceptance refused");
            }
            Err(e)
        }
    }
}
