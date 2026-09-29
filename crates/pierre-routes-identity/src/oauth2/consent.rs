// ABOUTME: The OAuth consent form's submission — the athlete's decision on an authorization request
// ABOUTME: An approval proves the form's synchronizer token, records the grant and mints the code; a denial reaches the client
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use axum::{
    extract::{Form, State},
    http::HeaderMap,
    response::{IntoResponse, Redirect, Response},
};
use chrono::Utc;
use pierre_auth::oauth2_server::models::OAuth2Error;
use pierre_core::models::{OAuthClientGrant, User, UserStatus};
use tracing::{error, warn};
use uuid::Uuid;

use super::{OAuth2Context, OAuth2Routes};
use crate::authorize_redirect::{error_redirect, rejection_response};

impl OAuth2Routes {
    /// Handle the OAuth consent form submission (POST /oauth2/consent).
    ///
    /// On approval, records a durable grant (so future authorizations for the same
    /// client + scope skip the prompt) and mints the authorization code. On denial,
    /// returns `access_denied` to the client's registered redirect URI.
    pub(super) async fn handle_consent_submit(
        State(context): State<OAuth2Context>,
        headers: HeaderMap,
        Form(form): Form<HashMap<String, String>>,
    ) -> Response {
        let request = match Self::parse_authorize_request(&form) {
            Ok(req) => req,
            Err(error) => return Self::render_oauth_error_response(&error),
        };
        // The form echoes the request the consent screen was rendered for, so
        // it is checked again: only a verified client and redirect_uri may be
        // redirected to, whether the user approves or denies.
        if let Err(rejection) = Self::authorization_server(&context)
            .check_authorize_request(&request)
            .await
        {
            return rejection_response(rejection, &request);
        }
        let redirect_uri = request.redirect_uri.clone();

        // The consent decision must belong to the authenticated user.
        let (user_id, tenant_id) = Self::extract_authenticated_user(&headers, &context);
        let Some(user_id) = user_id else {
            // Session lapsed between render and submit — restart through login.
            return Redirect::to(&Self::build_login_url_with_oauth_params(&request))
                .into_response();
        };

        let approved = form
            .get("decision")
            .is_some_and(|decision| decision == "approve");
        if !approved {
            return error_redirect(
                &redirect_uri,
                request.state.as_deref(),
                &OAuth2Error::access_denied("The user denied the authorization request"),
            );
        }

        // An approval must come from the consent screen this server rendered
        // for this user, not a form another site posts with the user's cookie;
        // a denial needs no proof, and must reach the client (RFC 6749 §4.1.2.1).
        let form_token = form.get("csrf_token").map_or("", String::as_str);
        if let Err(e) = context.csrf_manager.validate_token(form_token, user_id) {
            warn!(user_id = %user_id, "OAuth consent refused: {e}");
            return Self::render_oauth_error_response(&OAuth2Error::access_denied(
                "The consent screen expired or did not come from Dravr; start the connection again",
            ));
        }

        // The account may have been suspended since the screen was rendered.
        if let Err(refused) = Self::account_gate(&context, user_id).await {
            return *refused;
        }

        // Record the grant; a duplicate active grant is a no-op at the storage layer.
        let scope = request.scope.clone().unwrap_or_default();
        if let Some(tid) = Self::resolve_grant_tenant(&context, user_id, tenant_id.clone()).await {
            let grant = OAuthClientGrant {
                id: Uuid::new_v4().to_string(),
                user_id: user_id.to_string(),
                tenant_id: tid,
                client_id: request.client_id.clone(),
                scope,
                granted_at: Utc::now(),
                revoked_at: None,
            };
            if let Err(e) = context.oauth2_server.store_client_grant(&grant).await {
                error!("Failed to persist OAuth client grant: {e}");
            }
        }

        Self::mint_authorization_code(&context, request, user_id, tenant_id, redirect_uri).await
    }

    /// The signed-in account, when it may authorize a client: it exists and
    /// is active. A pending account is told it awaits approval and a
    /// suspended one is refused; neither is shown consent nor given a code.
    pub(super) async fn account_gate(
        context: &OAuth2Context,
        user_id: Uuid,
    ) -> Result<User, Box<Response>> {
        let refuse = |error: OAuth2Error| Box::new(Self::render_oauth_error_response(&error));
        let user = Self::signed_in_account(context, user_id)
            .await
            .map_err(refuse)?;
        Self::account_refusal(&user).map_or_else(|| Ok(user), |error| Err(refuse(error)))
    }

    /// The account a session names.
    async fn signed_in_account(
        context: &OAuth2Context,
        user_id: Uuid,
    ) -> Result<User, OAuth2Error> {
        match context.users.get_global(user_id).await {
            Ok(Some(user)) => Ok(user),
            Ok(None) => {
                warn!(user_id = %user_id, "OAuth authorization refused: the session names no account");
                Err(OAuth2Error::access_denied(
                    "The account you are signed in as no longer exists; sign in again",
                ))
            }
            Err(e) => {
                error!(user_id = %user_id, "OAuth authorization could not load the account: {e}");
                Err(OAuth2Error::server_error("The account could not be loaded"))
            }
        }
    }

    /// Why `user` may not authorize a client, if it may not.
    fn account_refusal(user: &User) -> Option<OAuth2Error> {
        match user.user_status {
            UserStatus::Active => None,
            UserStatus::Pending => {
                warn!(user_id = %user.id, "OAuth authorization refused: the account awaits approval");
                Some(OAuth2Error::access_denied(
                    "Your Dravr account is waiting for approval. Connect this app again once it is approved.",
                ))
            }
            UserStatus::Suspended => {
                warn!(user_id = %user.id, "OAuth authorization refused: the account is suspended");
                Some(OAuth2Error::access_denied(
                    "Your Dravr account is suspended",
                ))
            }
        }
    }
}
