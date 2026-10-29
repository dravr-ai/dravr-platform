// ABOUTME: Shared helper functions for provider-agnostic handler operations
// ABOUTME: Consolidates provider extraction and resolution, and the fetch helpers built on the auth chokepoint
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::activity_fetch::provider_auth_failure;
use crate::context::ToolExecutionContext;
use crate::protocol::auth::AuthService;
use crate::protocol::types::{
    UniversalResponse, UniversalToolExecutor, META_NO_PROVIDER_CONNECTED,
};
use crate::runtime::ToolRuntime;
use pierre_config::environment::default_provider;
use pierre_core::constants::oauth_providers::{
    is_user_provided, STRAVA, STRAVA_ARCHIVE, USER_PROVIDED_PROVIDERS,
};
use pierre_core::errors::{AppError, ErrorCode};
use pierre_core::models::{Activity, TenantId};
use pierre_providers::activity_source::resolve_activity_source;
use pierre_providers::backend_resolver::serving_backends;
use pierre_providers::core::FitnessProvider;
use pierre_providers::deduplication::{merge_duplicates, DedupConfig};
use pierre_tools_core::ToolResult;
use serde_json::{json, Value as JsonValue};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};
use uuid::Uuid;

/// Extract the literal `provider` argument from request parameters, when present.
///
/// Returns `None` when the caller didn't pass `provider` (or passed an empty string).
/// **Does not fall back to a synthetic-or-strava-default;** that historical behavior
/// silently bound untargeted tool calls to seed data in production. Use
/// [`resolve_provider_for_request`] to do the full priority chain (explicit arg →
/// env override → user's most-recently-used connection → reconnect signal).
#[must_use]
pub fn extract_provider(parameters: &serde_json::Map<String, JsonValue>) -> Option<String> {
    parameters
        .get("provider")
        .and_then(JsonValue::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// What the model is told when a tool needs a provider and the athlete has none.
///
/// Worded for the model, which writes the reply: there is no data to read, the
/// question may still be answerable without it, and connecting is the way to
/// get personal answers.
pub const NO_PROVIDER_CONNECTED_TEXT: &str = "No fitness provider connected: there is no \
     activity or health data for this athlete. Connect Strava, Garmin or another provider in \
     the app to use their own training; until then, answer from general knowledge any \
     question that does not need their data.";

/// The canonical no-provider refusal, minted in exactly one place.
///
/// Three sites refuse for the same reason — both resolvers here and the
/// dispatch chokepoint in `executor.rs` — and all carry
/// [`META_NO_PROVIDER_CONNECTED`]. It is deliberately NOT the re-auth signal
/// ([`crate::protocol::types::META_AUTH_REQUIRED_PROVIDER`]): that one stops
/// the tool loop and hands the turn to the `auth_recovery` stage, whose copy
/// says a connection *expired* — false for an account that never connected
/// one. The tool loop keeps going here, and the model answers from the
/// refusal text.
#[must_use]
pub fn no_provider_refusal() -> UniversalResponse {
    let mut metadata: HashMap<String, JsonValue> = HashMap::new();
    metadata.insert(META_NO_PROVIDER_CONNECTED.to_owned(), JsonValue::Bool(true));
    UniversalResponse {
        success: false,
        result: None,
        error: Some(NO_PROVIDER_CONNECTED_TEXT.to_owned()),
        metadata: Some(metadata),
    }
}

/// Resolve which fitness provider to serve a tool call from.
///
/// Priority chain — matches the user mental model of "what data are we looking at":
/// 1. Explicit `provider` argument in request parameters.
/// 2. Deployment-wide `PIERRE_DEFAULT_PROVIDER` env override.
/// 3. The user's most-recently-used connection from `provider_connections`
///    (ordered by `last_used_at DESC NULLS LAST, connected_at DESC`).
/// 4. Returns the [`no_provider_refusal`] when the user has no connection: the
///    model is told there is no data and answers what it can.
///
/// Replaces the historical `.and_then(|v| v.as_str()).map_or_else(default_provider, String::from)`
/// pattern duplicated across every fitness-API handler, which silently bound
/// untargeted calls to the synthetic seed provider in production.
///
/// # Errors
///
/// Returns a boxed `UniversalResponse` when the user has no provider connections at
/// all: the [`no_provider_refusal`].
pub async fn resolve_provider_for_request(
    parameters: &JsonValue,
    executor: &UniversalToolExecutor,
    user_uuid: Uuid,
    tenant_id: Option<&str>,
) -> Result<String, Box<UniversalResponse>> {
    let tenant = tenant_id.and_then(|t| TenantId::parse_str(t).ok());
    match elect_provider(parameters, &executor.resources, user_uuid, tenant).await {
        Ok(Some(provider)) => Ok(provider),
        Ok(None) => {
            warn!(
                user_id = %user_uuid,
                "no fitness provider connected for user — refusing with the no-provider signal"
            );
            // The race-window backstop: the dispatch chokepoint already refused
            // providerless users for REQUIRES_PROVIDER tools, so reaching this
            // branch means either a tool that calls a resolver without declaring
            // the capability, or a disconnect between the check and this read.
            Err(Box::new(no_provider_refusal()))
        }
        Err(e) => {
            warn!(
                user_id = %user_uuid,
                error = %e,
                "provider_connections lookup failed during resolution"
            );
            Err(Box::new(UniversalResponse {
                success: false,
                result: None,
                error: Some(format!(
                    "Failed to resolve fitness provider for this request: {e}"
                )),
                metadata: None,
            }))
        }
    }
}

/// `McpTool::execute`-shaped variant of [`resolve_provider_for_request`].
///
/// Same priority chain (explicit arg → env override → most-recent connection)
/// but reads from a [`crate::context::ToolExecutionContext`] (which carries
/// `Arc<dyn ToolRuntime>` + parsed `user_id`/`tenant_id`) instead of a
/// `UniversalRequest`. Returns a JSON `ToolResult::error` payload on no-provider
/// rather than minting a `UniversalResponse`, because `McpTool` callers consume
/// `ToolResult` directly.
///
/// # Errors
///
/// Returns `Err(ToolResult)` carrying the canonical `auth_required_provider`
/// payload when the user has no provider connections at all. Callers should
/// propagate this with `?` or an early return so the chat tool-loop's
/// `auth_recovery` stage renders the localized reconnect copy.
pub async fn resolve_provider_for_tool(
    args: &JsonValue,
    context: &ToolExecutionContext,
) -> Result<String, ToolResult> {
    let tenant = context.tenant_id.map(TenantId::from_uuid);
    match elect_provider(args, &context.resources, context.user_id, tenant).await {
        Ok(Some(provider)) => Ok(provider),
        Ok(None) | Err(_) => Err(ToolResult::error(json!({
            "error": NO_PROVIDER_CONNECTED_TEXT,
            META_NO_PROVIDER_CONNECTED: true
        }))),
    }
}

/// The one priority chain both resolvers answer with: the explicit `provider`
/// argument, then the deployment-wide `PIERRE_DEFAULT_PROVIDER`, then the
/// connection elected to answer activity questions
/// ([`resolve_activity_source`]: health, then capability, then recency). `None`
/// when the user has none.
async fn elect_provider(
    parameters: &JsonValue,
    runtime: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant: Option<TenantId>,
) -> Result<Option<String>, AppError> {
    // 1. Explicit arg
    if let Some(p) = parameters
        .get("provider")
        .and_then(JsonValue::as_str)
        .filter(|s| !s.is_empty())
    {
        return Ok(Some(p.to_owned()));
    }

    // 2. Env override
    if let Some(env_p) = default_provider() {
        return Ok(Some(env_p));
    }

    // 3. The connection elected to answer activity questions
    let Some(conn) = resolve_activity_source(
        runtime.repos().provider_connections.as_ref(),
        runtime.provider_registry(),
        user_id,
        tenant,
    )
    .await?
    else {
        return Ok(None);
    };
    // A diagnostic line, not a catalogued business event: on the notify target
    // it reached Slack and PostHog as a nameless event.
    info!(
        user_id = %user_id,
        provider = %conn.provider,
        "resolved provider from the user's elected activity source"
    );
    Ok(Some(conn.provider))
}

/// Create a standard auth error response
#[must_use]
pub fn create_auth_error_response(provider_name: &str, error: &str) -> UniversalResponse {
    UniversalResponse {
        success: true,
        result: Some(json!({
            "activities": [],
            "message": format!("Authentication error for {provider_name}: {error}"),
            "error": format!("Authentication error: {error}"),
            "provider": provider_name
        })),
        error: None,
        metadata: Some({
            let mut map = HashMap::new();
            map.insert("authentication_error".to_owned(), JsonValue::Bool(true));
            map.insert(
                "provider".to_owned(),
                JsonValue::String(provider_name.to_owned()),
            );
            map
        }),
    }
}

/// Fetch activities from the user's connected provider
///
/// Authenticates through [`AuthService::create_authenticated_provider`], the
/// chokepoint every provider read goes through, and fetches recent activities.
///
/// # Errors
///
/// Returns [`AppError`] when:
/// - the connection needs reconnecting ([`AppError::provider_auth_required`])
///   or the provider cannot be authenticated for another reason
/// - The provider's `get_activities` call fails (network, auth refresh, etc.)
pub async fn fetch_activities_from_provider(
    resources: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    provider_name: &str,
    tenant_id: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<Activity>, AppError> {
    let provider = configured_provider(resources, user_id, provider_name, tenant_id).await?;

    // Fetch activities. Preserve a `ProviderAuthRequired` error verbatim so it
    // keeps its code (and provider slug in `details`) across the `?` boundary:
    // the tool executor short-circuits on it and the chat-pipeline
    // `auth_recovery` stage mints a hosted-login link and renders the
    // reconnect copy. Wrapping it in `internal` here erased the code and the
    // user got a generic "internal error" instead of a reconnect prompt.
    provider.get_activities(limit, None).await.map_err(|e| {
        if e.code == ErrorCode::ProviderAuthRequired {
            e
        } else {
            AppError::internal(format!("Failed to fetch activities: {e}"))
        }
    })
}

/// Fetch ONE activity by id from the user's connected provider.
///
/// The single-activity sibling of [`fetch_activities_from_provider`]: the
/// same authentication, but one
/// `get_activity_with_streams` round trip instead of a paged list scan — 200× cheaper for the callers
/// that previously pulled a whole window to find one id, able to reach
/// activities older than any recent window, and on providers with a real
/// detail endpoint (Strava, Garmin) the returned activity carries the
/// laps/splits list rows never had.
///
/// # Errors
///
/// Returns [`AppError`] when authentication fails, and preserves a
/// `ProviderAuthRequired` error verbatim (same reconnect contract as the list
/// sibling); any other fetch failure reads as the activity being unreachable
/// by this id.
pub async fn fetch_activity_from_provider(
    resources: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    provider_name: &str,
    tenant_id: Option<&str>,
    activity_id: &str,
) -> Result<Activity, AppError> {
    let provider_name =
        &by_id_provider(resources, user_id, provider_name, tenant_id, activity_id).await;
    let provider = configured_provider(resources, user_id, provider_name, tenant_id).await?;

    provider
        .get_activity_with_streams(activity_id)
        .await
        .map_err(|e| {
            if e.code == ErrorCode::ProviderAuthRequired {
                e
            } else {
                AppError::not_found(format!(
                    "activity {activity_id} not found on {provider_name}: {e}"
                ))
            }
        })
}

/// The reader of `activity_id` when it is not `elected`'s, with its key.
///
/// [`by_id_provider`] decides, and the copy the athlete brought in is read
/// through the same chokepoint as every other provider
/// ([`configured_provider`]), which serves their stored rows and samples.
/// `None` when the read stays with `elected`, so a tool that already holds
/// `elected`'s provider reads through it.
///
/// The tools that answer about one activity (its metrics, intelligence,
/// comparison, weather) read its summary here, and the sample tools read it
/// through [`fetch_activity_from_provider`]: one routing for every by-id read.
///
/// # Errors
///
/// Returns [`AppError`] when the reader of the standing copy cannot be built.
pub async fn by_id_reader(
    resources: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    elected: &str,
    tenant_id: Option<&str>,
    activity_id: &str,
) -> Result<Option<(String, Box<dyn FitnessProvider>)>, AppError> {
    let read_from = by_id_provider(resources, user_id, elected, tenant_id, activity_id).await;
    if read_from == elected {
        return Ok(None);
    }
    let reader = configured_provider(resources, user_id, &read_from, tenant_id).await?;
    Ok(Some((read_from, reader)))
}

/// The provider a by-id read of `activity_id` goes to: the key of the copy
/// that stands for its workout.
///
/// The athlete's workout can be held more than once — by the connection
/// `elected` names (its API or its usr/pwd capture), and by what they brought
/// in themselves (an imported export, an uploaded file). The copies of it the
/// cache holds under this id are merged by [`merge_duplicates`], the one
/// duplicate-workout merge every list reader shares, and when the copy that
/// stands for the workout is one the athlete brought in, the read goes to its
/// key: the authentication chokepoint answers it with the reader of their own
/// stored rows and samples (`activity_streams`). Otherwise — the connection's
/// copy stands, or no copy is cached (an activity older than the cache still
/// reaches the provider) — the read goes to `elected`, as before.
///
/// The cache alone cannot rank the export against the Strava API: the API's
/// rows are pruned past the cache's retention while the export's never are,
/// so an old ride can find the export as its only cached copy. When the
/// athlete holds a Strava API token and `elected` is served by it, the API
/// stands for its own activity whatever the cache still holds — the export
/// outranks only the usr/pwd capture.
///
/// A failed cache or token read is logged and leaves the read with `elected`.
pub async fn by_id_provider(
    resources: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    elected: &str,
    tenant_id: Option<&str>,
    activity_id: &str,
) -> String {
    let Some(tenant) = tenant_id.and_then(|t| TenantId::parse_str(t).ok()) else {
        return elected.to_owned();
    };
    let cache = &resources.repos().activity_cache;
    let mut copies = Vec::new();
    let keys = serving_backends(elected)
        .into_iter()
        .chain(USER_PROVIDED_PROVIDERS.iter().map(|key| (*key).to_owned()));
    for key in keys {
        match cache
            .get_cached_activity(user_id, &tenant, &key, activity_id)
            .await
        {
            Ok(Some(copy)) => copies.push(copy),
            Ok(None) => {}
            Err(e) => {
                warn!(
                    user_id = %user_id,
                    provider = %key,
                    error = %e,
                    "by-id read: cached copy lookup failed; reading the elected provider"
                );
                return elected.to_owned();
            }
        }
    }
    let (sessions, _) = merge_duplicates(copies, &DedupConfig::from_env());
    let standing: Vec<&str> = sessions
        .iter()
        .filter(|session| session.id() == activity_id)
        .map(Activity::provider)
        .collect();
    match standing.as_slice() {
        [key]
            if *key == STRAVA_ARCHIVE
                && holds_strava_api(resources, user_id, &tenant, elected).await =>
        {
            elected.to_owned()
        }
        [key] if is_user_provided(key) => (*key).to_owned(),
        _ => elected.to_owned(),
    }
}

/// Whether `elected` is served by the Strava API for this athlete: it names a
/// provider the Strava API serves, and the athlete holds a Strava token row
/// (the row whose presence routes `strava` to the API, valid or stale).
async fn holds_strava_api(
    resources: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    tenant: &TenantId,
    elected: &str,
) -> bool {
    if !serving_backends(elected).iter().any(|key| key == STRAVA) {
        return false;
    }
    match resources
        .repos()
        .oauth_tokens
        .get_token(user_id, *tenant, STRAVA)
        .await
    {
        Ok(token) => token.is_some(),
        Err(e) => {
            warn!(
                user_id = %user_id,
                error = %e,
                "by-id read: Strava token lookup failed; reading the elected provider"
            );
            true
        }
    }
}

/// Build the authenticated provider both fetch helpers above read through.
///
/// The one chokepoint every provider read authenticates at
/// ([`AuthService::create_authenticated_provider`]), so a mirror backend, a
/// delegated `TrainingPeaks` link and a refreshed token serve these reads as
/// they serve every other.
///
/// A refusal that says the connection needs reconnecting keeps that shape as
/// [`AppError::provider_auth_required`]; any other refusal is an
/// external-service error carrying the refusal's words. `pub` for a caller
/// that asks the provider something before it reads — the Home route read
/// asks whether it serves streams at all.
///
/// # Errors
///
/// Returns [`AppError::provider_auth_required`] when the connection needs
/// reconnecting, and an external-service error for any other refusal.
pub async fn configured_provider(
    resources: &Arc<dyn ToolRuntime>,
    user_id: Uuid,
    provider_name: &str,
    tenant_id: Option<&str>,
) -> Result<Box<dyn FitnessProvider>, AppError> {
    AuthService::new(Arc::clone(resources))
        .create_authenticated_provider(provider_name, user_id, tenant_id)
        .await
        .map_err(|response| provider_auth_failure(provider_name, &response))
}
