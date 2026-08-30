// ABOUTME: Auth-recovery stage — turns a provider re-auth signal into a minted, clickable reconnect offer
// ABOUTME: Owns the reply when nothing could be served; joins the model's answer when a sibling served it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Provider re-auth recovery for the chat pipeline.
//!
//! Two signals reach this stage, and they differ in whether the athlete got an
//! answer.
//!
//! The hard one is `ToolLoopResult::pending_provider_auth_required`: the
//! multi-turn tool loop saw an `AppError::ProviderAuthRequired` during dispatch
//! (see [`pierre_tool_runtime::tool_execution`]) and exited with no reply, so
//! the reconnect message becomes the whole turn.
//!
//! The soft one is `ToolLoopResult::served_without_provider`: `get_activities`
//! answered the window from the athlete's healthy connections while the elected
//! one's token was dead. The reply is the model's own words over real data, and
//! the reconnect offer joins it rather than replacing it.
//!
//! Either way this stage:
//!
//! 1. Mints a one-time hosted-login URL via
//!    [`pierre_middleware::provider_link_token::mint_link_token`], reusing the
//!    same JWT-signed token shape that the channel-bot mint endpoint emits.
//! 2. Renders the matching
//!    [`pierre_contremaitre::messaging_strings`] key in the user's resolved
//!    locale, substituting the provider display name and the URL.
//! 3. Writes that copy into `ToolLoopResult::content` — replacing it on the hard
//!    signal so downstream stages (`post_process`, `persistence`) see a clean
//!    message instead of an empty string from the short-circuited tool loop,
//!    appending to it on the soft one so the athlete keeps the answer they were
//!    given — and returns the same prompt as a [`ReconnectPrompt`].
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

use crate::envelope::{ReconnectPrompt, ReplyBlockKind};
use crate::surface_profile::SurfaceProfile;
use crate::turn::TurnInput;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_PROVIDER_REAUTH_REQUIRED, KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK,
    KEY_PROVIDER_REAUTH_SERVED, KEY_PROVIDER_REAUTH_SERVED_NO_LINK,
};
use pierre_database::repositories::{shorten_url, ShortLinkRepository};
use pierre_middleware::provider_link_token::{mint_link_token, MintProviderLinkTokenArgs};
use pierre_tool_runtime::implementations::connection::mint_oauth_authorize_url;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::tool_loop_io::ToolLoopResult;

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

/// What [`apply_auth_recovery`] did to the turn.
pub struct AuthRecovery {
    /// The reconnect offer to carry onto the envelope, as its own field.
    ///
    /// `None` when the stage did not fire at all, and also when minting failed:
    /// the reply then names the dropped provider in words, and no surface draws
    /// a control that goes nowhere.
    pub prompt: Option<ReconnectPrompt>,
    /// Whether the stage's copy is the finished turn, owing nothing below it.
    ///
    /// True on exactly one shape: the hard signal with a minted URL. Nothing
    /// answered the ask, `result.content` is replaced wholesale, and the
    /// localized message plus its link is the entire reply — platform text that
    /// must never be re-asked or post-processed as if it were model output.
    ///
    /// False on the soft signal, where the model's own answer survives
    /// underneath the offer and still owes every content-aware stage below.
    /// Also false on a hard signal whose mint failed: that reply names the
    /// dropped provider and carries no link, so it stays on the ordinary path
    /// and the identity re-ask and post-processing both get their say over it.
    pub owns_reply: bool,
}

impl AuthRecovery {
    /// The stage did not fire: no provider needs reconnecting this turn.
    const fn inert() -> Self {
        Self {
            prompt: None,
            owns_reply: false,
        }
    }
}

/// Which of the two re-auth signals the turn is carrying.
enum ReconnectStanding {
    /// Nothing served the ask — the reply IS the reconnect message.
    Blank,
    /// A sibling connection answered — the offer joins that answer.
    Served,
}

impl ReconnectStanding {
    /// Which standing this turn is in, and the provider slug it names.
    ///
    /// The hard signal wins: a turn that raised both did not answer the ask the
    /// athlete actually made, so it still becomes the reconnect message.
    fn of(result: &ToolLoopResult) -> Option<(Self, String)> {
        if let Some(slug) = result.pending_provider_auth_required.clone() {
            return Some((Self::Blank, slug));
        }
        result
            .served_without_provider
            .clone()
            .map(|slug| (Self::Served, slug))
    }

    /// The messaging key for this standing, with and without a minted URL.
    const fn keys(&self) -> (&'static str, &'static str) {
        match self {
            Self::Blank => (
                KEY_PROVIDER_REAUTH_REQUIRED,
                KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK,
            ),
            Self::Served => (
                KEY_PROVIDER_REAUTH_SERVED,
                KEY_PROVIDER_REAUTH_SERVED_NO_LINK,
            ),
        }
    }

    /// Whether the reconnect copy REPLACES `result.content` rather than joining
    /// it. Governs delivery alone — whether the finished reply then short-
    /// circuits the stages below is [`AuthRecovery::owns_reply`], which a failed
    /// mint answers differently.
    const fn replaces_reply(&self) -> bool {
        matches!(self, Self::Blank)
    }
}

/// Apply provider re-auth recovery in place.
///
/// On the hard signal (`result.pending_provider_auth_required`) mint a
/// hosted-login URL and replace `result.content` with the localized reply. On
/// the soft one (`result.served_without_provider`) mint the same URL and append
/// the localized offer to the answer the model already wrote.
///
/// The returned [`AuthRecovery`] carries the prompt for the envelope and, in
/// [`AuthRecovery::owns_reply`], says whether the turn is finished here: a
/// caller skips LLM-content-aware post-processing (text guardrails, claim
/// verification) exactly when the reply is a blanked turn's minted platform
/// text, and runs the whole chain on everything else.
pub async fn apply_auth_recovery(
    deps: AuthRecoveryDeps<'_>,
    input: &TurnInput,
    profile: &SurfaceProfile,
    result: &mut ToolLoopResult,
) -> AuthRecovery {
    let Some((standing, provider_slug)) = ReconnectStanding::of(result) else {
        return AuthRecovery::inert();
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
            return AuthRecovery::inert();
        }
    };

    let locale = profile.locale.as_str();
    let display_name = provider_display_name(&provider_slug).to_owned();
    let (linked_key, bare_key) = standing.keys();
    let replaces_reply = standing.replaces_reply();

    // Minting can fail — a tenant with no OAuth credentials configured, or the
    // mint endpoint refusing. Returning early on that left the turn with no
    // content at all, and the athlete was told « je n'ai pas réussi à formuler
    // une réponse » when what was actually wrong was a disconnected provider.
    // Which provider dropped is most of the answer; the link is the
    // convenience, so its absence costs the link and not the message. On a
    // served turn it costs even less: the answer is already written, and this
    // only appends the sentence naming what dropped.
    let Some(url) = mint_reconnect_url(&deps, &provider_slug, user_id, input, profile).await else {
        let message =
            deps.messaging_strings_registry
                .render(bare_key, locale, &[display_name.as_str()]);
        warn!(
            user_id = %user_id,
            provider = %provider_slug,
            locale = %locale,
            replaces_reply,
            "auth_recovery: no reconnect URL to offer; telling the athlete which provider dropped"
        );
        deliver(result, &message, replaces_reply);
        // No ReconnectPrompt: there is no URL for a surface to draw a control
        // around, and a control that goes nowhere is worse than the sentence.
        //
        // `owns_reply: false` whichever standing this is. A linkless sentence
        // is not a finished turn: the athlete asked something nothing could
        // answer, and the stages below — the bounded identity re-ask, then
        // post-processing with its content blocks and guardrails — are what
        // shape a reply the short-circuit hands out verbatim.
        return AuthRecovery {
            prompt: None,
            owns_reply: false,
        };
    };

    let message = deps.messaging_strings_registry.render(
        linked_key,
        locale,
        &[display_name.as_str(), url.as_str()],
    );

    // A reconnect offer is either a control or a link in the sentence, never
    // both: where the surface draws the control, the sentence joined to the
    // answer names the provider and stops there, so the athlete is handed one
    // thing to tap instead of a raw URL repeated beside it. A surface without
    // the control keeps the link inline, because there it is the only way to
    // reach the offer. The prompt carries the linked copy either way, so what
    // the control renders does not change with the surface it lands on.
    //
    // Only the served standing chooses: a blanked turn's reply IS the reconnect
    // message, and the link belongs in it whatever the surface draws around it.
    let control_draws_the_link =
        !replaces_reply && profile.render.renders(ReplyBlockKind::Reconnect);
    let prose = if control_draws_the_link {
        deps.messaging_strings_registry
            .render(bare_key, locale, &[display_name.as_str()])
    } else {
        message.clone()
    };

    info!(
        user_id = %user_id,
        provider = %provider_slug,
        locale = %locale,
        replaces_reply,
        control_draws_the_link,
        "auth_recovery: emitting reconnect offer in chat reply"
    );

    deliver(result, &prose, replaces_reply);
    AuthRecovery {
        prompt: Some(ReconnectPrompt {
            provider: provider_slug,
            display_name,
            url,
            text: message,
        }),
        owns_reply: replaces_reply,
    }
}

/// Put the reconnect copy into the reply, the way this standing calls for.
///
/// A blanked turn's reply is REPLACED: nothing served the ask, so the tool loop
/// left the content empty and there are no model words to keep. A served turn's
/// copy is APPENDED as its own paragraph below the answer — that answer is the
/// athlete's data, and discarding it to say a connection dropped is the very
/// blanking the served path exists to avoid.
fn deliver(result: &mut ToolLoopResult, message: &str, replaces_reply: bool) {
    if replaces_reply {
        message.clone_into(&mut result.content);
        return;
    }
    let answer = result.content.trim_end();
    result.content = if answer.is_empty() {
        message.to_owned()
    } else {
        format!("{answer}\n\n{message}")
    };
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
