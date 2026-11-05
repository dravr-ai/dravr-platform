// ABOUTME: RFC 7009 token revocation — POST /oauth2/revoke retires a refresh token's whole family
// ABOUTME: The client authenticates as at the token endpoint; an unknown token is a success, another client's is refused

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Form, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use pierre_auth::oauth2_server::{
    client_auth::{is_basic_authorization, resolve_client_credentials},
    client_registration::ClientRegistrationManager,
    models::OAuth2Error,
};
use pierre_auth::rate_limiting::OAuth2Endpoint;
use secrecy::ExposeSecret;
use tracing::{error, info, warn};

use super::{authorization_header, OAuth2Context, OAuth2Routes};
use crate::oauth2_rate_limited::refusal;

impl OAuth2Routes {
    /// Handle token revocation (RFC 7009, `POST /oauth2/revoke`).
    ///
    /// A client that is being disconnected tells the server so, and the
    /// refresh token it holds (with every token rotated from it) stops
    /// exchanging. The client authenticates exactly as it does at the token
    /// endpoint. The reply is `200` for every token the client may revoke,
    /// known or not, so the endpoint cannot be used to probe for live tokens;
    /// a token issued to a different client is refused. An access token is a
    /// self-contained JWT that expires within the hour: it is accepted and
    /// left to expire, as RFC 7009 §2.2 allows.
    pub(super) async fn handle_revoke(
        State(context): State<OAuth2Context>,
        ConnectInfo(addr): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
        Form(form): Form<HashMap<String, String>>,
    ) -> Response {
        if let Some(refused) =
            refusal(&context.rate_limiter, OAuth2Endpoint::Token, addr, &headers).await
        {
            return refused;
        }

        let basic = is_basic_authorization(authorization_header(&headers));
        match Self::revoke_token(&context, &form, &headers).await {
            Ok(()) => (StatusCode::OK, [(header::CACHE_CONTROL, "no-store")]).into_response(),
            Err(error) => Self::token_error_response(&error, basic),
        }
    }

    /// Authenticate the client, then retire the family of the presented
    /// refresh token.
    async fn revoke_token(
        context: &OAuth2Context,
        form: &HashMap<String, String>,
        headers: &HeaderMap,
    ) -> Result<(), OAuth2Error> {
        let credentials = resolve_client_credentials(
            authorization_header(headers),
            form.get("client_id").map(String::as_str),
            form.get("client_secret").map(String::as_str),
        )?;
        let token = form
            .get("token")
            .filter(|token| !token.is_empty())
            .ok_or_else(|| OAuth2Error::invalid_request("Missing token parameter"))?;

        let client = ClientRegistrationManager::new(context.oauth2_server.clone())
            .authenticate_client(
                &credentials.client_id,
                credentials.client_secret.expose_secret(),
            )
            .await?;

        let stored = context
            .oauth2_server
            .get_refresh_token_by_value(token)
            .await
            .map_err(|e| {
                error!("Failed to look up a token for revocation: {e}");
                OAuth2Error::server_error("Failed to revoke the token")
            })?;
        let Some(stored) = stored else {
            // An unknown token, or an access token, which has no row: nothing
            // to revoke.
            return Ok(());
        };
        if stored.client_id != client.client_id {
            warn!(
                client_id = %client.client_id,
                "Revocation refused: the token was issued to another client"
            );
            return Err(OAuth2Error::unauthorized_client(
                "The token was not issued to this client",
            ));
        }

        let revoked = context
            .oauth2_server
            .revoke_refresh_token_family(token)
            .await
            .map_err(|e| {
                error!("Failed to revoke a refresh token family: {e}");
                OAuth2Error::server_error("Failed to revoke the token")
            })?;
        info!(
            client_id = %client.client_id,
            revoked,
            "Refresh token family revoked at the client's request"
        );
        Ok(())
    }
}
