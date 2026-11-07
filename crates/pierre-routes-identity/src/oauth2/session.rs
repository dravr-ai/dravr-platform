// ABOUTME: The authorization server's sign-in cookie: its name, its Set-Cookie value, and reading it back
// ABOUTME: Also accepts the web app's session cookie as a bridge, each with the audience it carries
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_auth::dto::auth::LoginResponse;
use pierre_auth::security::cookies::{host_cookie_name, SameSitePolicy, SecureCookieConfig};
use pierre_core::errors::{AppError, AppResult};

use super::{OAuth2Context, OAuth2Routes};

/// Name of the authorization server's own session cookie, before the
/// `__Host-` prefix an HTTPS issuer gives it
const SESSION_COOKIE: &str = "pierre_session";

/// Lifetime of the session cookie: the 24 hours of the JWT it carries
const SESSION_COOKIE_MAX_AGE_SECS: i64 = 86_400;

/// Which cookie signed the athlete in at `/oauth2/authorize`, which decides
/// the audience its token must carry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SignInCookie {
    /// The authorization server's own `pierre_session`, set by the hosted
    /// login: the authorize-flow audience only (carnet#787).
    AuthorizationServer,
    /// The web app's session cookie, bridged so a signed-in athlete is not
    /// asked to log in twice: the platform audience.
    WebApp,
}

impl OAuth2Routes {
    /// Whether this deployment's cookies are `Secure`: its issuer is HTTPS.
    pub(super) fn cookies_secure(context: &OAuth2Context) -> bool {
        context.config.issuer_url.starts_with("https://")
    }

    /// The name of the authorization server's session cookie here:
    /// `__Host-pierre_session` on an HTTPS issuer, so a sibling host cannot
    /// set one, and `pierre_session` over plain HTTP.
    pub(super) fn session_cookie_name(context: &OAuth2Context) -> String {
        host_cookie_name(SESSION_COOKIE, Self::cookies_secure(context))
    }

    /// The authorization server's own sign-in for an account the login rules
    /// just admitted (carnet#787).
    ///
    /// Never the first-party session the login minted for the web app: the
    /// hosted login form is a page any script can post a password to and read
    /// the cookie off, so what it hands back is a token whose audience only
    /// `/oauth2/authorize` and its consent form accept. Chat, REST, MCP and A2A
    /// refuse it.
    pub(super) async fn authorization_session(
        context: &OAuth2Context,
        login: &LoginResponse,
    ) -> AppResult<String> {
        let user_id = uuid::Uuid::parse_str(&login.user.user_id)
            .map_err(|e| AppError::internal(format!("Login named a malformed user id: {e}")))?;
        // SECURITY: Global lookup — the account the login rules just admitted.
        let user = context
            .users
            .get_global(user_id)
            .await?
            .ok_or_else(|| AppError::not_found(format!("User {user_id}")))?;
        context.auth_manager.generate_authorization_session_token(
            &user,
            &context.jwks_manager,
            login.user.tenant_id.clone(),
        )
    }

    /// The `Set-Cookie` value carrying the authorization server's sign-in
    /// token, the same for the password and the Google sign-in: `HttpOnly`,
    /// `Secure` on an HTTPS issuer, `SameSite=Lax` so the redirect into
    /// `/oauth2/authorize` sends it, and the JWT's own 24-hour lifetime.
    pub(super) fn session_cookie(context: &OAuth2Context, token: &str) -> String {
        SecureCookieConfig {
            name: Self::session_cookie_name(context),
            value: token.to_owned(),
            max_age_secs: SESSION_COOKIE_MAX_AGE_SECS,
            http_only: true,
            secure: Self::cookies_secure(context),
            same_site: SameSitePolicy::Lax,
            path: "/".to_owned(),
        }
        .build()
    }

    /// Extract the sign-in token from the cookie header, with the cookie it
    /// came from
    pub(super) fn extract_session_token(
        cookie_header: &str,
        session_cookie: &str,
        app_cookie: &str,
    ) -> Option<(SignInCookie, String)> {
        // Accept the authorization server's own session cookie and, as a
        // bridge, the first-party web app's session cookie. This lets a user
        // already logged into the web app authorize an MCP client without a
        // second login. Each carries its own audience (carnet#787): the
        // server's cookie the authorize-flow audience, the app's the platform
        // one, and each is validated for exactly that.
        // `app_cookie` is `auth_cookie_name_on(issuer is HTTPS)`: `__Host-auth_token`
        // when either the issuer or `BASE_URL` is HTTPS, so a plain `auth_token`
        // planted by a sibling host is never read here, and `auth_token` only
        // when both are plain HTTP. The session cookie wins when both are present.
        let mut app_token = None;
        for cookie in cookie_header.split(';') {
            let Some((name, value)) = cookie.trim().split_once('=') else {
                continue;
            };
            if name == session_cookie {
                return Some((SignInCookie::AuthorizationServer, value.to_owned()));
            }
            if name == app_cookie {
                app_token = Some((SignInCookie::WebApp, value.to_owned()));
            }
        }
        app_token
    }
}
