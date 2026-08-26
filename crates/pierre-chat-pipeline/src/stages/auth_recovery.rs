// ABOUTME: Auth-recovery stage — short-circuits a turn when a tool returned ProviderAuthRequired
// ABOUTME: Mints a hosted-login URL and returns it as a structured reconnect prompt for the envelope
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
//!    instead of an empty string from the short-circuited tool loop, and
//!    returns the same prompt as a [`ReconnectPrompt`].
//!
//! The URL leaves here as a field, not only as a substring of a sentence. A
//! surface with [`crate::BlockSupport::reconnect_cta`] renders a control from
//! it — [`crate::build_envelope`] emits a [`crate::ReplyBlock::Reconnect`] and
//! keeps the sentence out of the prose — while a surface without one gets the
//! sentence folded in, with the link autolinked where the transport does that.
//!
//! Without this stage the user receives a generic "your connection expired"
//! message with no actionable path forward — exactly what triggered this
//! work in the first place.

use std::sync::Arc;

use tracing::{info, warn};
use uuid::Uuid;

use crate::envelope::ReconnectPrompt;
use crate::surface_profile::SurfaceProfile;
use crate::turn::TurnInput;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_PROVIDER_REAUTH_REQUIRED, KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK,
};
use pierre_database::repositories::{shorten_url, ShortLinkRepository};
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
    /// URL shortener store — wraps the dotty hosted-login link in a dot-free
    /// `<base>/r/<code>` so `WhatsApp` linkifies the whole reconnect link.
    pub short_links: &'a Arc<dyn ShortLinkRepository>,
}

/// Apply provider re-auth recovery in place.
///
/// When `result.pending_provider_auth_required` carries a provider slug,
/// mint a hosted-login URL and replace `result.content` with the
/// localized reply.
///
/// Returns `Some(prompt)` when the stage fired, so callers can skip
/// LLM-content-aware post-processing (text guardrails, claim verification)
/// and carry the URL onto the envelope as its own field; returns `None` when
/// the dispatch result was clean and downstream stages should run normally.
pub async fn apply_auth_recovery(
    deps: AuthRecoveryDeps<'_>,
    input: &TurnInput,
    profile: &SurfaceProfile,
    result: &mut ToolLoopResult,
) -> Option<ReconnectPrompt> {
    let provider_slug = result.pending_provider_auth_required.clone()?;

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
            return None;
        }
    };

    let locale = profile.locale.as_str();
    let display_name = provider_display_name(&provider_slug).to_owned();

    // Minting can fail — a tenant with no OAuth credentials configured, or the
    // mint endpoint refusing. Returning early on that left the turn with no
    // content at all, and the athlete was told « je n'ai pas réussi à formuler
    // une réponse » when what was actually wrong was a disconnected provider.
    // Which provider dropped is most of the answer; the link is the
    // convenience, so its absence costs the link and not the message.
    let Some(url) = mint_reconnect_url(&deps, &provider_slug, user_id, input, profile).await else {
        let message = deps.messaging_strings_registry.render(
            KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK,
            locale,
            &[display_name.as_str()],
        );
        warn!(
            user_id = %user_id,
            provider = %provider_slug,
            locale = %locale,
            "auth_recovery: no reconnect URL to offer; telling the athlete which provider dropped"
        );
        result.content.clone_from(&message);
        // No ReconnectPrompt: there is no URL for a surface to draw a control
        // around, and a control that goes nowhere is worse than the sentence.
        return None;
    };

    let message = deps.messaging_strings_registry.render(
        KEY_PROVIDER_REAUTH_REQUIRED,
        locale,
        &[display_name.as_str(), url.as_str()],
    );

    info!(
        user_id = %user_id,
        provider = %provider_slug,
        locale = %locale,
        "auth_recovery: emitting reconnect URL in chat reply"
    );

    result.content.clone_from(&message);
    Some(ReconnectPrompt {
        provider: provider_slug,
        display_name,
        url,
        text: message,
    })
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
    profile: &SurfaceProfile,
) -> Option<String> {
    if matches!(provider_slug, "sciotte" | "sciotte_garmin") {
        let target = sciotte_target_for_provider(provider_slug);
        let token = match mint_link_token(
            &MintProviderLinkTokenArgs {
                user_id,
                tenant_id: input.tool_tenant_id.as_uuid(),
                provider: "sciotte",
                target,
                channel: profile.surface.as_str(),
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
        let full_url = format!(
            "{}/providers/sciotte/login?token={}",
            deps.base_url,
            urlencoding::encode(&token)
        );
        // Shorten to a dot-free `<base>/r/<code>` so WhatsApp keeps the whole
        // link tappable; degrades to the full URL if the store write fails.
        return Some(
            shorten_url(
                deps.short_links.as_ref(),
                deps.base_url,
                &full_url,
                &input.tool_tenant_id.as_uuid().to_string(),
                &user_id.to_string(),
            )
            .await,
        );
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
/// and English copies stay short. An unknown slug passes through as-is — it
/// renders in every chat locale, unlike any hardcoded fallback word.
fn provider_display_name(provider_slug: &str) -> &str {
    match provider_slug {
        "sciotte_garmin" | "garmin" => "Garmin",
        "sciotte" | "strava" => "Strava",
        "whoop" => "WHOOP",
        "fitbit" => "Fitbit",
        "coros" => "COROS",
        "terra" => "Terra",
        other => other,
    }
}
