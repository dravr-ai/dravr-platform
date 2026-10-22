// ABOUTME: The hosted login form's submission — a metered password sign-in that continues the authorization flow
// ABOUTME: Reads the address and account windows first, counts a refused password, and redirects a sign-in to /oauth2/authorize
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use secrecy::SecretString;
use std::collections::HashMap;

use axum::{
    extract::{Form, State},
    http::{header, HeaderMap, StatusCode},
    response::{AppendHeaders, IntoResponse, Response},
};
use pierre_auth::dto::auth::LoginRequest;
use pierre_auth::oauth2_server::models::OAuth2Error;
use pierre_auth::oauth2_server::{
    known_device_cookie_name, known_device_set_cookie, SignInAttempt,
};
use pierre_auth::security::cookies::get_cookie_value;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_middleware::redaction::mask_email;
use pierre_middleware::PeerAddress;
use tracing::{error, info, warn};

use super::login_text::LoginText;
use super::{OAuth2Context, OAuth2Routes};
use crate::oauth2_rate_limited::sign_in_page_admission;

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

        // The attempt is counted in the sign-in windows before the password
        // is checked (carnet#804, carnet#908); a browser that signed in to
        // this account before is metered on its own (carnet#1088)
        let limiter = &context.rate_limiter;
        let client = peer.0.map(|peer| limiter.client_address(peer, &headers));
        let known_device = get_cookie_value(
            &headers,
            &known_device_cookie_name(Self::cookies_secure(&context)),
        );
        let text = LoginText::for_request(
            &context.strings,
            &headers,
            form.get("ui_locales").map(String::as_str),
        );
        let render = |error: &OAuth2Error| {
            Self::login_failure_response(&form, &text, &text.refusal(error), error.http_status())
        };
        let admission =
            sign_in_page_admission(limiter, client, email, known_device.as_deref(), render).await;
        let attempt = match admission {
            Ok(attempt) => attempt,
            Err(refused) => return *refused,
        };

        // The account rules every password login follows: a suspended account
        // is refused here, and the login is recorded as the web app's is
        match Self::password_sign_in(&context, email, password).await {
            Ok(token) => {
                limiter.refund_sign_in(attempt).await;
                Self::continue_to_authorize(&context, &form, email, &token)
            }
            Err(e) => Self::refused_sign_in(&context, attempt, &form, &text, &e).await,
        }
    }

    /// The page a hosted-form sign-in whose login failed answers with. A
    /// refused password keeps its hit in the sign-in windows and is answered
    /// with the generic retry page. A suspended account (checked only once
    /// its password verified) and a server fault are neither the caller's
    /// guess: the hit is given back, and the page says what happened rather
    /// than calling the password wrong — the apps sign in here (carnet#787).
    async fn refused_sign_in(
        context: &OAuth2Context,
        attempt: SignInAttempt,
        form: &HashMap<String, String>,
        text: &LoginText<'_>,
        e: &AppError,
    ) -> Response {
        if e.code == ErrorCode::AccountSuspended || e.is_server_fault() {
            context.rate_limiter.refund_sign_in(attempt).await;
        }
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
        Self::login_failure_response(
            form,
            text,
            &text.invalid_credentials(),
            StatusCode::UNAUTHORIZED,
        )
    }

    /// The redirect a signed-in hosted form continues to: the authorization
    /// flow with the OAuth parameters the form carried (PKCE and the RFC 8707
    /// resource included), carrying the authorization server's session and a
    /// fresh known-device cookie for the account (carnet#1088).
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

        let secure = Self::cookies_secure(context);
        let known_device =
            known_device_set_cookie(context.rate_limiter.issue_known_device(email), secure);
        (
            StatusCode::FOUND,
            [(header::LOCATION, auth_url)],
            AppendHeaders([
                (header::SET_COOKIE, Self::session_cookie(context, token)),
                (header::SET_COOKIE, known_device),
            ]),
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
                password: SecretString::from(password),
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
