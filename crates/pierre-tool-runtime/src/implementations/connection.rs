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

use crate::annotations::{annotations, Effect, Reach};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use pierre_core::models::TenantId;
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{json, Value};
use tracing::{error, info, warn};

use super::calendar::{calendars_held, recorded_calendar, CalendarConnections};
use super::field_dictionary::{dictionary, FieldDictionary};
use crate::context::ToolExecutionContext;
use crate::conversions::{
    answers_with, object_schema, ok_typed, tool_definition, tool_result_to_response,
};
use crate::protocol::provider_offer::{
    describe_offers, provider_offers, readable_over, APP_ONLY_NOTE,
};
use crate::protocol::provider_states::{
    offered_providers, provider_state, read_status_by_provider, user_facing_providers,
    ProviderState,
};
use crate::runtime::ToolRuntime;
use crate::security::RuntimeTool;
use dravr_tronc::mcp::schema::{Tool, ToolResponse};
use dravr_tronc::mcp::tool::{McpTool, ToolCapabilities, ToolContext};
use pierre_auth::connect_order::{new_connection_through_oauth, order_and_seats};
/// The seat refusal [`mint_oauth_authorize_url`] answers with, read where its
/// callers branch on it.
pub use pierre_auth::garmin_seats::is_no_seat_refusal;
use pierre_auth::garmin_seats::require_garmin_seat;
use pierre_config::constants::oauth_config::AUTHORIZATION_EXPIRES_MINUTES;
use pierre_core::constants::oauth::providers::SCIOTTE_GARMIN;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::transport::Transport;
use pierre_mcp_schema::PropertySchema;
use pierre_providers::ai_scope;
use pierre_providers::backend_resolver::{self, BackendKind, CoalescedStatus};
use pierre_providers::field_support::FieldMode;
use pierre_providers::ProviderRegistry;
use pierre_services::oauth_flow::{AuthUrlOptions, OAuthService};
use pierre_services::provider_notice::{require_notice_accepted, NOTICE_REFUSAL_ACTION};
use pierre_services::provider_revocation::DisconnectReason;
use pierre_tools_core::ToolResult;

/// What `connect_provider` answers when no provider is named: the providers
/// this deployment offers with the athlete's state on each, read from the same
/// list `get_connection_status` gives. A caller on an external transport also
/// reads which of them keep their data inside the Dravr app. When the list
/// cannot be read it names none.
async fn name_a_provider_text(ctx: &ToolExecutionContext, transport: Option<Transport>) -> String {
    let transport = transport.unwrap_or(Transport::McpHttp);
    let offers = provider_offers(
        ctx.resources.as_ref(),
        ctx.user_id,
        ctx.tenant_id,
        transport,
    )
    .await;
    if offers.is_empty() {
        return "Missing required 'provider' parameter. Nothing was started. Call \
                get_connection_status for the providers on offer, then ask the athlete which one."
            .to_owned();
    }
    format!(
        "Missing required 'provider' parameter, so nothing was started. Ask the athlete \
         which of these to connect:\n{}",
        describe_offers(&offers)
    )
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

/// Whether a chat connect of `provider` hands off to its account sign-in.
///
/// It does for every provider with a mirror unless the operator's order puts
/// its OAuth first and a seat is free there ([`new_connection_through_oauth`],
/// the rule the provider card reads), and always for a provider whose grant
/// the athlete holds dead: a dead grant reconnects only through the hosted
/// sign-in (carnet#1494). A provider with no OAuth Pierre can call
/// (TrainingPeaks, COROS) has no seat, so it always signs in.
async fn signs_in_first(
    ctx: &ToolExecutionContext,
    user_id: uuid::Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> bool {
    if backend_resolver::mirror_backend_for(provider).is_none() {
        return false;
    }
    let repos = ctx.resources.repos();
    if backend_resolver::flagged_grant(repos, user_id, tenant_id, provider)
        .await
        .is_some()
    {
        return true;
    }
    let (order, seats) = order_and_seats(
        repos.system_settings.as_ref(),
        repos.oauth_tokens.as_ref(),
        repos.provider_connections.as_ref(),
        &ctx.resources.config().oauth,
        provider,
    )
    .await;
    !new_connection_through_oauth(order, seats)
}

/// Mint a provider OAuth authorization URL and persist its CSRF state row.
///
/// Shared by `connect_provider` (interactive connect) and the chat auth-recovery stage
/// (reconnect a dead OAuth provider). Applies the provider's notice precondition
/// ([`require_notice_accepted`]) first, then mints the URL through the authorize builder
/// every surface shares ([`OAuthService::get_auth_url`]): the client the code exchange will
/// resolve, the provider's endpoints, scopes and PKCE, and the state row the callback
/// redeems. Returns `(authorization_url, state)`.
///
/// Neither caller can carry an acceptance: the notice (WHOOP's owner authorization) is
/// accepted by ticking it on a connect surface that shows it — the app's connect screens
/// or the hosted connect picker — never by a model's tool argument. So a provider whose
/// notice is outstanding for the account is refused here, and nothing is minted.
///
/// Garmin's OAuth takes a seat of a capped pool, so its authorization URL is minted only
/// while [`require_garmin_seat`] lets this athlete through — the rule every other surface
/// that starts Garmin's OAuth reads. With the pool full the caller offers the usr/pwd
/// sign-in instead.
///
/// # Errors
///
/// Returns the precondition's refusal (`details.action = "accept_provider_notice"`) while
/// the provider's notice is outstanding, the seat refusal ([`is_no_seat_refusal`]) for a
/// Garmin athlete with no grant while the pool is full, or an error if no OAuth app (the
/// user's own, the tenant's or the server's) is configured for the provider, the
/// authorization URL cannot be generated, or the CSRF state row cannot be persisted.
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
    require_garmin_seat(
        resources.repos().oauth_tokens.as_ref(),
        resources.repos().provider_connections.as_ref(),
        &resources.config().oauth,
        user_id,
        tenant_id,
        provider,
    )
    .await?;

    let authorization = OAuthService::new(resources.data(), resources.config().clone())
        .get_auth_url(
            user_id,
            tenant_id,
            provider,
            AuthUrlOptions {
                return_redirect: redirect_url,
            },
        )
        .await?;

    Ok((authorization.authorization_url, authorization.state))
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

// ============================================================================
// ConnectProviderTool - Initiate OAuth connection flow
// ============================================================================

/// What `connect_provider` answers with.
#[derive(Serialize, JsonSchema)]
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

impl fmt::Debug for ConnectProviderResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectProviderResult")
            .field("provider", &self.provider)
            // The URL carries `state` in its query string.
            .field("authorization_url", &"[REDACTED]")
            .field("state", &"[REDACTED]")
            .field("instructions", &self.instructions)
            .field("expires_in_minutes", &self.expires_in_minutes)
            .field("status", &self.status)
            .finish()
    }
}

/// One provider's connection state, as reported inside the all-providers map.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ProviderConnectionStatus {
    /// Whether a usable token is on file, or a coach's link serves it.
    pub connected: bool,
    /// The state in words: `connected`, `disconnected`, `needs_reauth`,
    /// `coach_reconnect_needed` while the coach whose session serves a
    /// delegated provider must sign in again, or why a delegated provider
    /// reads nothing (`athlete_email_missing`, `athlete_email_mismatch`,
    /// `member_email_unverified`) until the coach links the athlete again.
    pub status: String,
    /// Whether the athlete must authorize again.
    pub needs_reauth: bool,
    /// Which backend serves this provider: `oauth`, `mirror`, `delegated`
    /// (read through the group coach's own session) or `none`.
    pub backend: String,
    /// The coach whose session a `delegated` provider is read through.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegated_by: Option<String>,
    /// Which activity fields each way of reading the provider supplies, and
    /// which way serves. Absent for a provider that serves no activities.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fields: Option<FieldDictionary>,
}

impl ProviderConnectionStatus {
    /// `provider`'s reported state, with its field dictionary.
    fn of(registry: &ProviderRegistry, provider: &str, state: ProviderState) -> Self {
        Self {
            connected: state.connected,
            status: state.status.to_owned(),
            needs_reauth: state.needs_reauth,
            backend: state.backend.as_str().to_owned(),
            fields: dictionary(registry, provider, serving_mode(provider, state.backend)),
            delegated_by: state.delegated_by,
        }
    }
}

/// The mode the field dictionary names for a provider served by `backend`:
/// a delegated read goes through the coach's session on the provider's
/// mirror when it has one (TrainingPeaks), its API otherwise.
fn serving_mode(provider: &str, backend: BackendKind) -> Option<FieldMode> {
    match backend {
        BackendKind::Mirror => Some(FieldMode::Mirror),
        BackendKind::Oauth => Some(FieldMode::Oauth),
        BackendKind::Delegated => Some(
            if backend_resolver::mirror_backend_for(provider).is_some() {
                FieldMode::Mirror
            } else {
                FieldMode::Oauth
            },
        ),
        BackendKind::None => None,
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
        /// `coach_reconnect_needed` while the coach whose session serves a
        /// delegated provider must sign in again, or why a delegated provider
        /// reads nothing (`athlete_email_missing`, `athlete_email_mismatch`,
        /// `member_email_unverified`) until the coach links the athlete again.
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
        /// Which activity fields each way of reading the provider supplies,
        /// and which way serves. Absent for a provider that serves no
        /// activities.
        #[serde(skip_serializing_if = "Option::is_none")]
        fields: Option<FieldDictionary>,
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
        /// Which calendar Dravr's plan and workout pushes go to, present when
        /// the athlete holds several calendars or recorded one (carnet#1393).
        #[serde(skip_serializing_if = "Option::is_none")]
        calendar: Option<CalendarChoiceStatus>,
    },
}

/// The calendar Dravr's pushes go to, as `get_connection_status` reports it.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CalendarChoiceStatus {
    /// The calendar the athlete chose (`set_preferred_calendar`, or the app's
    /// Connections settings); absent while they have chosen none, and a push
    /// with several connected asks which.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chosen: Option<String>,
    /// The connected calendars that can be chosen, as the push tools'
    /// `provider` names them. A chosen calendar missing from them is no
    /// longer connected, and pushes pass it over.
    pub connected: Vec<String>,
}

/// The calendar block of `get_connection_status`: `None` for an athlete who
/// holds at most one calendar and recorded none, where there is nothing to
/// choose.
async fn calendar_choice_status(
    ctx: &ToolExecutionContext,
    tenant_id: TenantId,
    user_id: uuid::Uuid,
) -> AppResult<Option<CalendarChoiceStatus>> {
    let chosen = recorded_calendar(ctx, tenant_id, user_id).await?;
    let connected = calendars_held(ctx, tenant_id, user_id, CalendarConnections::Own).await?;
    Ok((chosen.is_some() || connected.len() > 1)
        .then_some(CalendarChoiceStatus { chosen, connected }))
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
#[derive(Debug)]
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
                    "Provider to connect, as get_connection_status names it".to_owned(),
                ),
                ..Default::default()
            },
        );
        properties.insert(
            "redirect_url".to_owned(),
            PropertySchema {
                property_type: "string".to_owned(),
                description: Some(
                    "Redirect URL for mobile-app OAuth flows (dravr://, exp://, http://localhost \
                     or https://)"
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        let schema = object_schema(properties, Some(vec!["provider".to_owned()]));

        answers_with::<ConnectProviderResult>(tool_definition(
            "connect_provider",
            "Start the OAuth flow for the provider the athlete names, from those \
             get_connection_status lists.",
            schema,
            annotations(
                "Connect fitness provider",
                Effect::AdditiveWrite,
                Reach::AthleteAccount,
            ),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        // WRITES_DATA was missing: linking a provider writes an OAuth
        // grant onto the athlete's account. Its own twin,
        // `disconnect_provider`, has always declared the write — one half
        // of a pair declaring it and the other not is the evidence this
        // was an oversight rather than a decision. PROFILE because what
        // changes is which accounts are linked, i.e. who the athlete is.
        ToolCapabilities::REQUIRES_AUTH
            | ToolCapabilities::REQUIRES_TENANT
            | ToolCapabilities::WRITES_DATA
            | ToolCapabilities::PROFILE
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
            // No provider named starts nothing: picking one for the athlete
            // would open a flow for an account they may not have. The answer
            // is the list, with the athlete's state on each.
            let transport = ai_scope::declared_transport();
            return Ok(ToolResult::error(json!({
                "error": name_a_provider_text(&ctx, transport).await
            })));
        };

        // The refusal never echoes a mirror backend back: the model would
        // read it as a provider name and the athlete never chose it.
        if backend_resolver::is_mirror_backend(provider) {
            return Ok(ToolResult::error(json!({
                "error": format!(
                    "Unknown provider. Use '{}' instead.",
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

        // A provider with a mirror is connected through the hosted sign-in
        // by default, never through an OAuth URL minted here: the operator's
        // connect order (directive 5, every provider) puts the sign-in first
        // unless it was flipped to the OAuth with a seat free, and a dead
        // grant reconnects only through the sign-in (carnet#1494). Raise the
        // provider's auth-required signal instead: the chat pipeline answers
        // it with a minted hosted link, and an MCP client gets the structured
        // reconnect code.
        if signs_in_first(&ctx, user_uuid, tenant_id, provider).await {
            if let Some(mirror) = backend_resolver::mirror_backend_for(provider) {
                info!(
                    user_id = %user_uuid,
                    provider = provider,
                    "connect_provider: the account sign-in goes first, handing off to the hosted login"
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
                let mut payload = build_oauth_success_payload(provider, &url, &state);
                if let Some(transport) = ai_scope::declared_transport() {
                    if !readable_over(registry.as_ref(), provider, transport) {
                        payload.instructions.push_str("\n\n");
                        payload.instructions.push_str(APP_ONLY_NOTE);
                    }
                }
                ok_typed("connect_provider", payload)
            }
            Err(e) if is_notice_refusal(&e) => {
                info!(
                    user_id = %user_uuid,
                    provider = provider,
                    "connect_provider: the provider's notice is outstanding, nothing minted"
                );
                Ok(notice_required_result(provider, &e.message))
            }
            // Garmin's pool is full and the athlete holds no grant: the
            // usr/pwd sign-in is the way in, handed off as the mirror-default
            // providers are, to the hosted login the chat pipeline mints.
            Err(e) if is_no_seat_refusal(&e) => {
                info!(
                    user_id = %user_uuid,
                    provider = provider,
                    "connect_provider: no Garmin seat left, handing off to the usr/pwd sign-in"
                );
                Err(AppError::provider_auth_required(SCIOTTE_GARMIN))
            }
            Err(e) => {
                // No OAuth app to run under (the athlete has none of their
                // own and the server's is unset) is configuration, not a
                // server fault, and must not page.
                if e.is_server_fault() {
                    error!("OAuth URL generation failed for {}: {}", provider, e);
                } else {
                    warn!("OAuth URL refused for {}: {}", provider, e);
                }
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
#[derive(Debug)]
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
                    "A specific provider, by the name this tool lists; omit to check all supported providers."
                        .to_owned(),
                ),
                ..Default::default()
            },
        );

        let schema = object_schema(properties, None);

        answers_with::<ConnectionStatusResult>(tool_definition(
            "get_connection_status",
            "Check which fitness providers are connected: each one's state (connected, \
             disconnected, needs_reauth, or why a provider linked through a human coach reads \
             nothing), whether it must be authorized again, and which backend serves it.",
            schema,
            annotations(
                "Get provider connection status",
                Effect::ReadOnly,
                Reach::Internal,
            ),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::READS_DATA | ToolCapabilities::PROFILE
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
            let status_by_provider =
                read_status_by_provider(ctx.resources.repos(), user_uuid, tenant_id)
                    .await
                    .unwrap_or_default();

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

                let repos = ctx.resources.repos();
                let coalesced = match user_facing_canonical(
                    ctx.resources.provider_registry(),
                    specific_provider,
                ) {
                    Some(canonical) => {
                        backend_resolver::coalesced_status(repos, user_uuid, tenant_id, canonical)
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

                let fields = dictionary(
                    ctx.resources.provider_registry(),
                    specific_provider,
                    serving_mode(specific_provider, state.backend),
                );
                ok_typed(
                    "get_connection_status",
                    ConnectionStatusResult::Single {
                        provider: specific_provider.to_owned(),
                        status: state.status.to_owned(),
                        connected: state.connected,
                        needs_reauth: state.needs_reauth,
                        backend: state.backend.as_str().to_owned(),
                        delegated_by: state.delegated_by,
                        fields,
                    },
                )
            } else {
                let providers_status: BTreeMap<String, ProviderConnectionStatus> =
                    offered_providers(
                        ctx.resources.repos(),
                        ctx.resources.provider_registry(),
                        user_uuid,
                        tenant_id,
                        &status_by_provider,
                    )
                    .await
                    .into_iter()
                    .map(|(name, state)| {
                        let status = ProviderConnectionStatus::of(
                            ctx.resources.provider_registry(),
                            name,
                            state,
                        );
                        (name.to_owned(), status)
                    })
                    .collect();

                let calendar = calendar_choice_status(&ctx, tenant_id, user_uuid).await?;
                ok_typed(
                    "get_connection_status",
                    ConnectionStatusResult::All {
                        providers: providers_status,
                        calendar,
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
#[derive(Debug)]
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
                    "Provider to disconnect, as get_connection_status names it".to_owned(),
                ),
                ..Default::default()
            },
        );

        let schema = object_schema(properties, Some(vec!["provider".to_owned()]));

        answers_with::<DisconnectProviderResult>(tool_definition(
            "disconnect_provider",
            "Disconnect from a fitness data provider by removing stored OAuth tokens",
            schema,
            annotations(
                "Disconnect fitness provider",
                Effect::Destructive,
                Reach::AthleteAccount,
            ),
        ))
    }

    fn capabilities(&self) -> ToolCapabilities {
        ToolCapabilities::REQUIRES_AUTH | ToolCapabilities::WRITES_DATA | ToolCapabilities::PROFILE
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

#[cfg(test)]
mod tests {
    use super::ConnectProviderResult;

    #[test]
    fn a_connect_provider_result_never_prints_its_oauth_state() {
        let result = ConnectProviderResult {
            provider: "strava".to_owned(),
            authorization_url:
                "https://www.strava.com/oauth/authorize?state=sentinel-oauth-state-7f3a".to_owned(),
            state: "sentinel-oauth-state-7f3a".to_owned(),
            instructions: "Open the link".to_owned(),
            expires_in_minutes: 10,
            status: "pending_authorization".to_owned(),
        };
        for printed in [format!("{result:?}"), format!("{result:#?}")] {
            assert!(!printed.contains("sentinel-oauth-state-7f3a"), "{printed}");
            assert!(printed.contains("ConnectProviderResult"), "{printed}");
        }
    }
}
