// ABOUTME: Auth-recovery stage — short-circuits a turn when a tool returned ProviderAuthRequired
// ABOUTME: Mints a hosted-login URL and renders a deterministic locale-aware reply with the link
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider re-auth recovery for the chat pipeline.
//!
//! When the multi-turn tool loop detects an `AppError::ProviderAuthRequired`
//! during dispatch (see [`pierre_tool_runtime::tool_execution`]), it exits
//! immediately and propagates the provider slug via
//! `ToolLoopResult::pending_provider_auth_required`. This stage observes that
//! flag and:
//!
//! 1. Mints a one-time hosted-login URL via
//!    [`pierre_middleware::provider_link_token::mint_link_token`], reusing the
//!    same JWT-signed token shape that the channel-bot mint endpoint emits.
//! 2. Renders [`pierre_contremaitre::messaging_strings::KEY_PROVIDER_REAUTH_REQUIRED`]
//!    in the user's resolved locale, substituting the provider display name
//!    and the URL.
//! 3. Overrides `ToolLoopResult::content` with that deterministic reply so
//!    downstream stages (`post_process`, `persistence`) see a clean message
//!    instead of an empty string from the short-circuited tool loop.
//!
//! Without this stage the user receives a generic "your connection expired"
//! message with no actionable path forward — exactly what triggered this
//! work in the first place.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tracing::{info, warn};
use uuid::Uuid;

use crate::channel_profile::ChannelProfile;
use crate::turn::TurnInput;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, DEFAULT_LOCALE, KEY_PROVIDER_REAUTH_REQUIRED,
};
use pierre_middleware::provider_link_token::{mint_link_token, MintProviderLinkTokenArgs};
use pierre_tool_runtime::implementations::connection::mint_oauth_authorize_url;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::tool_execution::ToolLoopResult;

/// Inputs to [`apply_auth_recovery`].
///
/// Bundles the per-call dependencies so the function signature stays under
/// clippy's `too_many_arguments` ceiling without forcing callers to pass the
/// full [`crate::ChatPipelineContext`]. All fields are borrowed
/// references, so the struct is `Copy` and cheap to pass by value.
#[derive(Clone, Copy)]
pub struct AuthRecoveryDeps<'a> {
    /// JWT secret used to mint the hosted-login link token.
    pub admin_jwt_secret: &'a Arc<str>,
    /// Server base URL used to build the hosted-login redirect.
    pub base_url: &'a str,
    /// Localized messaging strings used to render the user-facing reply.
    pub messaging_strings_registry: &'a Arc<MessagingStringsRegistry>,
    /// Tool runtime used to mint a real OAuth authorization URL (WHOOP/Fitbit/…) for
    /// non-sciotte providers — the sciotte mirror keeps its hosted-login mint above.
    pub tool_runtime: &'a Arc<dyn ToolRuntime>,
}

/// Apply provider re-auth recovery in place.
///
/// When `result.pending_provider_auth_required` carries a provider slug,
/// mint a hosted-login URL and replace `result.content` with the
/// localized reply.
///
/// Returns `true` when the stage fired so callers can skip
/// LLM-content-aware post-processing (text guardrails, claim verification);
/// returns `false` when the dispatch result was clean and downstream stages
/// should run normally.
///
/// `recovery_dispatched` is updated atomically so observability hooks can
/// surface the short-circuit alongside the assistant message.
pub async fn apply_auth_recovery(
    deps: AuthRecoveryDeps<'_>,
    input: &TurnInput,
    profile: &ChannelProfile,
    result: &mut ToolLoopResult,
    recovery_dispatched: &AtomicBool,
) -> bool {
    let Some(provider_slug) = result.pending_provider_auth_required.clone() else {
        return false;
    };

    let user_id = match Uuid::parse_str(&input.user_id) {
        Ok(uuid) => uuid,
        Err(e) => {
            // The tool loop produced a recoverable signal but we cannot mint
            // without a valid user UUID — fall back to the LLM path so the
            // user at least sees the upstream error variant.
            warn!(
                user_id = %input.user_id,
                error = %e,
                "auth_recovery: invalid user_id UUID, skipping mint"
            );
            return false;
        }
    };

    let Some(url) = mint_reconnect_url(&deps, &provider_slug, user_id, input, profile).await else {
        return false;
    };

    let locale = input
        .locale
        .as_deref()
        .filter(|l| !l.is_empty())
        .unwrap_or(DEFAULT_LOCALE);
    let display_name = provider_display_name(&provider_slug);
    let message = deps.messaging_strings_registry.render(
        KEY_PROVIDER_REAUTH_REQUIRED,
        locale,
        &[display_name, &url],
    );

    info!(
        user_id = %user_id,
        provider = %provider_slug,
        locale = %locale,
        "auth_recovery: emitting reconnect URL in chat reply"
    );

    result.content = message;
    recovery_dispatched.store(true, Ordering::Relaxed);
    true
}

/// Mint the reconnect URL for `provider_slug`.
///
/// Scrape-mirror providers (`sciotte`, `sciotte_garmin`) use the Dravr-hosted login page
/// (email + password) — the same short-TTL link-token mint the channel bots use. OAuth
/// providers (WHOOP, Fitbit, Strava, Garmin, …) get their real provider authorization URL
/// plus a persisted CSRF state row. Returns `None` (fall back to the LLM path) on failure.
async fn mint_reconnect_url(
    deps: &AuthRecoveryDeps<'_>,
    provider_slug: &str,
    user_id: Uuid,
    input: &TurnInput,
    profile: &ChannelProfile,
) -> Option<String> {
    if matches!(provider_slug, "sciotte" | "sciotte_garmin") {
        let target = sciotte_target_for_provider(provider_slug);
        let token = match mint_link_token(
            &MintProviderLinkTokenArgs {
                user_id,
                tenant_id: input.tool_tenant_id.0,
                provider: "sciotte",
                target,
                channel: profile.channel.as_str(),
                channel_thread: None,
            },
            deps.admin_jwt_secret,
        ) {
            Ok(t) => t,
            Err(e) => {
                warn!(
                    user_id = %user_id,
                    provider = %provider_slug,
                    error = %e,
                    "auth_recovery: failed to mint hosted-login token"
                );
                return None;
            }
        };
        return Some(format!(
            "{}/providers/sciotte/login?token={}",
            deps.base_url,
            urlencoding::encode(&token)
        ));
    }

    match mint_oauth_authorize_url(
        deps.tool_runtime.as_ref(),
        user_id,
        input.tool_tenant_id,
        provider_slug,
        None,
    )
    .await
    {
        Ok((url, _state)) => Some(url),
        Err(e) => {
            warn!(
                user_id = %user_id,
                provider = %provider_slug,
                error = %e,
                "auth_recovery: failed to mint OAuth authorization URL"
            );
            None
        }
    }
}

/// Map a provider slug returned from the tool loop to the `target` field
/// required by the hosted-login mint endpoint. Sciotte's hosted UI takes
/// `target=strava | garmin` and the slug already encodes which platform is
/// wedged.
fn sciotte_target_for_provider(provider_slug: &str) -> &'static str {
    match provider_slug {
        "sciotte_garmin" => "garmin",
        // Default: the historical sciotte slug is Strava-specific.
        _ => "strava",
    }
}

/// Human-readable display name for a provider slug, used in the localized
/// re-auth message. Strings are deliberately platform brand names so French
/// and English copies stay short.
fn provider_display_name(provider_slug: &str) -> &'static str {
    match provider_slug {
        "sciotte_garmin" | "garmin" => "Garmin",
        "sciotte" | "strava" => "Strava",
        "whoop" => "WHOOP",
        "fitbit" => "Fitbit",
        "coros" => "COROS",
        "terra" => "Terra",
        _ => "ton fournisseur",
    }
}
