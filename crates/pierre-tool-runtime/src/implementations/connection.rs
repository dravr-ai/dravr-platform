// ABOUTME: Connection management tools implementing the McpTool trait.
// ABOUTME: Provides connect_provider, get_connection_status, disconnect_provider tools.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Connection Management Tools
//!
//! This module contains tools for managing provider connections:
//! - `ConnectProviderTool` - Initiate OAuth flow for a provider
//! - `GetConnectionStatusTool` - Check provider connection status
//! - `DisconnectProviderTool` - Disconnect and revoke OAuth tokens

use std::collections::{BTreeMap, HashMap};
use std::env;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use pierre_auth::oauth2_client::OAuthClientState;
use pierre_auth::tenant::TenantContext;
use pierre_core::models::{ConnectionStatus, TenantId};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::{error, info, warn};

use crate::capabilities::ToolCapabilities;
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, capabilities_to_tronc, object_schema, ok_typed, tool_definition,
    tool_result_to_response,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities as TroncCapabilities, ToolContext};
use pierre_config::constants::oauth_config::AUTHORIZATION_EXPIRES_MINUTES;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::untrusted::display_line;
use pierre_database::RepositoryRegistry;
use pierre_mcp_schema::{PropertySchema, ToolAnnotations};
use pierre_providers::backend_resolver::{self, BackendKind, CoalescedStatus};
use pierre_providers::ProviderRegistry;
use pierre_services::delegated_connections::describe_link;
use pierre_services::oauth_flow::OAuthService;
use pierre_services::provider_notice::{require_notice_accepted, NOTICE_REFUSAL_ACTION};
use pierre_services::provider_revocation::DisconnectReason;
use pierre_tools_core::ToolResult;

/// The user-facing providers this build actually ships, in name order.
///
/// Read off [`ProviderRegistry`], which registers each provider under its own
/// cargo feature, with every mirror backend (`sciotte`, `sciotte_garmin`,
/// `sciotte_trainingpeaks`) folded into the user-facing provider it serves
/// (`strava`, `garmin`, `trainingpeaks`). Folding rather than dropping matters
/// for TrainingPeaks: it has no backend of its own, so it is only ever named
/// through its mirror. Deriving it means a provider the binary cannot connect
/// can never be named: the hand-written list this replaced went on offering
/// `fitbit`, `terra` and `coros` to MCP clients long after the 2026-Q2 cleanup
/// left them out of `server-production` (carnet#233).
fn user_facing_providers(registry: &ProviderRegistry) -> Vec<&'static str> {
    let mut names: Vec<&'static str> = registry
        .supported_providers()
        .into_iter()
        .map(backend_resolver::user_facing_name)
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// Whether the connection serving `user_facing` needs the athlete to reconnect.
///
/// Connection rows are stored under the BACKEND name (`sciotte_garmin`) while
/// the status tool reports the user-facing one (`garmin`), so a lookup by the
/// user-facing name never saw a mirror's dead session. When a backend serves
/// the provider, that backend's row decides — a dormant mirror row behind a
/// working Strava OAuth grant is not a reconnect. With nothing serving, any
/// serving backend's flagged row counts: a dead session whose token is gone is
/// still a reconnect, not a fresh connect.
fn serving_row_needs_reauth(
    status_by_provider: &HashMap<String, ConnectionStatus>,
    user_facing: &str,
    backend_kind: BackendKind,
) -> bool {
    let flagged = |backend: &str| {
        status_by_provider
            .get(backend)
            .is_some_and(ConnectionStatus::requires_reauth)
    };
    match backend_kind {
        BackendKind::Oauth => flagged(user_facing),
        BackendKind::Mirror => {
            backend_resolver::mirror_backend_for(user_facing).is_some_and(flagged)
        }
        // The session is the coach's, and nothing the athlete re-authorizes
        // renews it: the coach's own state is reported instead.
        BackendKind::Delegated => false,
        BackendKind::None => backend_resolver::serving_backends(user_facing)
            .iter()
            .any(|backend| flagged(backend)),
    }
}

/// Longest coach name the status reports for a delegated provider. The name
/// is the coach's own Dravr display name, so it reaches the model as one
/// defanged line.
const COACH_NAME_MAX_CHARS: usize = 60;

/// One provider's state, as both status shapes report it.
struct ProviderState {
    connected: bool,
    status: &'static str,
    needs_reauth: bool,
    backend: BackendKind,
    delegated_by: Option<String>,
}

/// The state of `provider`, which `status` coalesced, as the athlete reads it.
///
/// A provider read through the group coach's session (a delegated link)
/// names the coach, and reads `coach_reconnect_needed` while the coach's own
/// session needs the coach to sign in again: the athlete has nothing to
/// re-authorize, so `needs_reauth` stays false.
async fn provider_state(
    repos: &RepositoryRegistry,
    status_by_provider: &HashMap<String, ConnectionStatus>,
    provider: &str,
    status: CoalescedStatus,
) -> ProviderState {
    let needs_reauth = serving_row_needs_reauth(status_by_provider, provider, status.backend_kind);
    let (delegated_by, coach_needs_reconnect) = match status.delegation {
        Some(link) => match describe_link(repos, link).await {
            Ok(Some(view)) => (
                Some(display_line(&view.coach_name, COACH_NAME_MAX_CHARS)),
                view.coach_needs_reconnect,
            ),
            Ok(None) => (None, false),
            Err(e) => {
                warn!(error = %e, "Could not read the coach behind a delegated provider");
                (None, false)
            }
        },
        None => (None, false),
    };
    let word = if needs_reauth {
        "needs_reauth"
    } else if coach_needs_reconnect {
        "coach_reconnect_needed"
    } else if status.connected {
        "connected"
    } else {
        "disconnected"
    };
    ProviderState {
        connected: status.connected,
        status: word,
        needs_reauth,
        backend: status.backend_kind,
        delegated_by,
    }
}

/// Canonicalise a provider name into its static entry from
/// [`user_facing_providers`], or `None` if this build does not surface it.
fn user_facing_canonical(registry: &ProviderRegistry, name: &str) -> Option<&'static str> {
    user_facing_providers(registry)
        .into_iter()
        .find(|provider| *provider == name)
}

/// Validate redirect URL scheme for OAuth mobile flows
fn validate_redirect_url_scheme(url: &str) -> bool {
    url.starts_with("dravr://")
        || url.starts_with("exp://")
        || url.starts_with("http://localhost")
        || url.starts_with("https://")
}

/// Build OAuth state string with optional redirect URL
fn build_oauth_state(user_uuid: uuid::Uuid, redirect_url: Option<&str>) -> String {
    use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
    redirect_url.map_or_else(
        || format!("{}:{}", user_uuid, uuid::Uuid::new_v4()),
        |url| {
            let encoded_url = URL_SAFE_NO_PAD.encode(url.as_bytes());
            format!("{}:{}:{}", user_uuid, uuid::Uuid::new_v4(), encoded_url)
        },
    )
}

/// Build successful OAuth connection payload
fn build_oauth_success_payload(
    provider: &str,
    authorization_url: &str,
    state: &str,
) -> ConnectProviderResult {
    ConnectProviderResult {
        provider: provider.to_owned(),
        authorization_url: authorization_url.to_owned(),
        state: state.to_owned(),
        instructions: format!(
            "To connect your {provider} account:\n\
             1. Visit the authorization URL\n\
             2. Log in to {provider} and approve the connection\n\
             3. You will be redirected back to complete the connection\n\
             4. Once connected, you can access your {provider} data through Dravr"
        ),
        expires_in_minutes: AUTHORIZATION_EXPIRES_MINUTES,
        status: "pending_authorization".to_owned(),
    }
}

/// The error payload for a connect the provider's notice refused: where the
/// athlete accepts it, since no tool argument can.
fn notice_required_result(provider: &str, error: &str) -> ToolResult {
    ToolResult::error(json!({
        "error": format!(
            "{error}. The athlete accepts it by connecting {provider} from the Connections \
             screen of the Dravr app, which shows the notice with the box to tick."
        ),
        "error_type": "provider_notice_required",
        "provider": provider,
    }))
}

/// Build OAuth error payload merged into a `ToolResult` error.
fn oauth_error_result(provider: &str, error: &str) -> ToolResult {
    ToolResult::error(json!({
        "error": format!(
            "Failed to generate authorization URL: {error}. \
             Please check that OAuth credentials are configured for provider '{provider}'."
        ),
        "error_type": "oauth_configuration_error",
        "provider": provider,
    }))
}

/// Mint a provider OAuth authorization URL and persist its CSRF state row.
///
/// Shared by `connect_provider` (interactive connect) and the chat auth-recovery stage
/// (reconnect a dead OAuth provider). Applies the provider's notice precondition
/// ([`require_notice_accepted`]) first, builds the opaque `state`, asks the tenant OAuth
/// client for the provider's authorization URL, and stores an [`OAuthClientState`] row so
/// the callback can validate the round-trip. Returns `(authorization_url, state)`.
///
/// Neither caller can carry an acceptance: the notice (WHOOP's owner authorization) is
/// accepted by ticking it on a connect surface that shows it — the app's connect screens
/// or the hosted connect picker — never by a model's tool argument. So a provider whose
/// notice is outstanding for the account is refused here, and nothing is minted.
///
/// # Errors
///
/// Returns the precondition's refusal (`details.action = "accept_provider_notice"`) while
/// the provider's notice is outstanding, or an error if the tenant has no OAuth client for
/// the provider, the authorization URL cannot be generated, or the CSRF state row cannot
/// be persisted.
pub async fn mint_oauth_authorize_url(
    resources: &dyn ToolRuntime,
    user_id: uuid::Uuid,
    tenant_id: TenantId,
    provider: &str,
    redirect_url: Option<&str>,
) -> AppResult<(String, String)> {
    let brand = resources
        .provider_registry()
        .get_descriptor(provider)
        .map_or_else(|| provider.to_owned(), |d| d.display_name().to_owned());
    require_notice_accepted(
        resources.repos(),
        tenant_id.as_uuid(),
        user_id,
        provider,
        &brand,
        false,
    )
    .await?;

    let tenant_name = resources
        .repos()
        .tenants
        .get_by_id(tenant_id)
        .await
        .map_or_else(|_| "Unknown Tenant".to_owned(), |t| t.name);
    // Minting an authorize URL for a caller that already holds both ids; no
    // membership lookup happened, so no role is asserted.
    let tenant_context =
        TenantContext::for_tenant_scoped_operation(tenant_id, tenant_name, user_id);

    let state = build_oauth_state(user_id, redirect_url);

    let authorization = resources
        .tenant_oauth_client()
        .get_authorization_url(
            &tenant_context,
            provider,
            &state,
            resources.repos().tenants.as_ref(),
            resources.repos().oauth_tokens.as_ref(),
        )
        .await?;

    let now = Utc::now();
    let base_url = env::var("BASE_URL")
        .unwrap_or_else(|_| format!("http://localhost:{}", resources.config().http_port));
    let oauth_callback_uri = format!("{base_url}/api/oauth/callback/{provider}");
    let client_state = OAuthClientState {
        state: state.clone(),
        provider: provider.to_owned(),
        user_id: Some(user_id),
        tenant_id: Some(tenant_id.to_string()),
        redirect_uri: oauth_callback_uri,
        scope: None,
        pkce_code_verifier: None,
        // The shared-pool app the URL names, so the exchange uses its client.
        oauth_app_client_id: authorization.oauth_app_client_id,
        created_at: now,
        expires_at: now + Duration::minutes(i64::from(AUTHORIZATION_EXPIRES_MINUTES)),
        used: false,
    };
    resources
        .repos()
        .oauth_client_state
        .store_oauth_client_state(&client_state)
        .await?;

    Ok((authorization.url, state))
}

/// Whether `error` is the notice precondition's refusal rather than a failure
/// to build the authorization URL.
#[must_use]
pub fn is_notice_refusal(error: &AppError) -> bool {
    error
        .details
        .as_ref()
        .and_then(|details| details.get("action"))
        .and_then(Value::as_str)
        == Some(NOTICE_REFUSAL_ACTION)
}

/// Annotations for tools that interact with external OAuth services
fn open_world_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        open_world_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// Annotations for read-only connection status checks
fn read_only_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(true),
        destructive_hint: Some(false),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

/// Annotations for destructive operations like disconnect
fn destructive_annotations() -> ToolAnnotations {
    ToolAnnotations {
        read_only_hint: Some(false),
        destructive_hint: Some(true),
        idempotent_hint: Some(true),
        ..ToolAnnotations::default()
    }
}

// ============================================================================
// ConnectProviderTool - Initiate OAuth connection flow
// ============================================================================

/// What `connect_provider` answers with.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ConnectProviderResult {
    /// The provider being connected.
    pub provider: String,
    /// Where the athlete authorizes, single use.
    pub authorization_url: String,
    /// CSRF state tying the callback to this request.
    pub state: String,
    /// The steps, written for the athlete rather than the client.
    pub instructions: String,
    /// How long the URL stays valid, minutes.
    pub expires_in_minutes: u32,
    /// Always `pending_authorization`: the URL is issued, nothing is connected
    /// until the athlete comes back through the callback.
    pub status: String,
}

/// One provider's connection state, as reported inside the all-providers map.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProviderConnectionStatus {
    /// Whether a usable token is on file, or a coach's link serves it.
    pub connected: bool,
    /// The state in words: `connected`, `disconnected`, `needs_reauth`, or
    /// `coach_reconnect_needed` while the coach whose session serves a
    /// delegated provider must sign in again.
    pub status: String,
    /// Whether the athlete must authorize again.
    pub needs_reauth: bool,
    /// Which backend serves this provider: `oauth`, `mirror`, `delegated`
    /// (read through the group coach's own session) or `none`.
    pub backend: String,
    /// The coach whose session a `delegated` provider is read through.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegated_by: Option<String>,
}

impl From<ProviderState> for ProviderConnectionStatus {
    fn from(state: ProviderState) -> Self {
        Self {
            connected: state.connected,
            status: state.status.to_owned(),
            needs_reauth: state.needs_reauth,
            backend: state.backend.as_str().to_owned(),
            delegated_by: state.delegated_by,
        }
    }
}

/// What `get_connection_status` answers with.
///
/// Genuinely polymorphic: the tool answers a different shape depending on what
/// was asked. Modelled as an untagged enum so the derived schema is an
/// `anyOf` over the three real shapes, rather than one struct with everything
/// optional — which would describe none of them and would let a client
/// believe a `providers` map might arrive alongside a `provider` field.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ConnectionStatusResult {
    /// A named, known provider was asked about.
    Single {
        /// The provider asked about.
        provider: String,
        /// The state in words: `connected`, `disconnected`, `needs_reauth`,
        /// or `coach_reconnect_needed` while the coach whose session serves a
        /// delegated provider must sign in again.
        status: String,
        /// Whether a usable token is on file, or a coach's link serves it.
        connected: bool,
        /// Whether the athlete must authorize again.
        needs_reauth: bool,
        /// Which backend serves it: `oauth`, `mirror`, `delegated` (read
        /// through the group coach's own session) or `none`.
        backend: String,
        /// The coach whose session a `delegated` provider is read through.
        #[serde(skip_serializing_if = "Option::is_none")]
        delegated_by: Option<String>,
    },
    /// A name that is not a provider was asked about. Carries `note` instead of
    /// `needs_reauth`, because there is nothing to re-authorize.
    Unknown {
        /// The name that was asked about.
        provider: String,
        /// Always `disconnected`.
        status: String,
        /// Always false.
        connected: bool,
        /// Always `none`.
        backend: String,
        /// What to ask for instead.
        note: String,
    },
    /// No provider named, so every provider's state, keyed by name.
    All {
        /// Each provider's state.
        providers: BTreeMap<String, ProviderConnectionStatus>,
    },
}

/// What `disconnect_provider` answers with.
#[derive(Debug, Serialize, JsonSchema)]
pub struct DisconnectProviderResult {
    /// The provider that was disconnected, under its athlete-facing name — the
    /// mirror backend behind it is internal and stays that way.
    pub provider: String,
    /// Always `disconnected`.
    pub status: String,
    /// Confirmation for the athlete.
    pub message: String,
}

/// Tool for initiating OAuth connection flow with a fitness provider.
///
/// Generates an authorization URL that the user can visit to authenticate
/// with the provider. Supports optional redirect URL for mobile app flows.
pub struct ConnectProviderTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for ConnectProviderTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Provider to connect (e.g., 'strava', 'garmin', 'whoop')".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "redirect_url".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Optional redirect URL for mobile app OAuth flows (supports dravr://, exp://, http://localhost, https://)".to_owned(),
                ),
                ..Default::default()
            },
        );

        let schema = object_schema(properties, Some(vec!["provider".to_owned()]));

        answers_with::<ConnectProviderResult>(tool_definition(
            "connect_provider",
            "Initiate OAuth connection flow to connect a fitness data provider like Strava, Garmin, or WHOOP",
            schema,
            Some(open_world_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        // WRITES_DATA was missing: linking a provider writes an OAuth
        // grant onto the athlete's account. Its own twin,
        // `disconnect_provider`, has always declared the write — one half
        // of a pair declaring it and the other not is the evidence this
        // was an oversight rather than a decision. PROFILE because what
        // changes is which accounts are linked, i.e. who the athlete is.
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::REQUIRES_TENANT
                | ToolCapabilities::WRITES_DATA
                | ToolCapabilities::PROFILE,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
        let user_uuid = ctx.user_id;
        let registry = &ctx.resources.provider_registry();
        let repos = &ctx.resources.repos();

        let Some(provider) = args.get("provider").and_then(Value::as_str) else {
            let supported = user_facing_providers(registry).join(", ");
            return Ok(ToolResult::error(json!({
                "error": format!(
                    "Missing required 'provider' parameter. Supported providers: {supported}"
                )
            })));
        };

        if backend_resolver::is_mirror_backend(provider) {
            return Ok(ToolResult::error(json!({
                "error": format!(
                    "Unknown provider '{provider}'. Use '{}' instead.",
                    backend_resolver::user_facing_name(provider)
                )
            })));
        }
        let Some(provider) = user_facing_canonical(registry, provider) else {
            let supported = user_facing_providers(registry).join(", ");
            return Ok(ToolResult::error(json!({
                "error": format!(
                    "Provider '{provider}' is not supported. Supported providers: {supported}"
                )
            })));
        };

        let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

        // A provider whose mirror is its ONLY backend (Garmin, TrainingPeaks)
        // has no OAuth flow to start: Garmin's API is partner-gated and
        // uncredentialed, TrainingPeaks has none Pierre can call. Minting an
        // OAuth URL for either fails with a configuration error the agent then
        // relays as if the provider were broken. Raise the provider's
        // auth-required signal instead, whether or not a session row exists —
        // the chat pipeline answers it with a minted hosted-login link, and an
        // MCP client gets the structured reconnect code.
        //
        // Strava is not mirror-only: its OAuth backend is real, so a
        // sciotte-Strava user may still authorize OAuth, after which
        // `resolve_backend` routes them to the API and the mirror goes dormant.
        if let Some(mirror) = backend_resolver::mirror_backend_for(provider) {
            if backend_resolver::serving_backends(provider) == [mirror] {
                info!(
                    user_id = %user_uuid,
                    provider = provider,
                    "connect_provider: mirror-only provider, handing off to the hosted login"
                );
                return Err(AppError::provider_auth_required(mirror));
            }
        }

        let redirect_url = args.get("redirect_url").and_then(Value::as_str);
        if let Some(url) = redirect_url {
            if !validate_redirect_url_scheme(url) {
                return Ok(ToolResult::error(json!({
                    "error": "Invalid redirect_url scheme. Allowed: dravr://, exp://, http://localhost, https://"
                })));
            }
        }

        // SECURITY: Global lookup — connection handler, tenant resolved from user's membership
        match repos.users.get_global(user_uuid).await {
            Ok(Some(_)) => {}
            Ok(None) => {
                return Ok(ToolResult::error(json!({
                    "error": format!("User {user_uuid} not found")
                })))
            }
            Err(e) => {
                return Ok(ToolResult::error(json!({
                    "error": format!("Database error: {e}")
                })))
            }
        }

        // Security: always verify membership before using request.tenant_id.
        let tenants = repos
            .tenants
            .list_for_user(user_uuid)
            .await
            .unwrap_or_default();
        if !tenants.iter().any(|t| t.id == tenant_id) {
            return Ok(ToolResult::error(json!({
                "error": format!(
                    "User {user_uuid} is not a member of tenant {tenant_id}"
                )
            })));
        }
        match mint_oauth_authorize_url(
            ctx.resources.as_ref(),
            user_uuid,
            tenant_id,
            provider,
            redirect_url,
        )
        .await
        {
            Ok((url, state)) => {
                let flow_type = if redirect_url.is_some() {
                    " (mobile flow)"
                } else {
                    ""
                };
                info!(
                    "Generated OAuth URL for user {} provider {}{}",
                    user_uuid, provider, flow_type
                );
                ok_typed(
                    "connect_provider",
                    build_oauth_success_payload(provider, &url, &state),
                )
            }
            Err(e) if is_notice_refusal(&e) => {
                info!(
                    user_id = %user_uuid,
                    provider = provider,
                    "connect_provider: the provider's notice is outstanding, nothing minted"
                );
                Ok(notice_required_result(provider, &e.message))
            }
            Err(e) => {
                error!("OAuth URL generation failed for {}: {}", provider, e);
                Ok(oauth_error_result(provider, &e.to_string()))
            }
        }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// GetConnectionStatusTool - Check OAuth connection status
// ============================================================================

/// Tool for checking the connection status of fitness providers.
///
/// Can check a single provider's status or all supported providers.
pub struct GetConnectionStatusTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for GetConnectionStatusTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Optional: specific provider to check (e.g., 'strava'). If omitted, checks all providers.".to_owned(),
                ),
                ..Default::default()
            },
        );

        let schema = object_schema(properties, None);

        answers_with::<ConnectionStatusResult>(tool_definition(
            "get_connection_status",
            "Check the connection status of fitness data providers. If no provider is specified, returns status for all supported providers.",
            schema,
            Some(read_only_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::READS_DATA
                | ToolCapabilities::PROFILE,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let user_uuid = ctx.user_id;
            let tenant_id = TenantId::from_uuid(ctx.require_tenant()?);

            // Lifecycle status per connected provider (best-effort). Lets us report a
            // connected-but-dead provider as `needs_reauth` — a token row still exists, but a
            // non-recoverable refresh failure means the user must reconnect before data flows.
            let status_by_provider: HashMap<String, ConnectionStatus> = ctx
                .resources
                .repos()
                .provider_connections
                .get_for_user(user_uuid, Some(tenant_id))
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|c| (c.provider, c.status))
                .collect();

            if let Some(specific_provider) = args.get("provider").and_then(Value::as_str) {
                // Mirror backends are internal-only.
                if backend_resolver::is_mirror_backend(specific_provider) {
                    return ok_typed(
                        "get_connection_status",
                        ConnectionStatusResult::Unknown {
                            provider: specific_provider.to_owned(),
                            status: "disconnected".to_owned(),
                            connected: false,
                            backend: "none".to_owned(),
                            note: format!(
                                "Unknown provider. Use '{}' instead.",
                                backend_resolver::user_facing_name(specific_provider)
                            ),
                        },
                    );
                }

                let auth_repos = ctx.resources.repos().auth_repos();
                let coalesced = match user_facing_canonical(
                    ctx.resources.provider_registry(),
                    specific_provider,
                ) {
                    Some(canonical) => {
                        backend_resolver::coalesced_status(
                            &auth_repos,
                            user_uuid,
                            tenant_id,
                            canonical,
                        )
                        .await
                    }
                    None => CoalescedStatus {
                        user_facing: "",
                        connected: false,
                        backend_kind: BackendKind::None,
                        delegation: None,
                    },
                };
                let state = provider_state(
                    ctx.resources.repos(),
                    &status_by_provider,
                    specific_provider,
                    coalesced,
                )
                .await;

                ok_typed(
                    "get_connection_status",
                    ConnectionStatusResult::Single {
                        provider: specific_provider.to_owned(),
                        status: state.status.to_owned(),
                        connected: state.connected,
                        needs_reauth: state.needs_reauth,
                        backend: state.backend.as_str().to_owned(),
                        delegated_by: state.delegated_by,
                    },
                )
            } else {
                let mut providers_status: BTreeMap<String, ProviderConnectionStatus> =
                    BTreeMap::new();

                let auth_repos = ctx.resources.repos().auth_repos();
                for user_facing in user_facing_providers(ctx.resources.provider_registry()) {
                    let status = backend_resolver::coalesced_status(
                        &auth_repos,
                        user_uuid,
                        tenant_id,
                        user_facing,
                    )
                    .await;
                    let state = provider_state(
                        ctx.resources.repos(),
                        &status_by_provider,
                        user_facing,
                        status,
                    )
                    .await;
                    providers_status.insert(user_facing.to_owned(), state.into());
                }

                ok_typed(
                    "get_connection_status",
                    ConnectionStatusResult::All {
                        providers: providers_status,
                    },
                )
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// DisconnectProviderTool - Disconnect OAuth provider
// ============================================================================

/// Tool for disconnecting from a fitness provider by removing OAuth tokens.
pub struct DisconnectProviderTool;

#[async_trait]
impl McpTool<dyn ToolRuntime> for DisconnectProviderTool {
    fn definition(&self) -> Tool {
        let mut properties = HashMap::new();
        properties.insert(
            "provider".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Provider to disconnect (e.g., 'strava', 'garmin', 'whoop')".to_owned(),
                ),
                ..Default::default()
            },
        );

        let schema = object_schema(properties, Some(vec!["provider".to_owned()]));

        answers_with::<DisconnectProviderResult>(tool_definition(
            "disconnect_provider",
            "Disconnect from a fitness data provider by removing stored OAuth tokens",
            schema,
            Some(destructive_annotations()),
        ))
    }

    fn capabilities(&self) -> TroncCapabilities {
        capabilities_to_tronc(
            ToolCapabilities::REQUIRES_AUTH
                | ToolCapabilities::WRITES_DATA
                | ToolCapabilities::PROFILE,
        )
    }

    async fn execute(
        &self,
        state: &Arc<dyn ToolRuntime>,
        ctx: &ToolContext,
        args: Value,
    ) -> ToolResponse {
        let ctx = ToolExecutionContext::from_tronc(state, ctx);
        let result: AppResult<ToolResult> = async move {
            let user_uuid = ctx.user_id;

            let Some(provider) = args.get("provider").and_then(Value::as_str) else {
                // The user-facing names, never the registry's raw keys: those
                // include the mirror backends, which must not reach the LLM.
                let supported = user_facing_providers(ctx.resources.provider_registry()).join(", ");
                return Ok(ToolResult::error(json!({
                    "error": format!(
                        "Missing required 'provider' parameter. Supported providers: {supported}"
                    )
                })));
            };

            let tenant_id = ctx.require_tenant()?;

            // The domain chokepoint: `OAuthService::disconnect_provider`
            // resolves the sciotte mirror backend, deletes the token + the
            // connection row in lockstep, and emits the catalogued
            // `provider.disconnected` notify event. The disconnect never uses
            // the notification sender, so `None` is the correct third arg.
            let service = OAuthService::new(ctx.resources.data(), ctx.resources.config().clone());

            match service
                .disconnect_provider(
                    user_uuid,
                    provider,
                    Some(tenant_id),
                    DisconnectReason::Athlete,
                )
                .await
            {
                // Report the user-facing name — the mirror backend is internal.
                Ok(_) => ok_typed(
                    "disconnect_provider",
                    DisconnectProviderResult {
                        provider: provider.to_owned(),
                        status: "disconnected".to_owned(),
                        message: format!("Successfully disconnected from {provider}"),
                    },
                ),
                Err(e) => Ok(ToolResult::error(json!({
                    "error": format!("Failed to disconnect from {provider}: {e}")
                }))),
            }
        }
        .await;
        tool_result_to_response(result)
    }
}

// ============================================================================
// Module exports
// ============================================================================

/// Create all connection tools for registration
#[must_use]
pub fn create_connection_tools() -> Vec<Box<dyn RuntimeTool>> {
    vec![
        Box::new(ConnectProviderTool),
        Box::new(GetConnectionStatusTool),
        Box::new(DisconnectProviderTool),
    ]
}

// Guardian security classifications (see `crate::security`). Co-located here so
// each impl sits under this module's existing feature gate; the compiler forces
// every registered tool to classify (the registry stores `Arc<dyn RuntimeTool>`).
crate::declare_security!(DisconnectProviderTool => IRREVERSIBLE);
crate::declare_security!(ConnectProviderTool => empty);
crate::declare_security!(GetConnectionStatusTool => empty);
