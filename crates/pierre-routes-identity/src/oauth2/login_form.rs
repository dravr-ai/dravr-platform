// ABOUTME: The hosted login form's submission — a metered password sign-in that continues the authorization flow
// ABOUTME: Reads the address and account windows first, counts a refused password, and redirects a sign-in to /oauth2/authorize
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::net::IpAddr;

use axum::{
    extract::{Form, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use pierre_auth::dto::auth::LoginRequest;
use pierre_auth::oauth2_server::models::OAuth2Error;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_middleware::redaction::mask_email;
use pierre_middleware::PeerAddress;
use tracing::{error, info, warn};

use super::login_text::LoginText;
use super::{OAuth2Context, OAuth2Routes};
use crate::oauth2_rate_limited::sign_in_page_refusal;

impl OAuth2Routes {
    /// Handle OAuth login form submission (POST /oauth2/login)
    pub(super) async fn handle_oauth_login_submit(
        State(context): State<OAuth2Context>,
        peer: PeerAddress,
        headers: HeaderMap,
        Form(form): Form<HashMap<String, String>>,
    ) -> Response {
        // Extract credentials from form
        let Some(email) = form.get("email") else {
            return (StatusCode::BAD_REQUEST, "Missing email").into_response();
        };

        let Some(password) = form.get("password") else {
            return (StatusCode::BAD_REQUEST, "Missing password").into_response();
        };

        // The sign-in windows are read before the password is checked
        // (carnet#804)
        let limiter = &context.rate_limiter;
        let client = peer.0.map(|peer| limiter.client_address(peer, &headers));
        let text = LoginText::for_request(
            &context.strings,
            &headers,
            form.get("ui_locales").map(String::as_str),
        );
        let render = |error: &OAuth2Error| {
            Self::login_failure_response(&form, &text, &text.refusal(error), error.http_status())
        };
        if let Some(refused) = sign_in_page_refusal(limiter, client, email, render).await {
            return refused;
        }

        // The account rules every password login follows: a suspended account
        // is refused here, and the login is recorded as the web app's is
        match Self::password_sign_in(&context, email, password).await {
            Ok(token) => Self::continue_to_authorize(&context, &form, email, &token),
            Err(e) => Self::refused_sign_in(&context, client, &form, &text, email, &e).await,
        }
    }

    /// The page a hosted-form sign-in whose login failed answers with. A
    /// refused password is counted in the sign-in windows and answered with
    /// the generic retry page. A suspended account (checked only once its
    /// password verified) and a server fault are neither the caller's guess:
    /// nothing is counted, and the page says what happened rather than
    /// calling the password wrong — the apps sign in here (carnet#787).
    async fn refused_sign_in(
        context: &OAuth2Context,
        client: Option<IpAddr>,
        form: &HashMap<String, String>,
        text: &LoginText<'_>,
        email: &str,
        e: &AppError,
    ) -> Response {
        if e.code == ErrorCode::AccountSuspended {
            warn!("OAuth login refused: the account is suspended");
            return Self::login_failure_response(
                form,
                text,
                &text.suspended(),
                StatusCode::BAD_REQUEST,
            );
        }
        if e.is_server_fault() {
            error!("OAuth login could not complete: {}", e);
            return Self::login_failure_response(
                form,
                text,
                &text.unavailable(),
                StatusCode::INTERNAL_SERVER_ERROR,
            );
        }
        warn!("Authentication failed for OAuth login: {}", e);
        context
            .rate_limiter
            .count_failed_sign_in(client, email)
            .await;
        Self::login_failure_response(
            form,
            text,
            &text.invalid_credentials(),
            StatusCode::UNAUTHORIZED,
        )
    }

    /// The redirect a signed-in hosted form continues to: the authorization
    /// flow with the OAuth parameters the form carried (PKCE and the RFC 8707
    /// resource included), carrying the authorization server's session.
    fn continue_to_authorize(
        context: &OAuth2Context,
        form: &HashMap<String, String>,
        email: &str,
        token: &str,
    ) -> Response {
        let auth_url = Self::build_authorization_url_from_form(form);

        info!(
            "User {} authenticated successfully for OAuth, redirecting to authorization",
            mask_email(email)
        );

        (
            StatusCode::FOUND,
            [
                (header::LOCATION, auth_url),
                (header::SET_COOKIE, Self::session_cookie(context, token)),
            ],
        )
            .into_response()
    }

    /// Sign a password in by the account rules every password login follows —
    /// a suspended account is refused, the login is recorded as the web app's
    /// is — and return the authorization server's sign-in token for it.
    async fn password_sign_in(
        context: &OAuth2Context,
        email: &str,
        password: &str,
    ) -> AppResult<String> {
        let login = context
            .accounts
            .login(LoginRequest {
                email: email.to_owned(),
                password: password.to_owned(),
                timezone: None,
            })
            .await?;
        // Every successful sign-in raises user.login, as the web app's does
        info!(
            target: "notify",
            event = "user.login",
            user_id = %login.user.user_id,
            tenant_id = %login.user.tenant_id.as_deref().unwrap_or_default(),
            "user authenticated"
        );
        Self::authorization_session(context, &login).await
    }
}
