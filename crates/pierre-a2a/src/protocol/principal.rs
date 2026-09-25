// ABOUTME: A2A protocol-surface authentication — who is calling, and which tasks they may reach
// ABOUTME: User JWTs pass the shared auth pipeline; client-credentials tokens act as their registering owner
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Principal resolution and task access for the A2A protocol methods.
//!
//! [`A2AServer::authenticate_request`] turns a request's bearer into the
//! acting [`AuthPrincipal`], and the access helpers confine that principal
//! to the tasks keyed to clients it owns.

use pierre_core::constants::http_status::INTERNAL_SERVER_ERROR;
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::permissions::scopes::OAuthScope;
use serde_json::{Number, Value};
use tracing::error;
use uuid::Uuid;

use super::{A2AResources, A2AServer, A2ATask, AuthPrincipal};
use crate::protocol_types::{rate_limit_details, A2ASpecError};
use crate::{A2AErrorResponse, A2ARequest, A2AResponse};

impl A2AServer {
    /// Authenticate the A2A request and resolve the acting principal.
    ///
    /// Two token shapes are accepted, matching the card's `securitySchemes`:
    /// user JWTs (`sub` = user UUID), and `OAuth2` client-credentials JWTs
    /// (`sub` = `client:{id}`, minted by `/a2a/auth`). A client token
    /// acts as the client's registering user — the same semantics as
    /// `A2AAuthenticator::authenticate_oauth2` — with the client identity
    /// kept on the principal for task keying and scoping.
    ///
    /// A user JWT is admitted through
    /// [`McpAuthMiddleware`](pierre_middleware::McpAuthMiddleware), the pipeline
    /// every other route uses: the account-status gate, the monthly request
    /// budget, the usage row the budget counts, and the report the
    /// `X-RateLimit-*` headers render. A spent budget is refused with the
    /// `RATE_LIMIT_EXCEEDED` reason and a retry window, which the bindings
    /// answer with 429 and `Retry-After`.
    ///
    /// Authentication is transport-level per the spec; a refused credential
    /// carries the `AUTHENTICATION_REQUIRED` reason so the HTTP layer can
    /// surface 401.
    pub(super) async fn authenticate_request(
        request: &A2ARequest,
        resources: &A2AResources,
    ) -> Result<AuthPrincipal, Box<A2AResponse>> {
        let request_id = request
            .id
            .clone() // Safe: JSON value ownership for request ID
            .unwrap_or_else(|| Value::Number(Number::from(0)));

        // Extract auth token from request
        let auth_token = request.auth_token.as_deref().ok_or_else(|| {
            Box::new(Self::auth_error(
                "Authentication token required",
                Some(request_id.clone()),
            ))
        })?;

        // Validate token signature/expiry
        let claims = resources
            .ctx
            .auth_manager()
            .validate_token(auth_token, resources.ctx.jwks_manager())
            .map_err(|_| {
                Box::new(Self::auth_error(
                    "Invalid authentication token",
                    Some(request_id.clone()),
                ))
            })?;

        // User JWT: subject is the user UUID.
        if Uuid::parse_str(&claims.sub).is_ok() {
            let admitted = resources
                .auth_middleware
                .authenticate_request(Some(&format!("Bearer {auth_token}")))
                .await
                .map_err(|e| Box::new(Self::user_auth_refusal(&e, request_id)))?;
            return Ok(AuthPrincipal {
                user_id: admitted.user_id,
                client_id: None,
                scopes: admitted.scopes,
            });
        }

        // Client-credentials JWT: subject is client:{id} (the shape
        // /a2a/auth mints via generate_client_credentials_token).
        let Some(client_id) = claims.sub.strip_prefix("client:") else {
            return Err(Box::new(Self::auth_error(
                "Invalid principal in authentication token",
                Some(request_id),
            )));
        };

        let granted = OAuthScope::parse_granted(&claims.scope);
        Self::resolve_client_principal(client_id, granted, resources, request_id).await
    }

    /// Resolve a client-credentials subject into its acting principal: the
    /// client must exist and be active; the principal acts as the client's
    /// registering user with the client identity kept for task scoping.
    ///
    /// LIMITATION(registre#620): `resolve_client_principal` enforces, records
    /// and reports no request budget for a client-credentials principal.
    async fn resolve_client_principal(
        client_id: &str,
        scopes: Vec<OAuthScope>,
        resources: &A2AResources,
        request_id: Value,
    ) -> Result<AuthPrincipal, Box<A2AResponse>> {
        let client = match resources.ctx.repos().a2a.get_client(client_id).await {
            Ok(Some(client)) => client,
            Ok(None) => {
                return Err(Box::new(Self::auth_error(
                    "Unknown A2A client in authentication token",
                    Some(request_id),
                )))
            }
            Err(e) => {
                error!("Failed to resolve A2A client during authentication: {e}");
                return Err(Box::new(Self::a2a_error(
                    -32000,
                    "Failed to resolve A2A client",
                    Some(request_id),
                )));
            }
        };
        if !client.is_active {
            return Err(Box::new(Self::auth_error(
                "A2A client is deactivated",
                Some(request_id),
            )));
        }

        Ok(AuthPrincipal {
            user_id: client.user_id,
            client_id: Some(client.id),
            scopes,
        })
    }

    /// Get client IDs owned by a user
    pub(super) async fn get_owned_client_ids(
        user_id: &Uuid,
        resources: &A2AResources,
    ) -> Result<Vec<String>, String> {
        resources
            .ctx
            .repos()
            .a2a
            .list_clients(user_id)
            .await
            .map(|clients| clients.into_iter().map(|c| c.id).collect())
            .map_err(|e| format!("Failed to list A2A clients: {e}"))
    }

    /// Verify the principal may access tasks keyed to `client_id`.
    ///
    /// Client-credentials principals are confined to their own client; user
    /// principals may access any client they registered.
    async fn verify_client_access(
        client_id: &str,
        principal: &AuthPrincipal,
        resources: &A2AResources,
        request_id: Option<&Value>,
    ) -> Result<(), Box<A2AResponse>> {
        if let Some(acting_client) = &principal.client_id {
            if acting_client == client_id {
                return Ok(());
            }
            // Do not reveal whether the task exists for another principal.
            return Err(Box::new(Self::spec_error(
                A2ASpecError::TaskNotFound,
                request_id.cloned(),
            )));
        }

        let owned_ids = Self::get_owned_client_ids(&principal.user_id, resources)
            .await
            .map_err(|e| {
                error!("Failed to resolve client ownership: {e}");
                Box::new(Self::a2a_error(
                    -32000,
                    "Failed to resolve client ownership",
                    request_id.cloned(),
                ))
            })?;

        // Do not reveal whether the task exists for another principal.
        owned_ids.iter().any(|id| id == client_id).ok_or_else(|| {
            Box::new(Self::spec_error(
                A2ASpecError::TaskNotFound,
                request_id.cloned(),
            ))
        })
    }

    /// Load a task and verify the principal may access it.
    pub(super) async fn load_owned_task(
        resources: &A2AResources,
        task_id: &str,
        principal: &AuthPrincipal,
        request_id: Option<&Value>,
    ) -> Result<A2ATask, Box<A2AResponse>> {
        let task = match resources.ctx.repos().a2a.get_task(task_id).await {
            Ok(Some(task)) => task,
            Ok(None) => {
                return Err(Box::new(Self::spec_error(
                    A2ASpecError::TaskNotFound,
                    request_id.cloned(),
                )))
            }
            Err(e) => {
                error!("A2A task lookup database error: {e}");
                return Err(Box::new(Self::a2a_error(
                    -32000,
                    "Database error",
                    request_id.cloned(),
                )));
            }
        };

        Self::verify_client_access(&task.client_id, principal, resources, request_id).await?;

        Ok(task)
    }

    /// The refusal for a user credential the auth pipeline did not admit.
    ///
    /// A spent budget keeps its retry window and is never an authentication
    /// failure: a client told its token is bad re-authenticates, and is
    /// refused again. A server-side failure while authenticating says nothing
    /// about the credential either. Mirrors `AppError::into_auth_refusal` on
    /// the REST routes.
    fn user_auth_refusal(error: &AppError, request_id: Value) -> A2AResponse {
        if error.code == ErrorCode::RateLimitExceeded {
            return Self::rate_limited_error(
                error.sanitized_message(),
                error.retry_after_secs().unwrap_or(1),
                Some(request_id),
            );
        }
        if error.http_status() >= INTERNAL_SERVER_ERROR {
            error!("A2A authentication could not complete: {error}");
            return Self::a2a_error(
                -32000,
                "Authentication could not complete",
                Some(request_id),
            );
        }
        Self::auth_error(
            format!("Authentication refused: {}", error.sanitized_message()),
            Some(request_id),
        )
    }

    /// Create a refusal for a spent request budget. Carries the
    /// `RATE_LIMIT_EXCEEDED` reason and the wait as a `google.rpc.RetryInfo`,
    /// which HTTP bindings map to 429 and `Retry-After`.
    fn rate_limited_error(
        message: impl Into<String>,
        retry_after_secs: u64,
        request_id: Option<Value>,
    ) -> A2AResponse {
        A2AResponse {
            jsonrpc: "2.0".into(),
            result: None,
            error: Some(A2AErrorResponse {
                code: -32000,
                message: message.into(),
                data: Some(rate_limit_details(retry_after_secs)),
            }),
            id: request_id,
        }
    }
}
