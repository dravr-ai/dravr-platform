// ABOUTME: RFC 7662 token introspection — POST /oauth2/introspect for the resource servers an admin named
// ABOUTME: An access token is active while it verifies and its consent grant stands; otherwise it says why it is not
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Token introspection (RFC 7662).
//!
//! A resource server that cannot verify this server's tokens itself, or that
//! must know whether the consent behind one still stands, asks here. Only a
//! confidential client named in `OAUTH2_INTROSPECTION_CLIENTS` may ask: the
//! answer describes an athlete's grant. Everyone else is refused 401
//! `invalid_client`.
//!
//! An access token is active while its signature, expiry, issuer and
//! audience (any resource this server mints for) check, and:
//! - a delegated token's consent grant stands, read exactly as `/mcp` reads
//!   it (`ClientGrantStanding::admits_token`, the `client_grant_id` rule);
//! - a client-credentials token's client is still registered and unexpired.
//!
//! Anything else is `{"active": false}` with `health_inactive_cause`
//! (`revoked`, `expired` or `unknown`). `grant_id` is the token's
//! `client_grant_id`, the consent grant; the JWT's own `grant_id` claim is
//! the refresh family and is not returned. A token under a preset grant also
//! carries the grant's `authorization_details`, the resolved document: the
//! preset by name, never a stream.
//!
//! [`OAuth2Routes::introspect`] is the one implementation: `/oauth2/introspect`
//! answers it to a resource server, and `/oauth2/token-validate` reads its
//! `active` for the SDK.

use std::collections::HashMap;
use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Form, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use dravr_tronc::dates::Timestamp;
use pierre_auth::auth::{Claims, Confirmation, JwtValidationError};
use pierre_auth::dpop::{BEARER_SCHEME, DPOP_SCHEME};
use pierre_auth::oauth2_server::authorization_details::ResolvedDetails;
use pierre_auth::oauth2_server::client_auth::{is_basic_authorization, resolve_client_credentials};
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::models::OAuth2Error;
use pierre_auth::oauth2_server::step_up_echo::StepUpEcho;
use pierre_auth::rate_limiting::OAuth2Endpoint;
use pierre_core::errors::AppError;
use secrecy::ExposeSecret;
use serde::Serialize;
use tracing::{error, warn};
use uuid::Uuid;

use super::{authorization_header, OAuth2Context, OAuth2Routes};
use crate::oauth2_rate_limited::refusal;

/// Why an inactive token is inactive (owl PDPP §5, `health_inactive_cause`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum InactiveCause {
    /// The athlete withdrew the consent, or the client is gone
    Revoked,
    /// The token, its grant or its client is past its expiry
    Expired,
    /// Not a token this server issued for a resource, or one introspection
    /// does not describe (a refresh token, a first-party session)
    Unknown,
}

/// The introspection response (RFC 7662 §2.2). An inactive token carries
/// only `active` and `health_inactive_cause`.
#[derive(Debug, Default, Serialize)]
pub(super) struct Introspection {
    pub(super) active: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sub: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    aud: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    iss: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    exp: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    iat: Option<i64>,
    /// `DPoP` for a key-bound token, `Bearer` otherwise (RFC 9449 §6.2)
    #[serde(skip_serializing_if = "Option::is_none")]
    token_type: Option<&'static str>,
    /// The key a bound token is confirmed by (RFC 9449 §6.2, RFC 7800)
    #[serde(skip_serializing_if = "Option::is_none")]
    cnf: Option<Confirmation>,
    /// The token's PDPP kind (owl §5.1), derived from its claims
    #[serde(skip_serializing_if = "Option::is_none")]
    pdpp_token_kind: Option<&'static str>,
    /// The consent grant the token was minted under (`client_grant_id`)
    #[serde(skip_serializing_if = "Option::is_none")]
    grant_id: Option<String>,
    /// What a preset grant was approved for (RFC 9396 §9.1); absent for a
    /// plain grant
    #[serde(skip_serializing_if = "Option::is_none")]
    authorization_details: Option<ResolvedDetails>,
    /// The live step-up child the token reads beside its grant (owl §7.6):
    /// its details and `step_up_result`; absent when the grant has none
    #[serde(skip_serializing_if = "Option::is_none")]
    step_up: Option<StepUpEcho>,
    #[serde(skip_serializing_if = "Option::is_none")]
    health_inactive_cause: Option<InactiveCause>,
}

/// What a delegated token's standing grant was approved for: its details
/// (`None` for a plain grant) and its live step-up child.
type Admitted = (Option<ResolvedDetails>, Option<StepUpEcho>);

impl Introspection {
    /// The answer for an inactive token.
    fn inactive(cause: InactiveCause) -> Self {
        Self {
            health_inactive_cause: Some(cause),
            ..Self::default()
        }
    }

    /// The answer for an active token, from its verified claims and the
    /// details its grant was approved for.
    fn active(claims: Claims, (authorization_details, step_up): Admitted) -> Self {
        let token_type = if claims.cnf.is_some() {
            DPOP_SCHEME
        } else {
            BEARER_SCHEME
        };
        let pdpp_token_kind = claims.pdpp_token_kind();
        Self {
            active: true,
            scope: Some(claims.scope),
            client_id: claims.client_id,
            sub: Some(claims.sub),
            aud: Some(claims.aud),
            iss: Some(claims.iss),
            exp: Some(claims.exp),
            iat: Some(claims.iat),
            token_type: Some(token_type),
            cnf: claims.cnf,
            pdpp_token_kind,
            grant_id: claims.client_grant_id,
            authorization_details,
            step_up,
            health_inactive_cause: None,
        }
    }
}

impl OAuth2Routes {
    /// Handle token introspection (POST /oauth2/introspect, RFC 7662).
    pub(super) async fn handle_introspect(
        State(context): State<OAuth2Context>,
        ConnectInfo(addr): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
        Form(form): Form<HashMap<String, String>>,
    ) -> Response {
        if let Some(refused) = refusal(
            &context.rate_limiter,
            OAuth2Endpoint::Introspect,
            addr,
            &headers,
        )
        .await
        {
            return refused;
        }
        let basic = is_basic_authorization(authorization_header(&headers));
        if let Err(error) = Self::authenticate_introspector(&context, &headers, &form).await {
            return Self::introspector_refusal(&error, basic);
        }
        let Some(token) = form.get("token").filter(|token| !token.is_empty()) else {
            return (
                StatusCode::BAD_REQUEST,
                Json(OAuth2Error::invalid_request("Missing token parameter")),
            )
                .into_response();
        };
        // `token_type_hint` only speeds a lookup (RFC 7662 §2.1); every token
        // this endpoint describes is an access token, so it changes nothing.
        match Self::introspect(&context, token).await {
            Ok(answer) => {
                let mut response = (StatusCode::OK, Json(answer)).into_response();
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                response
            }
            Err(e) => {
                error!("Token introspection could not complete: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(OAuth2Error::server_error(
                        "The token could not be introspected right now",
                    )),
                )
                    .into_response()
            }
        }
    }

    /// Authenticate the caller as a confidential client named in
    /// `OAUTH2_INTROSPECTION_CLIENTS`.
    async fn authenticate_introspector(
        context: &OAuth2Context,
        headers: &HeaderMap,
        form: &HashMap<String, String>,
    ) -> Result<(), OAuth2Error> {
        let credentials = resolve_client_credentials(
            authorization_header(headers),
            form.get("client_id").map(String::as_str),
            form.get("client_secret").map(String::as_str),
        )
        .map_err(|_| OAuth2Error::invalid_client())?;
        let client_id = credentials.client_id;
        let secret = credentials.client_secret.expose_secret();
        if !context.config.introspection_clients.contains(&client_id) {
            warn!(client_id = %client_id, "Token introspection refused: the client is not an introspection client");
            return Err(OAuth2Error::invalid_client());
        }
        // A public client presents no secret: it is never a resource server.
        if secret.is_empty() {
            warn!(client_id = %client_id, "Token introspection refused: no client secret");
            return Err(OAuth2Error::invalid_client());
        }
        ClientRegistrationManager::new(context.oauth2_server.clone())
            .authenticate_client(&client_id, secret)
            .await
            .map(|_| ())
    }

    /// The refusal of a caller that may not introspect: 401 for any failed
    /// authentication (RFC 7662 §2.3), with the Basic challenge when it
    /// tried Basic; a server fault keeps its own status.
    fn introspector_refusal(error: &OAuth2Error, basic: bool) -> Response {
        if error.error != "invalid_client" {
            return (error.http_status(), Json(error)).into_response();
        }
        let mut response = (StatusCode::UNAUTHORIZED, Json(error)).into_response();
        if basic {
            response.headers_mut().insert(
                header::WWW_AUTHENTICATE,
                HeaderValue::from_static("Basic realm=\"oauth2\""),
            );
        }
        response
    }

    /// What `token` is, as introspection describes it.
    pub(super) async fn introspect(
        context: &OAuth2Context,
        token: &str,
    ) -> Result<Introspection, AppError> {
        let a2a = context.config.a2a_resources();
        let served: Vec<&str> = context
            .config
            .mcp_resources()
            .into_iter()
            .chain(a2a.iter().map(String::as_str))
            .chain(
                context
                    .config
                    .satellite_resources
                    .iter()
                    .map(String::as_str),
            )
            .collect();
        let claims = match context.auth_manager.validate_resource_token_detailed(
            token,
            &context.jwks_manager,
            &served,
        ) {
            Ok(claims) => claims,
            Err(JwtValidationError::TokenExpired { .. }) => {
                return Ok(Introspection::inactive(InactiveCause::Expired));
            }
            Err(_) => return Ok(Introspection::inactive(InactiveCause::Unknown)),
        };
        let standing = if let Some(client_id) = claims.client_credentials_client() {
            Self::client_standing(context, client_id)
                .await?
                .map_or(Ok((None, None)), Err)
        } else if claims.is_delegated() {
            Self::grant_standing(context, &claims).await?
        } else {
            // A first-party session or an impersonation token: no client
            // holds it, so no resource server introspects it.
            Err(InactiveCause::Unknown)
        };
        Ok(match standing {
            Ok(details) => Introspection::active(claims, details),
            Err(cause) => Introspection::inactive(cause),
        })
    }

    /// Whether a client-credentials token's client still stands: `None` when
    /// it does, else why not.
    async fn client_standing(
        context: &OAuth2Context,
        client_id: &str,
    ) -> Result<Option<InactiveCause>, AppError> {
        let Some(client) = context.oauth2_server.get_client(client_id).await? else {
            return Ok(Some(InactiveCause::Revoked));
        };
        let expired = client
            .expires_at
            .is_some_and(|expires_at| expires_at <= Timestamp::now());
        Ok(expired.then_some(InactiveCause::Expired))
    }

    /// Whether a delegated token's consent grant stands, read as `/mcp` reads
    /// it: the details the grant was approved for when it does (`None` for a
    /// plain grant) and its live step-up child, else why not.
    async fn grant_standing(
        context: &OAuth2Context,
        claims: &Claims,
    ) -> Result<Result<Admitted, InactiveCause>, AppError> {
        let (Some(client_id), Ok(user_id)) =
            (claims.client_id.as_deref(), Uuid::parse_str(&claims.sub))
        else {
            return Ok(Err(InactiveCause::Unknown));
        };
        let tenant_id = match claims.active_tenant_id.clone() {
            Some(tenant_id) => tenant_id,
            None => match context.tenants.first_for_user(user_id).await? {
                Some(tenant_id) => tenant_id.to_string(),
                None => return Ok(Err(InactiveCause::Unknown)),
            },
        };
        let now = Timestamp::now();
        let standing = context
            .accounts
            .grant_standings()
            .read(user_id, &tenant_id, client_id)
            .await?;
        let client_grant_id = claims.client_grant_id.as_deref();
        if standing.admits_token(claims.iat, client_grant_id, now) {
            let held =
                client_grant_id.and_then(|id| standing.grants.iter().find(|grant| grant.id == id));
            let details = held
                .and_then(|grant| grant.authorization_details.as_deref())
                .map(ResolvedDetails::from_document)
                .transpose()?;
            let step_up = held
                .and_then(|grant| grant.child.as_ref())
                .filter(|child| child.expires_at.is_none_or(|expires_at| now < expires_at))
                .map(StepUpEcho::of_standing)
                .transpose()?;
            return Ok(Ok((details, step_up)));
        }
        let Some(client_grant_id) = client_grant_id else {
            return Ok(Err(InactiveCause::Revoked));
        };
        let grant = context
            .oauth2_server
            .get_client_grant(client_grant_id, &user_id.to_string(), &tenant_id)
            .await?;
        Ok(Err(match grant {
            Some(grant) if grant.revoked_at.is_some() => InactiveCause::Revoked,
            Some(grant) if grant.expires_at.is_some_and(|expires_at| expires_at <= now) => {
                InactiveCause::Expired
            }
            // Live in the row but not in the cached standing: it changes
            // within the cache window; until then it is not admitted.
            Some(_) => InactiveCause::Unknown,
            None => InactiveCause::Revoked,
        }))
    }
}
