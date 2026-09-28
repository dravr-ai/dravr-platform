// ABOUTME: A2A protocol-surface authentication — who is calling, and which tasks they may reach
// ABOUTME: User JWTs pass the shared auth pipeline; client-credentials tokens spend their client's own budget
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Principal resolution and task access for the A2A protocol methods.
//!
//! [`A2AServer::authenticate_request`] turns a request's bearer into the
//! acting [`AuthPrincipal`], and the access helpers confine that principal
//! to the tasks keyed to clients it owns.

use chrono::Utc;
use pierre_auth::rate_limiting::{a2a_client_window_start, calculate_a2a_client_rate_limit};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::a2a::A2AClient;
use pierre_core::permissions::scopes::OAuthScope;
use pierre_middleware::rate_limiting::{
    enforce_request_budget, report_a2a_client_request, report_request_budget,
    report_request_operation, A2AClientCall,
};
use serde_json::{Number, Value};
use tracing::error;
use uuid::Uuid;

use super::{A2AResources, A2AServer, A2ATask, AuthPrincipal};
use crate::protocol_types::{rate_limit_details, A2ASpecError, A2A_VERSION};
use crate::{A2AErrorResponse, A2ARequest, A2AResponse};

impl A2AServer {
    /// Authenticate the A2A request and resolve the acting principal.
    ///
    /// Two token shapes are accepted, matching the card's `securitySchemes`:
    /// user JWTs (`sub` = user UUID), and `OAuth2` client-credentials JWTs
    /// (`sub` = `client:{id}`, minted by the `/oauth2/token`
    /// `client_credentials` grant). A client token acts as the client's
    /// registering user, with the client identity kept on the principal for
    /// task keying and scoping.
    ///
    /// A user JWT is admitted through
    /// [`McpAuthMiddleware`](pierre_middleware::McpAuthMiddleware), the pipeline
    /// every other route uses: the account-status gate, the monthly request
    /// budget, the usage row the budget counts, and the report the
    /// `X-RateLimit-*` headers render. A client-credentials token spends its
    /// client's own budget instead (see
    /// [`resolve_client_principal`](Self::resolve_client_principal)). Either
    /// way a spent budget is refused with the `RATE_LIMIT_EXCEEDED` reason and
    /// a retry window, which the bindings answer with 429 and `Retry-After`.
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
                .map_err(|e| Box::new(Self::auth_refusal(&e, request_id)))?;
            return Ok(AuthPrincipal {
                user_id: admitted.user_id,
                client_id: None,
                scopes: admitted.scopes,
            });
        }

        // Client-credentials JWT: subject is client:{id} (the shape
        // /oauth2/token mints via generate_client_credentials_token).
        let Some(client_id) = claims.sub.strip_prefix("client:") else {
            return Err(Box::new(Self::auth_error(
                "Invalid principal in authentication token",
                Some(request_id),
            )));
        };

        let granted = OAuthScope::parse_granted(&claims.scope);
        Self::resolve_client_principal(client_id, granted, &request.method, resources, request_id)
            .await
    }

    /// Resolve a client-credentials subject into its acting principal: the
    /// client must exist and be active; the principal acts as the client's
    /// registering user with the client identity kept for task scoping.
    ///
    /// The call spends the client's own request budget, the shape an API key
    /// has: its row's `rate_limit_requests` over a sliding
    /// `rate_limit_window_seconds`, counted over the client's `a2a_usage`
    /// rows. The budget that decides the call is reported for the
    /// `X-RateLimit-*` headers; an admitted call is reported as the client's,
    /// so the request-budget layer writes its `a2a_usage` row with the real
    /// outcome, named after the A2A `method`.
    async fn resolve_client_principal(
        client_id: &str,
        scopes: Vec<OAuthScope>,
        method: &str,
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

        Self::admit_client_call(&client, &scopes, method, resources, &request_id).await?;

        Ok(AuthPrincipal {
            user_id: client.user_id,
            client_id: Some(client.id),
            scopes,
        })
    }

    /// Gate a client-credentials call on its client's own sliding-window
    /// budget, and report what decided it.
    ///
    /// A spent budget is reported and refused with its retry window; a
    /// server fault reading the window reports nothing and says nothing
    /// about the credential. An admitted call reports its budget, names its
    /// method, and hands the request-budget layer the client whose
    /// `a2a_usage` row it writes once the call has an outcome.
    async fn admit_client_call(
        client: &A2AClient,
        scopes: &[OAuthScope],
        method: &str,
        resources: &A2AResources,
        request_id: &Value,
    ) -> Result<(), Box<A2AResponse>> {
        let now = Utc::now();
        let budget = resources
            .ctx
            .repos()
            .a2a
            .get_client_window_usage(&client.id, a2a_client_window_start(client, now))
            .await
            .map(|usage| calculate_a2a_client_rate_limit(client, &usage, now))
            .map_err(|e| Box::new(Self::auth_refusal(&e, request_id.clone())))?;

        report_request_budget(budget);
        enforce_request_budget(budget, now)
            .map_err(|refusal| Box::new(Self::auth_refusal(&refusal, request_id.clone())))?;

        report_request_operation(method);
        report_a2a_client_request(A2AClientCall {
            client_id: client.id.clone(),
            protocol_version: A2A_VERSION.to_owned(),
            client_capabilities: client.capabilities.clone(),
            granted_scopes: scopes
                .iter()
                .map(|scope| scope.as_str().to_owned())
                .collect(),
        });
        Ok(())
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

    /// The refusal for a credential authentication did not admit: a user
    /// JWT the auth pipeline refused, or a client-credentials call over its
    /// client's budget.
    ///
    /// A spent budget keeps its retry window and is never an authentication
    /// failure: a client told its token is bad re-authenticates, and is
    /// refused again. A server-side failure while authenticating says nothing
    /// about the credential either. Mirrors `AppError::into_auth_refusal` on
    /// the REST routes.
    fn auth_refusal(error: &AppError, request_id: Value) -> A2AResponse {
        if error.code == ErrorCode::RateLimitExceeded {
            return Self::rate_limited_error(
                error.sanitized_message(),
                error.retry_after_secs().unwrap_or(1),
                Some(request_id),
            );
        }
        if error.is_server_fault() {
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
