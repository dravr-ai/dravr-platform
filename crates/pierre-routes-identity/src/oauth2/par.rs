// ABOUTME: RFC 9126 pushed authorization requests — POST /oauth2/par, and resolving a request_uri at each page of the flow
// ABOUTME: The client authenticates as at the token endpoint; the request_uri resolves for its client alone and mints one code
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Pushed authorization requests (RFC 9126).
//!
//! `POST /oauth2/par` takes a whole authorization request from an
//! authenticated client, checks it as `/oauth2/authorize` would, stores its
//! parameters under the SHA-256 of a fresh `request_uri`, and answers that URI
//! with its 90-second lifetime. `GET /oauth2/authorize?client_id&request_uri`
//! then reads the stored parameters, and every other query parameter is
//! ignored. The first resolve extends the request to 30 minutes; the login
//! page, the Google hop and the consent form carry the `request_uri` and
//! resolve it again. Minting a code spends it, so it yields one code.

use std::collections::HashMap;
use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Form, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_auth::oauth2_server::cimd::is_metadata_document_id;
use pierre_auth::oauth2_server::client_auth::{is_basic_authorization, resolve_client_credentials};
use pierre_auth::oauth2_server::client_registration::ClientRegistrationManager;
use pierre_auth::oauth2_server::first_party::is_first_party;
use pierre_auth::oauth2_server::models::{AuthorizeRejection, AuthorizeRequest, OAuth2Error};
use pierre_auth::oauth2_server::pushed::{
    new_request_uri, request_uri_hash, PUSHED_PARAMETERS, PUSHED_REQUEST_LIFETIME_SECS,
    RESOLVED_REQUEST_LIFETIME_SECS,
};
use pierre_auth::rate_limiting::OAuth2Endpoint;
use pierre_core::errors::AppError;
use pierre_core::models::OAuth2PushedRequest;
use secrecy::ExposeSecret;
use serde::Serialize;
use tracing::{error, info, warn};
use uuid::Uuid;

use super::{authorization_header, OAuth2Context, OAuth2Routes};
use crate::oauth2_rate_limited::refusal;

/// The refusal of a `request_uri` that does not resolve: the English of
/// `oauth.error.invalidRequestUri`, so the refusal page words it in the
/// reader's language.
pub(super) const REQUEST_URI_REFUSED: &str =
    "This connection link has expired or was already used. Start connecting the app again.";

/// The refusal when a pushed request could not be read or written.
const PUSHED_REQUEST_UNAVAILABLE: &str = "The authorization request could not be processed";

/// The answer to a push (RFC 9126 §2.2).
#[derive(Serialize)]
struct PushedRequestResponse {
    /// The URI the client sends the browser to `/oauth2/authorize` with
    request_uri: String,
    /// Seconds it resolves for, unused
    expires_in: i64,
}

/// An authorization request's parameters, as the flow reads them.
pub(super) struct AuthorizeParams {
    /// The request's parameters: the query's, or a pushed request's stored
    /// ones with its `request_uri`
    pub(super) params: HashMap<String, String>,
    /// Whether `prompt` speaks for this hop: always for a request sent in
    /// the query, which the login page sends back without it; for a pushed
    /// request only on its first resolve, since the pages after it carry the
    /// same stored `prompt` back and would otherwise send the athlete to the
    /// login page again and again
    pub(super) honours_prompt: bool,
}

impl OAuth2Routes {
    /// Handle a pushed authorization request (POST /oauth2/par, RFC 9126).
    pub(super) async fn handle_par(
        State(context): State<OAuth2Context>,
        ConnectInfo(addr): ConnectInfo<SocketAddr>,
        headers: HeaderMap,
        Form(form): Form<HashMap<String, String>>,
    ) -> Response {
        if let Some(refused) =
            refusal(&context.rate_limiter, OAuth2Endpoint::Par, addr, &headers).await
        {
            return refused;
        }
        let basic = is_basic_authorization(authorization_header(&headers));
        match Self::push_request(&context, &headers, &form).await {
            Ok(pushed) => {
                let mut response = (StatusCode::CREATED, Json(pushed)).into_response();
                response
                    .headers_mut()
                    .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
                response
            }
            Err(error) => Self::token_error_response(&error, basic),
        }
    }

    /// Authenticate the pushing client, check its request, and store it.
    async fn push_request(
        context: &OAuth2Context,
        headers: &HeaderMap,
        form: &HashMap<String, String>,
    ) -> Result<PushedRequestResponse, OAuth2Error> {
        let client_id = Self::authenticate_pushing_client(context, headers, form).await?;
        let mut params: HashMap<String, String> = PUSHED_PARAMETERS
            .iter()
            .filter_map(|name| {
                form.get(*name)
                    .map(|value| ((*name).to_owned(), value.clone()))
            })
            .collect();
        params.insert("client_id".to_owned(), client_id.clone());
        // RFC 9449 §10.1: a proof at PAR binds the code to its key, as a
        // `dpop_jkt` parameter does; given both, they must name one key.
        if let Some(jkt) = Self::proven_dpop_key(context, headers, "/oauth2/par").await? {
            if params.get("dpop_jkt").is_some_and(|named| *named != jkt) {
                return Err(OAuth2Error::invalid_dpop_proof(
                    "dpop_jkt does not name the key the DPoP proof is signed with",
                ));
            }
            params.insert("dpop_jkt".to_owned(), jkt);
        }
        let request = Self::parse_authorize_request(&params)?;
        Self::authorization_server(context)
            .check_authorize_request(&request)
            .await
            .map_err(AuthorizeRejection::into_error)?;
        Self::store_pushed(context, &client_id, &params).await
    }

    /// The client a push comes from, authenticated as at the token endpoint:
    /// a confidential client proves its secret, a public one presents its
    /// `client_id` alone.
    async fn authenticate_pushing_client(
        context: &OAuth2Context,
        headers: &HeaderMap,
        form: &HashMap<String, String>,
    ) -> Result<String, OAuth2Error> {
        let credentials = resolve_client_credentials(
            authorization_header(headers),
            form.get("client_id").map(String::as_str),
            form.get("client_secret").map(String::as_str),
        )?;
        let client_id = credentials.client_id;
        // Dravr's own apps sign in without a consent screen and never push.
        if is_first_party(&client_id) {
            warn!(client_id = %client_id, "Pushed authorization request refused: a first-party client");
            return Err(OAuth2Error::invalid_client());
        }
        // RFC 9126 §2.1: a push carries the request itself, never a reference.
        if form.contains_key("request_uri") {
            return Err(OAuth2Error::invalid_request(
                "request_uri is not allowed in a pushed authorization request",
            ));
        }
        // A client known by its metadata document is public and may have no
        // row before an athlete signs in: `check_authorize_request` resolves it.
        if is_metadata_document_id(&client_id) {
            if !credentials.client_secret.expose_secret().is_empty() {
                warn!(client_id = %client_id, "Pushed authorization request refused: a metadata document client presented a secret");
                return Err(OAuth2Error::invalid_client());
            }
        } else {
            ClientRegistrationManager::new(context.oauth2_server.clone())
                .authenticate_client(&client_id, credentials.client_secret.expose_secret())
                .await?;
        }
        Ok(client_id)
    }

    /// Store `params`, a checked request `client_id` pushed, under a fresh
    /// `request_uri`, and answer it.
    async fn store_pushed(
        context: &OAuth2Context,
        client_id: &str,
        params: &HashMap<String, String>,
    ) -> Result<PushedRequestResponse, OAuth2Error> {
        let request_uri = new_request_uri().map_err(|e| {
            error!("Pushed authorization request not stored: {e}");
            OAuth2Error::server_error(PUSHED_REQUEST_UNAVAILABLE)
        })?;
        let parameters = serde_json::to_string(params).map_err(|e| {
            error!("Pushed authorization request not stored: {e}");
            OAuth2Error::server_error(PUSHED_REQUEST_UNAVAILABLE)
        })?;
        let now = Timestamp::now();
        let row = OAuth2PushedRequest {
            id: Uuid::new_v4().to_string(),
            request_uri_hash: request_uri_hash(&request_uri),
            client_id: client_id.to_owned(),
            parameters,
            expires_at: now + Elapsed::seconds(PUSHED_REQUEST_LIFETIME_SECS),
            used_at: None,
            created_at: now,
        };
        context
            .oauth2_server
            .store_pushed_request(&row)
            .await
            .map_err(|e| {
                error!(client_id = %client_id, "Pushed authorization request not stored: {e}");
                OAuth2Error::server_error(PUSHED_REQUEST_UNAVAILABLE)
            })?;
        info!(client_id = %client_id, "Pushed authorization request stored");
        Ok(PushedRequestResponse {
            request_uri,
            expires_in: PUSHED_REQUEST_LIFETIME_SECS,
        })
    }

    /// The parameters an authorization request is read from at
    /// `/oauth2/authorize`, the Google hop and the consent form.
    ///
    /// Without a `request_uri` they are `params` as given. With one, they are
    /// the stored parameters of the request `client_id` pushed under it, and
    /// nothing else `params` carries; the first resolve extends the request
    /// to [`RESOLVED_REQUEST_LIFETIME_SECS`].
    ///
    /// # Errors
    /// `invalid_request_uri` when the URI is unknown, expired, used or
    /// another client's; `server_error` when the store cannot be read.
    pub(super) async fn resolve_authorize_params(
        context: &OAuth2Context,
        params: HashMap<String, String>,
    ) -> Result<AuthorizeParams, OAuth2Error> {
        let Some(request_uri) = params
            .get("request_uri")
            .filter(|value| !value.is_empty())
            .cloned()
        else {
            return Ok(AuthorizeParams {
                params,
                honours_prompt: true,
            });
        };
        let Some(client_id) = params.get("client_id").filter(|value| !value.is_empty()) else {
            return Err(OAuth2Error::invalid_request("Missing client_id parameter"));
        };
        let unavailable = |e| {
            error!(client_id = %client_id, "Pushed authorization request could not be resolved: {e}");
            OAuth2Error::server_error(PUSHED_REQUEST_UNAVAILABLE)
        };
        let hash = request_uri_hash(&request_uri);
        let now = Timestamp::now();
        let Some(row) = context
            .oauth2_server
            .find_pushed_request(&hash, client_id, now)
            .await
            .map_err(unavailable)?
        else {
            warn!(client_id = %client_id, "request_uri refused: unknown, expired, used or another client's");
            return Err(OAuth2Error::invalid_request_uri(REQUEST_URI_REFUSED));
        };
        let first_resolve =
            row.expires_at - row.created_at <= Elapsed::seconds(PUSHED_REQUEST_LIFETIME_SECS);
        if first_resolve {
            context
                .oauth2_server
                .extend_pushed_request(
                    &hash,
                    client_id,
                    now + Elapsed::seconds(RESOLVED_REQUEST_LIFETIME_SECS),
                )
                .await
                .map_err(unavailable)?;
        }
        let mut stored: HashMap<String, String> = serde_json::from_str(&row.parameters)
            .map_err(|e| unavailable(AppError::database(e.to_string())))?;
        stored.insert("client_id".to_owned(), row.client_id);
        stored.insert("request_uri".to_owned(), request_uri);
        Ok(AuthorizeParams {
            params: stored,
            honours_prompt: first_resolve,
        })
    }

    /// Spend the pushed request `request` was resolved from, before a code is
    /// minted from it: a request yields one code (RFC 9126 §4). A request
    /// sent in the query has nothing to spend.
    ///
    /// # Errors
    /// `invalid_request_uri` when it was spent or expired meanwhile;
    /// `server_error` when the store cannot be written.
    pub(super) async fn spend_pushed_request(
        context: &OAuth2Context,
        request: &AuthorizeRequest,
    ) -> Result<(), OAuth2Error> {
        let Some(request_uri) = request.request_uri.as_deref() else {
            return Ok(());
        };
        let spent = context
            .oauth2_server
            .consume_pushed_request(&request_uri_hash(request_uri), &request.client_id, Timestamp::now())
            .await
            .map_err(|e| {
                error!(client_id = %request.client_id, "Pushed authorization request could not be spent: {e}");
                OAuth2Error::server_error(PUSHED_REQUEST_UNAVAILABLE)
            })?;
        if spent {
            return Ok(());
        }
        warn!(client_id = %request.client_id, "request_uri refused at the code: already used or expired");
        Err(OAuth2Error::invalid_request_uri(REQUEST_URI_REFUSED))
    }
}
