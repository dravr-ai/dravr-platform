// ABOUTME: OAuth flow business logic extracted from route handlers
// ABOUTME: State parsing, redirect URL validation, PKCE, token exchange, provider disconnect, connection status
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::{
    collections::{HashMap, HashSet},
    env,
    fmt::Write,
    sync::Arc,
};

use chrono::Utc;
use tracing::{debug, error, field, info, Span};
use urlencoding::encode;

use crate::analytics::cache_user_email;
use crate::oauth_bridge_notify;
use crate::provider_revocation;
use crate::strava_reconnect::{self, ReplacedGrants, StorePrecondition};
use pierre_auth::config::oauth::get_oauth_config;
use pierre_auth::dto::auth::{ConnectionStatus, OAuthAuthorizationResponse};
use pierre_auth::oauth2_client::{
    OAuth2Client, OAuth2Config, OAuth2Token, OAuthClientState, PkceParams,
};
use pierre_auth::strava_pool;
use pierre_auth::tenant::oauth_manager::{issuing_client, IssuingClient, IssuingLookup};
use pierre_config::environment::ServerConfig;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::{ConnectionType, TenantId, User, UserOAuthToken};
use pierre_database::database::repositories::UserRepository;
use pierre_mcp_transport::OAuthCallbackResponse;

pub use crate::oauth_bridge_notify::{BridgeCallbackToken, BRIDGE_CALLBACK_TOKEN_HEADER};
pub use crate::oauth_state_redeem::ParsedOAuthState;
use pierre_providers::backend_resolver;
use pierre_runtime_context::DataContext;

/// What a flow's starter asks of it beyond the authorization itself.
#[derive(Debug, Default, Clone, Copy)]
pub struct AuthUrlOptions<'a> {
    /// A post-OAuth return URL, embedded as the third (base64) state segment —
    /// the same channel the mobile deep-link flow uses. The session-less
    /// callback decodes it and redirects success/failure there instead of the
    /// SPA. The channel-initiated hosted connect flow passes its picker URL so
    /// a failed Strava OAuth bounces the user back to the picker (which opens
    /// the Sciotte credential fallback) rather than stranding them on the SPA
    /// error page. The URL is validated against the redirect allowlist by the
    /// callback before it is honored.
    pub return_redirect: Option<&'a str>,
    /// The per-flow token of the SDK bridge listener that started the flow.
    /// Stored with the state and presented on the success notification, the
    /// only one this flow sends: a flow without it notifies no bridge.
    pub bridge_callback_token: Option<&'a BridgeCallbackToken>,
}

// ---------------------------------------------------------------------------
// OAuthService — core business logic for OAuth flows
// ---------------------------------------------------------------------------

/// OAuth service for OAuth flow business logic
#[derive(Clone)]
pub struct OAuthService {
    /// Crate-visible for the `provider_revocation` companion module.
    pub(crate) data: DataContext,
    config: Arc<ServerConfig>,
}

impl OAuthService {
    /// Creates a new OAuth service instance
    #[must_use]
    pub const fn new(data_context: DataContext, config: Arc<ServerConfig>) -> Self {
        Self {
            data: data_context,
            config,
        }
    }

    /// Get server configuration
    #[must_use]
    pub const fn config(&self) -> &Arc<ServerConfig> {
        &self.config
    }

    /// Handle OAuth callback
    ///
    /// Validates the state parameter against server-side storage to prevent CSRF attacks,
    /// then exchanges the authorization code for tokens. Uses PKCE when the code verifier
    /// was stored with the state during authorization URL generation.
    ///
    /// [`Self::redeem_state`] then [`Self::complete_callback`]; a caller that
    /// must know the verified mobile redirect even when the exchange fails
    /// calls the two itself.
    ///
    /// # Errors
    /// Returns error if OAuth state is invalid/expired/reused or callback processing fails
    pub async fn handle_callback(
        &self,
        code: &str,
        state: &str,
        provider: &str,
    ) -> AppResult<OAuthCallbackResponse> {
        let parsed_state = self.redeem_state(state, provider).await?;
        self.complete_callback(code, provider, parsed_state).await
    }

    /// Exchange the authorization code of a flow whose state
    /// [`Self::redeem_state`] already redeemed, store the token, and notify.
    ///
    /// # Errors
    /// Returns an error when the user or tenant cannot be resolved, the code
    /// exchange fails, or the token cannot be stored.
    #[tracing::instrument(
        skip(self, code, parsed_state),
        fields(
            provider = %provider,
            user_id = field::Empty,
            tenant_id = field::Empty,
        )
    )]
    pub async fn complete_callback(
        &self,
        code: &str,
        provider: &str,
        parsed_state: ParsedOAuthState,
    ) -> AppResult<OAuthCallbackResponse> {
        let user_id = parsed_state.user_id;
        // The shared-app pool member pinned at authorize (Strava). Used to
        // resolve the same client_id/secret at exchange and recorded on the
        // token so refresh uses that app's secret. `None` = env-default app.
        let oauth_app_client_id = parsed_state.oauth_app_client_id;
        let mobile_redirect_url = parsed_state.mobile_redirect_url;
        let bridge_callback_token = parsed_state.bridge_callback_token;
        let flow_label = if mobile_redirect_url.is_some() {
            " (mobile flow)"
        } else {
            ""
        };

        info!("Processing OAuth callback for user {user_id} provider {provider}{flow_label}");

        // Get user and tenant from database
        let (user, tenant_id) = self
            .get_user_and_tenant(user_id, provider, parsed_state.tenant_id)
            .await?;

        // Record IDs on the current span so the NotifyLayer can attribute the
        // provider.connected event without re-passing tenant/user fields.
        let span = Span::current();
        span.record("user_id", field::display(&user_id));
        span.record("tenant_id", field::display(&tenant_id));

        // The Strava tokens this connect replaces, read before the exchange so
        // the store lands only over the row read here (`strava_reconnect`).
        let storage_tenant = TenantId::parse_str(&tenant_id)
            .map_err(|_| AppError::internal(format!("Invalid tenant_id: {tenant_id}")))?;
        let replaced = ReplacedGrants::read(self, user_id, storage_tenant, provider).await;

        // Exchange OAuth code for access token (with PKCE if verifier was stored)
        // Pass tenant_id from state so exchange uses tenant-specific credentials if available
        let token = self
            .exchange_oauth_code(
                code,
                provider,
                user_id,
                parsed_state.pkce_code_verifier.as_deref(),
                parsed_state.tenant_id,
                oauth_app_client_id.as_deref(),
            )
            .await?;

        info!("Successfully exchanged OAuth code for user {user_id} provider {provider}");

        // A token that arrived without its provider-side owner id gets it now,
        // so a provider push event can be routed to this user.
        let token = self.with_provider_user_id(provider, user_id, token).await;

        // Persist token and dispatch all post-connection side effects. A failure
        // there still settles the grants: see `ReplacedGrants::abandon`.
        let attribution = oauth_app_client_id.as_deref();
        let expires_at = match self
            .finalize_oauth_connection(
                user_id,
                tenant_id,
                provider,
                &token,
                attribution,
                replaced.precondition(),
            )
            .await
        {
            Ok(expires_at) => expires_at,
            Err(error) => {
                replaced
                    .abandon(self, user_id, storage_tenant, &token, attribution)
                    .await;
                return Err(error);
            }
        };
        // The bridge that started this flow, if one did, learns it completed;
        // its listener accepts the POST only with the flow's own token.
        if let Some(callback_token) = bridge_callback_token.as_deref() {
            oauth_bridge_notify::notify_bridge_oauth_success(
                &self.config,
                provider,
                &token,
                callback_token,
            )
            .await;
        }
        // Now that the token is durable, a move onto another Strava app
        // withdraws the grants it supersedes.
        replaced.revoke_superseded(self, attribution).await;

        // notify: provider successfully linked. Fires after token persist +
        // notifications dispatch so a Slack ping only goes out for a usable link.
        // Warm the identity cache first — the enricher resolves user_email from
        // user_id, and this event is now the only connect notification, so
        // without it the ping would name the provider but not the athlete.
        cache_user_email(&user_id.to_string(), &user.email);
        info!(
            target: "notify",
            event = "provider.connected",
            provider = %provider,
            "user connected fitness provider"
        );

        Ok(OAuthCallbackResponse {
            user_id: user_id.to_string(),
            provider: provider.to_owned(),
            expires_at: expires_at.to_rfc3339(),
            scopes: token.scope.unwrap_or_else(|| "read".to_owned()),
            mobile_redirect_url,
        })
    }

    /// Persist the OAuth token and dispatch all post-connection side effects.
    ///
    /// Stores the token and the UI notification. The bridge notification and
    /// the `provider.connected` notify event are raised by the caller, after
    /// this returns, so neither goes out for a link that did not persist.
    async fn finalize_oauth_connection(
        &self,
        user_id: uuid::Uuid,
        tenant_id: String,
        provider: &str,
        token: &OAuth2Token,
        oauth_app_client_id: Option<&str>,
        precondition: StorePrecondition,
    ) -> AppResult<chrono::DateTime<chrono::Utc>> {
        let expires_at = self
            .store_oauth_token(
                user_id,
                tenant_id,
                provider,
                token,
                oauth_app_client_id,
                precondition,
            )
            .await?;
        self.store_oauth_notification(user_id, provider, &expires_at)
            .await?;

        // Health data backfill is triggered by the callback handler after this returns.
        // The scheduler will also auto-detect this user on subsequent cycles.
        #[cfg(feature = "health-sync")]
        {
            tracing::info!(
                user_id = %user_id,
                provider = provider,
                "Health data sync: provider connected, backfill triggered from callback handler"
            );
        }

        Ok(expires_at)
    }

    /// Validate that the provider is registered, or — like `trainingpeaks`,
    /// which only a mirror serves — that a backend serving it is.
    pub(crate) fn validate_provider(&self, provider: &str) -> AppResult<()> {
        let registry = self.data.provider_registry();
        let serving = backend_resolver::serving_backends(provider);
        if registry.is_supported(provider) || serving.iter().any(|b| registry.is_supported(b)) {
            return Ok(());
        }
        Err(AppError::invalid_input(format!(
            "Unsupported provider: {provider}"
        )))
    }

    /// Exchange OAuth code for access token, using PKCE when a code verifier is available
    ///
    /// When `tenant_id` is provided, attempts to use tenant-specific OAuth credentials
    /// (`client_id`, `client_secret`) before falling back to environment configuration.
    async fn exchange_oauth_code(
        &self,
        code: &str,
        provider: &str,
        user_id: uuid::Uuid,
        pkce_code_verifier: Option<&str>,
        tenant_id: Option<uuid::Uuid>,
        oauth_app_client_id: Option<&str>,
    ) -> AppResult<OAuth2Token> {
        let oauth_config = self
            .create_oauth_config_with_user(provider, user_id, tenant_id, oauth_app_client_id)
            .await?;
        let oauth_client = OAuth2Client::new(oauth_config)?;

        let token = if let Some(verifier) = pkce_code_verifier {
            // Use PKCE-enhanced token exchange when verifier was stored with the state
            let pkce = PkceParams {
                code_verifier: verifier.to_owned(),
                code_challenge: String::new(),
                code_challenge_method: "S256".to_owned(),
            };
            oauth_client
                .exchange_code_with_pkce(code, &pkce)
                .await
                .map_err(|e| {
                    error!(
                        "OAuth PKCE token exchange failed for {provider} - user_id: {user_id}, error: {e}",
                    );
                    AppError::internal(format!("Failed to exchange OAuth code for token: {e}"))
                })?
        } else {
            oauth_client.exchange_code(code).await.map_err(|e| {
                error!(
                    "OAuth token exchange failed for {provider} - user_id: {user_id}, error: {e}",
                );
                AppError::internal(format!("Failed to exchange OAuth code for token: {e}"))
            })?
        };

        Ok(token)
    }

    /// Create `OAuth2` config with user-specific credential priority
    ///
    /// The client is the one [`issuing_client`] resolves: the Strava pool app
    /// `oauth_app_client_id` names (pinned at authorize for the exchange,
    /// recorded on the token for a revocation), else the user's own app, the
    /// tenant's credentials, then the environment. The endpoints, token URL
    /// and PKCE flag come from the provider's descriptor.
    ///
    /// This ensures the token exchange uses the same credentials as the authorization
    /// URL generation, preventing `client_id` mismatches that cause "invalid code" errors.
    ///
    /// # Errors
    /// Returns error if provider is unsupported, a credential lookup fails, or
    /// no credentials are configured
    pub(crate) async fn create_oauth_config_with_user(
        &self,
        provider: &str,
        user_id: uuid::Uuid,
        tenant_id: Option<uuid::Uuid>,
        oauth_app_client_id: Option<&str>,
    ) -> AppResult<OAuth2Config> {
        let descriptor = self
            .data
            .provider_registry()
            .get_descriptor(provider)
            .ok_or_else(|| AppError::invalid_input(format!("Unsupported provider: {provider}")))?;
        let endpoints = descriptor.oauth_endpoints().ok_or_else(|| {
            AppError::invalid_input(format!("Provider {provider} does not support OAuth"))
        })?;
        let params = descriptor.oauth_params().ok_or_else(|| {
            AppError::invalid_input(format!("Provider {provider} OAuth params not configured"))
        })?;

        let server_level = get_oauth_config(provider);
        let repos = self.data.repos();
        let client = issuing_client(
            IssuingLookup {
                user_id: Some(user_id),
                tenant_id: tenant_id.map(TenantId::from_uuid),
                provider,
                issuing_app: oauth_app_client_id,
                server_level: &server_level,
                cached_tenant: None,
            },
            repos.tenants.as_ref(),
            repos.oauth_tokens.as_ref(),
        )
        .await?;

        let default_scopes = || {
            descriptor
                .default_scopes()
                .iter()
                .map(|s| (*s).to_owned())
                .collect::<Vec<_>>()
                .join(params.scope_separator)
        };
        let (client_id, client_secret, redirect_uri, scopes) = match client {
            IssuingClient::UserApp(app) => (
                app.client_id,
                app.client_secret,
                app.redirect_uri,
                default_scopes(),
            ),
            IssuingClient::Tenant(credentials) => {
                let scopes = credentials.scopes.join(params.scope_separator);
                (
                    credentials.client_id,
                    credentials.client_secret,
                    credentials.redirect_uri,
                    scopes,
                )
            }
            IssuingClient::StravaPool {
                client_id,
                client_secret,
            }
            | IssuingClient::ServerLevel {
                client_id,
                client_secret,
            } => {
                // Use BASE_URL when set, for tunnel/external access
                let redirect_uri = server_level.redirect_uri.clone().unwrap_or_else(|| {
                    let base_url = env::var("BASE_URL")
                        .unwrap_or_else(|_| format!("http://localhost:{}", self.config.http_port));
                    format!("{base_url}/api/oauth/callback/{provider}")
                });
                (client_id, client_secret, redirect_uri, default_scopes())
            }
        };

        Ok(OAuth2Config {
            client_id,
            client_secret,
            auth_url: endpoints.auth_url.to_owned(),
            token_url: self.token_url_for(provider, &endpoints),
            redirect_uri,
            scopes: vec![scopes],
            use_pkce: params.use_pkce,
        })
    }

    /// Store OAuth token in database, over the row `precondition` names
    async fn store_oauth_token(
        &self,
        user_id: uuid::Uuid,
        tenant_id: String,
        provider: &str,
        token: &OAuth2Token,
        oauth_app_client_id: Option<&str>,
        precondition: StorePrecondition,
    ) -> AppResult<chrono::DateTime<chrono::Utc>> {
        let expires_at = token
            .expires_at
            .unwrap_or_else(|| chrono::Utc::now() + chrono::Duration::hours(1));

        let user_oauth_token = UserOAuthToken {
            id: uuid::Uuid::new_v4().to_string(),
            user_id,
            tenant_id,
            provider: provider.to_owned(),
            access_token: token.access_token.clone(),
            refresh_token: token.refresh_token.clone(),
            token_type: token.token_type.clone(),
            expires_at: Some(expires_at),
            scope: token.scope.clone(),
            // Provider-side owner id (Strava athlete id) captured at token
            // exchange. Persisting it lets provider push events (e.g.
            // Strava webhooks) be routed to the single owning user.
            provider_user_id: token.provider_user_id.clone(),
            // Which shared-app pool member issued this token (pinned at authorize)
            // so refresh resolves the matching secret. `None` = env-default app.
            oauth_app_client_id: oauth_app_client_id.map(str::to_owned),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        strava_reconnect::store_guarded(
            self.data.repos().oauth_tokens.as_ref(),
            &user_oauth_token,
            precondition,
        )
        .await?;

        // Register provider connection alongside the OAuth token
        let raw_tenant = &user_oauth_token.tenant_id;
        let connection_tenant_id = TenantId::parse_str(raw_tenant).map_err(|_| {
            AppError::internal(format!("Invalid tenant_id in OAuth token: {raw_tenant}"))
        })?;
        self.data
            .repos()
            .provider_connections
            .register_connection(
                user_id,
                connection_tenant_id,
                provider,
                &ConnectionType::OAuth,
                None,
            )
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to register provider connection: {e}"))
            })?;

        Ok(expires_at)
    }

    /// Store OAuth notification in database
    async fn store_oauth_notification(
        &self,
        user_id: uuid::Uuid,
        provider: &str,
        expires_at: &chrono::DateTime<chrono::Utc>,
    ) -> AppResult<()> {
        let notification_id = self
            .data
            .repos()
            .notifications
            .store(
                user_id,
                provider,
                true,
                "OAuth authorization completed successfully",
                Some(&expires_at.to_rfc3339()),
            )
            .await
            .map_err(|e| AppError::database(format!("Failed to store OAuth notification: {e}")))?;

        info!(
            "Created OAuth completion notification {} for user {} provider {}",
            notification_id, user_id, provider
        );

        Ok(())
    }

    /// Disconnect OAuth provider for user
    ///
    /// The domain chokepoint for provider disconnects: every surface (REST
    /// route, chat tool loop, `/mcp` + SSE carve-out) funnels here, so the
    /// backend resolution, the lockstep deletes and the `provider.disconnected`
    /// notify event cannot drift apart per transport; `reason` says on that
    /// event who asked. Returns what the provider said about the grant, which
    /// never blocks the local deletion.
    ///
    /// # Errors
    /// Returns error if provider is unsupported or disconnection fails
    pub async fn disconnect_provider(
        &self,
        user_id: uuid::Uuid,
        provider: &str,
        active_tenant_id: Option<uuid::Uuid>,
        reason: provider_revocation::DisconnectReason,
    ) -> AppResult<provider_revocation::RevocationOutcome> {
        debug!(
            "Processing OAuth provider disconnect for user {} provider {}",
            user_id, provider
        );

        // Validate provider is supported
        self.validate_provider(provider)?;

        // Use active_tenant_id from JWT claims (user's selected tenant)
        let tenant_id = active_tenant_id.map(TenantId::from_uuid).ok_or_else(|| {
            AppError::auth_invalid("No active tenant in session — cannot disconnect provider")
        })?;

        // Clear the whole coalesced pair, not just the resolved backend: the
        // card the user acted on may be served by either half, and the clients
        // name it differently, so the pairing belongs here rather than in each
        // of them. See `backend_pair_for` for why resolution alone is not
        // enough.
        let user_facing = backend_resolver::user_facing_name(provider).to_owned();
        let backends = backend_resolver::backend_pair_for(provider);

        let mut outcomes = Vec::with_capacity(backends.len());
        for backend in &backends {
            outcomes.push(
                provider_revocation::clear_backend(self, &self.data, user_id, tenant_id, backend)
                    .await?,
            );
        }

        // Post-condition: prove the state changed before reporting success (see
        // `surviving_rows` for why a false success is invisible here). ERROR,
        // not WARN, because the alert policy filters `severity>=ERROR` — a WARN
        // would reach nobody, which is the same gap in a quieter form.
        let survivors =
            provider_revocation::surviving_rows(&self.data, user_id, tenant_id, &backends).await;
        if !survivors.is_empty() {
            error!(
                requested = %provider,
                user_facing = %user_facing,
                user_id = %user_id,
                tenant_id = %tenant_id,
                survivors = ?survivors,
                "disconnect deleted its backends but rows survived — refusing to report success the client would render as disconnected"
            );
            return Err(AppError::internal(format!(
                "Disconnect of {user_facing} did not clear every stored credential"
            )));
        }

        // notify: provider revoked. Emitted here rather than on any transport
        // so chat/MCP disconnects count the same as REST ones, and carries
        // user_id/tenant_id inline (the user-facing provider name, not the
        // mirror backend) because the messaging ingress span has neither and
        // the PostHog sink drops events it cannot attribute.
        info!(
            target: "notify",
            event = "provider.disconnected",
            provider = %user_facing,
            user_id = %user_id,
            tenant_id = %tenant_id,
            reason = reason.as_str(),
            "user disconnected fitness provider"
        );

        Ok(provider_revocation::RevocationOutcome::combine(outcomes))
    }

    /// Generate OAuth authorization URL for provider
    ///
    /// This function supports both multi-tenant and single-tenant modes:
    /// - Multi-tenant: Uses tenant-specific OAuth credentials from database
    /// - Single-tenant: Falls back to server-level configuration
    ///
    /// Stores the OAuth state server-side with TTL for CSRF protection, and generates
    /// PKCE parameters when the provider declares `use_pkce=true`. What else the
    /// flow's starter asks of it — a post-OAuth return URL, a bridge to notify —
    /// rides [`AuthUrlOptions`].
    ///
    /// # Errors
    /// Returns error if provider is unsupported or OAuth credentials not configured
    pub async fn get_auth_url(
        &self,
        user_id: uuid::Uuid,
        tenant_id: TenantId,
        provider: &str,
        options: AuthUrlOptions<'_>,
    ) -> AppResult<OAuthAuthorizationResponse> {
        // Get provider descriptor from registry
        let descriptor = self
            .data
            .provider_registry()
            .get_descriptor(provider)
            .ok_or_else(|| AppError::invalid_input(format!("Unsupported provider: {provider}")))?;

        // Get OAuth endpoints and params from descriptor
        let endpoints = descriptor.oauth_endpoints().ok_or_else(|| {
            AppError::invalid_input(format!("Provider {provider} does not support OAuth"))
        })?;
        let params = descriptor.oauth_params().ok_or_else(|| {
            AppError::invalid_input(format!("Provider {provider} OAuth params not configured"))
        })?;

        let use_pkce = params.use_pkce;

        // Check for tenant-specific OAuth credentials first (multi-tenant mode)
        let tenant_creds = self
            .data
            .repos()
            .tenants
            .get_oauth_credentials(tenant_id, provider)
            .await
            .map_err(|e| {
                AppError::database(format!("Failed to get tenant OAuth credentials: {e}"))
            })?;

        // Embed the optional return URL as the third state segment (base64), so
        // the callback bounces success/failure there. base64 URL_SAFE_NO_PAD
        // never emits ':' so the segment split stays unambiguous.
        let state = options.return_redirect.map_or_else(
            || format!("{}:{}", user_id, uuid::Uuid::new_v4()),
            |url| {
                use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
                format!(
                    "{}:{}:{}",
                    user_id,
                    uuid::Uuid::new_v4(),
                    URL_SAFE_NO_PAD.encode(url.as_bytes())
                )
            },
        );
        // Use BASE_URL environment variable if set, otherwise fall back to localhost.
        // This allows dynamic OAuth callbacks when using tunnels for local development.
        let base_url = env::var("BASE_URL")
            .unwrap_or_else(|_| format!("http://localhost:{}", self.config.http_port));
        let redirect_uri = format!("{base_url}/api/oauth/callback/{provider}");

        // Generate PKCE parameters when provider supports it
        let pkce = if use_pkce {
            Some(PkceParams::generate())
        } else {
            None
        };

        // URL-encode parameters for OAuth URLs
        let encoded_state = encode(&state);
        let encoded_redirect_uri = encode(&redirect_uri);

        // Determine client_id, scopes, and (for Strava) the shared-app pool
        // attribution to pin on the state so the exchange uses the same app.
        let (client_id, scope, oauth_app_attribution) = if let Some(creds) = tenant_creds {
            // Multi-tenant: use tenant-specific credentials (no pool involved).
            let scope = creds.scopes.join(params.scope_separator);
            (creds.client_id, scope, None)
        } else if provider.eq_ignore_ascii_case("strava") {
            // Server-level Strava: an athlete reconnects on the app that issued
            // their token while it has room; anyone else fills the env-default
            // app first, then a pool app with a free seat. The choice is pinned
            // on the state so the token exchange uses the same client_id/secret.
            let selected = strava_pool::select_strava_app(
                self.data.repos().oauth_tokens.as_ref(),
                user_id,
                tenant_id,
            )
            .await?;
            let scope = descriptor.default_scopes().join(params.scope_separator);
            (selected.client_id, scope, selected.attribution)
        } else {
            // Single-tenant: use environment configuration
            let env_config = get_oauth_config(provider);
            let client_id = env_config.client_id.ok_or_else(|| {
                AppError::invalid_input(format!(
                    "{provider} client_id not configured (set in environment or database)"
                ))
            })?;
            let scope = descriptor.default_scopes().join(params.scope_separator);
            (client_id, scope, None)
        };

        let encoded_scope = encode(&scope);

        // Build authorization URL with provider-specific parameters
        let mut auth_url = format!(
            "{}?client_id={}&response_type=code&redirect_uri={}&scope={}&state={}",
            endpoints.auth_url, client_id, encoded_redirect_uri, encoded_scope, encoded_state
        );

        // Add PKCE code_challenge to authorization URL when enabled
        if let Some(ref pkce_params) = pkce {
            use Write;
            let _ = write!(
                &mut auth_url,
                "&code_challenge={}&code_challenge_method={}",
                encode(&pkce_params.code_challenge),
                encode(&pkce_params.code_challenge_method)
            );
        }

        // Add provider-specific additional parameters
        for (key, value) in params.additional_auth_params {
            use Write;
            // Writing to String cannot fail
            let _ = write!(&mut auth_url, "&{}={}", encode(key), encode(value));
        }

        let authorization_url = auth_url;

        // Store state server-side for CSRF protection with 10-minute TTL.
        // The code_challenge field stores the PKCE code_verifier (needed during
        // token exchange to prove we initiated the authorization request).
        let now = Utc::now();
        let client_state = OAuthClientState {
            state: state.clone(),
            provider: provider.to_owned(),
            user_id: Some(user_id),
            tenant_id: Some(tenant_id.to_string()),
            redirect_uri,
            scope: Some(scope),
            pkce_code_verifier: pkce.as_ref().map(|p| p.code_verifier.clone()),
            oauth_app_client_id: oauth_app_attribution,
            bridge_callback_token: options
                .bridge_callback_token
                .map(|token| token.as_str().to_owned()),
            created_at: now,
            expires_at: now + chrono::Duration::minutes(10),
            used: false,
        };

        self.data
            .repos()
            .oauth_client_state
            .store_oauth_client_state(&client_state)
            .await
            .map_err(|e| {
                error!("Failed to store OAuth state for CSRF protection: {}", e);
                AppError::internal(format!("Failed to initiate OAuth flow: {e}"))
            })?;

        debug!(
            "Generated OAuth authorization URL for user {} tenant {} provider {}",
            user_id, tenant_id, provider
        );

        Ok(OAuthAuthorizationResponse {
            authorization_url,
            state,
            instructions: format!("Click the link to authorize {provider} access"),
            expires_in_minutes: 10,
        })
    }

    /// Get connection status for all providers for a user
    ///
    /// Uses `provider_connections` table as the single source of truth.
    /// For OAuth connections, also looks up token expiry and scope info.
    ///
    /// # Errors
    /// Returns error if database operation fails
    pub async fn get_connection_status(
        &self,
        user_id: uuid::Uuid,
    ) -> AppResult<Vec<ConnectionStatus>> {
        debug!("Getting provider connection status for user {}", user_id);

        // Get all provider connections (cross-tenant view)
        let connections = self
            .data
            .repos()
            .provider_connections
            .get_for_user(user_id, None)
            .await
            .map_err(|e| AppError::database(format!("Failed to get provider connections: {e}")))?;

        // For OAuth connections, look up token expiry/scope info
        let oauth_tokens = self
            .data
            .repos()
            .oauth_tokens
            .get_tokens(user_id, None)
            .await
            .unwrap_or_default();

        let token_map: HashMap<String, &UserOAuthToken> = oauth_tokens
            .iter()
            .map(|t| (t.provider.clone(), t))
            .collect();

        let mut providers_seen = HashSet::new();
        let mut statuses = Vec::new();

        // Build status for each connected provider
        for conn in &connections {
            if providers_seen.insert(conn.provider.clone()) {
                let (expires_at, scopes) = if conn.connection_type == ConnectionType::OAuth {
                    // Look up OAuth token details for expiry/scope info
                    token_map.get(&conn.provider).map_or((None, None), |t| {
                        (t.expires_at.map(|dt| dt.to_rfc3339()), t.scope.clone())
                    })
                } else {
                    (None, None)
                };

                statuses.push(ConnectionStatus {
                    provider: conn.provider.clone(),
                    connected: true,
                    connection_type: Some(conn.connection_type.as_str().to_owned()),
                    expires_at,
                    scopes,
                });
            }
        }

        // Add default disconnected status for all registered OAuth providers not in connections
        for provider_name in self.data.provider_registry().oauth_providers() {
            if !providers_seen.contains(provider_name) {
                statuses.push(ConnectionStatus {
                    provider: provider_name.to_owned(),
                    connected: false,
                    connection_type: None,
                    expires_at: None,
                    scopes: None,
                });
            }
        }

        Ok(statuses)
    }
}

// ---------------------------------------------------------------------------
// Helper functions used by OAuth handlers
// ---------------------------------------------------------------------------

/// Parse a user ID string to UUID
///
/// # Errors
/// Returns `AppError::invalid_input` if the string is not a valid UUID.
pub fn parse_user_id(user_id_str: &str) -> Result<uuid::Uuid, AppError> {
    uuid::Uuid::parse_str(user_id_str).map_err(|_| {
        error!("Invalid user_id format: {}", user_id_str);
        AppError::invalid_input("Invalid user ID format")
    })
}

/// Retrieve user from database with proper error handling
///
/// # Errors
/// Returns `AppError::not_found` when the user does not exist, or
/// `AppError::database` when the underlying repository call fails.
pub async fn get_user_for_oauth(
    users: &dyn UserRepository,
    user_id: uuid::Uuid,
) -> Result<User, AppError> {
    match users.get_global(user_id).await {
        Ok(Some(user)) => Ok(user),
        Ok(None) => {
            error!("User {} not found in database", user_id);
            Err(AppError::not_found("User account not found"))
        }
        Err(e) => {
            error!("Failed to get user {} for OAuth: {}", user_id, e);
            Err(AppError::database(format!(
                "Failed to retrieve user information: {e}"
            )))
        }
    }
}

/// Extract tenant ID for OAuth operations from JWT claims
///
/// Returns the `active_tenant_id` from the user's JWT session.
///
/// # Errors
/// Returns `AppError::auth_invalid` when no active tenant is set in the session.
pub fn extract_tenant_id(active_tenant_id: Option<TenantId>) -> Result<TenantId, AppError> {
    active_tenant_id.ok_or_else(|| AppError::auth_invalid("No active tenant in session"))
}

/// Categorize OAuth errors for better user messaging
#[must_use]
pub fn categorize_oauth_error(error: &AppError) -> (&'static str, Option<&'static str>) {
    let error_str = error.to_string().to_lowercase();

    if error_str.contains("jwt") && error_str.contains("expired") {
        (
            "Your session has expired",
            Some("Please log in again to continue with OAuth authorization"),
        )
    } else if error_str.contains("jwt") && error_str.contains("invalid signature") {
        (
            "Invalid authentication token",
            Some("The authentication token signature is invalid. This may happen if the server's secret key has changed. Please log in again."),
        )
    } else if error_str.contains("jwt") && error_str.contains("malformed") {
        (
            "Malformed authentication token",
            Some("The authentication token format is invalid. Please log in again."),
        )
    } else if error_str.contains("jwt") {
        (
            "Authentication token validation failed",
            Some("There was an issue validating your authentication token. Please log in again."),
        )
    } else if error_str.contains("user not found") {
        (
            "User account not found",
            Some("The user account associated with this OAuth request could not be found."),
        )
    } else if error_str.contains("tenant") {
        (
            "Tenant configuration error",
            Some("There was an issue with your account's tenant configuration. Please contact support."),
        )
    } else if error_str.contains("oauth code") || error_str.contains("token exchange") {
        (
            "OAuth token exchange failed",
            Some("Failed to exchange the authorization code for an access token. The provider may have rejected the request."),
        )
    } else if error_str.contains("state parameter") {
        (
            "Invalid OAuth state",
            Some("The OAuth state parameter is invalid or has been tampered with. This is a security measure to prevent CSRF attacks."),
        )
    } else {
        (
            "OAuth authorization failed",
            Some("An unexpected error occurred during the OAuth authorization process."),
        )
    }
}
