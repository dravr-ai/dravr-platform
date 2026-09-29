// ABOUTME: "Continue with Google" on the hosted OAuth login page — straight to Google's account chooser and back to consent
// ABOUTME: The pending authorization and the sign-in's secrets ride a sealed, browser-bound cookie; the return is rebuilt from it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Server-side Google sign-in for the authorization server.
//!
//! `GET /oauth2/login/google?<authorize params>` checks the pending
//! authorization request, begins a Google `OpenID` Connect sign-in, seals the
//! sign-in's `state`, `nonce`, PKCE verifier and the request into a short-lived
//! cookie bound to this browser, and sends the athlete to Google's account
//! chooser. `GET /oauth2/login/google/callback` opens that cookie, checks the
//! `state` Google returned against it, exchanges the code, verifies the ID
//! token, signs the athlete in by the account rules the web app follows, sets
//! the session cookie and returns to `/oauth2/authorize` for the sealed
//! request — rebuilt here, never read from the callback's query, so the return
//! cannot be redirected elsewhere.
//!
//! The cookie is what makes a callback belong to the browser that started it
//! (login CSRF): a callback carrying another sign-in's `state` is refused
//! before Google is called. It is `SameSite=Lax`, since Google's redirect back
//! is a cross-site top-level navigation that `Strict` would not send it on.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{ConnectInfo, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::Utc;
use pierre_auth::google_oidc::{
    google_callback_url, GoogleIdentity, GoogleOidcClient, GOOGLE_PROVIDER, GOOGLE_SIGN_IN_TTL,
};
use pierre_auth::oauth2_server::models::{AuthorizeRequest, OAuth2Error};
use pierre_auth::security::cookies::{
    get_cookie_value, host_cookie_name, SameSitePolicy, SecureCookieConfig,
};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_database::database::repositories::SecurityRepository;
use pierre_middleware::redaction::mask_email;
use pierre_services::auth::{FederatedIdentity, SignupSource};
use serde::{Deserialize, Serialize};
use tracing::{error, info, warn};

use super::{OAuth2Context, OAuth2Routes};
use crate::authorize_redirect::rejection_response;
use crate::oauth2_rate_limited::page_refusal;

/// Name of the transaction cookie, before the `__Host-` prefix an HTTPS
/// issuer gives it
const TRANSACTION_COOKIE: &str = "pierre_google_sign_in";

/// Additional authenticated data the transaction is sealed under, so no
/// other sealed value of this server opens as one
const TRANSACTION_AAD: &str = "oauth2:google-sign-in:v1";

/// Largest sealed transaction set as a cookie: browsers drop a cookie past
/// 4096 bytes, name and attributes included
const MAX_SEALED_TRANSACTION_BYTES: usize = 3800;

/// The refusal shown for any sign-in Google or the account rules turned
/// down; the reason stays in the log
const SIGN_IN_REFUSED: &str = "Google sign-in could not be completed for this account.";

/// The refusal shown for a callback this browser did not start, or started
/// too long ago
const TRANSACTION_REFUSED: &str =
    "This Google sign-in expired or was started in another browser. Start again from the app you are connecting.";

/// Google sign-in on the hosted login page, as the routes hold it.
#[derive(Clone)]
pub struct GoogleSignIn {
    /// Shared so Google's signing-key cache survives across sign-ins
    pub oidc: Arc<GoogleOidcClient>,
    /// AES-256-GCM sealing of the browser-bound transaction cookie
    pub security: Arc<dyn SecurityRepository>,
}

/// One sign-in between the start and the callback, sealed into the cookie.
#[derive(Serialize, Deserialize)]
struct GoogleSignInTransaction {
    /// The `state` Google must return
    state: String,
    /// The `nonce` the ID token must carry
    nonce: String,
    /// The PKCE verifier the code exchange proves
    code_verifier: String,
    /// Unix seconds after which the transaction is refused
    expires_at: i64,
    /// The authorization request the athlete returns to
    request: AuthorizeRequest,
}

/// What the callback's transaction cookie turned out to hold.
enum OpenedTransaction {
    /// A live transaction for this callback's `state`
    Live(Box<GoogleSignInTransaction>),
    /// A transaction this browser holds, past its lifetime
    Expired,
    /// None this server sealed, or another sign-in's
    Refused,
}

impl GoogleSignIn {
    /// Seal `transaction` for the cookie.
    fn seal(&self, transaction: &GoogleSignInTransaction) -> AppResult<String> {
        let json = serde_json::to_string(transaction)
            .map_err(|e| AppError::internal(format!("Google sign-in transaction: {e}")))?;
        self.security.encrypt_data_with_aad(&json, TRANSACTION_AAD)
    }

    /// Open the transaction cookie `headers` carry under `cookie_name`, and
    /// judge it against the callback's `state`.
    fn open(
        &self,
        headers: &HeaderMap,
        cookie_name: &str,
        state: Option<&str>,
    ) -> OpenedTransaction {
        let Some(sealed) = get_cookie_value(headers, cookie_name) else {
            return OpenedTransaction::Refused;
        };
        let Some(transaction) = self
            .security
            .decrypt_data_with_aad(&sealed, TRANSACTION_AAD)
            .ok()
            .and_then(|json| serde_json::from_str::<GoogleSignInTransaction>(&json).ok())
        else {
            return OpenedTransaction::Refused;
        };
        if transaction.expires_at <= Utc::now().timestamp() {
            return OpenedTransaction::Expired;
        }
        if state != Some(transaction.state.as_str()) {
            return OpenedTransaction::Refused;
        }
        OpenedTransaction::Live(Box::new(transaction))
    }
}

impl OAuth2Routes {
    /// `GET /oauth2/login/google`: begin a Google sign-in for the pending
    /// authorization request the query carries.
    pub(super) async fn handle_google_login_start(
        State(context): State<OAuth2Context>,
        ConnectInfo(addr): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
        Query(params): Query<HashMap<String, String>>,
    ) -> Response {
        let render = Self::render_oauth_error_response;
        if let Some(refused) = page_refusal(&context.rate_limiter, addr, &headers, render).await {
            return no_store(refused);
        }
        let Some(google) = context.google_sign_in.as_ref() else {
            return no_store(Self::google_unavailable());
        };

        let request = match Self::parse_authorize_request(&params) {
            Ok(request) => request,
            Err(error) => return no_store(Self::render_oauth_error_response(&error)),
        };
        // Only a request this server would authorize is carried to Google
        // and back: an unknown client or unregistered redirect_uri is refused
        // here, before the athlete chooses an account.
        if let Err(rejection) = Self::authorization_server(&context)
            .check_authorize_request(&request)
            .await
        {
            return no_store(rejection_response(rejection, &request));
        }

        let redirect_uri = google_callback_url(&context.config.issuer_url);
        let start = match google.oidc.begin(&redirect_uri) {
            Ok(start) => start,
            Err(e) => return no_store(Self::google_refusal(&e)),
        };
        let expires_at = Utc::now().timestamp()
            + i64::try_from(GOOGLE_SIGN_IN_TTL.as_secs()).unwrap_or(i64::MAX);
        let transaction = GoogleSignInTransaction {
            state: start.state,
            nonce: start.nonce,
            code_verifier: start.code_verifier,
            expires_at,
            request,
        };
        let sealed = match google.seal(&transaction) {
            Ok(sealed) => sealed,
            Err(e) => return no_store(Self::google_refusal(&e)),
        };
        if sealed.len() > MAX_SEALED_TRANSACTION_BYTES {
            warn!(
                sealed_bytes = sealed.len(),
                "Google sign-in refused: the authorization request is too large for its cookie"
            );
            return no_store(Self::render_oauth_error_response(
                &OAuth2Error::invalid_request("This sign-in request is too large"),
            ));
        }

        let cookie = Self::transaction_cookie(&context, sealed, GOOGLE_SIGN_IN_TTL.as_secs());
        let mut response = StatusCode::SEE_OTHER.into_response();
        set_header(&mut response, header::LOCATION, &start.authorization_url);
        append_cookie(&mut response, &cookie);
        no_store(response)
    }

    /// `GET /oauth2/login/google/callback`: finish the Google sign-in this
    /// browser began, and return to the authorization request it carried.
    pub(super) async fn handle_google_login_callback(
        State(context): State<OAuth2Context>,
        ConnectInfo(addr): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
        Query(params): Query<HashMap<String, String>>,
    ) -> Response {
        let render = Self::render_oauth_error_response;
        if let Some(refused) = page_refusal(&context.rate_limiter, addr, &headers, render).await {
            return no_store(refused);
        }
        let Some(google) = context.google_sign_in.as_ref() else {
            return no_store(Self::google_unavailable());
        };

        let secure = Self::cookies_secure(&context);
        let cookie_name = host_cookie_name(TRANSACTION_COOKIE, secure);
        let clear = Self::transaction_cookie(&context, String::new(), 0);
        let transaction = match google.open(
            &headers,
            &cookie_name,
            params.get("state").map(String::as_str),
        ) {
            OpenedTransaction::Live(transaction) => transaction,
            OpenedTransaction::Expired => {
                warn!("Google sign-in refused: the transaction expired");
                let mut response = Self::render_oauth_error_response(
                    &OAuth2Error::invalid_request(TRANSACTION_REFUSED),
                );
                append_cookie(&mut response, &clear);
                return no_store(response);
            }
            OpenedTransaction::Refused => {
                // The cookie is left alone: it may be another tab's
                // sign-in, still in flight.
                warn!(
                    "Google sign-in refused: no transaction of this browser matches the callback"
                );
                return no_store(Self::render_oauth_error_response(
                    &OAuth2Error::invalid_request(TRANSACTION_REFUSED),
                ));
            }
        };

        // The transaction is spent from here on, whatever the outcome.
        let mut response =
            Self::finish_google_sign_in(&context, google, &params, &transaction).await;
        append_cookie(&mut response, &clear);
        no_store(response)
    }

    /// The callback's answer for a live transaction: back to the login page
    /// when the athlete cancelled at Google, a refusal page, or the session
    /// cookie and the return to `/oauth2/authorize`.
    async fn finish_google_sign_in(
        context: &OAuth2Context,
        google: &GoogleSignIn,
        params: &HashMap<String, String>,
        transaction: &GoogleSignInTransaction,
    ) -> Response {
        let request = &transaction.request;
        if let Some(error) = params.get("error") {
            return Self::google_error_answer(error, request);
        }
        let Some(code) = params.get("code").filter(|code| !code.is_empty()) else {
            return Self::render_oauth_error_response(&OAuth2Error::invalid_request(
                "Google returned no authorization code",
            ));
        };

        // The client may have been deleted while the athlete was at Google.
        if let Err(rejection) = Self::authorization_server(context)
            .check_authorize_request(request)
            .await
        {
            return rejection_response(rejection, request);
        }

        let redirect_uri = google_callback_url(&context.config.issuer_url);
        let identity =
            match Self::verified_google_identity(google, code, &redirect_uri, transaction).await {
                Ok(identity) => identity,
                Err(e) => return Self::google_refusal(&e),
            };

        let token = match Self::google_session(context, &identity).await {
            Ok(token) => token,
            Err(e) => return Self::google_refusal(&e),
        };

        info!(
            "User {} signed in with Google for OAuth, redirecting to authorization",
            mask_email(&identity.email)
        );
        let mut response = StatusCode::SEE_OTHER.into_response();
        set_header(
            &mut response,
            header::LOCATION,
            &Self::authorize_params_url("/oauth2/authorize", request),
        );
        append_cookie(&mut response, &Self::session_cookie(context, &token));
        response
    }

    /// The answer to Google returning `error` instead of a code: back to
    /// the login page when the athlete cancelled at the chooser, a refusal
    /// page otherwise.
    fn google_error_answer(error: &str, request: &AuthorizeRequest) -> Response {
        if error == "access_denied" {
            info!("Google sign-in cancelled by the athlete; back to the login page");
            let mut response = StatusCode::SEE_OTHER.into_response();
            set_header(
                &mut response,
                header::LOCATION,
                &Self::build_login_url_with_oauth_params(request),
            );
            return response;
        }
        let code = if error.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
            error
        } else {
            "unrecognized"
        };
        warn!(error = code, "Google sign-in refused by Google");
        Self::render_oauth_error_response(&OAuth2Error::access_denied(SIGN_IN_REFUSED))
    }

    /// Sign the proven Google identity in by the account rules the web app
    /// follows, and return its session JWT.
    async fn google_session(
        context: &OAuth2Context,
        identity: &GoogleIdentity,
    ) -> AppResult<String> {
        let login = context
            .accounts
            .login_with_federated_identity(FederatedIdentity {
                firebase_uid: None,
                google_subject: Some(&identity.subject),
                email: identity.email.clone(),
                email_verified: true,
                display_name: identity.name.as_deref(),
                provider: GOOGLE_PROVIDER,
                signup_source: SignupSource::Google,
            })
            .await?;
        login
            .jwt_token
            .ok_or_else(|| AppError::internal("Google sign-in minted no session token"))
    }

    /// Exchange the code over the back channel and verify the ID token it
    /// yields against this sign-in's nonce.
    async fn verified_google_identity(
        google: &GoogleSignIn,
        code: &str,
        redirect_uri: &str,
        transaction: &GoogleSignInTransaction,
    ) -> AppResult<GoogleIdentity> {
        let id_token = google
            .oidc
            .exchange_code(code, &transaction.code_verifier, redirect_uri)
            .await?;
        google
            .oidc
            .verify_id_token(&id_token, &transaction.nonce)
            .await
    }

    /// The page for a sign-in that failed: a server fault is logged at
    /// ERROR and shown as such; anything else is a refusal of this account,
    /// logged at WARN. Neither page carries the reason.
    fn google_refusal(error: &AppError) -> Response {
        if error.is_server_fault() {
            error!("Google sign-in could not complete: {error}");
            let shown = if error.code == ErrorCode::ExternalServiceError
                || error.code == ErrorCode::ExternalServiceUnavailable
            {
                OAuth2Error::temporarily_unavailable("Google sign-in is unavailable right now")
            } else {
                OAuth2Error::server_error("Google sign-in could not be completed")
            };
            return Self::render_oauth_error_response(&shown);
        }
        warn!("Google sign-in refused: {error}");
        Self::render_oauth_error_response(&OAuth2Error::access_denied(SIGN_IN_REFUSED))
    }

    /// The page both routes answer when Google sign-in is not configured.
    fn google_unavailable() -> Response {
        Self::render_oauth_error_response(&OAuth2Error::invalid_request(
            "Google sign-in is not available on this server",
        ))
    }

    /// The transaction cookie carrying `value` for `max_age_secs`; an empty
    /// value with no lifetime clears it.
    fn transaction_cookie(context: &OAuth2Context, value: String, max_age_secs: u64) -> String {
        let secure = Self::cookies_secure(context);
        SecureCookieConfig {
            name: host_cookie_name(TRANSACTION_COOKIE, secure),
            value,
            max_age_secs: i64::try_from(max_age_secs).unwrap_or(i64::MAX),
            http_only: true,
            secure,
            same_site: SameSitePolicy::Lax,
            path: "/".to_owned(),
        }
        .build()
    }
}

/// Set `name` to `value` on `response`; a value no header can carry is left out.
fn set_header(response: &mut Response, name: header::HeaderName, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        response.headers_mut().insert(name, value);
    }
}

/// Add one more `Set-Cookie` to `response`.
fn append_cookie(response: &mut Response, cookie: &str) {
    if let Ok(value) = HeaderValue::from_str(cookie) {
        response.headers_mut().append(header::SET_COOKIE, value);
    }
}

/// Keep `response` out of every cache: it carries a sign-in's cookie or
/// its outcome.
fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
