// ABOUTME: Hot-reloadable locale-aware registry for user-facing messaging strings
// ABOUTME: Compiled-in FR/EN/ES/DE/PT defaults; extra locales layer on via contremaitre
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// file-size-ok: string table, not logic — the invariant is `entries == keys × 5`,
// so every new user-facing string is required to add five lines here. Length is
// a function of how much the product says, not of this module's complexity.

//! # Messaging Strings Registry
//!
//! Holds short user-facing strings that are sent back to Telegram, WhatsApp,
//! Discord, Slack, and other messaging channels. This registry covers:
//!
//! - Chat-pipeline fallbacks (generic error, empty reply, guardrail rewrites,
//!   claim-verification warnings)
//! - Slash-command handler output (`/status`, `/help`, `/logout`, `/privacy`,
//!   `/group`, `/coach`) — every user-visible string goes through the registry
//!   so operators can hot-reload translations via the contremaitre repo without
//!   a code change or redeploy
//!
//! These are distinct from [`super::PromptRegistry`] which holds the system
//! prompts sent *to* the LLM. This registry holds strings shown *to the user*.
//!
//! ## Locale model
//!
//! Each string is stored per BCP-47 locale code (`"fr"`, `"en"`, `"es"`,
//! `"de"`, `"pt"`, …). Lookups follow the chain:
//!
//! 1. `(key, requested_locale)` — exact match
//! 2. `(key, DEFAULT_LOCALE)` — fall back to the default locale (`"fr"`)
//! 3. Compiled-in default for `(key, DEFAULT_LOCALE)`
//! 4. Empty string
//!
//! Extra locales can be added to the contremaitre repo without any code
//! change — just drop files under `strings/messaging/<locale>/<key>.md`
//! and list them in `manifest.json`. The registry picks them up on the
//! next webhook sync.
//!
//! ## Templating
//!
//! Values may contain positional placeholders (`{0}`, `{1}`, …) that
//! callers fill in via [`format_template`] (Option B from the 2026-04-15
//! audit gist — zero new dependencies, unambiguous indexing).

use std::collections::HashMap;
use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use chrono::{DateTime, Utc};

use super::manifest::compute_sha256;
use super::registry::PromptSource;

/// Default locale used when a caller does not specify one.
///
/// Also used when the requested locale is missing from the registry.
/// Currently French because the majority user base is francophone; change
/// this constant (and add the corresponding compiled-in defaults) when
/// that shifts.
pub const DEFAULT_LOCALE: &str = "fr";

// ── Chat pipeline keys ────────────────────────────────────────────────────

/// Key: LLM dispatch failed, user-facing apology with correlation `short_id`.
pub const KEY_ERROR_GENERIC: &str = "messaging.error.generic";
/// Key: a tool action was blocked by the security guardian, user-facing denial.
pub const KEY_GUARDIAN_DENIED: &str = "messaging.guardian.denied";

/// Key: the guardian parked a destructive action pending user confirmation.
/// Args: `{0}` tool name, `{1}` claim token.
pub const KEY_GUARDIAN_CONFIRM_PROMPT: &str = "messaging.guardian.confirm_prompt";

/// Key: a confirmed action executed successfully. Args: `{0}` tool name.
pub const KEY_GUARDIAN_CONFIRM_DONE: &str = "messaging.guardian.confirm_done";

/// Key: a confirmed action failed to execute. Args: `{0}` tool name.
pub const KEY_GUARDIAN_CONFIRM_FAILED: &str = "messaging.guardian.confirm_failed";

/// Key: the user denied a parked action. No format placeholders.
pub const KEY_GUARDIAN_CONFIRM_DENIED: &str = "messaging.guardian.confirm_denied";

/// Key: the parked action's confirmation window elapsed. No format placeholders.
pub const KEY_GUARDIAN_CONFIRM_EXPIRED: &str = "messaging.guardian.confirm_expired";

/// Key: no claimable pending action matches the given code. No format placeholders.
pub const KEY_GUARDIAN_CONFIRM_NOT_FOUND: &str = "messaging.guardian.confirm_not_found";
/// Key: LLM returned an empty reply, reformulation request.
pub const KEY_EMPTY_REPLY: &str = "messaging.empty_reply";
/// Key: reply withheld at the response boundary — the canary scan proved it
/// exposed system-prompt content, or the narration scrub emptied it.
pub const KEY_REPLY_WITHHELD: &str = "messaging.reply_withheld";
/// Key: text-guardrails rejected an over-long response.
pub const KEY_GUARDRAIL_TOO_LONG: &str = "messaging.guardrail.too_long";
/// Key: text-guardrails rejected a blocked-topic response.
pub const KEY_GUARDRAIL_BLOCKED_TOPIC: &str = "messaging.guardrail.blocked_topic";
/// Key: claim-verification `Warn` fallback suffix appended below the LLM reply.
pub const KEY_VERIFICATION_WARN_SUFFIX: &str = "messaging.verification.warn_suffix";
/// Key: claim-verification `Block` fallback that fully replaces the LLM reply.
pub const KEY_VERIFICATION_BLOCK_FALLBACK: &str = "messaging.verification.block_fallback";
/// Key: canonical refusal for off-scope requests.
///
/// Emitted when the user asks something outside the fitness-assistant
/// scope (pricing, trivia, general web lookups, etc.). Interpolated into
/// [`super::registry::PromptRegistry::pierre_system_prompt`] at turn-time
/// so the LLM emits exactly this string instead of translating.
pub const KEY_SCOPE_REFUSAL: &str = "messaging.scope.refusal";
/// Key: canonical refusal for missing-capability requests.
///
/// Emitted when the user asks for a capability the assistant does not have
/// (web scraping, image generation, etc.). Interpolated into the system
/// prompt at turn-time for deterministic output.
pub const KEY_CAPABILITY_REFUSAL: &str = "messaging.capability.refusal";
/// Key: Nutrition-coach carve-out for the generic scope list.
///
/// Generic `pierre_system.md` lists "food/meal finders" as out-of-scope
/// alongside restaurant prices and delivery apps. For Nutrition coaches
/// that collides with their core purpose — answering meal/dinner/snack
/// questions grounded in training data. This string reaffirms nutrition
/// questions ARE in scope and is injected into the system prompt whenever
/// the active coach's category is Nutrition.
pub const KEY_COACH_SCOPE_CARVE_OUT_NUTRITION: &str = "messaging.scope.carve_out.nutrition";
/// Key: Recipes-coach carve-out for the generic scope list.
///
/// Same rationale as [`KEY_COACH_SCOPE_CARVE_OUT_NUTRITION`] — Recipes
/// coaches exist to suggest meals and food choices, so the generic
/// "food/meal finders" refusal must not fire for them.
pub const KEY_COACH_SCOPE_CARVE_OUT_RECIPES: &str = "messaging.scope.carve_out.recipes";
/// Key: short placeholder shown in-channel (Telegram/Slack/Discord) while
/// the LLM is still generating the reply — e.g. "thinking…" / "réflexion…".
pub const KEY_THINKING_PLACEHOLDER: &str = "messaging.thinking_placeholder";
/// Key: reply for unmatched slash commands.
///
/// Fires when the user types a `/something` prefix that doesn't match any
/// registered command (typos like `/.coach`, obsolete names, etc.).
/// Short-circuits the LLM dispatch so typos don't eat quota or spam the
/// channel.
pub const KEY_UNKNOWN_COMMAND: &str = "messaging.unknown_command";
/// Key: progress status shown during prompt-assembly stage.
pub const KEY_STATUS_READING_QUESTION: &str = "messaging.status.reading_question";
/// Key: progress status shown during LLM dispatch stage.
pub const KEY_STATUS_GENERATING_RESPONSE: &str = "messaging.status.generating_response";
/// Key: progress status shown when a tool call starts. `{0}` = tool name.
pub const KEY_STATUS_CALLING_TOOL: &str = "messaging.status.calling_tool";
/// Key: progress status shown when the pipeline errors. `{0}` = error text.
pub const KEY_STATUS_ERROR: &str = "messaging.status.error";

// ── Coach onboarding proposal keys ────────────────────────────────────────
// Rendered by the messaging auto-send when a user's first provider-connected
// turn leads with inferred-profile coach suggestions. The per-coach reason
// lines are localized upstream (LLM re-rank prompt / deterministic fallback);
// these wrap them with a locale-aware lead-in and footer.

/// Key: coach-proposal lead-in when a recent sport profile is known.
/// `{0}` = primary sport display name, `{1}` = coach count.
pub const KEY_COACH_PROPOSAL_WELCOME: &str = "messaging.coach_proposal.welcome";
/// Key: coach-proposal lead-in for a cold-start user (no profile yet).
/// `{0}` = coach count.
pub const KEY_COACH_PROPOSAL_WELCOME_GENERIC: &str = "messaging.coach_proposal.welcome_generic";
/// Key: coach-proposal closing line inviting the user to reply with a number.
pub const KEY_COACH_PROPOSAL_FOOTER: &str = "messaging.coach_proposal.footer";

/// Key: account-approval welcome, sent on each linked messaging channel when a
/// user's account is approved. No placeholders.
pub const KEY_REGISTRATION_APPROVED: &str = "messaging.account.registration_approved";

/// Key: historical-backfill completion notice.
///
/// Pushed back to the channel that asked for an old activity window once the
/// background backfill finished warming the durable cache. `{0}` = count of
/// activities loaded. Used as the fallback nudge when the warmed cache reads
/// back empty (it shouldn't, post-backfill).
pub const KEY_BACKFILL_READY: &str = "messaging.backfill.ready";

/// Key: header line for the backfill completion notice that pushes the list.
///
/// Sits above the actual activity list (not just a nudge). `{0}` = count of
/// activities loaded. The per-activity lines and the truncation footer are
/// rendered in Rust; only this header phrase is localized.
pub const KEY_BACKFILL_LIST_HEADER: &str = "messaging.backfill.list_header";

/// Key: footer appended to the backfill activity list when it is truncated.
/// `{0}` = number of additional activities not shown.
pub const KEY_BACKFILL_LIST_MORE: &str = "messaging.backfill.list_more";

// ── Commitment verdict keys ───────────────────────────────────────────────
// Emitted by the commitment sweep once a promise's window closes. Composed
// from counts and the sanitized sport slug only — never from the athlete's or
// the coach's stored text — because the sweep reads provider activity data and
// an activity title must never be able to author a sentence the athlete reads.

/// Key: the athlete completed everything they promised.
/// `{0}` = sessions done, `{1}` = sessions promised, `{2}` = activity noun.
pub const KEY_COMMITMENT_MET: &str = "messaging.commitment.met";
/// Key: the athlete completed some but not all of what they promised.
/// `{0}` = sessions done, `{1}` = sessions promised, `{2}` = activity noun.
pub const KEY_COMMITMENT_PARTIAL: &str = "messaging.commitment.partial";
/// Key: the athlete completed none of what they promised.
/// `{0}` = sessions promised, `{1}` = activity noun.
pub const KEY_COMMITMENT_MISSED: &str = "messaging.commitment.missed";
/// Key: generic activity noun used when a commitment counts any sport, so the
/// three verdict templates need no sport-present and sport-absent variants.
pub const KEY_COMMITMENT_ACTIVITY_ANY: &str = "messaging.commitment.activity_any";
/// Key: notification title when a verdict is delivered as an app push.
pub const KEY_COMMITMENT_PUSH_TITLE: &str = "messaging.commitment.push_title";

/// French default for [`KEY_COMMITMENT_MET`].
pub(crate) const FR_COMMITMENT_MET: &str =
    "Tu avais dit {1} × {2}. C'est fait : {0}/{1}. \u{1f44f}";
/// English default for [`KEY_COMMITMENT_MET`].
pub(crate) const EN_COMMITMENT_MET: &str = "You said {1} × {2}. Done: {0}/{1}. \u{1f44f}";
/// Spanish default for [`KEY_COMMITMENT_MET`].
pub(crate) const ES_COMMITMENT_MET: &str = "Dijiste {1} × {2}. Hecho: {0}/{1}. \u{1f44f}";
/// German default for [`KEY_COMMITMENT_MET`].
pub(crate) const DE_COMMITMENT_MET: &str =
    "Du hattest {1} × {2} gesagt. Geschafft: {0}/{1}. \u{1f44f}";
/// Portuguese default for [`KEY_COMMITMENT_MET`].
pub(crate) const PT_COMMITMENT_MET: &str = "Disseste {1} × {2}. Feito: {0}/{1}. \u{1f44f}";

/// French default for [`KEY_COMMITMENT_PARTIAL`].
pub(crate) const FR_COMMITMENT_PARTIAL: &str =
    "Tu avais dit {1} × {2}. Tu en as fait {0}. On garde le rythme cette semaine ?";
/// English default for [`KEY_COMMITMENT_PARTIAL`].
pub(crate) const EN_COMMITMENT_PARTIAL: &str =
    "You said {1} × {2}. You got {0} in. Want to pick the rhythm back up this week?";
/// Spanish default for [`KEY_COMMITMENT_PARTIAL`].
pub(crate) const ES_COMMITMENT_PARTIAL: &str =
    "Dijiste {1} × {2}. Hiciste {0}. ¿Retomamos el ritmo esta semana?";
/// German default for [`KEY_COMMITMENT_PARTIAL`].
pub(crate) const DE_COMMITMENT_PARTIAL: &str =
    "Du hattest {1} × {2} gesagt. Geschafft hast du {0}. Diese Woche wieder in den Rhythmus?";
/// Portuguese default for [`KEY_COMMITMENT_PARTIAL`].
pub(crate) const PT_COMMITMENT_PARTIAL: &str =
    "Disseste {1} × {2}. Fizeste {0}. Retomamos o ritmo esta semana?";

/// French default for [`KEY_COMMITMENT_MISSED`].
pub(crate) const FR_COMMITMENT_MISSED: &str =
    "Tu avais dit {0} × {1} — rien d'enregistré. Qu'est-ce qui s'est mis en travers ?";
/// English default for [`KEY_COMMITMENT_MISSED`].
pub(crate) const EN_COMMITMENT_MISSED: &str =
    "You said {0} × {1} — nothing landed. What got in the way?";
/// Spanish default for [`KEY_COMMITMENT_MISSED`].
pub(crate) const ES_COMMITMENT_MISSED: &str =
    "Dijiste {0} × {1} y no quedó registrado ninguno. ¿Qué se cruzó?";
/// German default for [`KEY_COMMITMENT_MISSED`].
pub(crate) const DE_COMMITMENT_MISSED: &str =
    "Du hattest {0} × {1} gesagt — aufgezeichnet wurde nichts. Was ist dazwischengekommen?";
/// Portuguese default for [`KEY_COMMITMENT_MISSED`].
pub(crate) const PT_COMMITMENT_MISSED: &str =
    "Disseste {0} × {1} e não ficou nada registado. O que atrapalhou?";

/// French default for [`KEY_COMMITMENT_ACTIVITY_ANY`].
pub(crate) const FR_COMMITMENT_ACTIVITY_ANY: &str = "séances";
/// English default for [`KEY_COMMITMENT_ACTIVITY_ANY`].
pub(crate) const EN_COMMITMENT_ACTIVITY_ANY: &str = "sessions";
/// Spanish default for [`KEY_COMMITMENT_ACTIVITY_ANY`].
pub(crate) const ES_COMMITMENT_ACTIVITY_ANY: &str = "sesiones";
/// German default for [`KEY_COMMITMENT_ACTIVITY_ANY`].
pub(crate) const DE_COMMITMENT_ACTIVITY_ANY: &str = "Einheiten";
/// Portuguese default for [`KEY_COMMITMENT_ACTIVITY_ANY`].
pub(crate) const PT_COMMITMENT_ACTIVITY_ANY: &str = "sessões";

/// French default for [`KEY_COMMITMENT_PUSH_TITLE`].
pub(crate) const FR_COMMITMENT_PUSH_TITLE: &str = "Ton engagement de la semaine";
/// English default for [`KEY_COMMITMENT_PUSH_TITLE`].
pub(crate) const EN_COMMITMENT_PUSH_TITLE: &str = "Your commitment this week";
/// Spanish default for [`KEY_COMMITMENT_PUSH_TITLE`].
pub(crate) const ES_COMMITMENT_PUSH_TITLE: &str = "Tu compromiso de la semana";
/// German default for [`KEY_COMMITMENT_PUSH_TITLE`].
pub(crate) const DE_COMMITMENT_PUSH_TITLE: &str = "Deine Zusage diese Woche";
/// Portuguese default for [`KEY_COMMITMENT_PUSH_TITLE`].
pub(crate) const PT_COMMITMENT_PUSH_TITLE: &str = "O teu compromisso desta semana";

// ── OTP / channel-linking flow keys ───────────────────────────────────────
// Emitted by messaging_ingress while the user is not yet linked to a Dravr
// account, so locale resolution here cannot consult `users.locale`. Callers
// fall back to `DEFAULT_LOCALE` and optionally the channel-link override if
// one exists from a prior logout.

/// Key: fallback prompt when link state creation fails.
pub const KEY_LINK_FALLBACK_PROMPT: &str = "messaging.link.fallback_prompt";
/// Key: initial "link your account" prompt with clickable URL.
/// `{0}` = full link URL.
pub const KEY_LINK_INITIAL_PROMPT: &str = "messaging.link.initial_prompt";
/// Key: opening prompt of the in-chat OTP flow, asking for the email address.
///
/// Distinct from [`KEY_LINK_OTP_PROMPT`], which asks for the 6-digit code one
/// step later.
pub const KEY_LINK_EMAIL_PROMPT: &str = "messaging.link.email_prompt";
/// Key: logout confirmation sent after the channel link is torn down.
pub const KEY_LINK_LOGOUT_COMPLETE: &str = "messaging.link.logout_complete";
/// Key: user typed `cancel` during the OTP flow.
pub const KEY_LINK_CANCELLED: &str = "messaging.link.cancelled";
/// Key: generic "something went wrong" for unexpected DB/infra errors.
pub const KEY_LINK_GENERIC_ERROR: &str = "messaging.link.generic_error";
/// Offer to create an account in-conversation when the address is unknown.
/// `{0}` = the address they gave.
pub const KEY_LINK_SIGNUP_OFFER: &str = "messaging.link.signup_offer";
/// Confirmation that the account was created and a code is on its way.
pub const KEY_LINK_SIGNUP_CREATED: &str = "messaging.link.signup_created";
/// The account could not be created (e.g. the address was taken meanwhile).
pub const KEY_LINK_SIGNUP_FAILED: &str = "messaging.link.signup_failed";
/// Key: email resolved to a user with zero tenant memberships.
pub const KEY_LINK_NO_TENANT: &str = "messaging.link.no_tenant";
/// Key: server missing an email provider configuration.
pub const KEY_LINK_EMAIL_NOT_CONFIGURED: &str = "messaging.link.email_not_configured";
/// Key: email provider returned a send error.
pub const KEY_LINK_EMAIL_SEND_FAILED: &str = "messaging.link.email_send_failed";
/// Key: user typed something that isn't an email address.
pub const KEY_LINK_INVALID_EMAIL: &str = "messaging.link.invalid_email";
/// Key: OTP sent confirmation. `{0}` = masked email (e.g. `j***@dravr.ai`).
pub const KEY_LINK_OTP_SENT: &str = "messaging.link.otp_sent";
/// Key: OTP attempt limit exhausted, flow cancelled.
pub const KEY_LINK_TOO_MANY_ATTEMPTS: &str = "messaging.link.too_many_attempts";
/// Key: wrong OTP entered, retry allowed. `{0}` = remaining attempt count.
pub const KEY_LINK_INCORRECT_CODE: &str = "messaging.link.incorrect_code";
/// Key: DB error during OTP verification.
pub const KEY_LINK_VERIFICATION_ERROR: &str = "messaging.link.verification_error";
/// Key: channel-identity uniqueness violation when creating the link.
pub const KEY_LINK_IDENTITY_COLLISION: &str = "messaging.link.identity_collision";
/// Key: user typed non-digits at the OTP step — re-prompt.
pub const KEY_LINK_OTP_PROMPT: &str = "messaging.link.otp_prompt";
/// Key: OTP link state expired or was consumed.
pub const KEY_LINK_SESSION_EXPIRED: &str = "messaging.link.session_expired";
/// Key: account linked successfully, flow complete.
pub const KEY_LINK_SUCCESS: &str = "messaging.link.success";
/// Key: linked or messaging account is awaiting admin approval.
///
/// Surfaced by [`super::super::services::user_status_gate`] when an inbound
/// message — or the final step of an OTP/deep-link channel-linking flow —
/// resolves to a user whose [`pierre_core::models::UserStatus`] is
/// [`pierre_core::models::UserStatus::Pending`]. The channel link is still
/// created (so once an admin flips status to `Active`, the next message just
/// works) but Dravr returns this template instead of dispatching to the LLM.
pub const KEY_ACCOUNT_PENDING: &str = "messaging.account.pending";
/// Key: linked or messaging account is suspended.
///
/// Counterpart to [`KEY_ACCOUNT_PENDING`] for users whose
/// [`pierre_core::models::UserStatus`] is
/// [`pierre_core::models::UserStatus::Suspended`].
pub const KEY_ACCOUNT_SUSPENDED: &str = "messaging.account.suspended";

/// Key: the sender is rate-limited.
///
/// Messages are arriving faster than the plan's request budget allows. The
/// turn is refused before dispatch; the reply asks them to slow down and
/// retry, instead of the silence this breach used to produce (registre#8).
pub const KEY_RATE_LIMITED: &str = "messaging.rate_limited";

/// Key: a chat quota is exhausted for the sender's plan.
///
/// Daily/weekly messages or tokens, sent when the turn service's pre-turn
/// check refuses the turn outright; distinct from [`KEY_RATE_LIMITED`], which
/// is about request pacing rather than a consumed budget.
pub const KEY_QUOTA_EXCEEDED: &str = "messaging.quota_exceeded";

/// Key: a chat quota is close, or the sender is inside its burst allowance.
///
/// The soft counterpart to [`KEY_QUOTA_EXCEEDED`]: the turn ran and the reply
/// went out, and this line rides with it so the athlete learns the budget is
/// nearly spent before it refuses them. The in-app client draws the same
/// standing as a notice element; a chat channel has no element to draw, so it
/// gets the sentence. `{0}` = counter value, `{1}` = the cap, `{2}` = when it
/// resets.
pub const KEY_QUOTA_WARNING: &str = "messaging.quota_warning";

/// Key: user has not connected any fitness provider yet.
///
/// Surfaced by [`super::super::services::onboarding_gate`] when a messaging
/// channel resolves to a user with zero rows in `provider_connections`. The
/// LLM has no activity data to reason about, so we refuse the turn rather
/// than let the model hallucinate specifics from the user's message. The web
/// and mobile clients hit a structured 403 + redirect instead — this template
/// is for channels (Slack/Telegram/Discord/Messenger/WhatsApp) where the only
/// surface is the chat itself.
pub const KEY_NO_PROVIDER_CONNECTED: &str = "messaging.account.no_provider";

/// Key: no-provider denial that explicitly names the account email.
///
/// Same as [`KEY_NO_PROVIDER_CONNECTED`] but with `{1}` set to the user's
/// email so the chat reply tells them which account to sign in with.
/// Surfaced when the onboarding gate fires and the email is resolvable
/// from the channel link (the common case — the gate fires *after*
/// `authenticate_channel` succeeds). `{0}` = dravr web connect URL,
/// `{1}` = user's account email. Falls back to
/// [`KEY_NO_PROVIDER_CONNECTED`] when the email lookup fails (transient
/// DB hiccup, user deleted between channel-link resolution and email
/// fetch, etc.).
pub const KEY_NO_PROVIDER_CONNECTED_WITH_EMAIL: &str = "messaging.account.no_provider_with_email";

/// Key: coach-voice prompt for the in-chat "Connect your account" Card.
pub const KEY_CONNECT_PROMPT: &str = "messaging.connect.prompt";

/// Key: button label for the in-chat "Connect your account" Card.
pub const KEY_CONNECT_BUTTON: &str = "messaging.connect.button";

/// Key: title/header for the in-chat "Connect your account" Card.
///
/// Must be non-empty: Slack renders the Card title as a `header` block and
/// Messenger as a generic-template title, and both platform APIs reject an
/// empty string (the whole Card send fails, not just the title).
pub const KEY_CONNECT_TITLE: &str = "messaging.connect.title";

/// Key: a connected fitness provider needs to be (re)authenticated.
///
/// Surfaced deterministically by the chat pipeline's auth-recovery
/// short-circuit when the underlying scrape returns
/// `AppError::ProviderAuthRequired`. `{0}` = provider display name
/// (e.g. `Garmin Connect`, `Strava`); `{1}` = one-time hosted-login URL.
pub const KEY_PROVIDER_REAUTH_REQUIRED: &str = "messaging.provider.reauth_required";
/// The same standing as [`KEY_PROVIDER_REAUTH_REQUIRED`], said without a link.
///
/// Minting the hosted-login URL can fail — no OAuth credentials configured for
/// the tenant, or the mint endpoint refusing. `auth_recovery` used to bail out
/// on that, leaving the turn with no content at all, and the athlete was told
/// the coach could not formulate a response when what was actually wrong was a
/// disconnected provider. Knowing which provider dropped is most of the answer;
/// the link is the convenience.
///
/// Takes the provider display name as `{0}` and carries no URL placeholder.
pub const KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK: &str = "messaging.provider.reauth_required_no_link";

/// Key: button label on the reconnect Card a card-rendering channel gets
/// alongside the reauth sentence.
///
/// The sentence ([`KEY_PROVIDER_REAUTH_REQUIRED`]) already carries the URL as
/// text for channels that only autolink. Where buttons render, the same URL
/// also rides a `url` `CardAction` labelled with this string, so the athlete
/// taps instead of copying. `{0}` = provider display name.
pub const KEY_PROVIDER_RECONNECT_BUTTON: &str = "messaging.provider.reconnect_button";

// ── /status command keys ──────────────────────────────────────────────────

/// Key: `/status` opening line.
pub const KEY_STATUS_HEADER: &str = "commands.status.header";
/// Key: `/status` when the user has zero provider connections.
pub const KEY_STATUS_PROVIDERS_NONE: &str = "commands.status.providers_none";
/// Key: `/status` providers label. `{0}` = comma-separated provider list.
pub const KEY_STATUS_PROVIDERS_LABEL: &str = "commands.status.providers_label";
/// Key: `/status` groups label. `{0}` = group count (integer).
pub const KEY_STATUS_GROUPS_LABEL: &str = "commands.status.groups_label";
/// Key: `/status` channel label. `{0}` = channel type ("telegram", "slack", …).
pub const KEY_STATUS_CHANNEL_LABEL: &str = "commands.status.channel_label";

// ── /help command keys ────────────────────────────────────────────────────

/// Key: `/help` opening line.
pub const KEY_HELP_HEADER: &str = "commands.help.header";
/// Key: `/help` domain heading — general commands.
pub const KEY_HELP_DOMAIN_GENERAL: &str = "commands.help.domain.general";
/// Key: `/help` domain heading — group coaching commands.
pub const KEY_HELP_DOMAIN_GROUP: &str = "commands.help.domain.group";
/// Key: `/help` domain heading — coaching selection commands.
pub const KEY_HELP_DOMAIN_COACH: &str = "commands.help.domain.coach";
/// Key: `/help` domain heading — fitness data commands.
pub const KEY_HELP_DOMAIN_DATA: &str = "commands.help.domain.data";
/// Key: `/help` domain heading — provider management commands.
pub const KEY_HELP_DOMAIN_PROVIDER: &str = "commands.help.domain.provider";
/// Key: `/help` domain heading — account commands.
pub const KEY_HELP_DOMAIN_ACCOUNT: &str = "commands.help.domain.account";
/// Key: `/help` domain heading — training plan and calibration commands.
pub const KEY_HELP_DOMAIN_TRAINING: &str = "commands.help.domain.training";
/// Key: `/help` domain heading for the coach catalogue (`/discover`).
pub const KEY_HELP_DOMAIN_DISCOVER: &str = "commands.help.domain.discover";
/// Key: `/help` closing line inviting the user to chat.
pub const KEY_HELP_FOOTER: &str = "commands.help.footer";

// ── /logout command keys ──────────────────────────────────────────────────

/// Key: `/logout` confirmation prompt. `{0}` = channel type.
pub const KEY_LOGOUT_CONFIRM_PROMPT: &str = "commands.logout.confirm_prompt";

// ── /reset command keys ───────────────────────────────────────────────────

/// Key: `/reset` (`/nouveau`) confirmation after rotating the session onto a
/// fresh conversation. No template args.
pub const KEY_RESET_CONFIRM: &str = "commands.reset.confirm";
/// French default for [`KEY_RESET_CONFIRM`].
pub(crate) const FR_RESET_CONFIRM: &str =
    "🔄 Nouvelle conversation démarrée — on repart à neuf. Ton historique précédent reste archivé.";
/// English default for [`KEY_RESET_CONFIRM`].
pub(crate) const EN_RESET_CONFIRM: &str =
    "🔄 New conversation started — fresh slate. Your previous thread stays archived.";
/// Spanish default for [`KEY_RESET_CONFIRM`].
pub(crate) const ES_RESET_CONFIRM: &str =
    "🔄 Nueva conversación iniciada — empezamos de cero. Tu conversación anterior queda archivada.";
/// German default for [`KEY_RESET_CONFIRM`].
pub(crate) const DE_RESET_CONFIRM: &str =
    "🔄 Neue Unterhaltung gestartet — wir fangen frisch an. Dein bisheriger Verlauf bleibt archiviert.";
/// Portuguese default for [`KEY_RESET_CONFIRM`].
pub(crate) const PT_RESET_CONFIRM: &str =
    "🔄 Nova conversa iniciada — recomeçando do zero. O teu histórico anterior fica arquivado.";

// ── /privacy command keys ─────────────────────────────────────────────────

/// Key: `/privacy` status line. `{0}` = localized "enabled" / "disabled" word.
pub const KEY_PRIVACY_STATUS_LINE: &str = "commands.privacy.status_line";
/// Key: localized word for the "enabled" analytics state.
pub const KEY_PRIVACY_STATUS_ENABLED: &str = "commands.privacy.status.enabled";
/// Key: localized word for the "disabled" analytics state.
pub const KEY_PRIVACY_STATUS_DISABLED: &str = "commands.privacy.status.disabled";
/// Key: `/privacy on` confirmation message.
pub const KEY_PRIVACY_ON_CONFIRMATION: &str = "commands.privacy.on_confirmation";
/// Key: `/privacy off` confirmation message.
pub const KEY_PRIVACY_OFF_CONFIRMATION: &str = "commands.privacy.off_confirmation";

// ── /timezone command keys ────────────────────────────────────────────────

/// Key: `/timezone` confirmation once a valid IANA name is stored. `{0}` = the
/// stored timezone name.
pub const KEY_TIMEZONE_SET: &str = "commands.timezone.set";
/// Key: `/timezone` rejection when the argument is missing or not a valid IANA
/// timezone database name.
pub const KEY_TIMEZONE_INVALID: &str = "commands.timezone.invalid";

// ── /pillars command keys ─────────────────────────────────────────────────

/// Key: `/pillars` opener — the first question of the guided profile walk.
///
/// Persisted as the conversation's first assistant message, so the coach sees
/// the question it is credited with asking when the athlete's answer arrives.
pub const KEY_PILLARS_OPENER: &str = "commands.pillars.opener";
/// Key: `/pillars` refusal outside a 1:1 DM.
///
/// The walk asks about motivations, sleep, stress and recovery habits; in a
/// shared room the answers would be stamped under the channel tenant, a fact
/// space disjoint from the athlete's own dossier.
pub const KEY_PILLARS_DM_ONLY: &str = "commands.pillars.dm_only";
/// Key: `/pillars` failure when the walk could not be activated on the
/// conversation.
pub const KEY_PILLARS_START_FAILED: &str = "commands.pillars.start_failed";
/// Key: `/reset` note appended when the reset ended an in-progress profile walk.
pub const KEY_RESET_WALK_INTERRUPTED: &str = "commands.reset.walk_interrupted";

// ── messaging intake keys ─────────────────────────────────────────────────
/// Key: intake opener — sent with the first question after a channel is linked.
pub const KEY_INTAKE_OPENER: &str = "messaging.intake.opener";
/// Key: intake profile-type question (athlete vs coach), numbered 1/2.
pub const KEY_INTAKE_PERSONA: &str = "messaging.intake.persona";
/// Key: framing sent with the first PAR-Q+ question — a "yes" blocks nothing.
pub const KEY_INTAKE_PARQ_INTRO: &str = "messaging.intake.parq.intro";
/// Key: PAR-Q+ Q1 — diagnosed heart condition.
pub const KEY_INTAKE_PARQ_HEART_CONDITION: &str = "messaging.intake.parq.heart_condition";
/// Key: PAR-Q+ Q2 — chest pain at rest or on exertion.
pub const KEY_INTAKE_PARQ_CHEST_PAIN: &str = "messaging.intake.parq.chest_pain";
/// Key: PAR-Q+ Q3 — dizziness or loss of consciousness.
pub const KEY_INTAKE_PARQ_DIZZINESS: &str = "messaging.intake.parq.dizziness";
/// Key: PAR-Q+ Q4 — another diagnosed chronic condition.
pub const KEY_INTAKE_PARQ_CHRONIC_CONDITION: &str = "messaging.intake.parq.chronic_condition";
/// Key: PAR-Q+ Q5 — prescribed medication for a chronic condition.
pub const KEY_INTAKE_PARQ_MEDICATION: &str = "messaging.intake.parq.medication";
/// Key: PAR-Q+ Q6 — bone, joint or soft-tissue problem.
pub const KEY_INTAKE_PARQ_JOINT_PROBLEM: &str = "messaging.intake.parq.joint_problem";
/// Key: PAR-Q+ Q7 — advised to train only under medical supervision.
pub const KEY_INTAKE_PARQ_SUPERVISED_ONLY: &str = "messaging.intake.parq.supervised_only";
/// Key: the 1/2 answer hint appended to every PAR-Q+ question, so the seven question strings stay the instrument's own wording.
pub const KEY_INTAKE_YESNO_HINT: &str = "messaging.intake.yesno_hint";
/// Key: re-ask after an unparsed answer. `{0}` is the question, repeated verbatim.
pub const KEY_INTAKE_RETRY: &str = "messaging.intake.retry";
/// Key: intake wrap-up when no PAR-Q+ flag was raised.
pub const KEY_INTAKE_COMPLETE_CLEAR: &str = "messaging.intake.complete_clear";
/// Key: intake wrap-up when at least one flag was raised. `{0}` is the count.
pub const KEY_INTAKE_COMPLETE_FLAGGED: &str = "messaging.intake.complete_flagged";

// ── /calibrate command keys ───────────────────────────────────────────────

/// Key: `/calibrate` opener — states what was inferred and what follows.
///
/// Persisted as the conversation's first assistant message for the same reason
/// as the `/pillars` opener: without it the coach receives the athlete's first
/// answer with no question attached, and the turn counts as message #1, which
/// arms the first-turn startup prefetch.
pub const KEY_CALIBRATE_OPENER: &str = "commands.calibrate.opener";
/// Key: `/calibrate` refusal outside a 1:1 DM.
///
/// Same rationale as `/pillars`: in a shared room the answers would be stamped
/// under the channel tenant, a fact space disjoint from the athlete's dossier.
pub const KEY_CALIBRATE_DM_ONLY: &str = "commands.calibrate.dm_only";
/// Key: `/calibrate` failure when the interview could not be activated.
pub const KEY_CALIBRATE_START_FAILED: &str = "commands.calibrate.start_failed";
/// Key: `/calibrate` completion header. `{0}` = answers captured, `{1}` = asked.
pub const KEY_CALIBRATE_COMPLETE_HEADER: &str = "commands.calibrate.complete_header";
/// Key: `/calibrate` completion note for a safety-critical answer that never
/// landed. `{0}` = the topic label.
///
/// The interview says so rather than reporting success: the answers that *did*
/// land all argue for more load, and the two that bound it are exactly these.
pub const KEY_CALIBRATE_COMPLETE_MISSING: &str = "commands.calibrate.complete_missing";
/// Key: `/calibrate` follow-up offered when the athlete already has a plan.
pub const KEY_CALIBRATE_FOLLOWUP_PLAN: &str = "commands.calibrate.followup_plan";
/// Key: `/calibrate` follow-up offered when the athlete has no plan yet.
pub const KEY_CALIBRATE_FOLLOWUP_NO_PLAN: &str = "commands.calibrate.followup_no_plan";
/// Key: label for the injury-history calibration topic, used by the completion
/// message when that answer is missing.
pub const KEY_CALIBRATE_TOPIC_INJURY: &str = "commands.calibrate.topic_injury";
/// Key: label for the recovery-speed calibration topic, used by the completion
/// message when that answer is missing.
pub const KEY_CALIBRATE_TOPIC_RECOVERY: &str = "commands.calibrate.topic_recovery";

// ── /plan command keys ────────────────────────────────────────────────────

/// Key: `/plan` goal header. `{0}` = race name, `{1}` = race date, `{2}` = days out.
pub const KEY_PLAN_GOAL_LINE: &str = "commands.plan.goal_line";
/// Key: `/plan` block/phase line. `{0}` = phase, `{1}` = weekly hours note.
pub const KEY_PLAN_BLOCK_LINE: &str = "commands.plan.block_line";
/// Key: `/plan` single-day line. `{0}` = day label, `{1}` = the session.
pub const KEY_PLAN_DAY_LINE: &str = "commands.plan.day_line";
/// Key: `/plan` rest-day session text.
pub const KEY_PLAN_REST: &str = "commands.plan.rest";
/// Key: `/plan` day label for today.
pub const KEY_PLAN_TODAY: &str = "commands.plan.today";
/// Key: `/plan` day label for tomorrow.
pub const KEY_PLAN_TOMORROW: &str = "commands.plan.tomorrow";
/// Key: `/plan week` header. `{0}` = week start date, `{1}` = the week's focus.
pub const KEY_PLAN_WEEK_HEADER: &str = "commands.plan.week_header";
/// Key: `/plan` when a stored week spans the date but prescribes nothing on it.
pub const KEY_PLAN_NO_SESSION: &str = "commands.plan.no_session";
/// Key: `/plan` when no stored week spans the date at all — the plan has a hole
/// here, which is a different fact from a deliberate empty day.
pub const KEY_PLAN_NO_COVERAGE: &str = "commands.plan.no_coverage";
/// Key: `/plan` note naming the date the plan picks up again. `{0}` = that date.
pub const KEY_PLAN_RESUMES: &str = "commands.plan.resumes";
/// Key: `/plan` empty state — nothing saved yet.
pub const KEY_PLAN_EMPTY: &str = "commands.plan.empty";
/// Key: `/plan` note appended when the plan's goal fact has been superseded.
pub const KEY_PLAN_STALE_GOAL: &str = "commands.plan.stale_goal";

// ── /group command keys ───────────────────────────────────────────────────

/// Key: `/group` when the user belongs to zero groups.
pub const KEY_GROUP_LIST_EMPTY: &str = "commands.group.list_empty";
/// Key: `/group` list header. `{0}` = group count.
pub const KEY_GROUP_LIST_HEADER: &str = "commands.group.list_header";
/// Key: `/group` list item. `{0}` = name, `{1}` = member count, `{2}` = role.
pub const KEY_GROUP_LIST_ITEM: &str = "commands.group.list_item";
/// Key: error returned when a group subcommand is used outside a group.
pub const KEY_GROUP_NOT_A_MEMBER: &str = "commands.group.not_a_member";
/// Key: `/group status` summary.
/// `{0}` = name, `{1}` = members, `{2}` = active, `{3}` = peer sharing (on/off).
pub const KEY_GROUP_STATUS_SUMMARY: &str = "commands.group.status_summary";
/// Key: localized "on" for peer sharing toggle.
pub const KEY_GROUP_PEER_SHARING_ON: &str = "commands.group.peer_sharing.on";
/// Key: localized "off" for peer sharing toggle.
pub const KEY_GROUP_PEER_SHARING_OFF: &str = "commands.group.peer_sharing.off";
/// Key: `/group members` header. `{0}` = group name, `{1}` = member count.
pub const KEY_GROUP_MEMBERS_HEADER: &str = "commands.group.members_header";
/// Key: `/group members` fallback when a display name is missing.
pub const KEY_GROUP_MEMBERS_UNKNOWN: &str = "commands.group.members_unknown";
/// Key: `/group members` item. `{0}` = name, `{1}` = role.
pub const KEY_GROUP_MEMBERS_ITEM: &str = "commands.group.members_item";
/// Key: localized label for the `Owner` group role.
pub const KEY_GROUP_ROLE_OWNER: &str = "commands.group.role.owner";
/// Key: localized label for the `Admin` group role.
pub const KEY_GROUP_ROLE_ADMIN: &str = "commands.group.role.admin";
/// Key: localized label for the `Member` group role.
pub const KEY_GROUP_ROLE_MEMBER: &str = "commands.group.role.member";
/// Key: `/group invite` rejection when the user lacks admin rights.
pub const KEY_GROUP_INVITE_FORBIDDEN: &str = "commands.group.invite_forbidden";
/// Key: `/group invite` success body.
/// `{0}` = group name, `{1}` = invite code (URL), `{2}` = invite code (display).
pub const KEY_GROUP_INVITE_BODY: &str = "commands.group.invite_body";
/// Key: `/group invite` unavailable when the groups feature is disabled.
pub const KEY_GROUP_INVITE_UNAVAILABLE: &str = "commands.group.invite_unavailable";
/// Key: coach-invite success body, shared by `/coach invite` and
/// `/group invite coach`. `{0}` = group name, `{1}` = invite code (URL),
/// `{2}` = invite code (display).
pub const KEY_COACH_INVITE_BODY: &str = "commands.coach.invite_body";
/// Key: `/group leave` confirmation prompt. `{0}` = group name.
pub const KEY_GROUP_LEAVE_PROMPT: &str = "commands.group.leave_prompt";
/// Key: `/group consent` usage hint when the argument is missing or invalid.
pub const KEY_GROUP_CONSENT_USAGE: &str = "commands.group.consent_usage";
/// Key: `/group consent` confirmation. `{0}` = on/off (peer-sharing localized),
/// `{1}` = group name.
pub const KEY_GROUP_CONSENT_UPDATED: &str = "commands.group.consent_updated";
/// Key: `/group respond` usage hint when the argument is missing or invalid.
pub const KEY_GROUP_RESPOND_USAGE: &str = "commands.group.respond_usage";
/// Key: `/group respond mentions` confirmation — coach answers only when addressed.
pub const KEY_GROUP_RESPOND_MENTIONS: &str = "commands.group.respond_mentions";
/// Key: `/group respond all` confirmation — coach answers every message.
pub const KEY_GROUP_RESPOND_ALL: &str = "commands.group.respond_all";
/// Key: `/group status` line shown when the group is in mentions-only mode.
pub const KEY_GROUP_RESPOND_STATUS_MENTIONS: &str = "commands.group.respond_status_mentions";
/// Key: `/group coach detach` confirmation — the group's human coach was
/// cleared. `{0}` = group name.
pub const KEY_GROUP_COACH_DETACHED: &str = "commands.group.coach_detached";
/// Key: `/group create` usage hint when no name was typed.
pub const KEY_GROUP_CREATE_USAGE: &str = "commands.group.create_usage";
/// Key: `/group create` refusal when neither the conversation nor the
/// selection pointer names a coach for the new group.
pub const KEY_GROUP_CREATE_NO_COACH: &str = "commands.group.create_no_coach";
/// Key: `/group create` refusal when the tenant plan has no group coaching.
pub const KEY_GROUP_CREATE_UNAVAILABLE: &str = "commands.group.create_unavailable";
/// Key: `/group create` refusal when the tenant's `group_creation_policy`
/// reserves creation to its admins.
pub const KEY_GROUP_CREATE_FORBIDDEN: &str = "commands.group.create_forbidden";
/// Key: `/group create` success body. `{0}` = group name, `{1}` = coach title.
pub const KEY_GROUP_CREATED: &str = "commands.group.created";
/// Key: label of the `/group invite` button under a `/group create` reply.
pub const KEY_GROUP_INVITE_LABEL: &str = "commands.group.invite_label";
/// Key: `/group join` refusal for a missing, unknown, expired, exhausted or
/// ineligible invite code. Carries no placeholder: the typed code is never
/// echoed back.
pub const KEY_GROUP_JOIN_INVALID_CODE: &str = "commands.group.join_invalid_code";
/// Key: `/group join` when the caller already belongs to the group. `{0}` = group name.
pub const KEY_GROUP_JOIN_ALREADY_MEMBER: &str = "commands.group.join_already_member";
/// Key: `/group join` when the group is at its member cap. `{0}` = group name.
pub const KEY_GROUP_JOIN_FULL: &str = "commands.group.join_full";
/// Key: `/group join` member success. `{0}` = group name.
pub const KEY_GROUP_JOINED: &str = "commands.group.joined";
/// Key: `/group join` success for a coach-kind invite. `{0}` = group name.
pub const KEY_GROUP_JOINED_AS_COACH: &str = "commands.group.joined_as_coach";

// ── /discover command keys ────────────────────────────────────────────────

/// Key: `/discover` list card title.
pub const KEY_DISCOVER_CARD_TITLE: &str = "commands.discover.card_title";
/// Key: `/discover` list item. `{0}` = title, `{1}` = handle (no `@`),
/// `{2}` = category, `{3}` = description.
pub const KEY_DISCOVER_ITEM: &str = "commands.discover.item";
/// Key: `/discover` when a category or search matches nothing. `{0}` = what
/// was asked, as typed.
pub const KEY_DISCOVER_EMPTY: &str = "commands.discover.empty";
/// Key: bare `/discover` when nothing is published at all.
pub const KEY_DISCOVER_CATALOGUE_EMPTY: &str = "commands.discover.catalogue_empty";
/// Key: label of the next-page button under a `/discover` card.
pub const KEY_DISCOVER_MORE_LABEL: &str = "commands.discover.more_label";
/// Key: `/discover install` usage hint when no handle was typed.
pub const KEY_DISCOVER_INSTALL_USAGE: &str = "commands.discover.install_usage";
/// Key: `/discover install @handle` when no published coach answers to the
/// handle. `{0}` = the handle as typed, with its `@`.
pub const KEY_DISCOVER_INSTALL_UNKNOWN_HANDLE: &str = "commands.discover.install_unknown_handle";
/// Key: `/discover install` success — the post-install hint that teaches
/// `/coach add @handle` and the `@handle` mention. `{0}` = coach title,
/// `{1}` = handle (no `@`).
pub const KEY_DISCOVER_INSTALLED: &str = "commands.discover.installed";
/// Key: `/discover install` for a coach already on the caller's list — the
/// same hint, no second copy. `{0}` = coach title, `{1}` = handle (no `@`).
pub const KEY_DISCOVER_INSTALL_ALREADY: &str = "commands.discover.install_already";
/// Key: label of the `/coach add @handle` button under a `/discover install` reply.
pub const KEY_DISCOVER_ADD_LABEL: &str = "commands.discover.add_label";

// ── Notification messaging-sink keys ────────────────────────────

/// Key: wrapper the notification messaging sink renders around a dispatched
/// notification before sending it on a linked chat channel.
/// `{0}` = notification title, `{1}` = notification body.
pub const KEY_NOTIFICATION_CHANNEL_BODY: &str = "notifications.channel_body";

// ── /coach command keys ───────────────────────────────────────────────────

/// Key: `/coach` when the caller's list holds no coach.
pub const KEY_COACH_LIST_EMPTY: &str = "commands.coach.list_empty";
/// Key: `/coach` list card title.
pub const KEY_COACH_LIST_CARD_TITLE: &str = "commands.coach.list_card_title";
/// Key: `/coach` list item for a coach with a catalogue handle.
/// `{0}` = title, `{1}` = handle (without `@`), `{2}` = description.
pub const KEY_COACH_LIST_ITEM: &str = "commands.coach.list_item";
/// Key: `/coach` list item for a coach that owns no catalogue handle yet.
/// `{0}` = title, `{1}` = description.
pub const KEY_COACH_LIST_ITEM_NO_HANDLE: &str = "commands.coach.list_item_no_handle";
/// Key: `/coach` list footer teaching the mention and the `/coach add` forms.
pub const KEY_COACH_LIST_FOOTER: &str = "commands.coach.list_footer";
/// Key: `/coach` description fallback when the coach has no description set.
pub const KEY_COACH_NO_DESCRIPTION: &str = "commands.coach.no_description";
/// Key: `/coach add` in a group conversation / `/coach assign` success.
/// `{0}` = coach, `{1}` = group.
pub const KEY_COACH_GROUP_UPDATED: &str = "commands.coach.group_updated";
/// Key: `/coach add` success in a personal conversation. `{0}` = coach title.
/// Distinct from [`KEY_COACH_GROUP_UPDATED`] so personal replies don't mention any "group".
pub const KEY_COACH_USER_UPDATED: &str = "commands.coach.user_updated";
/// Key: `/coach assign` rejection when the user is not a group member.
pub const KEY_COACH_ASSIGN_NOT_A_MEMBER: &str = "commands.coach.assign_not_a_member";
/// Key: `/coach assign` and `/coach add` (group conversation) rejection when the
/// caller lacks admin rights in the group.
pub const KEY_COACH_ASSIGN_FORBIDDEN: &str = "commands.coach.assign_forbidden";
/// Key: `/coach add` typed without a coach.
pub const KEY_COACH_ADD_USAGE: &str = "commands.coach.add_usage";
/// Key: `/coach add` when no installed coach answers to the argument.
/// `{0}` = the handle as typed, with its `@`.
pub const KEY_COACH_ADD_UNKNOWN: &str = "commands.coach.add_unknown";
/// Key: `/coach remove` refused in a group conversation, where the coach is the group's.
pub const KEY_COACH_REMOVE_GROUP_THREAD: &str = "commands.coach.remove_group_thread";
/// Key: `/coach remove` when the conversation has no coach attached.
pub const KEY_COACH_REMOVE_NOTHING: &str = "commands.coach.remove_nothing";
/// Key: `/coach remove` success. `{0}` = coach title.
pub const KEY_COACH_REMOVED: &str = "commands.coach.removed";
/// Key: `/coach create` dispatched with no conversation to read.
pub const KEY_COACH_CREATE_NO_CONVERSATION: &str = "commands.coach.create_no_conversation";
/// Key: `/coach create` on a conversation with no message to draft from.
pub const KEY_COACH_CREATE_EMPTY: &str = "commands.coach.create_empty";
/// Key: `/coach create` with arguments that are neither empty nor `confirm token`.
pub const KEY_COACH_CREATE_USAGE: &str = "commands.coach.create_usage";
/// Key: `/coach create` proposal card title.
pub const KEY_COACH_CREATE_CARD_TITLE: &str = "commands.coach.create_card_title";
/// Key: `/coach create` proposal card body.
/// `{0}` = title, `{1}` = description, `{2}` = category, `{3}` = tags, `{4}` = claim token.
pub const KEY_COACH_CREATE_PROPOSAL_BODY: &str = "commands.coach.create_proposal_body";
/// Key: `/coach create` proposal card — the button that creates the coach.
pub const KEY_COACH_CREATE_CONFIRM_LABEL: &str = "commands.coach.create_confirm_label";
/// Key: `/coach create` proposal card — the button that discards the draft.
pub const KEY_COACH_CREATE_DISCARD_LABEL: &str = "commands.coach.create_discard_label";
/// Key: `/coach create confirm` refused by the per-user coach quota.
/// `{0}` = coaches the caller already has, `{1}` = the plan's maximum.
pub const KEY_COACH_CREATE_QUOTA: &str = "commands.coach.create_quota";
/// Key: `/coach create confirm` success, coach bound to the conversation.
/// `{0}` = title, `{1}` = handle (without `@`).
pub const KEY_COACH_CREATE_DONE: &str = "commands.coach.create_done";
/// Key: `/coach create confirm` success when the conversation could not take the
/// coach (a group whose settings the caller may not change). `{0}` = title, `{1}` = handle.
pub const KEY_COACH_CREATE_DONE_UNBOUND: &str = "commands.coach.create_done_unbound";
/// Key: `/deny` on a coach draft — the draft is dropped, nothing was created.
pub const KEY_COACH_CREATE_DISCARDED: &str = "commands.coach.create_discarded";

// ── Compiled-in defaults: French (DEFAULT_LOCALE) ────────────────────────

/// French default for [`KEY_ERROR_GENERIC`]. `{0}` = 8-char correlation id.
pub const FR_ERROR_GENERIC: &str = "Dravr est temporairement indisponible. L'équipe a été notifiée — réessaie dans quelques minutes. (ref: {0})";
/// French default for [`KEY_GUARDIAN_DENIED`]. No format placeholders.
pub const FR_GUARDIAN_DENIED: &str = "Cette action a été bloquée par sécurité. Reformule ta demande ou réessaie sans le contexte précédent.";

/// French default for [`KEY_GUARDIAN_CONFIRM_PROMPT`]. Args: `{0}` tool, `{1}` token.
pub const FR_GUARDIAN_CONFIRM_PROMPT: &str = "Par sécurité, l'action {0} doit être confirmée avant de s'exécuter. Réponds /confirm {1} pour l'approuver ou /deny {1} pour l'annuler (expire dans 5 minutes).";

/// French default for [`KEY_GUARDIAN_CONFIRM_DONE`]. Args: `{0}` tool.
pub const FR_GUARDIAN_CONFIRM_DONE: &str = "C'est fait — l'action {0} a été exécutée.";

/// French default for [`KEY_GUARDIAN_CONFIRM_FAILED`]. Args: `{0}` tool.
pub const FR_GUARDIAN_CONFIRM_FAILED: &str = "L'action {0} a été confirmée mais son exécution a échoué. Reformule ta demande pour réessayer.";

/// French default for [`KEY_GUARDIAN_CONFIRM_DENIED`]. No format placeholders.
pub const FR_GUARDIAN_CONFIRM_DENIED: &str = "Compris — l'action a été annulée.";

/// French default for [`KEY_GUARDIAN_CONFIRM_EXPIRED`]. No format placeholders.
pub const FR_GUARDIAN_CONFIRM_EXPIRED: &str =
    "Cette confirmation a expiré. Reformule ta demande pour relancer l'action.";

/// French default for [`KEY_GUARDIAN_CONFIRM_NOT_FOUND`]. No format placeholders.
pub const FR_GUARDIAN_CONFIRM_NOT_FOUND: &str =
    "Aucune action en attente ne correspond à ce code. Elle a peut-être déjà été traitée.";
/// French default for [`KEY_EMPTY_REPLY`].
pub const FR_EMPTY_REPLY: &str =
    "Hmm, je n'ai pas réussi à formuler une réponse. Peux-tu reformuler ta question?";
/// French default for [`KEY_REPLY_WITHHELD`].
pub const FR_REPLY_WITHHELD: &str = "Ma réponse n'est pas passée — elle mélangeait des détails techniques qui n'ont pas leur place ici. Renvoie ton dernier message et on reprend là où on en était.";
/// French default for [`KEY_GUARDRAIL_TOO_LONG`].
pub const FR_GUARDRAIL_TOO_LONG: &str = "J'ai une réponse plus longue prête, mais elle dépasse la limite de longueur configurée. Veux-tu que je te la résume plus brièvement?";
/// French default for [`KEY_GUARDRAIL_BLOCKED_TOPIC`].
pub const FR_GUARDRAIL_BLOCKED_TOPIC: &str = "Je préfère ne pas aborder ce sujet ici. Restons concentrés sur ton entraînement et ta récupération. Y a-t-il quelque chose de précis sur lequel je peux t'aider?";
/// French default for [`KEY_VERIFICATION_WARN_SUFFIX`].
///
/// Used as the **header** for a bulleted list of the specific claims the
/// verifier could not back up — the list itself is built in Rust and
/// appended by the caller. Letting the user see *which* claims to push
/// back on is more actionable than an opaque "I'm unsure about N
/// things" tail.
///
/// The caller joins this header to the main reply with `\n\n---\n`, then
/// appends each flagged claim as a `- {claim}` bullet line; both halves
/// stay externalized so translators only see prose.
pub const FR_VERIFICATION_WARN_SUFFIX: &str = "⚠️ Quelques affirmations que je n'ai pas pu étayer formellement — corrige-moi si l'une d'elles est à côté :";
/// French default for [`KEY_VERIFICATION_BLOCK_FALLBACK`].
pub const FR_VERIFICATION_BLOCK_FALLBACK: &str = "J'ai commencé à répondre, mais quelques-unes des affirmations que j'allais faire ne correspondaient pas aux sources que je considère fiables. Laisse-moi reformuler — peux-tu me reposer la question avec un peu plus de contexte sur ce que tu cherches à comprendre?";
/// French canonical refusal for off-scope requests.
pub const FR_SCOPE_REFUSAL: &str =
    "Ça sort de ce que je peux t'aider à faire — je suis ton assistant fitness.";
/// French canonical refusal for missing-capability requests.
pub const FR_CAPABILITY_REFUSAL: &str = "Je ne peux pas faire ça avec les outils dont je dispose.";
/// French Nutrition-coach carve-out for the generic scope list.
pub const FR_COACH_SCOPE_CARVE_OUT_NUTRITION: &str = "## Précision pour ton rôle de coach nutrition\n\nEn tant que coach nutrition, les questions sur les repas, dîners, petits-déjeuners, collations, choix alimentaires et la planification des repas liée à l'entraînement SONT entièrement dans ton domaine. Réponds directement à ces questions en t'appuyant sur les données d'entraînement de l'utilisateur (intensité, durée, dépense énergétique). La règle « recherche de nourriture hors périmètre » qui apparaît plus haut vise uniquement la recherche de restaurants, d'applications de livraison ou de menus en ligne — pas les conseils nutritionnels basés sur les preuves et sur les données d'entraînement. Ne refuse jamais « que manger après ma sortie », « idées de dîner après mon entraînement », « collation post-séance » ou équivalents.";
/// French Recipes-coach carve-out for the generic scope list.
pub const FR_COACH_SCOPE_CARVE_OUT_RECIPES: &str = "## Précision pour ton rôle de coach recettes et repas\n\nEn tant que coach recettes et planification de repas, les suggestions de repas, recettes, idées de plats et choix alimentaires adaptés à l'entraînement SONT entièrement dans ton domaine. La règle « recherche de nourriture hors périmètre » qui apparaît plus haut vise la recherche de restaurants et les applications de livraison — pas la suggestion de recettes maison ou la planification de repas fondées sur les données d'entraînement de l'utilisateur. Réponds directement à ces demandes.";
/// French placeholder shown while the LLM is composing its reply.
pub const FR_THINKING_PLACEHOLDER: &str = "réflexion…";
/// French unknown-command reply.
pub const FR_UNKNOWN_COMMAND: &str =
    "Commande inconnue. Tape /help pour voir la liste des commandes disponibles.";
/// French progress status during prompt-assembly.
pub const FR_STATUS_READING_QUESTION: &str = "lecture de ta question…";
/// French progress status during LLM dispatch.
pub const FR_STATUS_GENERATING_RESPONSE: &str = "génération de la réponse…";
/// French progress status for tool-call start. `{0}` = tool name.
pub const FR_STATUS_CALLING_TOOL: &str = "appel de {0}…";
/// French progress status for pipeline error. `{0}` = error text.
pub const FR_STATUS_ERROR: &str = "erreur : {0}";
/// French coach-proposal lead-in with profile. `{0}` = sport, `{1}` = count.
pub const FR_COACH_PROPOSAL_WELCOME: &str =
    "Bienvenue ! D'après ton entraînement récent en {0}, voici {1} coachs pour toi :\n\n";
/// French coach-proposal cold-start lead-in. `{0}` = count.
pub const FR_COACH_PROPOSAL_WELCOME_GENERIC: &str =
    "Bienvenue ! Voici {0} coachs pour bien démarrer :\n\n";
/// French coach-proposal footer.
pub const FR_COACH_PROPOSAL_FOOTER: &str =
    "\nRéponds avec un numéro pour commencer, ou pose-moi simplement ta question.";
/// French default for [`KEY_REGISTRATION_APPROVED`].
pub const FR_REGISTRATION_APPROVED: &str =
    "🎉 Ton compte Dravr a été approuvé ! Tu peux maintenant discuter avec ton coach ici. Pose-moi ta première question quand tu veux.";
/// French default for [`KEY_BACKFILL_READY`]. `{0}` = activity count.
pub const FR_BACKFILL_READY: &str =
    "✅ Ton historique est prêt — {0} activités récupérées. Redemande-moi ce que tu cherchais.";
/// French default for [`KEY_BACKFILL_LIST_HEADER`]. `{0}` = activity count.
pub const FR_BACKFILL_LIST_HEADER: &str = "✅ Ton historique est prêt — {0} activités :";
/// French default for [`KEY_BACKFILL_LIST_MORE`]. `{0}` = remaining count.
pub const FR_BACKFILL_LIST_MORE: &str = "… et {0} de plus";

pub(crate) const FR_LINK_FALLBACK_PROMPT: &str = "Pour discuter avec Dravr, relie d'abord ton compte. Ouvre l'app web Dravr pour connecter ce canal.";
pub(crate) const FR_LINK_INITIAL_PROMPT: &str = "Salut ! Pour discuter avec Dravr, relie d'abord ton compte :\n{0}\n\nCe lien expire dans 10 minutes.";
pub(crate) const FR_LINK_EMAIL_PROMPT: &str =
    "Salut ! Pour relier ton compte Dravr, écris ton adresse e-mail.\nTape « cancel » pour arrêter.";
pub(crate) const FR_LINK_LOGOUT_COMPLETE: &str =
    "Tu es déconnecté de Dravr. Envoie un message à tout moment pour relier ton compte.";
pub(crate) const FR_LINK_CANCELLED: &str =
    "Liaison annulée. Envoie un message à tout moment pour recommencer.";
pub(crate) const FR_LINK_GENERIC_ERROR: &str =
    "Quelque chose a mal tourné. Réessaie dans un moment.";
pub(crate) const FR_LINK_SIGNUP_OFFER: &str = "Aucun compte Dravr avec {0}. Je peux t'en creer un tout de suite - reponds « oui » pour continuer, ou « cancel » pour arreter.";
pub(crate) const FR_LINK_SIGNUP_CREATED: &str =
    "Compte cree. Je t'envoie un code par e-mail pour confirmer que l'adresse est bien a toi.";
pub(crate) const FR_LINK_SIGNUP_FAILED: &str =
    "Je n'ai pas pu creer le compte. Reessaie dans un instant, ou tape « cancel » pour arreter.";
pub(crate) const FR_LINK_NO_TENANT: &str =
    "Ce compte n'est associé à aucune organisation. Contacte le support.";
pub(crate) const FR_LINK_EMAIL_NOT_CONFIGURED: &str =
    "L'envoi d'e-mails n'est pas configuré. Contacte ton administrateur.";
pub(crate) const FR_LINK_EMAIL_SEND_FAILED: &str =
    "Impossible d'envoyer l'e-mail de vérification. Réessaie dans un moment.";
pub(crate) const FR_LINK_INVALID_EMAIL: &str =
    "Cela ne ressemble pas à une adresse e-mail. Tape l'adresse e-mail de ton compte Dravr.";
pub(crate) const FR_LINK_OTP_SENT: &str = "J'ai envoyé un code à 6 chiffres à {0}. Tape-le ici dans les 10 prochaines minutes.\nTape « cancel » pour arrêter.";
pub(crate) const FR_LINK_TOO_MANY_ATTEMPTS: &str = "Trop de tentatives incorrectes. La session de liaison est annulée. Envoie un message pour recommencer.";
pub(crate) const FR_LINK_INCORRECT_CODE: &str =
    "Code incorrect. Il te reste {0} tentative(s). Réessaie ou tape « cancel » pour arrêter.";
pub(crate) const FR_LINK_VERIFICATION_ERROR: &str =
    "Une erreur est survenue lors de la vérification de ton compte. Réessaie.";
pub(crate) const FR_LINK_IDENTITY_COLLISION: &str = "Impossible de lier ton compte. Cette identité de canal est peut-être déjà associée à un autre compte.";
pub(crate) const FR_LINK_OTP_PROMPT: &str =
    "Tape le code à 6 chiffres envoyé par e-mail, ou tape « cancel » pour arrêter.";
pub(crate) const FR_LINK_SESSION_EXPIRED: &str =
    "Ta session de liaison a expiré. Envoie un message pour recommencer.";
pub(crate) const FR_LINK_SUCCESS: &str = "Ton compte est maintenant lié ! Tu peux discuter avec Dravr depuis ce canal.\n\nTape « logout » à tout moment pour te déconnecter.";
pub(crate) const FR_ACCOUNT_PENDING: &str = "Ton compte est lié à ce canal, mais il est en attente d'approbation par un administrateur. Tu pourras discuter avec Dravr dès qu'il sera activé — on te préviendra ici.";
pub(crate) const FR_ACCOUNT_SUSPENDED: &str =
    "Ton compte Dravr est suspendu. Contacte le support pour rétablir l'accès.";
pub(crate) const FR_RATE_LIMITED: &str = "Tu envoies des messages un peu trop vite pour ton forfait. Attends un moment, puis réessaie — je serai là.";
pub(crate) const FR_QUOTA_EXCEEDED: &str = "Tu as atteint la limite de conversation de ton forfait pour le moment. Elle se réinitialise automatiquement — reviens un peu plus tard.";
pub(crate) const FR_QUOTA_WARNING: &str =
    "Petite note : tu as utilisé {0} de {1} sur ton forfait. Le compteur se réinitialise le {2}.";
pub(crate) const FR_NO_PROVIDER_CONNECTED: &str = "Avant de discuter, connecte un service de fitness (Strava, Garmin, Whoop) depuis l'app Dravr — sans ça je n'ai aucune donnée d'activité pour t'aider.\n\nConnecte-toi ici :\n{0}";
pub(crate) const FR_NO_PROVIDER_CONNECTED_WITH_EMAIL: &str = "Avant de discuter, connecte un service de fitness (Strava, Garmin, Whoop) depuis l'app Dravr — sans ça je n'ai aucune donnée d'activité pour t'aider.\n\nConnecte-toi avec ton compte {1} ici :\n{0}";
pub(crate) const FR_CONNECT_PROMPT: &str = "Connectons ton service de fitness (Strava, Garmin ou Whoop) pour que je puisse te coacher sur tes vraies données. Touche le bouton ci-dessous pour te connecter en toute sécurité.";
pub(crate) const FR_CONNECT_BUTTON: &str = "Connecter mon compte";
pub(crate) const FR_CONNECT_TITLE: &str = "Connecter un service de fitness";

pub(crate) const FR_PROVIDER_REAUTH_REQUIRED: &str = "La connexion à {0} a expiré — je ne peux pas récupérer tes données pour le moment. Reconnecte ton compte ici (lien valide 24 heures) :\n\n{1}\n\nUne fois reconnecté, repose-moi ta question.";
pub(crate) const FR_PROVIDER_REAUTH_REQUIRED_NO_LINK: &str = "La connexion à {0} a expiré — je ne peux pas récupérer tes données pour le moment. Reconnecte ton compte {0} depuis les réglages, puis repose-moi ta question.";
pub(crate) const FR_PROVIDER_RECONNECT_BUTTON: &str = "Reconnecter {0}";

pub(crate) const FR_STATUS_HEADER: &str = "Ton statut Dravr :\n";
pub(crate) const FR_STATUS_PROVIDERS_NONE: &str = "\nFournisseurs : aucun connecté";
pub(crate) const FR_STATUS_PROVIDERS_LABEL: &str = "\nFournisseurs : {0}";
pub(crate) const FR_STATUS_GROUPS_LABEL: &str = "\nGroupes : {0}";
pub(crate) const FR_STATUS_CHANNEL_LABEL: &str = "\nCanal : {0}";

pub(crate) const FR_HELP_HEADER: &str = "Commandes disponibles :\n";
pub(crate) const FR_HELP_DOMAIN_GENERAL: &str = "Général";
pub(crate) const FR_HELP_DOMAIN_GROUP: &str = "Coaching de groupe";
pub(crate) const FR_HELP_DOMAIN_COACH: &str = "Coaching";
pub(crate) const FR_HELP_DOMAIN_DATA: &str = "Données d'activité";
pub(crate) const FR_HELP_DOMAIN_PROVIDER: &str = "Fournisseurs";
pub(crate) const FR_HELP_DOMAIN_ACCOUNT: &str = "Compte";
pub(crate) const FR_HELP_DOMAIN_TRAINING: &str = "Entraînement";
pub(crate) const FR_HELP_DOMAIN_DISCOVER: &str = "Catalogue de coachs";
pub(crate) const FR_HELP_FOOTER: &str = "\nOu écris-moi simplement pour discuter avec ton coach.";

pub(crate) const FR_LOGOUT_CONFIRM_PROMPT: &str = "Ceci va délier ton compte {0} de Dravr.\nIl faudra le relier pour réutiliser la messagerie.\n\nTape « logout » pour confirmer.";

pub(crate) const FR_PRIVACY_STATUS_LINE: &str = "Le consentement aux statistiques anonymes est actuellement <b>{0}</b>.\n\nUtilise <code>/privacy on</code> pour l'activer ou <code>/privacy off</code> pour le désactiver.";
pub(crate) const FR_PRIVACY_STATUS_ENABLED: &str = "activé";
pub(crate) const FR_PRIVACY_STATUS_DISABLED: &str = "désactivé";
pub(crate) const FR_PRIVACY_ON_CONFIRMATION: &str = "Le consentement aux statistiques anonymes est maintenant <b>activé</b>. Merci de nous aider à améliorer Dravr !\n\nUtilise <code>/privacy off</code> pour te retirer à tout moment.";
pub(crate) const FR_PRIVACY_OFF_CONFIRMATION: &str = "Le consentement aux statistiques anonymes est maintenant <b>désactivé</b>. Aucune donnée d'usage anonyme ne sera collectée.\n\nUtilise <code>/privacy on</code> pour te réinscrire à tout moment.";
pub(crate) const FR_TIMEZONE_SET: &str = "Fuseau horaire réglé sur <b>{0}</b>. Les heures de départ de tes activités s'afficheront désormais dans ce fuseau.";
pub(crate) const FR_TIMEZONE_INVALID: &str = "Fuseau horaire invalide. Donne un nom IANA, par exemple : <code>/timezone America/Toronto</code>.";

pub(crate) const FR_PILLARS_OPENER: &str = "On va construire ton profil ensemble — je te pose quelques questions, une à la fois. Pour commencer : qu'est-ce qui te motive profondément à t'entraîner (ton « North Star ») ?";
pub(crate) const FR_PILLARS_DM_ONLY: &str = "Le profil se construit en privé. Écris-moi <code>/pillars</code> en message direct et on commence.";

pub(crate) const FR_INTAKE_OPENER: &str = "Avant de commencer, quelques questions rapides — une minute maintenant, et le coaching qui suit est plus précis et plus sûr.";
pub(crate) const FR_INTAKE_PERSONA: &str = "D'abord : tu t'entraînes pour toi, ou tu entraînes d'autres personnes ?\n\n<b>1</b> — Je m'entraîne pour moi\n<b>2</b> — J'entraîne d'autres personnes";
pub(crate) const FR_INTAKE_PARQ_INTRO: &str = "Maintenant sept questions de santé standard (le PAR-Q+). Un « oui » ne bloque rien — ça indique simplement à ton coach où être prudent.";
pub(crate) const FR_INTAKE_PARQ_HEART_CONDITION: &str =
    "Un médecin t'a-t-il déjà dit que tu avais un problème cardiaque ?";
pub(crate) const FR_INTAKE_PARQ_CHEST_PAIN: &str = "Ressens-tu des douleurs à la poitrine au repos, pendant tes activités quotidiennes ou à l'effort ?";
pub(crate) const FR_INTAKE_PARQ_DIZZINESS: &str = "Perds-tu l'équilibre à cause d'étourdissements, ou as-tu perdu connaissance au cours des 12 derniers mois ?";
pub(crate) const FR_INTAKE_PARQ_CHRONIC_CONDITION: &str =
    "As-tu reçu un diagnostic pour une autre maladie chronique (autre qu'une maladie cardiaque) ?";
pub(crate) const FR_INTAKE_PARQ_MEDICATION: &str =
    "Prends-tu actuellement des médicaments prescrits pour une maladie chronique ?";
pub(crate) const FR_INTAKE_PARQ_JOINT_PROBLEM: &str = "As-tu un problème osseux, articulaire ou des tissus mous qui pourrait être aggravé par l'activité physique ?";
pub(crate) const FR_INTAKE_PARQ_SUPERVISED_ONLY: &str = "Un médecin t'a-t-il dit que tu ne devais faire de l'activité physique que sous supervision médicale ?";
pub(crate) const FR_INTAKE_YESNO_HINT: &str = "Réponds <b>1</b> pour oui, <b>2</b> pour non.";
pub(crate) const FR_INTAKE_RETRY: &str =
    "Désolé — j'ai besoin du chiffre seul, pour bien l'enregistrer.\n\n{0}";
pub(crate) const FR_INTAKE_COMPLETE_CLEAR: &str =
    "C'est tout, rien à signaler. Pose-moi ce que tu veux sur ton entraînement.";
pub(crate) const FR_INTAKE_COMPLETE_FLAGGED: &str = "C'est tout. J'ai noté {0} point(s) à garder en tête pour ton coach — rien ne t'empêche de t'entraîner, et ça vaut un mot à ton médecin avant de forcer.";
pub(crate) const FR_PILLARS_START_FAILED: &str =
    "Je n'ai pas réussi à démarrer le profil sur cette conversation. Réessaie dans un instant.";
pub(crate) const FR_RESET_WALK_INTERRUPTED: &str =
    "\n\nTon profil était en cours — écris <code>/pillars</code> pour reprendre.";

pub(crate) const FR_CALIBRATE_OPENER: &str = "On va calibrer la difficulté de ton entraînement. Je pars de tes données récentes et je te pose six questions courtes — une à la fois. Première : tu veux que ça devienne plus dur comment — plus d'heures, plus de séances dures, ou des sorties longues plus longues ?";
pub(crate) const FR_CALIBRATE_DM_ONLY: &str = "Le calibrage se fait en privé. Écris-moi <code>/calibrate</code> en message direct et on commence.";
pub(crate) const FR_CALIBRATE_START_FAILED: &str =
    "Je n'ai pas réussi à démarrer le calibrage sur cette conversation. Réessaie dans un instant.";
pub(crate) const FR_CALIBRATE_COMPLETE_HEADER: &str =
    "Calibrage terminé — j'ai retenu {0} réponses sur {1}.";
pub(crate) const FR_CALIBRATE_COMPLETE_MISSING: &str = "Il me manque ta réponse sur {0}. C'est ce qui limite jusqu'où je peux pousser, alors reprenons cette question — écris <code>/calibrate</code>.";
pub(crate) const FR_CALIBRATE_FOLLOWUP_PLAN: &str =
    "Tu veux que je reconstruise tes semaines à venir avec ça ?";
pub(crate) const FR_CALIBRATE_FOLLOWUP_NO_PLAN: &str =
    "Tu veux que je te construise un plan là-dessus ?";
pub(crate) const FR_CALIBRATE_TOPIC_INJURY: &str = "tes blessures et douleurs récentes";
pub(crate) const FR_CALIBRATE_TOPIC_RECOVERY: &str = "ta vitesse de récupération";

pub(crate) const FR_PLAN_GOAL_LINE: &str = "Objectif : {0} — {1} ({2} jours)";
pub(crate) const FR_PLAN_BLOCK_LINE: &str = "Bloc : {0}{1}";
pub(crate) const FR_PLAN_DAY_LINE: &str = "{0} : {1}";
pub(crate) const FR_PLAN_REST: &str = "repos";
pub(crate) const FR_PLAN_TODAY: &str = "Aujourd'hui";
pub(crate) const FR_PLAN_TOMORROW: &str = "Demain";
pub(crate) const FR_PLAN_WEEK_HEADER: &str = "Semaine du {0}{1}";
pub(crate) const FR_PLAN_NO_SESSION: &str = "rien de prévu";
pub(crate) const FR_PLAN_NO_COVERAGE: &str = "pas couvert par le plan";
pub(crate) const FR_PLAN_RESUMES: &str = "Le plan reprend le {0}.";
pub(crate) const FR_PLAN_EMPTY: &str = "Aucun plan enregistré pour l'instant — demande à ton coach d'en construire un vers ton objectif.";
pub(crate) const FR_PLAN_STALE_GOAL: &str =
    "\n\n⚠️ Ton objectif a changé depuis — demande à ton coach de mettre le plan à jour.";

pub(crate) const FR_GROUP_LIST_EMPTY: &str =
    "Tu n'es membre d'aucun groupe.\nCrée ou rejoins un groupe via l'app web ou mobile.";
pub(crate) const FR_GROUP_LIST_HEADER: &str = "Tes groupes ({0}) :\n";
pub(crate) const FR_GROUP_LIST_ITEM: &str = "- {0} ({1} membres) [{2}]";
pub(crate) const FR_GROUP_NOT_A_MEMBER: &str = "Tu n'es membre d'aucun groupe";
pub(crate) const FR_GROUP_STATUS_SUMMARY: &str =
    "{0} — statistiques :\n- Membres : {1}\n- Actifs : {2}\n- Partage entre pairs : {3}";
pub(crate) const FR_GROUP_PEER_SHARING_ON: &str = "activé";
pub(crate) const FR_GROUP_PEER_SHARING_OFF: &str = "désactivé";
pub(crate) const FR_GROUP_MEMBERS_HEADER: &str = "{0} — membres ({1}) :\n";
pub(crate) const FR_GROUP_MEMBERS_UNKNOWN: &str = "Inconnu";
pub(crate) const FR_GROUP_MEMBERS_ITEM: &str = "- {0} [{1}]";
pub(crate) const FR_GROUP_ROLE_OWNER: &str = "propriétaire";
pub(crate) const FR_GROUP_ROLE_ADMIN: &str = "admin";
pub(crate) const FR_GROUP_ROLE_MEMBER: &str = "membre";
pub(crate) const FR_GROUP_INVITE_FORBIDDEN: &str =
    "Seuls les admins et les propriétaires peuvent générer des liens d'invitation.";
pub(crate) const FR_GROUP_INVITE_BODY: &str =
    "Lien d'invitation pour {0} :\nhttps://app.dravr.ai/groups/join/{1}\n\nCode : {2}\nValide 7 jours.";
pub(crate) const FR_GROUP_INVITE_UNAVAILABLE: &str =
    "Les invitations de groupe ne sont pas disponibles.";
pub(crate) const FR_COACH_INVITE_BODY: &str =
    "Invitation coach pour {0} — la personne qui l'utilise devient le coach humain du groupe :\nhttps://app.dravr.ai/groups/join/{1}\n\nCode : {2}\nValide 7 jours.";
pub(crate) const FR_GROUP_LEAVE_PROMPT: &str =
    "Veux-tu vraiment quitter {0} ?\nTape « YES » pour confirmer.";
pub(crate) const FR_GROUP_CONSENT_USAGE: &str = "Usage : /group consent yes  ou  /group consent no";
pub(crate) const FR_GROUP_RESPOND_USAGE: &str =
    "Usage : /group respond mentions  ou  /group respond all";
pub(crate) const FR_GROUP_RESPOND_MENTIONS: &str = "Le coach ne répond plus que lorsqu'on l'interpelle (mentionne-le avec @ ou réponds à l'un de ses messages). Il continue de suivre la discussion pour garder le contexte.";
pub(crate) const FR_GROUP_RESPOND_ALL: &str =
    "Le coach répond de nouveau à tous les messages du groupe.";
pub(crate) const FR_GROUP_COACH_DETACHED: &str = "{0} n'a plus de coach humain attitré.";
pub(crate) const FR_NOTIFICATION_CHANNEL_BODY: &str = "🔔 {0}\n\n{1}";
pub(crate) const FR_GROUP_RESPOND_STATUS_MENTIONS: &str = "Le coach ne répond que lorsqu'on l'interpelle. Pour revenir à tous les messages : /group respond all";
pub(crate) const FR_GROUP_CONSENT_UPDATED: &str =
    "Le partage de tes données avec les autres membres de {1} est maintenant {0}.";
pub(crate) const FR_GROUP_CREATE_USAGE: &str =
    "Usage : /group create nom-du-groupe — par exemple /group create Coureurs du dimanche";
pub(crate) const FR_GROUP_CREATE_NO_COACH: &str =
    "Choisis d'abord le coach du groupe : /coach add @handle dans cette discussion, puis relance /group create.";
pub(crate) const FR_GROUP_CREATE_UNAVAILABLE: &str =
    "Le coaching de groupe n'est pas inclus dans ton forfait.";
pub(crate) const FR_GROUP_CREATE_FORBIDDEN: &str =
    "La création de groupes est réservée aux admins de ton espace.";
pub(crate) const FR_GROUP_CREATED: &str =
    "Groupe « {0} » créé avec le coach {1}. Invite des membres avec /group invite.";
pub(crate) const FR_GROUP_INVITE_LABEL: &str = "Inviter des membres";
pub(crate) const FR_GROUP_JOIN_INVALID_CODE: &str =
    "Ce code d'invitation n'est pas valide ou a expiré. Demande un nouveau lien à un admin du groupe (/group invite).";
pub(crate) const FR_GROUP_JOIN_ALREADY_MEMBER: &str = "Tu fais déjà partie de {0}.";
pub(crate) const FR_GROUP_JOIN_FULL: &str = "{0} est complet.";
pub(crate) const FR_GROUP_JOINED: &str =
    "Bienvenue dans {0} ! La discussion de groupe est maintenant dans ta liste de conversations.";
pub(crate) const FR_GROUP_JOINED_AS_COACH: &str = "Tu es maintenant le coach humain de {0}.";
pub(crate) const FR_DISCOVER_CARD_TITLE: &str = "Catalogue de coachs";
pub(crate) const FR_DISCOVER_ITEM: &str = "• {0} — @{1} [{2}]\n  {3}\n";
pub(crate) const FR_DISCOVER_EMPTY: &str =
    "Aucun coach du catalogue ne correspond à « {0} ». Tape /discover pour voir les nouveautés.";
pub(crate) const FR_DISCOVER_CATALOGUE_EMPTY: &str = "Le catalogue de coachs est vide.";
pub(crate) const FR_DISCOVER_MORE_LABEL: &str = "Suivants";
pub(crate) const FR_DISCOVER_INSTALL_USAGE: &str =
    "Usage : /discover install @handle — le @handle s'affiche dans /discover.";
pub(crate) const FR_DISCOVER_INSTALL_UNKNOWN_HANDLE: &str =
    "Aucun coach publié ne répond à {0}. Tape /discover pour parcourir le catalogue.";
pub(crate) const FR_DISCOVER_INSTALLED: &str =
    "{0} est installé. Utilise-le dans n'importe quelle discussion : /coach add @{1}, ou mentionne @{1} dans un message pour un seul tour.";
pub(crate) const FR_DISCOVER_INSTALL_ALREADY: &str =
    "{0} est déjà installé. Utilise-le dans n'importe quelle discussion : /coach add @{1}, ou mentionne @{1} dans un message pour un seul tour.";
pub(crate) const FR_DISCOVER_ADD_LABEL: &str = "Utiliser ici";

pub(crate) const FR_COACH_LIST_EMPTY: &str =
    "Aucun coach sur ta liste. Tape /discover pour parcourir le catalogue, ou /coach create pour en créer un à partir de cette conversation.";
pub(crate) const FR_COACH_LIST_CARD_TITLE: &str = "Tes coachs";
pub(crate) const FR_COACH_LIST_ITEM: &str = "• {0} — @{1}\n  {2}\n";
pub(crate) const FR_COACH_LIST_ITEM_NO_HANDLE: &str = "• {0}\n  {1}\n";
pub(crate) const FR_COACH_LIST_FOOTER: &str =
    "Mentionne @handle dans un message pour lui confier ce seul message, ou tape /coach add @handle pour qu'il réponde ici à partir de maintenant.";
pub(crate) const FR_COACH_NO_DESCRIPTION: &str = "Sans description";
pub(crate) const FR_COACH_GROUP_UPDATED: &str = "Coach mis à jour sur {0} pour le groupe {1}.";
pub(crate) const FR_COACH_USER_UPDATED: &str = "Coach sélectionné : {0}.";
pub(crate) const FR_COACH_ASSIGN_NOT_A_MEMBER: &str = "Tu n'es pas membre de ce groupe";
pub(crate) const FR_COACH_ASSIGN_FORBIDDEN: &str =
    "Seuls les admins et propriétaires du groupe peuvent changer le coach.";
pub(crate) const FR_COACH_ADD_USAGE: &str =
    "Indique quel coach ajouter : /coach add @handle. Tape /coach pour voir ta liste.";
pub(crate) const FR_COACH_ADD_UNKNOWN: &str =
    "Aucun coach installé ne répond à {0}. Tape /coach pour voir ta liste, ou /discover pour l'installer.";
pub(crate) const FR_COACH_REMOVE_GROUP_THREAD: &str =
    "Dans un groupe, le coach appartient au groupe — utilise /group coach pour le changer.";
pub(crate) const FR_COACH_REMOVE_NOTHING: &str = "Aucun coach n'est attaché à cette conversation.";
pub(crate) const FR_COACH_REMOVED: &str = "{0} ne répond plus dans cette conversation.";
pub(crate) const FR_COACH_CREATE_NO_CONVERSATION: &str =
    "Ouvre d'abord une conversation : /coach create rédige un coach à partir de ce qui s'y est dit.";
pub(crate) const FR_COACH_CREATE_EMPTY: &str =
    "Cette conversation est encore vide. Échange d'abord quelques messages avec ton coach, puis relance /coach create.";
pub(crate) const FR_COACH_CREATE_USAGE: &str =
    "Utilisation : /coach create pour proposer un coach à partir de cette conversation, puis /coach create confirm token pour le créer.";
pub(crate) const FR_COACH_CREATE_CARD_TITLE: &str = "Brouillon de coach";
pub(crate) const FR_COACH_CREATE_PROPOSAL_BODY: &str =
    "{0}\n{1}\n\nCatégorie : {2}\nTags : {3}\n\nRéponds /coach create confirm {4} pour le créer, ou /deny {4} pour l'abandonner. Le brouillon expire dans 10 minutes.";
pub(crate) const FR_COACH_CREATE_CONFIRM_LABEL: &str = "Créer";
pub(crate) const FR_COACH_CREATE_DISCARD_LABEL: &str = "Abandonner";
pub(crate) const FR_COACH_CREATE_QUOTA: &str =
    "Tu as déjà {0} coachs, le maximum de ton forfait ({1}). Supprime-en un depuis Discover avant d'en créer un autre.";
pub(crate) const FR_COACH_CREATE_DONE: &str =
    "Coach {0} créé — @{1}. Il répond ici à partir de ton prochain message. Tape /coach add @{1} dans n'importe quelle autre conversation, ou modifie-le depuis sa fiche Discover.";
pub(crate) const FR_COACH_CREATE_DONE_UNBOUND: &str =
    "Coach {0} créé — @{1}. Tape /coach add @{1} dans une de tes conversations pour l'utiliser, ou modifie-le depuis sa fiche Discover.";
pub(crate) const FR_COACH_CREATE_DISCARDED: &str =
    "Brouillon abandonné. Relance /coach create quand tu veux.";

// ── Compiled-in defaults: English ─────────────────────────────────────────

/// English default for [`KEY_ERROR_GENERIC`]. `{0}` = 8-char correlation id.
pub const EN_ERROR_GENERIC: &str = "Dravr is temporarily unavailable. The team has been notified — please try again in a few minutes. (ref: {0})";
/// English default for [`KEY_GUARDIAN_DENIED`]. No format placeholders.
pub const EN_GUARDIAN_DENIED: &str = "That action was blocked for safety. Try rephrasing your request, or retry without the earlier context.";

/// English default for [`KEY_GUARDIAN_CONFIRM_PROMPT`]. Args: `{0}` tool, `{1}` token.
pub const EN_GUARDIAN_CONFIRM_PROMPT: &str = "For safety, the action {0} needs your confirmation before it runs. Reply /confirm {1} to approve or /deny {1} to cancel (expires in 5 minutes).";

/// English default for [`KEY_GUARDIAN_CONFIRM_DONE`]. Args: `{0}` tool.
pub const EN_GUARDIAN_CONFIRM_DONE: &str = "Done — the action {0} has been executed.";

/// English default for [`KEY_GUARDIAN_CONFIRM_FAILED`]. Args: `{0}` tool.
pub const EN_GUARDIAN_CONFIRM_FAILED: &str =
    "The action {0} was confirmed but failed to execute. Rephrase your request to try again.";

/// English default for [`KEY_GUARDIAN_CONFIRM_DENIED`]. No format placeholders.
pub const EN_GUARDIAN_CONFIRM_DENIED: &str = "Understood — the action has been cancelled.";

/// English default for [`KEY_GUARDIAN_CONFIRM_EXPIRED`]. No format placeholders.
pub const EN_GUARDIAN_CONFIRM_EXPIRED: &str =
    "That confirmation has expired. Rephrase your request to run the action again.";

/// English default for [`KEY_GUARDIAN_CONFIRM_NOT_FOUND`]. No format placeholders.
pub const EN_GUARDIAN_CONFIRM_NOT_FOUND: &str =
    "No pending action matches that code. It may already have been resolved.";
/// English default for [`KEY_EMPTY_REPLY`].
pub const EN_EMPTY_REPLY: &str =
    "Hmm, I couldn't put a reply together. Can you rephrase your question?";
/// English default for [`KEY_REPLY_WITHHELD`].
pub const EN_REPLY_WITHHELD: &str = "My reply didn't go through — it mixed in technical details that don't belong here. Send your last message again and we'll pick up where we left off.";
/// English default for [`KEY_GUARDRAIL_TOO_LONG`].
pub const EN_GUARDRAIL_TOO_LONG: &str = "I have a longer response prepared but it exceeds the configured length cap. Want me to break it into a shorter summary?";
/// English default for [`KEY_GUARDRAIL_BLOCKED_TOPIC`].
pub const EN_GUARDRAIL_BLOCKED_TOPIC: &str = "I'd rather not get into that here. Let's stay focused on your training and recovery. Is there something specific I can help with?";
/// English default for [`KEY_VERIFICATION_WARN_SUFFIX`].
pub const EN_VERIFICATION_WARN_SUFFIX: &str =
    "⚠️ A few claims I couldn't formally back up — push back if any of these are off:";
/// English default for [`KEY_VERIFICATION_BLOCK_FALLBACK`].
pub const EN_VERIFICATION_BLOCK_FALLBACK: &str = "I started to answer, but a couple of the claims I was about to make didn't match the evidence I trust. Let me reword that — can you ask me again with a bit more context on what you're trying to figure out?";
/// English canonical refusal for off-scope requests.
pub const EN_SCOPE_REFUSAL: &str =
    "That's outside what I can help with — I'm your fitness assistant.";
/// English canonical refusal for missing-capability requests.
pub const EN_CAPABILITY_REFUSAL: &str = "I can't do that with the tools I have.";
/// English Nutrition-coach carve-out for the generic scope list.
pub const EN_COACH_SCOPE_CARVE_OUT_NUTRITION: &str = "## Clarification for your nutrition-coach role\n\nAs a nutrition coach, questions about meals, dinner, breakfast, snacks, food choices, and training-linked meal planning are fully within your scope. Answer these directly, grounded in the user's training data (intensity, duration, energy expenditure). The \"food/meal finders out-of-scope\" rule above targets restaurant search, delivery apps, and online menu lookups only — it does not cover evidence-based nutrition advice or meal planning built on the user's training. Never refuse \"what should I eat after my run\", \"dinner ideas after my workout\", \"post-workout snack\" or equivalent.";
/// English Recipes-coach carve-out for the generic scope list.
pub const EN_COACH_SCOPE_CARVE_OUT_RECIPES: &str = "## Clarification for your recipes and meal-planning coach role\n\nAs a recipes and meal-planning coach, recipe ideas, meal suggestions, dish ideas, and food choices tuned to training are fully within your scope. The \"food/meal finders out-of-scope\" rule above targets restaurant search and delivery apps — not homemade recipe suggestions or meal planning grounded in the user's training data. Answer these requests directly.";
/// English placeholder shown while the LLM is composing its reply.
pub const EN_THINKING_PLACEHOLDER: &str = "thinking…";
/// English unknown-command reply.
pub const EN_UNKNOWN_COMMAND: &str = "Unknown command. Type /help to see the available commands.";
/// English progress status during prompt-assembly.
pub const EN_STATUS_READING_QUESTION: &str = "reading your question…";
/// English progress status during LLM dispatch.
pub const EN_STATUS_GENERATING_RESPONSE: &str = "generating response…";
/// English progress status for tool-call start. `{0}` = tool name.
pub const EN_STATUS_CALLING_TOOL: &str = "calling {0}…";
/// English progress status for pipeline error. `{0}` = error text.
pub const EN_STATUS_ERROR: &str = "error: {0}";
/// English coach-proposal lead-in with profile. `{0}` = sport, `{1}` = count.
pub const EN_COACH_PROPOSAL_WELCOME: &str =
    "Welcome! Based on your recent {0} training, here are {1} coaches for you:\n\n";
/// English coach-proposal cold-start lead-in. `{0}` = count.
pub const EN_COACH_PROPOSAL_WELCOME_GENERIC: &str =
    "Welcome! Here are {0} coaches to get you started:\n\n";
/// English coach-proposal footer.
pub const EN_COACH_PROPOSAL_FOOTER: &str =
    "\nReply with a number to start, or just ask me anything.";
/// English default for [`KEY_REGISTRATION_APPROVED`].
pub const EN_REGISTRATION_APPROVED: &str =
    "🎉 Your Dravr account has been approved! You can now chat with your coach right here. Ask me your first question whenever you're ready.";
/// English default for [`KEY_BACKFILL_READY`]. `{0}` = activity count.
pub const EN_BACKFILL_READY: &str =
    "✅ Your history is ready — {0} activities loaded. Ask me again what you were looking for.";
/// English default for [`KEY_BACKFILL_LIST_HEADER`]. `{0}` = activity count.
pub const EN_BACKFILL_LIST_HEADER: &str = "✅ Your history is ready — {0} activities:";
/// English default for [`KEY_BACKFILL_LIST_MORE`]. `{0}` = remaining count.
pub const EN_BACKFILL_LIST_MORE: &str = "… and {0} more";

pub(crate) const EN_LINK_FALLBACK_PROMPT: &str = "To chat with Dravr, please link your account first. Visit the Dravr web app to connect this channel.";
pub(crate) const EN_LINK_INITIAL_PROMPT: &str =
    "Hi! To chat with Dravr, link your account first:\n{0}\n\nThis link expires in 10 minutes.";
pub(crate) const EN_LINK_EMAIL_PROMPT: &str =
    "Hi! To link your Dravr account, please type your email address.\nType \"cancel\" to stop.";
pub(crate) const EN_LINK_LOGOUT_COMPLETE: &str =
    "You've been logged out from Dravr. Send a message anytime to link your account again.";
pub(crate) const EN_LINK_CANCELLED: &str =
    "Linking cancelled. Send a message anytime to start again.";
pub(crate) const EN_LINK_GENERIC_ERROR: &str = "Something went wrong. Please try again later.";
pub(crate) const EN_LINK_SIGNUP_OFFER: &str = "No Dravr account for {0}. I can create one right now - reply \"yes\" to continue, or \"cancel\" to stop.";
pub(crate) const EN_LINK_SIGNUP_CREATED: &str =
    "Account created. I'm emailing you a code to confirm the address is yours.";
pub(crate) const EN_LINK_SIGNUP_FAILED: &str =
    "I couldn't create the account. Try again in a moment, or type \"cancel\" to stop.";
pub(crate) const EN_LINK_NO_TENANT: &str =
    "This account is not associated with any organization. Please contact support.";
pub(crate) const EN_LINK_EMAIL_NOT_CONFIGURED: &str =
    "Email delivery is not configured. Please contact your administrator.";
pub(crate) const EN_LINK_EMAIL_SEND_FAILED: &str =
    "Failed to send the verification email. Please try again later.";
pub(crate) const EN_LINK_INVALID_EMAIL: &str =
    "That doesn't look like an email address. Please type your Dravr account email.";
pub(crate) const EN_LINK_OTP_SENT: &str = "I've sent a 6-digit code to {0}. Please type it here within 10 minutes.\nType \"cancel\" to stop.";
pub(crate) const EN_LINK_TOO_MANY_ATTEMPTS: &str = "Too many incorrect attempts. The linking session has been cancelled. Send a message to start again.";
pub(crate) const EN_LINK_INCORRECT_CODE: &str = "Incorrect code. You have {0} attempt(s) remaining. Please try again or type \"cancel\" to stop.";
pub(crate) const EN_LINK_VERIFICATION_ERROR: &str =
    "Something went wrong verifying your account. Please try again.";
pub(crate) const EN_LINK_IDENTITY_COLLISION: &str =
    "Failed to link your account. This channel identity may already be linked.";
pub(crate) const EN_LINK_OTP_PROMPT: &str =
    "Please type the 6-digit code sent to your email, or type \"cancel\" to stop.";
pub(crate) const EN_LINK_SESSION_EXPIRED: &str =
    "Your linking session has expired. Send a message to start again.";
pub(crate) const EN_LINK_SUCCESS: &str = "Your account has been linked successfully! You can now chat with Dravr through this channel.\n\nType \"logout\" anytime to disconnect.";
pub(crate) const EN_ACCOUNT_PENDING: &str = "Your account is linked to this channel, but it's still waiting for admin approval. You'll be able to chat with Dravr as soon as it's activated — you'll get a heads-up here.";
pub(crate) const EN_ACCOUNT_SUSPENDED: &str =
    "Your Dravr account is suspended. Contact support to restore access.";
pub(crate) const EN_RATE_LIMITED: &str = "You're sending messages a little faster than your plan allows. Give it a moment and try again — I'll be here.";
pub(crate) const EN_QUOTA_EXCEEDED: &str = "You've reached your plan's chat limit for now. It resets automatically — check back a bit later.";
pub(crate) const EN_QUOTA_WARNING: &str =
    "Heads-up: you've used {0} of {1} on your plan. The counter resets on {2}.";
pub(crate) const EN_NO_PROVIDER_CONNECTED: &str = "Before we chat, connect a fitness service (Strava, Garmin, Whoop) from the Dravr app — without one I have no activity data to coach you on.\n\nConnect here:\n{0}";
pub(crate) const EN_NO_PROVIDER_CONNECTED_WITH_EMAIL: &str = "Before we chat, connect a fitness service (Strava, Garmin, Whoop) from the Dravr app — without one I have no activity data to coach you on.\n\nSign in with your {1} account here:\n{0}";
pub(crate) const EN_CONNECT_PROMPT: &str = "Let's connect your fitness service (Strava, Garmin or Whoop) so I can coach you on your real data. Tap the button below to connect securely.";
pub(crate) const EN_CONNECT_BUTTON: &str = "Connect your account";
pub(crate) const EN_CONNECT_TITLE: &str = "Connect a fitness service";

pub(crate) const EN_PROVIDER_REAUTH_REQUIRED: &str = "Your {0} connection has expired — I can't fetch your data right now. Reconnect here (link valid for 24 hours):\n\n{1}\n\nOnce reconnected, ask me again.";
pub(crate) const EN_PROVIDER_REAUTH_REQUIRED_NO_LINK: &str = "Your {0} connection has expired — I can't fetch your data right now. Reconnect {0} from your settings, then ask me again.";
pub(crate) const EN_PROVIDER_RECONNECT_BUTTON: &str = "Reconnect {0}";

pub(crate) const EN_STATUS_HEADER: &str = "Your Dravr status:\n";
pub(crate) const EN_STATUS_PROVIDERS_NONE: &str = "\nProviders: none connected";
pub(crate) const EN_STATUS_PROVIDERS_LABEL: &str = "\nProviders: {0}";
pub(crate) const EN_STATUS_GROUPS_LABEL: &str = "\nGroups: {0}";
pub(crate) const EN_STATUS_CHANNEL_LABEL: &str = "\nChannel: {0}";

pub(crate) const EN_HELP_HEADER: &str = "Available commands:\n";
pub(crate) const EN_HELP_DOMAIN_GENERAL: &str = "General";
pub(crate) const EN_HELP_DOMAIN_GROUP: &str = "Group coaching";
pub(crate) const EN_HELP_DOMAIN_COACH: &str = "Coaching";
pub(crate) const EN_HELP_DOMAIN_DATA: &str = "Fitness data";
pub(crate) const EN_HELP_DOMAIN_PROVIDER: &str = "Providers";
pub(crate) const EN_HELP_DOMAIN_ACCOUNT: &str = "Account";
pub(crate) const EN_HELP_DOMAIN_TRAINING: &str = "Training";
pub(crate) const EN_HELP_DOMAIN_DISCOVER: &str = "Coach catalogue";
pub(crate) const EN_HELP_FOOTER: &str = "\nOr just send a message to chat with your coach.";

pub(crate) const EN_LOGOUT_CONFIRM_PROMPT: &str = "This will unlink your {0} account from Dravr.\nYou will need to re-link to use messaging again.\n\nType \"logout\" to confirm.";

pub(crate) const EN_PRIVACY_STATUS_LINE: &str = "Analytics consent is currently <b>{0}</b>.\n\nUse <code>/privacy on</code> to enable or <code>/privacy off</code> to disable anonymous analytics.";
pub(crate) const EN_PRIVACY_STATUS_ENABLED: &str = "enabled";
pub(crate) const EN_PRIVACY_STATUS_DISABLED: &str = "disabled";
pub(crate) const EN_PRIVACY_ON_CONFIRMATION: &str = "Analytics consent has been <b>enabled</b>. Thank you for helping us improve Dravr!\n\nUse <code>/privacy off</code> to opt out at any time.";
pub(crate) const EN_PRIVACY_OFF_CONFIRMATION: &str = "Analytics consent has been <b>disabled</b>. No anonymous usage data will be collected.\n\nUse <code>/privacy on</code> to opt back in at any time.";
pub(crate) const EN_TIMEZONE_SET: &str =
    "Timezone set to <b>{0}</b>. Your activity start times will now display in this timezone.";
pub(crate) const EN_TIMEZONE_INVALID: &str =
    "Invalid timezone. Provide an IANA name, e.g. <code>/timezone America/Toronto</code>.";

pub(crate) const EN_PILLARS_OPENER: &str = "Let's build your profile together — I'll ask about a few areas, one at a time. To start: what's the deeper reason you train — your North Star?";
pub(crate) const EN_PILLARS_DM_ONLY: &str = "Profile building happens in private. Send me <code>/pillars</code> in a direct message and we'll start.";

pub(crate) const EN_INTAKE_OPENER: &str = "Before we start, a couple of quick questions — a minute now, and the coaching that follows is sharper and safer.";
pub(crate) const EN_INTAKE_PERSONA: &str = "First: do you train for yourself, or do you coach other people?\n\n<b>1</b> — I train for myself\n<b>2</b> — I coach others";
pub(crate) const EN_INTAKE_PARQ_INTRO: &str = "Now seven standard health questions (the PAR-Q+). A \"yes\" never blocks anything — it just tells your coach where to be careful.";
pub(crate) const EN_INTAKE_PARQ_HEART_CONDITION: &str =
    "Has a doctor ever said that you have a heart condition?";
pub(crate) const EN_INTAKE_PARQ_CHEST_PAIN: &str =
    "Do you feel pain in your chest at rest, during daily activities, or during exercise?";
pub(crate) const EN_INTAKE_PARQ_DIZZINESS: &str =
    "Do you lose balance from dizziness, or have you lost consciousness in the last 12 months?";
pub(crate) const EN_INTAKE_PARQ_CHRONIC_CONDITION: &str =
    "Have you been diagnosed with another chronic medical condition (other than heart disease)?";
pub(crate) const EN_INTAKE_PARQ_MEDICATION: &str =
    "Are you currently taking prescribed medications for a chronic medical condition?";
pub(crate) const EN_INTAKE_PARQ_JOINT_PROBLEM: &str =
    "Do you have a bone, joint, or soft-tissue problem that could be made worse by activity?";
pub(crate) const EN_INTAKE_PARQ_SUPERVISED_ONLY: &str =
    "Has a doctor said you should only do medically supervised physical activity?";
pub(crate) const EN_INTAKE_YESNO_HINT: &str = "Reply <b>1</b> for yes, <b>2</b> for no.";
pub(crate) const EN_INTAKE_RETRY: &str =
    "Sorry — I need just the number, so I record this exactly right.\n\n{0}";
pub(crate) const EN_INTAKE_COMPLETE_CLEAR: &str =
    "That's everything, nothing to flag. Ask me anything about your training.";
pub(crate) const EN_INTAKE_COMPLETE_FLAGGED: &str = "That's everything. I've noted {0} thing(s) for your coach to keep in mind — none of it stops you training, and it's worth a word with your doctor before you push hard.";
pub(crate) const EN_PILLARS_START_FAILED: &str =
    "I couldn't start the profile walk on this conversation. Try again in a moment.";
pub(crate) const EN_RESET_WALK_INTERRUPTED: &str =
    "\n\nYour profile walk was in progress — send <code>/pillars</code> to resume.";

pub(crate) const EN_CALIBRATE_OPENER: &str = "Let's calibrate how hard your training should be. I'll start from your recent data and ask six short questions, one at a time. First: how do you want it to get harder — more hours, more hard days, or longer long sessions?";
pub(crate) const EN_CALIBRATE_DM_ONLY: &str = "Calibration happens in private. Send me <code>/calibrate</code> in a direct message and we'll start.";
pub(crate) const EN_CALIBRATE_START_FAILED: &str =
    "I couldn't start calibration on this conversation. Try again in a moment.";
pub(crate) const EN_CALIBRATE_COMPLETE_HEADER: &str =
    "Calibration done — I captured {0} of {1} answers.";
pub(crate) const EN_CALIBRATE_COMPLETE_MISSING: &str = "I'm missing your answer on {0}. That's what bounds how hard I can push, so let's redo that one — send <code>/calibrate</code>.";
pub(crate) const EN_CALIBRATE_FOLLOWUP_PLAN: &str =
    "Want me to rebuild your upcoming weeks with this?";
pub(crate) const EN_CALIBRATE_FOLLOWUP_NO_PLAN: &str = "Want me to build a plan around this?";
pub(crate) const EN_CALIBRATE_TOPIC_INJURY: &str = "your recent injuries and niggles";
pub(crate) const EN_CALIBRATE_TOPIC_RECOVERY: &str = "how fast you recover";

pub(crate) const EN_PLAN_GOAL_LINE: &str = "Goal: {0} — {1} ({2} days out)";
pub(crate) const EN_PLAN_BLOCK_LINE: &str = "Block: {0}{1}";
pub(crate) const EN_PLAN_DAY_LINE: &str = "{0}: {1}";
pub(crate) const EN_PLAN_REST: &str = "rest";
pub(crate) const EN_PLAN_TODAY: &str = "Today";
pub(crate) const EN_PLAN_TOMORROW: &str = "Tomorrow";
pub(crate) const EN_PLAN_WEEK_HEADER: &str = "Week of {0}{1}";
pub(crate) const EN_PLAN_NO_SESSION: &str = "nothing scheduled";
pub(crate) const EN_PLAN_NO_COVERAGE: &str = "not covered by the plan";
pub(crate) const EN_PLAN_RESUMES: &str = "The plan resumes on {0}.";
pub(crate) const EN_PLAN_EMPTY: &str =
    "No plan saved yet — ask your coach to build one toward your goal.";
pub(crate) const EN_PLAN_STALE_GOAL: &str =
    "\n\n⚠️ Your goal has changed since this plan — ask your coach to refresh it.";

pub(crate) const EN_GROUP_LIST_EMPTY: &str =
    "You are not a member of any groups.\nCreate or join a group via the web or mobile app.";
pub(crate) const EN_GROUP_LIST_HEADER: &str = "Your groups ({0}):\n";
pub(crate) const EN_GROUP_LIST_ITEM: &str = "- {0} ({1} members) [{2}]";
pub(crate) const EN_GROUP_NOT_A_MEMBER: &str = "You are not a member of any group";
pub(crate) const EN_GROUP_STATUS_SUMMARY: &str =
    "{0} stats:\n- Members: {1}\n- Active: {2}\n- Peer sharing: {3}";
pub(crate) const EN_GROUP_PEER_SHARING_ON: &str = "on";
pub(crate) const EN_GROUP_PEER_SHARING_OFF: &str = "off";
pub(crate) const EN_GROUP_MEMBERS_HEADER: &str = "{0} members ({1}):\n";
pub(crate) const EN_GROUP_MEMBERS_UNKNOWN: &str = "Unknown";
pub(crate) const EN_GROUP_MEMBERS_ITEM: &str = "- {0} [{1}]";
pub(crate) const EN_GROUP_ROLE_OWNER: &str = "owner";
pub(crate) const EN_GROUP_ROLE_ADMIN: &str = "admin";
pub(crate) const EN_GROUP_ROLE_MEMBER: &str = "member";
pub(crate) const EN_GROUP_INVITE_FORBIDDEN: &str =
    "Only admins and owners can generate invite links.";
pub(crate) const EN_GROUP_INVITE_BODY: &str =
    "Invite link for {0}:\nhttps://app.dravr.ai/groups/join/{1}\n\nCode: {2}\nValid for 7 days.";
pub(crate) const EN_GROUP_INVITE_UNAVAILABLE: &str = "Group invites are not available.";
pub(crate) const EN_COACH_INVITE_BODY: &str =
    "Coach invite for {0} — whoever redeems it becomes the group's human coach:\nhttps://app.dravr.ai/groups/join/{1}\n\nCode: {2}\nValid for 7 days.";
pub(crate) const EN_GROUP_LEAVE_PROMPT: &str =
    "Are you sure you want to leave {0}?\nType \"YES\" to confirm.";
pub(crate) const EN_GROUP_CONSENT_USAGE: &str = "Usage: /group consent yes  or  /group consent no";
pub(crate) const EN_GROUP_RESPOND_USAGE: &str =
    "Usage: /group respond mentions  or  /group respond all";
pub(crate) const EN_GROUP_RESPOND_MENTIONS: &str = "The coach now replies only when addressed (@-mention it or reply to one of its messages). It keeps following the discussion for context.";
pub(crate) const EN_GROUP_RESPOND_ALL: &str =
    "The coach now replies to every message in the group again.";
pub(crate) const EN_GROUP_COACH_DETACHED: &str = "{0} no longer has an attached human coach.";
pub(crate) const EN_NOTIFICATION_CHANNEL_BODY: &str = "🔔 {0}\n\n{1}";
pub(crate) const EN_GROUP_RESPOND_STATUS_MENTIONS: &str =
    "The coach replies only when addressed. To go back to every message: /group respond all";
pub(crate) const EN_GROUP_CONSENT_UPDATED: &str =
    "Sharing your data with the other members of {1} is now {0}.";
pub(crate) const EN_GROUP_CREATE_USAGE: &str =
    "Usage: /group create group-name — for example /group create Sunday Runners";
pub(crate) const EN_GROUP_CREATE_NO_COACH: &str =
    "Pick the group's coach first: /coach add @handle in this chat, then run /group create again.";
pub(crate) const EN_GROUP_CREATE_UNAVAILABLE: &str = "Group coaching is not included in your plan.";
pub(crate) const EN_GROUP_CREATE_FORBIDDEN: &str =
    "Group creation is reserved to your workspace's admins.";
pub(crate) const EN_GROUP_CREATED: &str =
    "Group \"{0}\" created with coach {1}. Invite members with /group invite.";
pub(crate) const EN_GROUP_INVITE_LABEL: &str = "Invite members";
pub(crate) const EN_GROUP_JOIN_INVALID_CODE: &str =
    "That invite code is not valid or has expired. Ask a group admin for a fresh link (/group invite).";
pub(crate) const EN_GROUP_JOIN_ALREADY_MEMBER: &str = "You are already a member of {0}.";
pub(crate) const EN_GROUP_JOIN_FULL: &str = "{0} is full.";
pub(crate) const EN_GROUP_JOINED: &str =
    "Welcome to {0}! The group chat is now in your conversation list.";
pub(crate) const EN_GROUP_JOINED_AS_COACH: &str = "You are now the human coach of {0}.";
pub(crate) const EN_DISCOVER_CARD_TITLE: &str = "Coach catalogue";
pub(crate) const EN_DISCOVER_ITEM: &str = "• {0} — @{1} [{2}]\n  {3}\n";
pub(crate) const EN_DISCOVER_EMPTY: &str =
    "No catalogue coach matches \"{0}\". Type /discover to see the newest ones.";
pub(crate) const EN_DISCOVER_CATALOGUE_EMPTY: &str = "The coach catalogue is empty.";
pub(crate) const EN_DISCOVER_MORE_LABEL: &str = "More";
pub(crate) const EN_DISCOVER_INSTALL_USAGE: &str =
    "Usage: /discover install @handle — the @handle is shown in /discover.";
pub(crate) const EN_DISCOVER_INSTALL_UNKNOWN_HANDLE: &str =
    "No published coach answers to {0}. Type /discover to browse the catalogue.";
pub(crate) const EN_DISCOVER_INSTALLED: &str =
    "{0} is installed. Use it in any chat: /coach add @{1}, or mention @{1} in a message for one turn.";
pub(crate) const EN_DISCOVER_INSTALL_ALREADY: &str =
    "{0} is already installed. Use it in any chat: /coach add @{1}, or mention @{1} in a message for one turn.";
pub(crate) const EN_DISCOVER_ADD_LABEL: &str = "Use in this chat";

pub(crate) const EN_COACH_LIST_EMPTY: &str =
    "No coach on your list yet. Type /discover to browse the catalogue, or /coach create to draft one from this conversation.";
pub(crate) const EN_COACH_LIST_CARD_TITLE: &str = "Your coaches";
pub(crate) const EN_COACH_LIST_ITEM: &str = "• {0} — @{1}\n  {2}\n";
pub(crate) const EN_COACH_LIST_ITEM_NO_HANDLE: &str = "• {0}\n  {1}\n";
pub(crate) const EN_COACH_LIST_FOOTER: &str =
    "Mention @handle in a message to hand it that one message, or type /coach add @handle so it answers here from now on.";
pub(crate) const EN_COACH_NO_DESCRIPTION: &str = "No description";
pub(crate) const EN_COACH_GROUP_UPDATED: &str = "Coach updated to {0} for group {1}.";
pub(crate) const EN_COACH_USER_UPDATED: &str = "Coach selected: {0}.";
pub(crate) const EN_COACH_ASSIGN_NOT_A_MEMBER: &str = "You are not a member of this group";
pub(crate) const EN_COACH_ASSIGN_FORBIDDEN: &str =
    "Only group admins and owners can change the coach.";
pub(crate) const EN_COACH_ADD_USAGE: &str =
    "Say which coach to add: /coach add @handle. Type /coach to see your list.";
pub(crate) const EN_COACH_ADD_UNKNOWN: &str =
    "No installed coach answers to {0}. Type /coach to see your list, or /discover to install one.";
pub(crate) const EN_COACH_REMOVE_GROUP_THREAD: &str =
    "In a group chat the coach belongs to the group — use /group coach to change it.";
pub(crate) const EN_COACH_REMOVE_NOTHING: &str = "No coach is attached to this conversation.";
pub(crate) const EN_COACH_REMOVED: &str = "{0} no longer answers in this conversation.";
pub(crate) const EN_COACH_CREATE_NO_CONVERSATION: &str =
    "Open a conversation first: /coach create drafts a coach from what was said in it.";
pub(crate) const EN_COACH_CREATE_EMPTY: &str =
    "This conversation is still empty. Exchange a few messages with your coach first, then run /coach create again.";
pub(crate) const EN_COACH_CREATE_USAGE: &str =
    "Usage: /coach create to draft a coach from this conversation, then /coach create confirm token to create it.";
pub(crate) const EN_COACH_CREATE_CARD_TITLE: &str = "Coach draft";
pub(crate) const EN_COACH_CREATE_PROPOSAL_BODY: &str =
    "{0}\n{1}\n\nCategory: {2}\nTags: {3}\n\nReply /coach create confirm {4} to create it, or /deny {4} to discard it. The draft expires in 10 minutes.";
pub(crate) const EN_COACH_CREATE_CONFIRM_LABEL: &str = "Create it";
pub(crate) const EN_COACH_CREATE_DISCARD_LABEL: &str = "Discard";
pub(crate) const EN_COACH_CREATE_QUOTA: &str =
    "You already have {0} coaches, the maximum for your plan ({1}). Delete one from Discover before creating another.";
pub(crate) const EN_COACH_CREATE_DONE: &str =
    "Coach {0} created — @{1}. It answers here from your next message on. Type /coach add @{1} in any other conversation, or edit it from its Discover page.";
pub(crate) const EN_COACH_CREATE_DONE_UNBOUND: &str =
    "Coach {0} created — @{1}. Type /coach add @{1} in one of your conversations to use it, or edit it from its Discover page.";
pub(crate) const EN_COACH_CREATE_DISCARDED: &str =
    "Draft discarded. Run /coach create again whenever you like.";

// ── Compiled-in defaults: Spanish ─────────────────────────────────────────

pub(crate) const ES_ERROR_GENERIC: &str = "Dravr no está disponible temporalmente. El equipo ha sido notificado — inténtalo de nuevo en unos minutos. (ref: {0})";
pub(crate) const ES_GUARDIAN_DENIED: &str = "Esa acción fue bloqueada por seguridad. Reformula tu solicitud o inténtalo de nuevo sin el contexto anterior.";
pub(crate) const ES_GUARDIAN_CONFIRM_PROMPT: &str = "Por seguridad, la acción {0} necesita tu confirmación antes de ejecutarse. Responde /confirm {1} para aprobarla o /deny {1} para cancelarla (caduca en 5 minutos).";
pub(crate) const ES_GUARDIAN_CONFIRM_DONE: &str = "Hecho — la acción {0} se ha ejecutado.";
pub(crate) const ES_GUARDIAN_CONFIRM_FAILED: &str = "La acción {0} fue confirmada pero su ejecución falló. Reformula tu solicitud para intentarlo de nuevo.";
pub(crate) const ES_GUARDIAN_CONFIRM_DENIED: &str = "Entendido — la acción ha sido cancelada.";
pub(crate) const ES_GUARDIAN_CONFIRM_EXPIRED: &str =
    "Esa confirmación ha caducado. Reformula tu solicitud para relanzar la acción.";
pub(crate) const ES_GUARDIAN_CONFIRM_NOT_FOUND: &str =
    "Ninguna acción pendiente coincide con ese código. Puede que ya se haya resuelto.";
pub(crate) const ES_EMPTY_REPLY: &str =
    "Hmm, no pude armar una respuesta. ¿Puedes reformular tu pregunta?";
pub(crate) const ES_REPLY_WITHHELD: &str = "Mi respuesta no salió — mezclaba detalles técnicos que no tienen lugar aquí. Envíame de nuevo tu último mensaje y retomamos donde estábamos.";
pub(crate) const ES_GUARDRAIL_TOO_LONG: &str = "Tengo una respuesta más larga lista, pero supera el límite configurado. ¿Quieres que te la resuma más brevemente?";
pub(crate) const ES_GUARDRAIL_BLOCKED_TOPIC: &str = "Prefiero no tratar ese tema aquí. Concentrémonos en tu entrenamiento y recuperación. ¿Hay algo concreto en lo que pueda ayudarte?";
pub(crate) const ES_VERIFICATION_WARN_SUFFIX: &str =
    "⚠️ Algunas afirmaciones que no pude respaldar formalmente — corrígeme si alguna está fuera:";
pub(crate) const ES_VERIFICATION_BLOCK_FALLBACK: &str = "Empecé a responder, pero algunas afirmaciones no coincidían con las fuentes que considero fiables. Déjame reformular — ¿puedes preguntarme de nuevo con un poco más de contexto sobre lo que intentas entender?";
/// Spanish canonical refusal for off-scope requests.
pub(crate) const ES_SCOPE_REFUSAL: &str =
    "Eso está fuera de lo que puedo hacer — soy tu asistente de fitness.";
/// Spanish canonical refusal for missing-capability requests.
pub(crate) const ES_CAPABILITY_REFUSAL: &str = "No puedo hacerlo con las herramientas que tengo.";
/// Spanish Nutrition-coach carve-out for the generic scope list.
pub(crate) const ES_COACH_SCOPE_CARVE_OUT_NUTRITION: &str = "## Aclaración para tu rol de coach de nutrición\n\nComo coach de nutrición, las preguntas sobre comidas, cenas, desayunos, snacks, elecciones alimentarias y planificación de comidas vinculadas al entrenamiento están plenamente dentro de tu ámbito. Responde directamente, apoyándote en los datos de entrenamiento del usuario (intensidad, duración, gasto energético). La regla «búsquedas de comida fuera de alcance» de más arriba apunta solo a la búsqueda de restaurantes, apps de entrega y menús en línea — no cubre los consejos nutricionales basados en evidencia ni la planificación de comidas fundada en los datos de entrenamiento. Nunca rechaces «qué comer después de correr», «ideas para la cena tras entrenar», «snack post-entrenamiento» o equivalentes.";
/// Spanish Recipes-coach carve-out for the generic scope list.
pub(crate) const ES_COACH_SCOPE_CARVE_OUT_RECIPES: &str = "## Aclaración para tu rol de coach de recetas y planificación de comidas\n\nComo coach de recetas y planificación de comidas, las ideas de recetas, sugerencias de platos y elecciones alimentarias ajustadas al entrenamiento están plenamente dentro de tu ámbito. La regla «búsquedas de comida fuera de alcance» de más arriba apunta a restaurantes y apps de entrega — no a recetas caseras ni a la planificación de comidas basada en los datos de entrenamiento del usuario. Responde directamente a estas peticiones.";
/// Spanish placeholder shown while the LLM is composing its reply.
pub(crate) const ES_THINKING_PLACEHOLDER: &str = "pensando…";
/// Spanish unknown-command reply.
pub(crate) const ES_UNKNOWN_COMMAND: &str =
    "Comando desconocido. Escribe /help para ver los comandos disponibles.";
/// Spanish progress status during prompt-assembly.
pub(crate) const ES_STATUS_READING_QUESTION: &str = "leyendo tu pregunta…";
/// Spanish progress status during LLM dispatch.
pub(crate) const ES_STATUS_GENERATING_RESPONSE: &str = "generando respuesta…";
/// Spanish progress status for tool-call start. `{0}` = tool name.
pub(crate) const ES_STATUS_CALLING_TOOL: &str = "llamando a {0}…";
/// Spanish progress status for pipeline error. `{0}` = error text.
pub(crate) const ES_STATUS_ERROR: &str = "error: {0}";
/// Spanish coach-proposal lead-in with profile. `{0}` = sport, `{1}` = count.
pub(crate) const ES_COACH_PROPOSAL_WELCOME: &str =
    "¡Bienvenido! Según tu entrenamiento reciente de {0}, aquí tienes {1} entrenadores para ti:\n\n";
/// Spanish coach-proposal cold-start lead-in. `{0}` = count.
pub(crate) const ES_COACH_PROPOSAL_WELCOME_GENERIC: &str =
    "¡Bienvenido! Aquí tienes {0} entrenadores para empezar:\n\n";
/// Spanish coach-proposal footer.
pub(crate) const ES_COACH_PROPOSAL_FOOTER: &str =
    "\nResponde con un número para empezar, o simplemente pregúntame lo que quieras.";
/// Spanish default for [`KEY_REGISTRATION_APPROVED`].
pub(crate) const ES_REGISTRATION_APPROVED: &str =
    "🎉 ¡Tu cuenta de Dravr ha sido aprobada! Ya puedes hablar con tu coach aquí. Hazme tu primera pregunta cuando quieras.";
/// Spanish default for [`KEY_BACKFILL_READY`]. `{0}` = activity count.
pub(crate) const ES_BACKFILL_READY: &str =
    "✅ Tu historial está listo — {0} actividades cargadas. Vuelve a preguntarme lo que buscabas.";
/// Spanish default for [`KEY_BACKFILL_LIST_HEADER`]. `{0}` = activity count.
pub(crate) const ES_BACKFILL_LIST_HEADER: &str = "✅ Tu historial está listo — {0} actividades:";
/// Spanish default for [`KEY_BACKFILL_LIST_MORE`]. `{0}` = remaining count.
pub(crate) const ES_BACKFILL_LIST_MORE: &str = "… y {0} más";

pub(crate) const ES_LINK_FALLBACK_PROMPT: &str = "Para hablar con Dravr, primero vincula tu cuenta. Abre la app web de Dravr para conectar este canal.";
pub(crate) const ES_LINK_INITIAL_PROMPT: &str = "¡Hola! Para hablar con Dravr, vincula primero tu cuenta:\n{0}\n\nEste enlace expira en 10 minutos.";
pub(crate) const ES_LINK_EMAIL_PROMPT: &str = "¡Hola! Para vincular tu cuenta de Dravr, escribe tu correo electrónico.\nEscribe «cancel» para detener.";
pub(crate) const ES_LINK_LOGOUT_COMPLETE: &str = "Has cerrado sesión en Dravr. Envía un mensaje cuando quieras para volver a vincular tu cuenta.";
pub(crate) const ES_LINK_CANCELLED: &str =
    "Vinculación cancelada. Envía un mensaje cuando quieras para empezar de nuevo.";
pub(crate) const ES_LINK_GENERIC_ERROR: &str = "Algo salió mal. Inténtalo de nuevo más tarde.";
pub(crate) const ES_LINK_SIGNUP_OFFER: &str = "No hay ninguna cuenta de Dravr con {0}. Puedo crearte una ahora mismo: responde «si» para continuar o «cancel» para detener.";
pub(crate) const ES_LINK_SIGNUP_CREATED: &str =
    "Cuenta creada. Te envio un codigo por correo para confirmar que la direccion es tuya.";
pub(crate) const ES_LINK_SIGNUP_FAILED: &str =
    "No pude crear la cuenta. Intentalo de nuevo en un momento o escribe «cancel» para detener.";
pub(crate) const ES_LINK_NO_TENANT: &str =
    "Esta cuenta no está asociada a ninguna organización. Contacta con el soporte.";
pub(crate) const ES_LINK_EMAIL_NOT_CONFIGURED: &str =
    "El envío de correos no está configurado. Contacta con tu administrador.";
pub(crate) const ES_LINK_EMAIL_SEND_FAILED: &str =
    "No se pudo enviar el correo de verificación. Inténtalo de nuevo más tarde.";
pub(crate) const ES_LINK_INVALID_EMAIL: &str =
    "Eso no parece una dirección de correo. Escribe el correo de tu cuenta Dravr.";
pub(crate) const ES_LINK_OTP_SENT: &str = "He enviado un código de 6 cifras a {0}. Escríbelo aquí en los próximos 10 minutos.\nEscribe «cancel» para detener.";
pub(crate) const ES_LINK_TOO_MANY_ATTEMPTS: &str = "Demasiados intentos incorrectos. La sesión de vinculación se ha cancelado. Envía un mensaje para empezar de nuevo.";
pub(crate) const ES_LINK_INCORRECT_CODE: &str = "Código incorrecto. Te quedan {0} intento(s). Inténtalo de nuevo o escribe «cancel» para detener.";
pub(crate) const ES_LINK_VERIFICATION_ERROR: &str =
    "Ocurrió un error al verificar tu cuenta. Inténtalo de nuevo.";
pub(crate) const ES_LINK_IDENTITY_COLLISION: &str = "No se pudo vincular tu cuenta. Puede que esta identidad de canal ya esté asociada a otra cuenta.";
pub(crate) const ES_LINK_OTP_PROMPT: &str =
    "Escribe el código de 6 cifras enviado a tu correo, o escribe «cancel» para detener.";
pub(crate) const ES_LINK_SESSION_EXPIRED: &str =
    "Tu sesión de vinculación ha expirado. Envía un mensaje para empezar de nuevo.";
pub(crate) const ES_LINK_SUCCESS: &str = "¡Tu cuenta se ha vinculado correctamente! Ya puedes hablar con Dravr desde este canal.\n\nEscribe «logout» en cualquier momento para desconectar.";
pub(crate) const ES_ACCOUNT_PENDING: &str = "Tu cuenta está vinculada a este canal, pero aún espera la aprobación de un administrador. Podrás hablar con Dravr en cuanto se active — te avisaremos por aquí.";
pub(crate) const ES_ACCOUNT_SUSPENDED: &str =
    "Tu cuenta de Dravr está suspendida. Contacta con soporte para recuperar el acceso.";
pub(crate) const ES_RATE_LIMITED: &str = "Estás enviando mensajes un poco más rápido de lo que permite tu plan. Espera un momento y vuelve a intentarlo — aquí estaré.";
pub(crate) const ES_QUOTA_EXCEEDED: &str = "Has alcanzado el límite de conversación de tu plan por ahora. Se reinicia automáticamente — vuelve un poco más tarde.";
pub(crate) const ES_QUOTA_WARNING: &str =
    "Aviso: has usado {0} de {1} de tu plan. El contador se reinicia el {2}.";
pub(crate) const ES_NO_PROVIDER_CONNECTED: &str = "Antes de chatear, conecta un servicio de fitness (Strava, Garmin, Whoop) desde la app Dravr — sin él no tengo datos de actividad para orientarte.\n\nConéctate aquí:\n{0}";
pub(crate) const ES_NO_PROVIDER_CONNECTED_WITH_EMAIL: &str = "Antes de chatear, conecta un servicio de fitness (Strava, Garmin, Whoop) desde la app Dravr — sin él no tengo datos de actividad para orientarte.\n\nInicia sesión con tu cuenta {1} aquí:\n{0}";
pub(crate) const ES_CONNECT_PROMPT: &str = "Conectemos tu servicio de fitness (Strava, Garmin o Whoop) para que pueda orientarte con tus datos reales. Toca el botón de abajo para conectarte de forma segura.";
pub(crate) const ES_CONNECT_BUTTON: &str = "Conectar mi cuenta";
pub(crate) const ES_CONNECT_TITLE: &str = "Conectar un servicio de fitness";

pub(crate) const ES_PROVIDER_REAUTH_REQUIRED: &str = "Tu conexión con {0} ha expirado — no puedo recuperar tus datos en este momento. Vuelve a conectar tu cuenta aquí (enlace válido durante 24 horas):\n\n{1}\n\nUna vez reconectado, vuelve a preguntarme.";
pub(crate) const ES_PROVIDER_REAUTH_REQUIRED_NO_LINK: &str = "Tu conexión con {0} ha expirado — no puedo recuperar tus datos en este momento. Vuelve a conectar {0} desde los ajustes y pregúntame de nuevo.";
pub(crate) const ES_PROVIDER_RECONNECT_BUTTON: &str = "Volver a conectar {0}";

pub(crate) const ES_STATUS_HEADER: &str = "Tu estado en Dravr:\n";
pub(crate) const ES_STATUS_PROVIDERS_NONE: &str = "\nProveedores: ninguno conectado";
pub(crate) const ES_STATUS_PROVIDERS_LABEL: &str = "\nProveedores: {0}";
pub(crate) const ES_STATUS_GROUPS_LABEL: &str = "\nGrupos: {0}";
pub(crate) const ES_STATUS_CHANNEL_LABEL: &str = "\nCanal: {0}";

pub(crate) const ES_HELP_HEADER: &str = "Comandos disponibles:\n";
pub(crate) const ES_HELP_DOMAIN_GENERAL: &str = "General";
pub(crate) const ES_HELP_DOMAIN_GROUP: &str = "Coaching en grupo";
pub(crate) const ES_HELP_DOMAIN_COACH: &str = "Coaching";
pub(crate) const ES_HELP_DOMAIN_DATA: &str = "Datos de actividad";
pub(crate) const ES_HELP_DOMAIN_PROVIDER: &str = "Proveedores";
pub(crate) const ES_HELP_DOMAIN_ACCOUNT: &str = "Cuenta";
pub(crate) const ES_HELP_DOMAIN_TRAINING: &str = "Entrenamiento";
pub(crate) const ES_HELP_DOMAIN_DISCOVER: &str = "Catálogo de coaches";
pub(crate) const ES_HELP_FOOTER: &str = "\nO simplemente escríbeme para conversar con tu coach.";

pub(crate) const ES_LOGOUT_CONFIRM_PROMPT: &str = "Esto desvinculará tu cuenta de {0} de Dravr.\nTendrás que volver a vincularla para usar la mensajería.\n\nEscribe «logout» para confirmar.";

pub(crate) const ES_PRIVACY_STATUS_LINE: &str = "El consentimiento de analíticas está actualmente <b>{0}</b>.\n\nUsa <code>/privacy on</code> para activarlo o <code>/privacy off</code> para desactivarlo.";
pub(crate) const ES_PRIVACY_STATUS_ENABLED: &str = "activado";
pub(crate) const ES_PRIVACY_STATUS_DISABLED: &str = "desactivado";
pub(crate) const ES_PRIVACY_ON_CONFIRMATION: &str = "El consentimiento de analíticas está ahora <b>activado</b>. ¡Gracias por ayudarnos a mejorar Dravr!\n\nUsa <code>/privacy off</code> para darte de baja en cualquier momento.";
pub(crate) const ES_PRIVACY_OFF_CONFIRMATION: &str = "El consentimiento de analíticas está ahora <b>desactivado</b>. No se recogerán datos de uso anónimos.\n\nUsa <code>/privacy on</code> para volver a activarlo cuando quieras.";
pub(crate) const ES_TIMEZONE_SET: &str = "Zona horaria establecida en <b>{0}</b>. Las horas de inicio de tus actividades se mostrarán ahora en esta zona horaria.";
pub(crate) const ES_TIMEZONE_INVALID: &str = "Zona horaria no válida. Indica un nombre IANA, por ejemplo <code>/timezone America/Toronto</code>.";

pub(crate) const ES_PILLARS_OPENER: &str = "Vamos a construir tu perfil juntos — te haré preguntas sobre algunos temas, uno a uno. Para empezar: ¿cuál es la razón profunda por la que entrenas, tu «North Star»?";
pub(crate) const ES_PILLARS_DM_ONLY: &str = "El perfil se construye en privado. Escríbeme <code>/pillars</code> por mensaje directo y empezamos.";

pub(crate) const ES_INTAKE_OPENER: &str = "Antes de empezar, unas preguntas rápidas: un minuto ahora y el acompañamiento que viene será más preciso y más seguro.";
pub(crate) const ES_INTAKE_PERSONA: &str = "Primero: ¿entrenas para ti o entrenas a otras personas?\n\n<b>1</b> — Entreno para mí\n<b>2</b> — Entreno a otras personas";
pub(crate) const ES_INTAKE_PARQ_INTRO: &str = "Ahora siete preguntas de salud estándar (el PAR-Q+). Un «sí» no bloquea nada: solo le indica a tu coach dónde ser prudente.";
pub(crate) const ES_INTAKE_PARQ_HEART_CONDITION: &str =
    "¿Alguna vez un médico te ha dicho que tienes un problema cardíaco?";
pub(crate) const ES_INTAKE_PARQ_CHEST_PAIN: &str =
    "¿Sientes dolor en el pecho en reposo, en tus actividades diarias o al hacer ejercicio?";
pub(crate) const ES_INTAKE_PARQ_DIZZINESS: &str =
    "¿Pierdes el equilibrio por mareos, o has perdido el conocimiento en los últimos 12 meses?";
pub(crate) const ES_INTAKE_PARQ_CHRONIC_CONDITION: &str =
    "¿Te han diagnosticado otra enfermedad crónica (distinta de una enfermedad cardíaca)?";
pub(crate) const ES_INTAKE_PARQ_MEDICATION: &str =
    "¿Tomas actualmente medicamentos recetados para una enfermedad crónica?";
pub(crate) const ES_INTAKE_PARQ_JOINT_PROBLEM: &str = "¿Tienes algún problema óseo, articular o de tejidos blandos que la actividad física pueda empeorar?";
pub(crate) const ES_INTAKE_PARQ_SUPERVISED_ONLY: &str =
    "¿Un médico te ha dicho que solo deberías hacer actividad física bajo supervisión médica?";
pub(crate) const ES_INTAKE_YESNO_HINT: &str = "Responde <b>1</b> para sí, <b>2</b> para no.";
pub(crate) const ES_INTAKE_RETRY: &str =
    "Perdona: necesito solo el número, para registrarlo con exactitud.\n\n{0}";
pub(crate) const ES_INTAKE_COMPLETE_CLEAR: &str =
    "Eso es todo, nada que señalar. Pregúntame lo que quieras sobre tu entrenamiento.";
pub(crate) const ES_INTAKE_COMPLETE_FLAGGED: &str = "Eso es todo. He anotado {0} punto(s) para que tu coach los tenga en cuenta: nada te impide entrenar, y conviene comentarlo con tu médico antes de exigirte a fondo.";
pub(crate) const ES_PILLARS_START_FAILED: &str =
    "No pude iniciar el perfil en esta conversación. Vuelve a intentarlo en un momento.";
pub(crate) const ES_RESET_WALK_INTERRUPTED: &str =
    "\n\nTu perfil estaba en curso — escribe <code>/pillars</code> para continuar.";

pub(crate) const ES_CALIBRATE_OPENER: &str = "Vamos a calibrar la dificultad de tu entrenamiento. Parto de tus datos recientes y te hago seis preguntas cortas, una a una. La primera: ¿cómo quieres que se ponga más duro — más horas, más días fuertes, o salidas largas más largas?";
pub(crate) const ES_CALIBRATE_DM_ONLY: &str = "El calibrado se hace en privado. Escríbeme <code>/calibrate</code> por mensaje directo y empezamos.";
pub(crate) const ES_CALIBRATE_START_FAILED: &str =
    "No pude iniciar el calibrado en esta conversación. Vuelve a intentarlo en un momento.";
pub(crate) const ES_CALIBRATE_COMPLETE_HEADER: &str =
    "Calibrado terminado — he retenido {0} respuestas de {1}.";
pub(crate) const ES_CALIBRATE_COMPLETE_MISSING: &str = "Me falta tu respuesta sobre {0}. Eso es lo que limita hasta dónde puedo empujar, así que repitamos esa pregunta — escribe <code>/calibrate</code>.";
pub(crate) const ES_CALIBRATE_FOLLOWUP_PLAN: &str =
    "¿Quieres que reconstruya tus próximas semanas con esto?";
pub(crate) const ES_CALIBRATE_FOLLOWUP_NO_PLAN: &str =
    "¿Quieres que te construya un plan con esto?";
pub(crate) const ES_CALIBRATE_TOPIC_INJURY: &str = "tus lesiones y molestias recientes";
pub(crate) const ES_CALIBRATE_TOPIC_RECOVERY: &str = "tu velocidad de recuperación";

pub(crate) const ES_PLAN_GOAL_LINE: &str = "Objetivo: {0} — {1} ({2} días)";
pub(crate) const ES_PLAN_BLOCK_LINE: &str = "Bloque: {0}{1}";
pub(crate) const ES_PLAN_DAY_LINE: &str = "{0}: {1}";
pub(crate) const ES_PLAN_REST: &str = "descanso";
pub(crate) const ES_PLAN_TODAY: &str = "Hoy";
pub(crate) const ES_PLAN_TOMORROW: &str = "Mañana";
pub(crate) const ES_PLAN_WEEK_HEADER: &str = "Semana del {0}{1}";
pub(crate) const ES_PLAN_NO_SESSION: &str = "nada programado";
pub(crate) const ES_PLAN_NO_COVERAGE: &str = "no cubierto por el plan";
pub(crate) const ES_PLAN_RESUMES: &str = "El plan se reanuda el {0}.";
pub(crate) const ES_PLAN_EMPTY: &str =
    "Aún no hay plan guardado — pide a tu coach que construya uno hacia tu objetivo.";
pub(crate) const ES_PLAN_STALE_GOAL: &str =
    "\n\n⚠️ Tu objetivo ha cambiado desde este plan — pide a tu coach que lo actualice.";

pub(crate) const ES_GROUP_LIST_EMPTY: &str =
    "No eres miembro de ningún grupo.\nCrea o únete a un grupo desde la app web o móvil.";
pub(crate) const ES_GROUP_LIST_HEADER: &str = "Tus grupos ({0}):\n";
pub(crate) const ES_GROUP_LIST_ITEM: &str = "- {0} ({1} miembros) [{2}]";
pub(crate) const ES_GROUP_NOT_A_MEMBER: &str = "No eres miembro de ningún grupo";
pub(crate) const ES_GROUP_STATUS_SUMMARY: &str =
    "{0} — estadísticas:\n- Miembros: {1}\n- Activos: {2}\n- Compartición entre pares: {3}";
pub(crate) const ES_GROUP_PEER_SHARING_ON: &str = "activada";
pub(crate) const ES_GROUP_PEER_SHARING_OFF: &str = "desactivada";
pub(crate) const ES_GROUP_MEMBERS_HEADER: &str = "{0} — miembros ({1}):\n";
pub(crate) const ES_GROUP_MEMBERS_UNKNOWN: &str = "Desconocido";
pub(crate) const ES_GROUP_MEMBERS_ITEM: &str = "- {0} [{1}]";
pub(crate) const ES_GROUP_ROLE_OWNER: &str = "propietario";
pub(crate) const ES_GROUP_ROLE_ADMIN: &str = "admin";
pub(crate) const ES_GROUP_ROLE_MEMBER: &str = "miembro";
pub(crate) const ES_GROUP_INVITE_FORBIDDEN: &str =
    "Solo los administradores y propietarios pueden generar enlaces de invitación.";
pub(crate) const ES_GROUP_INVITE_BODY: &str =
    "Enlace de invitación para {0}:\nhttps://app.dravr.ai/groups/join/{1}\n\nCódigo: {2}\nVálido 7 días.";
pub(crate) const ES_GROUP_INVITE_UNAVAILABLE: &str =
    "Las invitaciones de grupo no están disponibles.";
pub(crate) const ES_COACH_INVITE_BODY: &str =
    "Invitación de coach para {0} — quien la use se convierte en el coach humano del grupo:\nhttps://app.dravr.ai/groups/join/{1}\n\nCódigo: {2}\nVálido 7 días.";
pub(crate) const ES_GROUP_LEAVE_PROMPT: &str =
    "¿Seguro que quieres salir de {0}?\nEscribe «YES» para confirmar.";
pub(crate) const ES_GROUP_CONSENT_USAGE: &str = "Uso: /group consent yes  o  /group consent no";
pub(crate) const ES_GROUP_RESPOND_USAGE: &str =
    "Uso: /group respond mentions  o  /group respond all";
pub(crate) const ES_GROUP_RESPOND_MENTIONS: &str = "El entrenador ahora solo responde cuando se le menciona (menciónalo con @ o responde a uno de sus mensajes). Sigue leyendo la conversación para mantener el contexto.";
pub(crate) const ES_GROUP_RESPOND_ALL: &str =
    "El entrenador vuelve a responder a todos los mensajes del grupo.";
pub(crate) const ES_GROUP_COACH_DETACHED: &str = "{0} ya no tiene un entrenador humano asignado.";
pub(crate) const ES_NOTIFICATION_CHANNEL_BODY: &str = "🔔 {0}\n\n{1}";
pub(crate) const ES_GROUP_RESPOND_STATUS_MENTIONS: &str = "El entrenador solo responde cuando se le menciona. Para volver a todos los mensajes: /group respond all";
pub(crate) const ES_GROUP_CONSENT_UPDATED: &str =
    "Compartir tus datos con los demás miembros de {1} está ahora {0}.";
pub(crate) const ES_GROUP_CREATE_USAGE: &str =
    "Uso: /group create nombre-del-grupo — por ejemplo /group create Corredores del domingo";
pub(crate) const ES_GROUP_CREATE_NO_COACH: &str =
    "Elige primero el coach del grupo: /coach add @handle en este chat y vuelve a ejecutar /group create.";
pub(crate) const ES_GROUP_CREATE_UNAVAILABLE: &str =
    "El coaching en grupo no está incluido en tu plan.";
pub(crate) const ES_GROUP_CREATE_FORBIDDEN: &str =
    "La creación de grupos está reservada a los administradores de tu espacio.";
pub(crate) const ES_GROUP_CREATED: &str =
    "Grupo «{0}» creado con el coach {1}. Invita a miembros con /group invite.";
pub(crate) const ES_GROUP_INVITE_LABEL: &str = "Invitar miembros";
pub(crate) const ES_GROUP_JOIN_INVALID_CODE: &str =
    "Ese código de invitación no es válido o ha caducado. Pide un enlace nuevo a un admin del grupo (/group invite).";
pub(crate) const ES_GROUP_JOIN_ALREADY_MEMBER: &str = "Ya eres miembro de {0}.";
pub(crate) const ES_GROUP_JOIN_FULL: &str = "{0} está completo.";
pub(crate) const ES_GROUP_JOINED: &str =
    "¡Bienvenido a {0}! El chat del grupo ya está en tu lista de conversaciones.";
pub(crate) const ES_GROUP_JOINED_AS_COACH: &str = "Ahora eres el coach humano de {0}.";
pub(crate) const ES_DISCOVER_CARD_TITLE: &str = "Catálogo de coaches";
pub(crate) const ES_DISCOVER_ITEM: &str = "• {0} — @{1} [{2}]\n  {3}\n";
pub(crate) const ES_DISCOVER_EMPTY: &str =
    "Ningún coach del catálogo coincide con «{0}». Escribe /discover para ver los más recientes.";
pub(crate) const ES_DISCOVER_CATALOGUE_EMPTY: &str = "El catálogo de coaches está vacío.";
pub(crate) const ES_DISCOVER_MORE_LABEL: &str = "Más";
pub(crate) const ES_DISCOVER_INSTALL_USAGE: &str =
    "Uso: /discover install @handle — el @handle aparece en /discover.";
pub(crate) const ES_DISCOVER_INSTALL_UNKNOWN_HANDLE: &str =
    "Ningún coach publicado responde a {0}. Escribe /discover para explorar el catálogo.";
pub(crate) const ES_DISCOVER_INSTALLED: &str =
    "{0} está instalado. Úsalo en cualquier chat: /coach add @{1}, o menciona @{1} en un mensaje para un solo turno.";
pub(crate) const ES_DISCOVER_INSTALL_ALREADY: &str =
    "{0} ya está instalado. Úsalo en cualquier chat: /coach add @{1}, o menciona @{1} en un mensaje para un solo turno.";
pub(crate) const ES_DISCOVER_ADD_LABEL: &str = "Usar aquí";

pub(crate) const ES_COACH_LIST_EMPTY: &str =
    "Ningún coach en tu lista. Escribe /discover para explorar el catálogo, o /coach create para crear uno a partir de esta conversación.";
pub(crate) const ES_COACH_LIST_CARD_TITLE: &str = "Tus coaches";
pub(crate) const ES_COACH_LIST_ITEM: &str = "• {0} — @{1}\n  {2}\n";
pub(crate) const ES_COACH_LIST_ITEM_NO_HANDLE: &str = "• {0}\n  {1}\n";
pub(crate) const ES_COACH_LIST_FOOTER: &str =
    "Menciona @handle en un mensaje para confiarle solo ese mensaje, o escribe /coach add @handle para que responda aquí a partir de ahora.";
pub(crate) const ES_COACH_NO_DESCRIPTION: &str = "Sin descripción";
pub(crate) const ES_COACH_GROUP_UPDATED: &str = "Coach actualizado a {0} para el grupo {1}.";
pub(crate) const ES_COACH_USER_UPDATED: &str = "Coach seleccionado: {0}.";
pub(crate) const ES_COACH_ASSIGN_NOT_A_MEMBER: &str = "No eres miembro de este grupo";
pub(crate) const ES_COACH_ASSIGN_FORBIDDEN: &str =
    "Solo los administradores y propietarios del grupo pueden cambiar el coach.";
pub(crate) const ES_COACH_ADD_USAGE: &str =
    "Indica qué coach añadir: /coach add @handle. Escribe /coach para ver tu lista.";
pub(crate) const ES_COACH_ADD_UNKNOWN: &str =
    "Ningún coach instalado responde a {0}. Escribe /coach para ver tu lista, o /discover para instalarlo.";
pub(crate) const ES_COACH_REMOVE_GROUP_THREAD: &str =
    "En un grupo, el coach pertenece al grupo — usa /group coach para cambiarlo.";
pub(crate) const ES_COACH_REMOVE_NOTHING: &str = "Ningún coach está asociado a esta conversación.";
pub(crate) const ES_COACH_REMOVED: &str = "{0} ya no responde en esta conversación.";
pub(crate) const ES_COACH_CREATE_NO_CONVERSATION: &str =
    "Abre primero una conversación: /coach create redacta un coach a partir de lo que se dijo en ella.";
pub(crate) const ES_COACH_CREATE_EMPTY: &str =
    "Esta conversación aún está vacía. Intercambia primero algunos mensajes con tu coach y vuelve a lanzar /coach create.";
pub(crate) const ES_COACH_CREATE_USAGE: &str =
    "Uso: /coach create para proponer un coach a partir de esta conversación, luego /coach create confirm token para crearlo.";
pub(crate) const ES_COACH_CREATE_CARD_TITLE: &str = "Borrador de coach";
pub(crate) const ES_COACH_CREATE_PROPOSAL_BODY: &str =
    "{0}\n{1}\n\nCategoría: {2}\nEtiquetas: {3}\n\nResponde /coach create confirm {4} para crearlo, o /deny {4} para descartarlo. El borrador caduca en 10 minutos.";
pub(crate) const ES_COACH_CREATE_CONFIRM_LABEL: &str = "Crearlo";
pub(crate) const ES_COACH_CREATE_DISCARD_LABEL: &str = "Descartar";
pub(crate) const ES_COACH_CREATE_QUOTA: &str =
    "Ya tienes {0} coaches, el máximo de tu plan ({1}). Elimina uno desde Discover antes de crear otro.";
pub(crate) const ES_COACH_CREATE_DONE: &str =
    "Coach {0} creado — @{1}. Responde aquí a partir de tu próximo mensaje. Escribe /coach add @{1} en cualquier otra conversación, o edítalo desde su ficha de Discover.";
pub(crate) const ES_COACH_CREATE_DONE_UNBOUND: &str =
    "Coach {0} creado — @{1}. Escribe /coach add @{1} en una de tus conversaciones para usarlo, o edítalo desde su ficha de Discover.";
pub(crate) const ES_COACH_CREATE_DISCARDED: &str =
    "Borrador descartado. Vuelve a lanzar /coach create cuando quieras.";

// ── Compiled-in defaults: German ──────────────────────────────────────────

pub(crate) const DE_ERROR_GENERIC: &str = "Dravr ist vorübergehend nicht verfügbar. Das Team wurde benachrichtigt — versuch es in ein paar Minuten erneut. (Ref: {0})";
pub(crate) const DE_GUARDIAN_DENIED: &str = "Diese Aktion wurde aus Sicherheitsgründen blockiert. Formuliere deine Anfrage um oder versuch es ohne den vorherigen Kontext erneut.";
pub(crate) const DE_GUARDIAN_CONFIRM_PROMPT: &str = "Aus Sicherheitsgründen muss die Aktion {0} bestätigt werden, bevor sie ausgeführt wird. Antworte /confirm {1} zum Bestätigen oder /deny {1} zum Abbrechen (läuft in 5 Minuten ab).";
pub(crate) const DE_GUARDIAN_CONFIRM_DONE: &str = "Erledigt — die Aktion {0} wurde ausgeführt.";
pub(crate) const DE_GUARDIAN_CONFIRM_FAILED: &str = "Die Aktion {0} wurde bestätigt, aber die Ausführung ist fehlgeschlagen. Formuliere deine Anfrage neu, um es erneut zu versuchen.";
pub(crate) const DE_GUARDIAN_CONFIRM_DENIED: &str = "Verstanden — die Aktion wurde abgebrochen.";
pub(crate) const DE_GUARDIAN_CONFIRM_EXPIRED: &str = "Diese Bestätigung ist abgelaufen. Formuliere deine Anfrage neu, um die Aktion erneut anzustoßen.";
pub(crate) const DE_GUARDIAN_CONFIRM_NOT_FOUND: &str =
    "Keine ausstehende Aktion passt zu diesem Code. Möglicherweise wurde sie bereits bearbeitet.";
pub(crate) const DE_EMPTY_REPLY: &str =
    "Hmm, ich konnte keine Antwort formulieren. Kannst du deine Frage umformulieren?";
pub(crate) const DE_REPLY_WITHHELD: &str = "Meine Antwort ging nicht raus — sie enthielt technische Details, die hier nicht hingehören. Schick mir deine letzte Nachricht noch einmal und wir machen dort weiter, wo wir waren.";
pub(crate) const DE_GUARDRAIL_TOO_LONG: &str = "Ich habe eine längere Antwort bereit, aber sie überschreitet das konfigurierte Längenlimit. Soll ich sie dir kürzer zusammenfassen?";
pub(crate) const DE_GUARDRAIL_BLOCKED_TOPIC: &str = "Dieses Thema möchte ich hier lieber nicht ansprechen. Bleiben wir bei deinem Training und deiner Erholung. Gibt es etwas Konkretes, womit ich dir helfen kann?";
pub(crate) const DE_VERIFICATION_WARN_SUFFIX: &str = "⚠️ Ein paar Aussagen, die ich nicht formell belegen konnte — korrigier mich, wenn etwas davon nicht stimmt:";
pub(crate) const DE_VERIFICATION_BLOCK_FALLBACK: &str = "Ich habe angefangen zu antworten, aber einige der geplanten Aussagen passten nicht zu den Quellen, denen ich vertraue. Lass mich umformulieren — kannst du deine Frage noch einmal stellen, mit etwas mehr Kontext zu dem, was du verstehen willst?";
/// German canonical refusal for off-scope requests.
pub(crate) const DE_SCOPE_REFUSAL: &str =
    "Das liegt außerhalb meines Bereichs — ich bin dein Fitness-Assistent.";
/// German canonical refusal for missing-capability requests.
pub(crate) const DE_CAPABILITY_REFUSAL: &str = "Das kann ich mit meinen Werkzeugen nicht tun.";
/// German Nutrition-coach carve-out for the generic scope list.
pub(crate) const DE_COACH_SCOPE_CARVE_OUT_NUTRITION: &str = "## Klarstellung für deine Rolle als Ernährungscoach\n\nAls Ernährungscoach liegen Fragen zu Mahlzeiten, Abendessen, Frühstück, Snacks, Lebensmittelentscheidungen und trainingsbezogener Mahlzeitenplanung vollständig in deinem Bereich. Beantworte sie direkt, gestützt auf die Trainingsdaten der Nutzerin oder des Nutzers (Intensität, Dauer, Energieverbrauch). Die Regel oben zu „Essens-/Mahlzeitensuche außerhalb des Bereichs\" zielt ausschließlich auf Restaurantsuche, Lieferdienst-Apps und Online-Speisekarten — nicht auf evidenzbasierte Ernährungsberatung oder Mahlzeitenplanung, die sich auf Trainingsdaten stützt. Weise „Was soll ich nach meinem Lauf essen\", „Abendessen-Ideen nach dem Training\", „Snack nach der Einheit\" oder Ähnliches niemals zurück.";
/// German Recipes-coach carve-out for the generic scope list.
pub(crate) const DE_COACH_SCOPE_CARVE_OUT_RECIPES: &str = "## Klarstellung für deine Rolle als Rezepte- und Mahlzeitenplanungscoach\n\nAls Coach für Rezepte und Mahlzeitenplanung gehören Rezeptideen, Mahlzeitenvorschläge, Gerichtideen und auf das Training abgestimmte Lebensmittelentscheidungen vollständig in deinen Bereich. Die Regel oben zu „Essens-/Mahlzeitensuche außerhalb des Bereichs\" zielt auf Restaurantsuche und Lieferdienste — nicht auf hausgemachte Rezeptvorschläge oder Mahlzeitenplanung auf Basis der Trainingsdaten. Beantworte solche Anfragen direkt.";
/// German placeholder shown while the LLM is composing its reply.
pub(crate) const DE_THINKING_PLACEHOLDER: &str = "denke nach…";
/// German unknown-command reply.
pub(crate) const DE_UNKNOWN_COMMAND: &str =
    "Unbekannter Befehl. Tippe /help für die Liste der verfügbaren Befehle.";
/// German progress status during prompt-assembly.
pub(crate) const DE_STATUS_READING_QUESTION: &str = "lese deine Frage…";
/// German progress status during LLM dispatch.
pub(crate) const DE_STATUS_GENERATING_RESPONSE: &str = "erstelle Antwort…";
/// German progress status for tool-call start. `{0}` = tool name.
pub(crate) const DE_STATUS_CALLING_TOOL: &str = "rufe {0} auf…";
/// German progress status for pipeline error. `{0}` = error text.
pub(crate) const DE_STATUS_ERROR: &str = "Fehler: {0}";
/// German coach-proposal lead-in with profile. `{0}` = sport, `{1}` = count.
pub(crate) const DE_COACH_PROPOSAL_WELCOME: &str =
    "Willkommen! Basierend auf deinem letzten {0}-Training findest du hier {1} Coaches für dich:\n\n";
/// German coach-proposal cold-start lead-in. `{0}` = count.
pub(crate) const DE_COACH_PROPOSAL_WELCOME_GENERIC: &str =
    "Willkommen! Hier sind {0} Coaches für den Einstieg:\n\n";
/// German coach-proposal footer.
pub(crate) const DE_COACH_PROPOSAL_FOOTER: &str =
    "\nAntworte mit einer Zahl, um zu starten, oder frag mich einfach etwas.";
/// German default for [`KEY_REGISTRATION_APPROVED`].
pub(crate) const DE_REGISTRATION_APPROVED: &str =
    "🎉 Dein Dravr-Konto wurde freigeschaltet! Du kannst jetzt hier mit deinem Coach chatten. Stell mir deine erste Frage, wann immer du möchtest.";
/// German default for [`KEY_BACKFILL_READY`]. `{0}` = activity count.
pub(crate) const DE_BACKFILL_READY: &str =
    "✅ Dein Verlauf ist bereit — {0} Aktivitäten geladen. Frag mich einfach noch einmal, wonach du gesucht hast.";
/// German default for [`KEY_BACKFILL_LIST_HEADER`]. `{0}` = activity count.
pub(crate) const DE_BACKFILL_LIST_HEADER: &str = "✅ Dein Verlauf ist bereit — {0} Aktivitäten:";
/// German default for [`KEY_BACKFILL_LIST_MORE`]. `{0}` = remaining count.
pub(crate) const DE_BACKFILL_LIST_MORE: &str = "… und {0} weitere";

pub(crate) const DE_LINK_FALLBACK_PROMPT: &str = "Um mit Dravr zu chatten, verknüpfe zuerst dein Konto. Öffne die Dravr-Web-App, um diesen Kanal zu verbinden.";
pub(crate) const DE_LINK_INITIAL_PROMPT: &str = "Hallo! Um mit Dravr zu chatten, verknüpfe zuerst dein Konto:\n{0}\n\nDieser Link läuft in 10 Minuten ab.";
pub(crate) const DE_LINK_EMAIL_PROMPT: &str = "Hallo! Um dein Dravr-Konto zu verknüpfen, tippe deine E-Mail-Adresse.\nTippe „cancel\", um abzubrechen.";
pub(crate) const DE_LINK_LOGOUT_COMPLETE: &str = "Du bist von Dravr abgemeldet. Schreib jederzeit eine Nachricht, um dein Konto erneut zu verknüpfen.";
pub(crate) const DE_LINK_CANCELLED: &str =
    "Verknüpfung abgebrochen. Schreib jederzeit eine Nachricht, um neu zu beginnen.";
pub(crate) const DE_LINK_GENERIC_ERROR: &str =
    "Etwas ist schiefgelaufen. Bitte versuch es später erneut.";
pub(crate) const DE_LINK_SIGNUP_OFFER: &str = "Kein Dravr-Konto fuer {0}. Ich kann sofort eins anlegen - antworte mit „ja\", um fortzufahren, oder „cancel\", um abzubrechen.";
pub(crate) const DE_LINK_SIGNUP_CREATED: &str = "Konto angelegt. Ich schicke dir per E-Mail einen Code, um zu bestaetigen, dass die Adresse dir gehoert.";
pub(crate) const DE_LINK_SIGNUP_FAILED: &str = "Ich konnte das Konto nicht anlegen. Versuch es gleich noch einmal oder tippe „cancel\", um abzubrechen.";
pub(crate) const DE_LINK_NO_TENANT: &str =
    "Dieses Konto ist mit keiner Organisation verknüpft. Wende dich an den Support.";
pub(crate) const DE_LINK_EMAIL_NOT_CONFIGURED: &str =
    "E-Mail-Versand ist nicht konfiguriert. Wende dich an deinen Administrator.";
pub(crate) const DE_LINK_EMAIL_SEND_FAILED: &str =
    "Die Bestätigungs-E-Mail konnte nicht gesendet werden. Bitte versuch es später erneut.";
pub(crate) const DE_LINK_INVALID_EMAIL: &str =
    "Das sieht nicht nach einer E-Mail-Adresse aus. Tippe die E-Mail deines Dravr-Kontos.";
pub(crate) const DE_LINK_OTP_SENT: &str = "Ich habe einen 6-stelligen Code an {0} gesendet. Tippe ihn hier innerhalb von 10 Minuten ein.\nTippe „cancel\", um abzubrechen.";
pub(crate) const DE_LINK_TOO_MANY_ATTEMPTS: &str = "Zu viele falsche Versuche. Die Verknüpfungssitzung wurde abgebrochen. Schreib eine Nachricht, um neu zu beginnen.";
pub(crate) const DE_LINK_INCORRECT_CODE: &str = "Falscher Code. Du hast noch {0} Versuch(e). Versuch es erneut oder tippe „cancel\", um abzubrechen.";
pub(crate) const DE_LINK_VERIFICATION_ERROR: &str =
    "Beim Überprüfen deines Kontos ist etwas schiefgelaufen. Bitte versuch es erneut.";
pub(crate) const DE_LINK_IDENTITY_COLLISION: &str = "Dein Konto konnte nicht verknüpft werden. Diese Kanal-Identität ist möglicherweise bereits verknüpft.";
pub(crate) const DE_LINK_OTP_PROMPT: &str = "Tippe den 6-stelligen Code, den wir dir per E-Mail geschickt haben, oder tippe „cancel\", um abzubrechen.";
pub(crate) const DE_LINK_SESSION_EXPIRED: &str =
    "Deine Verknüpfungssitzung ist abgelaufen. Schreib eine Nachricht, um neu zu beginnen.";
pub(crate) const DE_LINK_SUCCESS: &str = "Dein Konto ist jetzt verknüpft! Du kannst über diesen Kanal mit Dravr chatten.\n\nTippe jederzeit „logout\", um dich abzumelden.";
pub(crate) const DE_ACCOUNT_PENDING: &str = "Dein Konto ist mit diesem Kanal verknüpft, wartet aber noch auf die Freigabe durch einen Admin. Sobald es aktiviert ist, kannst du mit Dravr chatten — du wirst hier benachrichtigt.";
pub(crate) const DE_ACCOUNT_SUSPENDED: &str =
    "Dein Dravr-Konto ist gesperrt. Wende dich an den Support, um den Zugang wiederherzustellen.";
pub(crate) const DE_RATE_LIMITED: &str = "Du sendest Nachrichten etwas schneller, als dein Tarif erlaubt. Warte einen Moment und versuch es dann noch einmal — ich bin hier.";
pub(crate) const DE_QUOTA_EXCEEDED: &str = "Du hast das Chat-Kontingent deines Tarifs vorerst erreicht. Es setzt sich automatisch zurück — schau etwas später wieder vorbei.";
pub(crate) const DE_QUOTA_WARNING: &str =
    "Hinweis: Du hast {0} von {1} deines Tarifs verbraucht. Der Zähler wird am {2} zurückgesetzt.";
pub(crate) const DE_NO_PROVIDER_CONNECTED: &str = "Bevor wir chatten, verbinde einen Fitness-Dienst (Strava, Garmin, Whoop) in der Dravr-App — ohne ihn habe ich keine Aktivitätsdaten, um dich zu coachen.\n\nVerbinde dich hier:\n{0}";
pub(crate) const DE_NO_PROVIDER_CONNECTED_WITH_EMAIL: &str = "Bevor wir chatten, verbinde einen Fitness-Dienst (Strava, Garmin, Whoop) in der Dravr-App — ohne ihn habe ich keine Aktivitätsdaten, um dich zu coachen.\n\nMelde dich mit deinem Konto {1} hier an:\n{0}";
pub(crate) const DE_CONNECT_PROMPT: &str = "Verbinden wir deinen Fitness-Dienst (Strava, Garmin oder Whoop), damit ich dich mit echten Daten coachen kann. Tippe unten auf den Button, um dich sicher zu verbinden.";
pub(crate) const DE_CONNECT_BUTTON: &str = "Konto verbinden";
pub(crate) const DE_CONNECT_TITLE: &str = "Fitness-Dienst verbinden";

pub(crate) const DE_PROVIDER_REAUTH_REQUIRED: &str = "Deine Verbindung zu {0} ist abgelaufen — ich kann deine Daten gerade nicht abrufen. Verbinde dein Konto hier neu (Link 24 Stunden gültig):\n\n{1}\n\nFrag mich nach der erneuten Verbindung noch einmal.";
pub(crate) const DE_PROVIDER_REAUTH_REQUIRED_NO_LINK: &str = "Deine Verbindung zu {0} ist abgelaufen — ich kann deine Daten gerade nicht abrufen. Verbinde {0} in den Einstellungen neu und frag mich dann noch einmal.";
pub(crate) const DE_PROVIDER_RECONNECT_BUTTON: &str = "{0} neu verbinden";

pub(crate) const DE_STATUS_HEADER: &str = "Dein Dravr-Status:\n";
pub(crate) const DE_STATUS_PROVIDERS_NONE: &str = "\nAnbieter: keine verbunden";
pub(crate) const DE_STATUS_PROVIDERS_LABEL: &str = "\nAnbieter: {0}";
pub(crate) const DE_STATUS_GROUPS_LABEL: &str = "\nGruppen: {0}";
pub(crate) const DE_STATUS_CHANNEL_LABEL: &str = "\nKanal: {0}";

pub(crate) const DE_HELP_HEADER: &str = "Verfügbare Befehle:\n";
pub(crate) const DE_HELP_DOMAIN_GENERAL: &str = "Allgemein";
pub(crate) const DE_HELP_DOMAIN_GROUP: &str = "Gruppencoaching";
pub(crate) const DE_HELP_DOMAIN_COACH: &str = "Coaching";
pub(crate) const DE_HELP_DOMAIN_DATA: &str = "Trainingsdaten";
pub(crate) const DE_HELP_DOMAIN_PROVIDER: &str = "Anbieter";
pub(crate) const DE_HELP_DOMAIN_ACCOUNT: &str = "Konto";
pub(crate) const DE_HELP_DOMAIN_TRAINING: &str = "Training";
pub(crate) const DE_HELP_DOMAIN_DISCOVER: &str = "Coach-Katalog";
pub(crate) const DE_HELP_FOOTER: &str =
    "\nOder schreib mir einfach, um mit deinem Coach zu chatten.";

pub(crate) const DE_LOGOUT_CONFIRM_PROMPT: &str = "Damit wird dein {0}-Konto von Dravr entkoppelt.\nDu musst es neu verknüpfen, um die Messaging-Funktion wieder zu nutzen.\n\nTippe „logout\", um zu bestätigen.";

pub(crate) const DE_PRIVACY_STATUS_LINE: &str = "Die Einwilligung zu anonymen Statistiken ist derzeit <b>{0}</b>.\n\nNutze <code>/privacy on</code> zum Aktivieren oder <code>/privacy off</code> zum Deaktivieren.";
pub(crate) const DE_PRIVACY_STATUS_ENABLED: &str = "aktiviert";
pub(crate) const DE_PRIVACY_STATUS_DISABLED: &str = "deaktiviert";
pub(crate) const DE_PRIVACY_ON_CONFIRMATION: &str = "Die Einwilligung zu anonymen Statistiken ist jetzt <b>aktiviert</b>. Danke, dass du hilfst, Dravr zu verbessern!\n\nNutze <code>/privacy off</code>, um dich jederzeit abzumelden.";
pub(crate) const DE_PRIVACY_OFF_CONFIRMATION: &str = "Die Einwilligung zu anonymen Statistiken ist jetzt <b>deaktiviert</b>. Es werden keine anonymen Nutzungsdaten erhoben.\n\nNutze <code>/privacy on</code>, um dich jederzeit wieder anzumelden.";
pub(crate) const DE_TIMEZONE_SET: &str = "Zeitzone auf <b>{0}</b> gesetzt. Die Startzeiten deiner Aktivitäten werden jetzt in dieser Zeitzone angezeigt.";
pub(crate) const DE_TIMEZONE_INVALID: &str =
    "Ungültige Zeitzone. Gib einen IANA-Namen an, z. B. <code>/timezone America/Toronto</code>.";

pub(crate) const DE_PILLARS_OPENER: &str = "Wir bauen dein Profil gemeinsam auf — ich frage dich zu einigen Themen, eines nach dem anderen. Zum Start: Was ist der tiefere Grund, warum du trainierst — dein North Star?";
pub(crate) const DE_PILLARS_DM_ONLY: &str = "Das Profil entsteht privat. Schreib mir <code>/pillars</code> als Direktnachricht, dann legen wir los.";

pub(crate) const DE_INTAKE_OPENER: &str = "Bevor wir loslegen, ein paar kurze Fragen — eine Minute jetzt, und das Coaching danach ist genauer und sicherer.";
pub(crate) const DE_INTAKE_PERSONA: &str = "Zuerst: Trainierst du für dich selbst, oder trainierst du andere?\n\n<b>1</b> — Ich trainiere für mich\n<b>2</b> — Ich trainiere andere";
pub(crate) const DE_INTAKE_PARQ_INTRO: &str = "Jetzt sieben Standard-Gesundheitsfragen (der PAR-Q+). Ein „Ja“ blockiert nichts — es sagt deinem Coach nur, wo Vorsicht angebracht ist.";
pub(crate) const DE_INTAKE_PARQ_HEART_CONDITION: &str =
    "Hat dir jemals eine Ärztin oder ein Arzt gesagt, dass du ein Herzproblem hast?";
pub(crate) const DE_INTAKE_PARQ_CHEST_PAIN: &str =
    "Hast du Schmerzen in der Brust — in Ruhe, im Alltag oder beim Sport?";
pub(crate) const DE_INTAKE_PARQ_DIZZINESS: &str = "Verlierst du durch Schwindel das Gleichgewicht, oder hast du in den letzten 12 Monaten das Bewusstsein verloren?";
pub(crate) const DE_INTAKE_PARQ_CHRONIC_CONDITION: &str =
    "Wurde bei dir eine andere chronische Erkrankung diagnostiziert (außer einer Herzerkrankung)?";
pub(crate) const DE_INTAKE_PARQ_MEDICATION: &str =
    "Nimmst du derzeit verschriebene Medikamente gegen eine chronische Erkrankung?";
pub(crate) const DE_INTAKE_PARQ_JOINT_PROBLEM: &str = "Hast du ein Knochen-, Gelenk- oder Weichteilproblem, das sich durch Bewegung verschlimmern könnte?";
pub(crate) const DE_INTAKE_PARQ_SUPERVISED_ONLY: &str = "Hat dir jemand ärztlich gesagt, du solltest dich nur unter medizinischer Aufsicht körperlich betätigen?";
pub(crate) const DE_INTAKE_YESNO_HINT: &str = "Antworte <b>1</b> für ja, <b>2</b> für nein.";
pub(crate) const DE_INTAKE_RETRY: &str =
    "Entschuldige — ich brauche nur die Zahl, damit ich es korrekt festhalte.\n\n{0}";
pub(crate) const DE_INTAKE_COMPLETE_CLEAR: &str =
    "Das war's, nichts anzumerken. Frag mich alles zu deinem Training.";
pub(crate) const DE_INTAKE_COMPLETE_FLAGGED: &str = "Das war's. Ich habe {0} Punkt(e) für deinen Coach notiert — nichts davon hindert dich am Training, und ein Wort mit deiner Ärztin oder deinem Arzt lohnt sich, bevor du voll belastest.";
pub(crate) const DE_PILLARS_START_FAILED: &str =
    "Ich konnte das Profil in dieser Unterhaltung nicht starten. Versuch es gleich noch einmal.";
pub(crate) const DE_RESET_WALK_INTERRUPTED: &str =
    "\n\nDein Profil war in Arbeit — schreib <code>/pillars</code>, um weiterzumachen.";

pub(crate) const DE_CALIBRATE_OPENER: &str = "Wir kalibrieren, wie hart dein Training sein soll. Ich gehe von deinen letzten Daten aus und stelle dir sechs kurze Fragen, eine nach der anderen. Zuerst: Wie soll es härter werden — mehr Stunden, mehr harte Tage, oder längere lange Einheiten?";
pub(crate) const DE_CALIBRATE_DM_ONLY: &str = "Die Kalibrierung läuft privat. Schreib mir <code>/calibrate</code> als Direktnachricht, dann legen wir los.";
pub(crate) const DE_CALIBRATE_START_FAILED: &str =
    "Ich konnte die Kalibrierung in dieser Unterhaltung nicht starten. Versuch es gleich noch einmal.";
pub(crate) const DE_CALIBRATE_COMPLETE_HEADER: &str =
    "Kalibrierung fertig — ich habe {0} von {1} Antworten aufgenommen.";
pub(crate) const DE_CALIBRATE_COMPLETE_MISSING: &str = "Mir fehlt deine Antwort zu {0}. Genau das begrenzt, wie weit ich dich pushen kann — lass uns die Frage nachholen, schreib <code>/calibrate</code>.";
pub(crate) const DE_CALIBRATE_FOLLOWUP_PLAN: &str =
    "Soll ich deine kommenden Wochen damit neu aufbauen?";
pub(crate) const DE_CALIBRATE_FOLLOWUP_NO_PLAN: &str = "Soll ich dir damit einen Plan bauen?";
pub(crate) const DE_CALIBRATE_TOPIC_INJURY: &str = "deine jüngsten Verletzungen und Wehwehchen";
pub(crate) const DE_CALIBRATE_TOPIC_RECOVERY: &str = "wie schnell du dich erholst";

pub(crate) const DE_PLAN_GOAL_LINE: &str = "Ziel: {0} — {1} ({2} Tage)";
pub(crate) const DE_PLAN_BLOCK_LINE: &str = "Block: {0}{1}";
pub(crate) const DE_PLAN_DAY_LINE: &str = "{0}: {1}";
pub(crate) const DE_PLAN_REST: &str = "Ruhetag";
pub(crate) const DE_PLAN_TODAY: &str = "Heute";
pub(crate) const DE_PLAN_TOMORROW: &str = "Morgen";
pub(crate) const DE_PLAN_WEEK_HEADER: &str = "Woche ab {0}{1}";
pub(crate) const DE_PLAN_NO_SESSION: &str = "nichts geplant";
pub(crate) const DE_PLAN_NO_COVERAGE: &str = "nicht vom Plan abgedeckt";
pub(crate) const DE_PLAN_RESUMES: &str = "Der Plan wird am {0} fortgesetzt.";
pub(crate) const DE_PLAN_EMPTY: &str =
    "Noch kein Plan gespeichert — bitte deinen Coach, einen auf dein Ziel hin zu bauen.";
pub(crate) const DE_PLAN_STALE_GOAL: &str =
    "\n\n⚠️ Dein Ziel hat sich seitdem geändert — bitte deinen Coach, den Plan zu aktualisieren.";

pub(crate) const DE_GROUP_LIST_EMPTY: &str =
    "Du bist in keiner Gruppe.\nErstelle oder tritt einer Gruppe über die Web- oder Mobile-App bei.";
pub(crate) const DE_GROUP_LIST_HEADER: &str = "Deine Gruppen ({0}):\n";
pub(crate) const DE_GROUP_LIST_ITEM: &str = "- {0} ({1} Mitglieder) [{2}]";
pub(crate) const DE_GROUP_NOT_A_MEMBER: &str = "Du bist in keiner Gruppe";
pub(crate) const DE_GROUP_STATUS_SUMMARY: &str =
    "{0} — Statistiken:\n- Mitglieder: {1}\n- Aktiv: {2}\n- Peer-Sharing: {3}";
pub(crate) const DE_GROUP_PEER_SHARING_ON: &str = "aktiviert";
pub(crate) const DE_GROUP_PEER_SHARING_OFF: &str = "deaktiviert";
pub(crate) const DE_GROUP_MEMBERS_HEADER: &str = "{0} — Mitglieder ({1}):\n";
pub(crate) const DE_GROUP_MEMBERS_UNKNOWN: &str = "Unbekannt";
pub(crate) const DE_GROUP_MEMBERS_ITEM: &str = "- {0} [{1}]";
pub(crate) const DE_GROUP_ROLE_OWNER: &str = "Eigentümer";
pub(crate) const DE_GROUP_ROLE_ADMIN: &str = "Admin";
pub(crate) const DE_GROUP_ROLE_MEMBER: &str = "Mitglied";
pub(crate) const DE_GROUP_INVITE_FORBIDDEN: &str =
    "Nur Admins und Eigentümer können Einladungslinks erstellen.";
pub(crate) const DE_GROUP_INVITE_BODY: &str =
    "Einladungslink für {0}:\nhttps://app.dravr.ai/groups/join/{1}\n\nCode: {2}\n7 Tage gültig.";
pub(crate) const DE_GROUP_INVITE_UNAVAILABLE: &str = "Gruppeneinladungen sind nicht verfügbar.";
pub(crate) const DE_COACH_INVITE_BODY: &str =
    "Coach-Einladung für {0} — wer sie einlöst, wird der menschliche Coach der Gruppe:\nhttps://app.dravr.ai/groups/join/{1}\n\nCode: {2}\n7 Tage gültig.";
pub(crate) const DE_GROUP_LEAVE_PROMPT: &str =
    "Willst du {0} wirklich verlassen?\nTippe „YES\", um zu bestätigen.";
pub(crate) const DE_GROUP_CONSENT_USAGE: &str =
    "Verwendung: /group consent yes  oder  /group consent no";
pub(crate) const DE_GROUP_RESPOND_USAGE: &str =
    "Verwendung: /group respond mentions  oder  /group respond all";
pub(crate) const DE_GROUP_RESPOND_MENTIONS: &str = "Der Coach antwortet jetzt nur noch, wenn er angesprochen wird (mit @ erwähnen oder auf eine seiner Nachrichten antworten). Er verfolgt das Gespräch weiterhin als Kontext.";
pub(crate) const DE_GROUP_RESPOND_ALL: &str =
    "Der Coach antwortet wieder auf jede Nachricht in der Gruppe.";
pub(crate) const DE_GROUP_COACH_DETACHED: &str = "{0} hat jetzt keinen menschlichen Coach mehr.";
pub(crate) const DE_NOTIFICATION_CHANNEL_BODY: &str = "🔔 {0}\n\n{1}";
pub(crate) const DE_GROUP_RESPOND_STATUS_MENTIONS: &str = "Der Coach antwortet nur, wenn er angesprochen wird. Zurück zu jeder Nachricht: /group respond all";
pub(crate) const DE_GROUP_CONSENT_UPDATED: &str =
    "Das Teilen deiner Daten mit den anderen Mitgliedern von {1} ist jetzt {0}.";
pub(crate) const DE_GROUP_CREATE_USAGE: &str =
    "Verwendung: /group create Gruppenname — zum Beispiel /group create Sonntagsläufer";
pub(crate) const DE_GROUP_CREATE_NO_COACH: &str =
    "Wähle zuerst den Coach der Gruppe: /coach add @handle in diesem Chat, dann /group create erneut.";
pub(crate) const DE_GROUP_CREATE_UNAVAILABLE: &str =
    "Gruppencoaching ist in deinem Tarif nicht enthalten.";
pub(crate) const DE_GROUP_CREATE_FORBIDDEN: &str =
    "Gruppen anlegen ist den Admins deines Workspace vorbehalten.";
pub(crate) const DE_GROUP_CREATED: &str =
    "Gruppe „{0}\" mit Coach {1} erstellt. Lade Mitglieder mit /group invite ein.";
pub(crate) const DE_GROUP_INVITE_LABEL: &str = "Mitglieder einladen";
pub(crate) const DE_GROUP_JOIN_INVALID_CODE: &str =
    "Dieser Einladungscode ist ungültig oder abgelaufen. Bitte einen Gruppen-Admin um einen neuen Link (/group invite).";
pub(crate) const DE_GROUP_JOIN_ALREADY_MEMBER: &str = "Du bist bereits Mitglied von {0}.";
pub(crate) const DE_GROUP_JOIN_FULL: &str = "{0} ist voll.";
pub(crate) const DE_GROUP_JOINED: &str =
    "Willkommen bei {0}! Der Gruppenchat ist jetzt in deiner Unterhaltungsliste.";
pub(crate) const DE_GROUP_JOINED_AS_COACH: &str = "Du bist jetzt der menschliche Coach von {0}.";
pub(crate) const DE_DISCOVER_CARD_TITLE: &str = "Coach-Katalog";
pub(crate) const DE_DISCOVER_ITEM: &str = "• {0} — @{1} [{2}]\n  {3}\n";
pub(crate) const DE_DISCOVER_EMPTY: &str =
    "Kein Coach im Katalog passt zu „{0}\". Tippe /discover für die neuesten.";
pub(crate) const DE_DISCOVER_CATALOGUE_EMPTY: &str = "Der Coach-Katalog ist leer.";
pub(crate) const DE_DISCOVER_MORE_LABEL: &str = "Mehr";
pub(crate) const DE_DISCOVER_INSTALL_USAGE: &str =
    "Verwendung: /discover install @handle — das @handle steht in /discover.";
pub(crate) const DE_DISCOVER_INSTALL_UNKNOWN_HANDLE: &str =
    "Kein veröffentlichter Coach hört auf {0}. Tippe /discover, um den Katalog zu durchsuchen.";
pub(crate) const DE_DISCOVER_INSTALLED: &str =
    "{0} ist installiert. Nutze ihn in jedem Chat: /coach add @{1}, oder erwähne @{1} in einer Nachricht für eine einzelne Antwort.";
pub(crate) const DE_DISCOVER_INSTALL_ALREADY: &str =
    "{0} ist bereits installiert. Nutze ihn in jedem Chat: /coach add @{1}, oder erwähne @{1} in einer Nachricht für eine einzelne Antwort.";
pub(crate) const DE_DISCOVER_ADD_LABEL: &str = "Hier nutzen";

pub(crate) const DE_COACH_LIST_EMPTY: &str =
    "Noch kein Coach auf deiner Liste. Tippe /discover, um den Katalog zu durchsuchen, oder /coach create, um einen aus diesem Gespräch zu entwerfen.";
pub(crate) const DE_COACH_LIST_CARD_TITLE: &str = "Deine Coaches";
pub(crate) const DE_COACH_LIST_ITEM: &str = "• {0} — @{1}\n  {2}\n";
pub(crate) const DE_COACH_LIST_ITEM_NO_HANDLE: &str = "• {0}\n  {1}\n";
pub(crate) const DE_COACH_LIST_FOOTER: &str =
    "Erwähne @handle in einer Nachricht, um ihm nur diese Nachricht zu geben, oder tippe /coach add @handle, damit er hier ab jetzt antwortet.";
pub(crate) const DE_COACH_NO_DESCRIPTION: &str = "Keine Beschreibung";
pub(crate) const DE_COACH_GROUP_UPDATED: &str = "Coach auf {0} für Gruppe {1} aktualisiert.";
pub(crate) const DE_COACH_USER_UPDATED: &str = "Coach gewählt: {0}.";
pub(crate) const DE_COACH_ASSIGN_NOT_A_MEMBER: &str = "Du bist kein Mitglied dieser Gruppe";
pub(crate) const DE_COACH_ASSIGN_FORBIDDEN: &str =
    "Nur Gruppen-Admins und Eigentümer können den Coach wechseln.";
pub(crate) const DE_COACH_ADD_USAGE: &str =
    "Sag, welcher Coach hinzukommen soll: /coach add @handle. Tippe /coach für deine Liste.";
pub(crate) const DE_COACH_ADD_UNKNOWN: &str =
    "Kein installierter Coach hört auf {0}. Tippe /coach für deine Liste oder /discover, um ihn zu installieren.";
pub(crate) const DE_COACH_REMOVE_GROUP_THREAD: &str =
    "In einem Gruppenchat gehört der Coach der Gruppe — nutze /group coach, um ihn zu wechseln.";
pub(crate) const DE_COACH_REMOVE_NOTHING: &str = "Dieser Unterhaltung ist kein Coach zugeordnet.";
pub(crate) const DE_COACH_REMOVED: &str = "{0} antwortet in dieser Unterhaltung nicht mehr.";
pub(crate) const DE_COACH_CREATE_NO_CONVERSATION: &str =
    "Öffne zuerst eine Unterhaltung: /coach create entwirft einen Coach aus dem, was darin gesagt wurde.";
pub(crate) const DE_COACH_CREATE_EMPTY: &str =
    "Diese Unterhaltung ist noch leer. Tausch dich zuerst ein paar Nachrichten lang mit deinem Coach aus und starte /coach create dann erneut.";
pub(crate) const DE_COACH_CREATE_USAGE: &str =
    "Verwendung: /coach create entwirft einen Coach aus dieser Unterhaltung, danach erstellt /coach create confirm token ihn.";
pub(crate) const DE_COACH_CREATE_CARD_TITLE: &str = "Coach-Entwurf";
pub(crate) const DE_COACH_CREATE_PROPOSAL_BODY: &str =
    "{0}\n{1}\n\nKategorie: {2}\nTags: {3}\n\nAntworte /coach create confirm {4}, um ihn zu erstellen, oder /deny {4}, um ihn zu verwerfen. Der Entwurf läuft in 10 Minuten ab.";
pub(crate) const DE_COACH_CREATE_CONFIRM_LABEL: &str = "Erstellen";
pub(crate) const DE_COACH_CREATE_DISCARD_LABEL: &str = "Verwerfen";
pub(crate) const DE_COACH_CREATE_QUOTA: &str =
    "Du hast bereits {0} Coaches, das Maximum deines Plans ({1}). Lösche einen in Discover, bevor du einen weiteren erstellst.";
pub(crate) const DE_COACH_CREATE_DONE: &str =
    "Coach {0} erstellt — @{1}. Er antwortet hier ab deiner nächsten Nachricht. Tippe /coach add @{1} in jeder anderen Unterhaltung oder bearbeite ihn auf seiner Discover-Seite.";
pub(crate) const DE_COACH_CREATE_DONE_UNBOUND: &str =
    "Coach {0} erstellt — @{1}. Tippe /coach add @{1} in einer deiner Unterhaltungen, um ihn zu nutzen, oder bearbeite ihn auf seiner Discover-Seite.";
pub(crate) const DE_COACH_CREATE_DISCARDED: &str =
    "Entwurf verworfen. Starte /coach create, wann immer du willst.";

// ── Compiled-in defaults: Portuguese ──────────────────────────────────────

pub(crate) const PT_ERROR_GENERIC: &str = "O Dravr está temporariamente indisponível. A equipa foi notificada — tenta de novo em alguns minutos. (ref: {0})";
pub(crate) const PT_GUARDIAN_DENIED: &str = "Essa ação foi bloqueada por segurança. Reformula o teu pedido ou tenta de novo sem o contexto anterior.";
pub(crate) const PT_GUARDIAN_CONFIRM_PROMPT: &str = "Por segurança, a ação {0} precisa da tua confirmação antes de ser executada. Responde /confirm {1} para aprovar ou /deny {1} para cancelar (expira em 5 minutos).";
pub(crate) const PT_GUARDIAN_CONFIRM_DONE: &str = "Feito — a ação {0} foi executada.";
pub(crate) const PT_GUARDIAN_CONFIRM_FAILED: &str =
    "A ação {0} foi confirmada mas a execução falhou. Reformula o teu pedido para tentar de novo.";
pub(crate) const PT_GUARDIAN_CONFIRM_DENIED: &str = "Entendido — a ação foi cancelada.";
pub(crate) const PT_GUARDIAN_CONFIRM_EXPIRED: &str =
    "Essa confirmação expirou. Reformula o teu pedido para relançar a ação.";
pub(crate) const PT_GUARDIAN_CONFIRM_NOT_FOUND: &str =
    "Nenhuma ação pendente corresponde a esse código. Talvez já tenha sido resolvida.";
pub(crate) const PT_EMPTY_REPLY: &str =
    "Hmm, não consegui formular uma resposta. Podes reformular a tua pergunta?";
pub(crate) const PT_REPLY_WITHHELD: &str = "A minha resposta não seguiu — misturava detalhes técnicos que não têm lugar aqui. Envia de novo a tua última mensagem e retomamos onde estávamos.";
pub(crate) const PT_GUARDRAIL_TOO_LONG: &str = "Tenho uma resposta mais longa pronta, mas excede o limite configurado. Queres que a resuma mais brevemente?";
pub(crate) const PT_GUARDRAIL_BLOCKED_TOPIC: &str = "Prefiro não abordar esse tema aqui. Vamos manter o foco no teu treino e recuperação. Há algo específico em que possa ajudar?";
pub(crate) const PT_VERIFICATION_WARN_SUFFIX: &str = "⚠️ Algumas afirmações que não consegui sustentar formalmente — corrige-me se alguma estiver errada:";
pub(crate) const PT_VERIFICATION_BLOCK_FALLBACK: &str = "Comecei a responder, mas algumas das afirmações que iria fazer não correspondiam às fontes em que confio. Deixa-me reformular — podes perguntar de novo com um pouco mais de contexto sobre o que queres entender?";
/// Portuguese canonical refusal for off-scope requests.
pub(crate) const PT_SCOPE_REFUSAL: &str =
    "Isso está fora do que posso ajudar — sou o teu assistente de fitness.";
/// Portuguese canonical refusal for missing-capability requests.
pub(crate) const PT_CAPABILITY_REFUSAL: &str =
    "Não consigo fazer isso com as ferramentas que tenho.";
/// Portuguese Nutrition-coach carve-out for the generic scope list.
pub(crate) const PT_COACH_SCOPE_CARVE_OUT_NUTRITION: &str = "## Esclarecimento para o teu papel de coach de nutrição\n\nComo coach de nutrição, as perguntas sobre refeições, jantares, pequenos-almoços, lanches, escolhas alimentares e planeamento de refeições ligado ao treino estão plenamente no teu domínio. Responde diretamente, apoiando-te nos dados de treino do utilizador (intensidade, duração, gasto energético). A regra «procura de comida fora do âmbito» acima visa apenas procura de restaurantes, apps de entrega e menus online — não cobre conselhos nutricionais baseados em evidência nem planeamento de refeições fundado nos dados de treino. Nunca recuses «o que comer depois da corrida», «ideias para o jantar após treinar», «lanche pós-treino» ou equivalente.";
/// Portuguese Recipes-coach carve-out for the generic scope list.
pub(crate) const PT_COACH_SCOPE_CARVE_OUT_RECIPES: &str = "## Esclarecimento para o teu papel de coach de receitas e planeamento de refeições\n\nComo coach de receitas e planeamento de refeições, ideias de receitas, sugestões de pratos e escolhas alimentares ajustadas ao treino estão plenamente no teu domínio. A regra «procura de comida fora do âmbito» acima visa restaurantes e apps de entrega — não receitas caseiras nem planeamento de refeições baseado nos dados de treino do utilizador. Responde diretamente a estes pedidos.";
/// Portuguese placeholder shown while the LLM is composing its reply.
pub(crate) const PT_THINKING_PLACEHOLDER: &str = "a pensar…";
/// Portuguese unknown-command reply.
pub(crate) const PT_UNKNOWN_COMMAND: &str =
    "Comando desconhecido. Escreve /help para ver os comandos disponíveis.";
/// Portuguese progress status during prompt-assembly.
pub(crate) const PT_STATUS_READING_QUESTION: &str = "a ler a tua pergunta…";
/// Portuguese progress status during LLM dispatch.
pub(crate) const PT_STATUS_GENERATING_RESPONSE: &str = "a gerar resposta…";
/// Portuguese progress status for tool-call start. `{0}` = tool name.
pub(crate) const PT_STATUS_CALLING_TOOL: &str = "a chamar {0}…";
/// Portuguese progress status for pipeline error. `{0}` = error text.
pub(crate) const PT_STATUS_ERROR: &str = "erro: {0}";
/// Portuguese coach-proposal lead-in with profile. `{0}` = sport, `{1}` = count.
pub(crate) const PT_COACH_PROPOSAL_WELCOME: &str =
    "Bem-vindo! Com base no teu treino recente de {0}, aqui estão {1} treinadores para ti:\n\n";
/// Portuguese coach-proposal cold-start lead-in. `{0}` = count.
pub(crate) const PT_COACH_PROPOSAL_WELCOME_GENERIC: &str =
    "Bem-vindo! Aqui estão {0} treinadores para começar:\n\n";
/// Portuguese coach-proposal footer.
pub(crate) const PT_COACH_PROPOSAL_FOOTER: &str =
    "\nResponde com um número para começar, ou pergunta-me o que quiseres.";
/// Portuguese default for [`KEY_REGISTRATION_APPROVED`].
pub(crate) const PT_REGISTRATION_APPROVED: &str =
    "🎉 A tua conta Dravr foi aprovada! Já podes falar com o teu coach aqui. Faz-me a tua primeira pergunta quando quiseres.";
/// Portuguese default for [`KEY_BACKFILL_READY`]. `{0}` = activity count.
pub(crate) const PT_BACKFILL_READY: &str =
    "✅ O teu histórico está pronto — {0} atividades carregadas. Pergunta-me de novo o que procuravas.";
/// Portuguese default for [`KEY_BACKFILL_LIST_HEADER`]. `{0}` = activity count.
pub(crate) const PT_BACKFILL_LIST_HEADER: &str = "✅ O teu histórico está pronto — {0} atividades:";
/// Portuguese default for [`KEY_BACKFILL_LIST_MORE`]. `{0}` = remaining count.
pub(crate) const PT_BACKFILL_LIST_MORE: &str = "… e mais {0}";

pub(crate) const PT_LINK_FALLBACK_PROMPT: &str = "Para falar com o Dravr, liga primeiro a tua conta. Abre a app web do Dravr para ligar este canal.";
pub(crate) const PT_LINK_INITIAL_PROMPT: &str = "Olá! Para falar com o Dravr, liga primeiro a tua conta:\n{0}\n\nEste link expira em 10 minutos.";
pub(crate) const PT_LINK_EMAIL_PROMPT: &str =
    "Olá! Para ligares a tua conta Dravr, escreve o teu e-mail.\nEscreve «cancel» para parar.";
pub(crate) const PT_LINK_LOGOUT_COMPLETE: &str = "Saíste da sessão do Dravr. Envia uma mensagem a qualquer momento para ligar novamente a tua conta.";
pub(crate) const PT_LINK_CANCELLED: &str =
    "Ligação cancelada. Envia uma mensagem a qualquer momento para começar de novo.";
pub(crate) const PT_LINK_GENERIC_ERROR: &str = "Algo correu mal. Tenta de novo mais tarde.";
pub(crate) const PT_LINK_SIGNUP_OFFER: &str = "Nenhuma conta Dravr com {0}. Posso criar uma agora mesmo - responde «sim» para continuar, ou «cancel» para parar.";
pub(crate) const PT_LINK_SIGNUP_CREATED: &str =
    "Conta criada. Vou enviar-te um codigo por e-mail para confirmar que o endereco e teu.";
pub(crate) const PT_LINK_SIGNUP_FAILED: &str =
    "Nao consegui criar a conta. Tenta novamente daqui a pouco, ou escreve «cancel» para parar.";
pub(crate) const PT_LINK_NO_TENANT: &str =
    "Esta conta não está associada a nenhuma organização. Contacta o suporte.";
pub(crate) const PT_LINK_EMAIL_NOT_CONFIGURED: &str =
    "O envio de e-mails não está configurado. Contacta o teu administrador.";
pub(crate) const PT_LINK_EMAIL_SEND_FAILED: &str =
    "Não foi possível enviar o e-mail de verificação. Tenta de novo mais tarde.";
pub(crate) const PT_LINK_INVALID_EMAIL: &str =
    "Isto não parece um endereço de e-mail. Escreve o e-mail da tua conta Dravr.";
pub(crate) const PT_LINK_OTP_SENT: &str = "Enviei um código de 6 dígitos para {0}. Escreve-o aqui nos próximos 10 minutos.\nEscreve «cancel» para parar.";
pub(crate) const PT_LINK_TOO_MANY_ATTEMPTS: &str = "Demasiadas tentativas incorretas. A sessão de ligação foi cancelada. Envia uma mensagem para começar de novo.";
pub(crate) const PT_LINK_INCORRECT_CODE: &str = "Código incorreto. Tens {0} tentativa(s) restantes. Tenta de novo ou escreve «cancel» para parar.";
pub(crate) const PT_LINK_VERIFICATION_ERROR: &str =
    "Algo correu mal a verificar a tua conta. Tenta de novo.";
pub(crate) const PT_LINK_IDENTITY_COLLISION: &str = "Não foi possível ligar a tua conta. Esta identidade de canal pode já estar ligada a outra conta.";
pub(crate) const PT_LINK_OTP_PROMPT: &str =
    "Escreve o código de 6 dígitos enviado por e-mail, ou escreve «cancel» para parar.";
pub(crate) const PT_LINK_SESSION_EXPIRED: &str =
    "A tua sessão de ligação expirou. Envia uma mensagem para começar de novo.";
pub(crate) const PT_LINK_SUCCESS: &str = "A tua conta foi ligada com sucesso! Já podes falar com o Dravr através deste canal.\n\nEscreve «logout» a qualquer momento para desligar.";
pub(crate) const PT_ACCOUNT_PENDING: &str = "A tua conta está ligada a este canal, mas ainda aguarda aprovação de um administrador. Vais poder falar com o Dravr assim que for ativada — avisamos-te por aqui.";
pub(crate) const PT_ACCOUNT_SUSPENDED: &str =
    "A tua conta Dravr está suspensa. Contacta o suporte para restabelecer o acesso.";
pub(crate) const PT_RATE_LIMITED: &str = "Estás a enviar mensagens um pouco mais depressa do que o teu plano permite. Aguarda um momento e tenta novamente — eu estarei aqui.";
pub(crate) const PT_QUOTA_EXCEEDED: &str = "Atingiste o limite de conversa do teu plano por agora. Ele repõe-se automaticamente — volta um pouco mais tarde.";
pub(crate) const PT_QUOTA_WARNING: &str =
    "Nota: já usaste {0} de {1} do teu plano. O contador repõe-se a {2}.";
pub(crate) const PT_NO_PROVIDER_CONNECTED: &str = "Antes de conversarmos, liga um serviço de fitness (Strava, Garmin, Whoop) na app Dravr — sem ele não tenho dados de atividade para te ajudar.\n\nLiga-te aqui:\n{0}";
pub(crate) const PT_NO_PROVIDER_CONNECTED_WITH_EMAIL: &str = "Antes de conversarmos, liga um serviço de fitness (Strava, Garmin, Whoop) na app Dravr — sem ele não tenho dados de atividade para te ajudar.\n\nEntra com a tua conta {1} aqui:\n{0}";
pub(crate) const PT_CONNECT_PROMPT: &str = "Vamos ligar o teu serviço de fitness (Strava, Garmin ou Whoop) para que eu possa ajudar-te com os teus dados reais. Toca no botão abaixo para te ligares em segurança.";
pub(crate) const PT_CONNECT_BUTTON: &str = "Ligar a minha conta";
pub(crate) const PT_CONNECT_TITLE: &str = "Ligar um serviço de fitness";

pub(crate) const PT_PROVIDER_REAUTH_REQUIRED: &str = "A tua ligação ao {0} expirou — não consigo aceder aos teus dados de momento. Liga novamente a tua conta aqui (link válido por 24 horas):\n\n{1}\n\nDepois de te reconectares, volta a perguntar-me.";
pub(crate) const PT_PROVIDER_REAUTH_REQUIRED_NO_LINK: &str = "A tua ligação ao {0} expirou — não consigo aceder aos teus dados de momento. Liga novamente o {0} nas definições e volta a perguntar-me.";
pub(crate) const PT_PROVIDER_RECONNECT_BUTTON: &str = "Ligar novamente {0}";

pub(crate) const PT_STATUS_HEADER: &str = "O teu estado no Dravr:\n";
pub(crate) const PT_STATUS_PROVIDERS_NONE: &str = "\nFornecedores: nenhum ligado";
pub(crate) const PT_STATUS_PROVIDERS_LABEL: &str = "\nFornecedores: {0}";
pub(crate) const PT_STATUS_GROUPS_LABEL: &str = "\nGrupos: {0}";
pub(crate) const PT_STATUS_CHANNEL_LABEL: &str = "\nCanal: {0}";

pub(crate) const PT_HELP_HEADER: &str = "Comandos disponíveis:\n";
pub(crate) const PT_HELP_DOMAIN_GENERAL: &str = "Geral";
pub(crate) const PT_HELP_DOMAIN_GROUP: &str = "Coaching em grupo";
pub(crate) const PT_HELP_DOMAIN_COACH: &str = "Coaching";
pub(crate) const PT_HELP_DOMAIN_DATA: &str = "Dados de atividade";
pub(crate) const PT_HELP_DOMAIN_PROVIDER: &str = "Fornecedores";
pub(crate) const PT_HELP_DOMAIN_ACCOUNT: &str = "Conta";
pub(crate) const PT_HELP_DOMAIN_TRAINING: &str = "Treino";
pub(crate) const PT_HELP_DOMAIN_DISCOVER: &str = "Catálogo de coaches";
pub(crate) const PT_HELP_FOOTER: &str = "\nOu escreve-me para conversar com o teu coach.";

pub(crate) const PT_LOGOUT_CONFIRM_PROMPT: &str = "Isto vai desvincular a tua conta {0} do Dravr.\nVais precisar de voltar a ligá-la para usar o messaging.\n\nEscreve «logout» para confirmar.";

pub(crate) const PT_PRIVACY_STATUS_LINE: &str = "O consentimento de estatísticas anónimas está atualmente <b>{0}</b>.\n\nUsa <code>/privacy on</code> para ativar ou <code>/privacy off</code> para desativar.";
pub(crate) const PT_PRIVACY_STATUS_ENABLED: &str = "ativado";
pub(crate) const PT_PRIVACY_STATUS_DISABLED: &str = "desativado";
pub(crate) const PT_PRIVACY_ON_CONFIRMATION: &str = "O consentimento de estatísticas anónimas está agora <b>ativado</b>. Obrigado por ajudares a melhorar o Dravr!\n\nUsa <code>/privacy off</code> para cancelar a qualquer momento.";
pub(crate) const PT_PRIVACY_OFF_CONFIRMATION: &str = "O consentimento de estatísticas anónimas está agora <b>desativado</b>. Não serão recolhidos dados de uso anónimos.\n\nUsa <code>/privacy on</code> para voltar a ativar a qualquer momento.";
pub(crate) const PT_TIMEZONE_SET: &str = "Fuso horário definido para <b>{0}</b>. As horas de início das tuas atividades passam a aparecer neste fuso horário.";
pub(crate) const PT_TIMEZONE_INVALID: &str = "Fuso horário inválido. Indica um nome IANA, por exemplo <code>/timezone America/Toronto</code>.";

pub(crate) const PT_PILLARS_OPENER: &str = "Vamos construir o teu perfil juntos — vou fazer-te perguntas sobre alguns temas, um a um. Para começar: qual é a razão profunda pela qual treinas — o teu «North Star»?";
pub(crate) const PT_PILLARS_DM_ONLY: &str = "O perfil constrói-se em privado. Escreve-me <code>/pillars</code> em mensagem direta e começamos.";

pub(crate) const PT_INTAKE_OPENER: &str = "Antes de começarmos, algumas perguntas rápidas — um minuto agora, e o acompanhamento seguinte é mais preciso e mais seguro.";
pub(crate) const PT_INTAKE_PERSONA: &str = "Primeiro: treinas para ti, ou treinas outras pessoas?\n\n<b>1</b> — Treino para mim\n<b>2</b> — Treino outras pessoas";
pub(crate) const PT_INTAKE_PARQ_INTRO: &str = "Agora sete perguntas de saúde padrão (o PAR-Q+). Um «sim» não bloqueia nada — apenas indica ao teu coach onde ser prudente.";
pub(crate) const PT_INTAKE_PARQ_HEART_CONDITION: &str =
    "Algum médico já te disse que tens um problema cardíaco?";
pub(crate) const PT_INTAKE_PARQ_CHEST_PAIN: &str =
    "Sentes dor no peito em repouso, nas atividades diárias ou durante o exercício?";
pub(crate) const PT_INTAKE_PARQ_DIZZINESS: &str =
    "Perdes o equilíbrio por tonturas, ou perdeste a consciência nos últimos 12 meses?";
pub(crate) const PT_INTAKE_PARQ_CHRONIC_CONDITION: &str =
    "Foste diagnosticado com outra doença crónica (que não seja doença cardíaca)?";
pub(crate) const PT_INTAKE_PARQ_MEDICATION: &str =
    "Tomas atualmente medicamentos prescritos para uma doença crónica?";
pub(crate) const PT_INTAKE_PARQ_JOINT_PROBLEM: &str = "Tens algum problema ósseo, articular ou dos tecidos moles que a atividade física possa agravar?";
pub(crate) const PT_INTAKE_PARQ_SUPERVISED_ONLY: &str =
    "Um médico disse-te que só deverias fazer atividade física sob supervisão médica?";
pub(crate) const PT_INTAKE_YESNO_HINT: &str = "Responde <b>1</b> para sim, <b>2</b> para não.";
pub(crate) const PT_INTAKE_RETRY: &str =
    "Desculpa — preciso só do número, para registar corretamente.\n\n{0}";
pub(crate) const PT_INTAKE_COMPLETE_CLEAR: &str =
    "É tudo, nada a assinalar. Pergunta-me o que quiseres sobre o teu treino.";
pub(crate) const PT_INTAKE_COMPLETE_FLAGGED: &str = "É tudo. Anotei {0} ponto(s) para o teu coach ter em conta — nada disto te impede de treinar, e vale a pena falar com o teu médico antes de forçares.";
pub(crate) const PT_PILLARS_START_FAILED: &str =
    "Não consegui iniciar o perfil nesta conversa. Tenta outra vez dentro de um momento.";
pub(crate) const PT_RESET_WALK_INTERRUPTED: &str =
    "\n\nO teu perfil estava em curso — escreve <code>/pillars</code> para retomar.";

pub(crate) const PT_CALIBRATE_OPENER: &str = "Vamos calibrar a dificuldade do teu treino. Parto dos teus dados recentes e faço-te seis perguntas curtas, uma a uma. Primeira: como queres que fique mais duro — mais horas, mais dias fortes, ou saídas longas mais longas?";
pub(crate) const PT_CALIBRATE_DM_ONLY: &str = "A calibração faz-se em privado. Escreve-me <code>/calibrate</code> em mensagem direta e começamos.";
pub(crate) const PT_CALIBRATE_START_FAILED: &str =
    "Não consegui iniciar a calibração nesta conversa. Tenta outra vez dentro de um momento.";
pub(crate) const PT_CALIBRATE_COMPLETE_HEADER: &str =
    "Calibração terminada — retive {0} respostas de {1}.";
pub(crate) const PT_CALIBRATE_COMPLETE_MISSING: &str = "Falta-me a tua resposta sobre {0}. É isso que limita até onde te posso empurrar, por isso vamos repetir essa pergunta — escreve <code>/calibrate</code>.";
pub(crate) const PT_CALIBRATE_FOLLOWUP_PLAN: &str =
    "Queres que reconstrua as tuas próximas semanas com isto?";
pub(crate) const PT_CALIBRATE_FOLLOWUP_NO_PLAN: &str = "Queres que te construa um plano com isto?";
pub(crate) const PT_CALIBRATE_TOPIC_INJURY: &str = "as tuas lesões e queixas recentes";
pub(crate) const PT_CALIBRATE_TOPIC_RECOVERY: &str = "a tua velocidade de recuperação";

pub(crate) const PT_PLAN_GOAL_LINE: &str = "Objetivo: {0} — {1} ({2} dias)";
pub(crate) const PT_PLAN_BLOCK_LINE: &str = "Bloco: {0}{1}";
pub(crate) const PT_PLAN_DAY_LINE: &str = "{0}: {1}";
pub(crate) const PT_PLAN_REST: &str = "descanso";
pub(crate) const PT_PLAN_TODAY: &str = "Hoje";
pub(crate) const PT_PLAN_TOMORROW: &str = "Amanhã";
pub(crate) const PT_PLAN_WEEK_HEADER: &str = "Semana de {0}{1}";
pub(crate) const PT_PLAN_NO_SESSION: &str = "nada planeado";
pub(crate) const PT_PLAN_NO_COVERAGE: &str = "não coberto pelo plano";
pub(crate) const PT_PLAN_RESUMES: &str = "O plano recomeça a {0}.";
pub(crate) const PT_PLAN_EMPTY: &str =
    "Ainda não há plano guardado — pede ao teu coach para construir um até ao teu objetivo.";
pub(crate) const PT_PLAN_STALE_GOAL: &str =
    "\n\n⚠️ O teu objetivo mudou desde este plano — pede ao teu coach para o atualizar.";

pub(crate) const PT_GROUP_LIST_EMPTY: &str =
    "Não és membro de nenhum grupo.\nCria ou junta-te a um grupo pela app web ou móvel.";
pub(crate) const PT_GROUP_LIST_HEADER: &str = "Os teus grupos ({0}):\n";
pub(crate) const PT_GROUP_LIST_ITEM: &str = "- {0} ({1} membros) [{2}]";
pub(crate) const PT_GROUP_NOT_A_MEMBER: &str = "Não és membro de nenhum grupo";
pub(crate) const PT_GROUP_STATUS_SUMMARY: &str =
    "{0} — estatísticas:\n- Membros: {1}\n- Ativos: {2}\n- Partilha entre pares: {3}";
pub(crate) const PT_GROUP_PEER_SHARING_ON: &str = "ativada";
pub(crate) const PT_GROUP_PEER_SHARING_OFF: &str = "desativada";
pub(crate) const PT_GROUP_MEMBERS_HEADER: &str = "{0} — membros ({1}):\n";
pub(crate) const PT_GROUP_MEMBERS_UNKNOWN: &str = "Desconhecido";
pub(crate) const PT_GROUP_MEMBERS_ITEM: &str = "- {0} [{1}]";
pub(crate) const PT_GROUP_ROLE_OWNER: &str = "proprietário";
pub(crate) const PT_GROUP_ROLE_ADMIN: &str = "admin";
pub(crate) const PT_GROUP_ROLE_MEMBER: &str = "membro";
pub(crate) const PT_GROUP_INVITE_FORBIDDEN: &str =
    "Apenas administradores e proprietários podem gerar links de convite.";
pub(crate) const PT_GROUP_INVITE_BODY: &str =
    "Link de convite para {0}:\nhttps://app.dravr.ai/groups/join/{1}\n\nCódigo: {2}\nVálido 7 dias.";
pub(crate) const PT_GROUP_INVITE_UNAVAILABLE: &str = "Convites de grupo não estão disponíveis.";
pub(crate) const PT_COACH_INVITE_BODY: &str =
    "Convite de coach para {0} — quem o usar torna-se o coach humano do grupo:\nhttps://app.dravr.ai/groups/join/{1}\n\nCódigo: {2}\nVálido 7 dias.";
pub(crate) const PT_GROUP_LEAVE_PROMPT: &str =
    "Tens a certeza que queres sair de {0}?\nEscreve «YES» para confirmar.";
pub(crate) const PT_GROUP_CONSENT_USAGE: &str = "Uso: /group consent yes  ou  /group consent no";
pub(crate) const PT_GROUP_RESPOND_USAGE: &str =
    "Uso: /group respond mentions  ou  /group respond all";
pub(crate) const PT_GROUP_RESPOND_MENTIONS: &str = "O treinador agora só responde quando é chamado (mencione-o com @ ou responda a uma das suas mensagens). Continua a acompanhar a conversa para manter o contexto.";
pub(crate) const PT_GROUP_RESPOND_ALL: &str =
    "O treinador volta a responder a todas as mensagens do grupo.";
pub(crate) const PT_GROUP_COACH_DETACHED: &str = "{0} já não tem um treinador humano associado.";
pub(crate) const PT_NOTIFICATION_CHANNEL_BODY: &str = "🔔 {0}\n\n{1}";
pub(crate) const PT_GROUP_RESPOND_STATUS_MENTIONS: &str = "O treinador só responde quando é chamado. Para voltar a todas as mensagens: /group respond all";
pub(crate) const PT_GROUP_CONSENT_UPDATED: &str =
    "Partilhar os teus dados com os outros membros de {1} está agora {0}.";
pub(crate) const PT_GROUP_CREATE_USAGE: &str =
    "Uso: /group create nome-do-grupo — por exemplo /group create Corredores de domingo";
pub(crate) const PT_GROUP_CREATE_NO_COACH: &str =
    "Escolhe primeiro o coach do grupo: /coach add @handle nesta conversa e volta a correr /group create.";
pub(crate) const PT_GROUP_CREATE_UNAVAILABLE: &str =
    "O coaching em grupo não está incluído no teu plano.";
pub(crate) const PT_GROUP_CREATE_FORBIDDEN: &str =
    "A criação de grupos está reservada aos administradores do teu espaço.";
pub(crate) const PT_GROUP_CREATED: &str =
    "Grupo «{0}» criado com o coach {1}. Convida membros com /group invite.";
pub(crate) const PT_GROUP_INVITE_LABEL: &str = "Convidar membros";
pub(crate) const PT_GROUP_JOIN_INVALID_CODE: &str =
    "Esse código de convite não é válido ou expirou. Pede um novo link a um admin do grupo (/group invite).";
pub(crate) const PT_GROUP_JOIN_ALREADY_MEMBER: &str = "Já és membro de {0}.";
pub(crate) const PT_GROUP_JOIN_FULL: &str = "{0} está cheio.";
pub(crate) const PT_GROUP_JOINED: &str =
    "Bem-vindo a {0}! A conversa do grupo já está na tua lista de conversas.";
pub(crate) const PT_GROUP_JOINED_AS_COACH: &str = "És agora o coach humano de {0}.";
pub(crate) const PT_DISCOVER_CARD_TITLE: &str = "Catálogo de coaches";
pub(crate) const PT_DISCOVER_ITEM: &str = "• {0} — @{1} [{2}]\n  {3}\n";
pub(crate) const PT_DISCOVER_EMPTY: &str =
    "Nenhum coach do catálogo corresponde a «{0}». Escreve /discover para veres os mais recentes.";
pub(crate) const PT_DISCOVER_CATALOGUE_EMPTY: &str = "O catálogo de coaches está vazio.";
pub(crate) const PT_DISCOVER_MORE_LABEL: &str = "Mais";
pub(crate) const PT_DISCOVER_INSTALL_USAGE: &str =
    "Uso: /discover install @handle — o @handle aparece em /discover.";
pub(crate) const PT_DISCOVER_INSTALL_UNKNOWN_HANDLE: &str =
    "Nenhum coach publicado responde a {0}. Escreve /discover para explorares o catálogo.";
pub(crate) const PT_DISCOVER_INSTALLED: &str =
    "{0} está instalado. Usa-o em qualquer conversa: /coach add @{1}, ou menciona @{1} numa mensagem para uma única resposta.";
pub(crate) const PT_DISCOVER_INSTALL_ALREADY: &str =
    "{0} já está instalado. Usa-o em qualquer conversa: /coach add @{1}, ou menciona @{1} numa mensagem para uma única resposta.";
pub(crate) const PT_DISCOVER_ADD_LABEL: &str = "Usar aqui";

pub(crate) const PT_COACH_LIST_EMPTY: &str =
    "Nenhum treinador na tua lista. Escreve /discover para explorares o catálogo, ou /coach create para criares um a partir desta conversa.";
pub(crate) const PT_COACH_LIST_CARD_TITLE: &str = "Os teus treinadores";
pub(crate) const PT_COACH_LIST_ITEM: &str = "• {0} — @{1}\n  {2}\n";
pub(crate) const PT_COACH_LIST_ITEM_NO_HANDLE: &str = "• {0}\n  {1}\n";
pub(crate) const PT_COACH_LIST_FOOTER: &str =
    "Menciona @handle numa mensagem para lhe confiares só essa mensagem, ou escreve /coach add @handle para que responda aqui a partir de agora.";
pub(crate) const PT_COACH_NO_DESCRIPTION: &str = "Sem descrição";
pub(crate) const PT_COACH_GROUP_UPDATED: &str = "Coach atualizado para {0} no grupo {1}.";
pub(crate) const PT_COACH_USER_UPDATED: &str = "Treinador selecionado: {0}.";
pub(crate) const PT_COACH_ASSIGN_NOT_A_MEMBER: &str = "Não és membro deste grupo";
pub(crate) const PT_COACH_ASSIGN_FORBIDDEN: &str =
    "Apenas administradores e proprietários do grupo podem mudar o coach.";
pub(crate) const PT_COACH_ADD_USAGE: &str =
    "Indica que treinador adicionar: /coach add @handle. Escreve /coach para veres a tua lista.";
pub(crate) const PT_COACH_ADD_UNKNOWN: &str =
    "Nenhum treinador instalado responde a {0}. Escreve /coach para veres a tua lista, ou /discover para o instalares.";
pub(crate) const PT_COACH_REMOVE_GROUP_THREAD: &str =
    "Num grupo, o treinador pertence ao grupo — usa /group coach para o mudares.";
pub(crate) const PT_COACH_REMOVE_NOTHING: &str = "Nenhum treinador está associado a esta conversa.";
pub(crate) const PT_COACH_REMOVED: &str = "{0} já não responde nesta conversa.";
pub(crate) const PT_COACH_CREATE_NO_CONVERSATION: &str =
    "Abre primeiro uma conversa: /coach create redige um treinador a partir do que lá foi dito.";
pub(crate) const PT_COACH_CREATE_EMPTY: &str =
    "Esta conversa ainda está vazia. Troca primeiro algumas mensagens com o teu treinador e volta a lançar /coach create.";
pub(crate) const PT_COACH_CREATE_USAGE: &str =
    "Utilização: /coach create para propor um treinador a partir desta conversa, depois /coach create confirm token para o criares.";
pub(crate) const PT_COACH_CREATE_CARD_TITLE: &str = "Rascunho de treinador";
pub(crate) const PT_COACH_CREATE_PROPOSAL_BODY: &str =
    "{0}\n{1}\n\nCategoria: {2}\nEtiquetas: {3}\n\nResponde /coach create confirm {4} para o criares, ou /deny {4} para o descartares. O rascunho expira em 10 minutos.";
pub(crate) const PT_COACH_CREATE_CONFIRM_LABEL: &str = "Criar";
pub(crate) const PT_COACH_CREATE_DISCARD_LABEL: &str = "Descartar";
pub(crate) const PT_COACH_CREATE_QUOTA: &str =
    "Já tens {0} treinadores, o máximo do teu plano ({1}). Elimina um a partir do Discover antes de criares outro.";
pub(crate) const PT_COACH_CREATE_DONE: &str =
    "Treinador {0} criado — @{1}. Responde aqui a partir da tua próxima mensagem. Escreve /coach add @{1} em qualquer outra conversa, ou edita-o a partir da sua página no Discover.";
pub(crate) const PT_COACH_CREATE_DONE_UNBOUND: &str =
    "Treinador {0} criado — @{1}. Escreve /coach add @{1} numa das tuas conversas para o usares, ou edita-o a partir da sua página no Discover.";
pub(crate) const PT_COACH_CREATE_DISCARDED: &str =
    "Rascunho descartado. Volta a lançar /coach create quando quiseres.";

/// Compiled-in `(key, locale, content)` triples loaded into the registry
/// at construction. Any new locale added here automatically becomes
/// available as a fallback target without code changes at call sites.
#[rustfmt::skip]
const COMPILED_IN: &[(&str, &str, &str)] = &[
    // ── French (DEFAULT_LOCALE) ─────────────────────────────────────────
    (KEY_COMMITMENT_MET, "fr", FR_COMMITMENT_MET),
    (KEY_COMMITMENT_PARTIAL, "fr", FR_COMMITMENT_PARTIAL),
    (KEY_COMMITMENT_MISSED, "fr", FR_COMMITMENT_MISSED),
    (KEY_COMMITMENT_ACTIVITY_ANY, "fr", FR_COMMITMENT_ACTIVITY_ANY),
    (KEY_COMMITMENT_PUSH_TITLE, "fr", FR_COMMITMENT_PUSH_TITLE),
    (KEY_ERROR_GENERIC, "fr", FR_ERROR_GENERIC),
    (KEY_GUARDIAN_DENIED, "fr", FR_GUARDIAN_DENIED),
    (KEY_GUARDIAN_CONFIRM_PROMPT, "fr", FR_GUARDIAN_CONFIRM_PROMPT),
    (KEY_GUARDIAN_CONFIRM_DONE, "fr", FR_GUARDIAN_CONFIRM_DONE),
    (KEY_GUARDIAN_CONFIRM_FAILED, "fr", FR_GUARDIAN_CONFIRM_FAILED),
    (KEY_GUARDIAN_CONFIRM_DENIED, "fr", FR_GUARDIAN_CONFIRM_DENIED),
    (KEY_GUARDIAN_CONFIRM_EXPIRED, "fr", FR_GUARDIAN_CONFIRM_EXPIRED),
    (KEY_GUARDIAN_CONFIRM_NOT_FOUND, "fr", FR_GUARDIAN_CONFIRM_NOT_FOUND),
    (KEY_EMPTY_REPLY, "fr", FR_EMPTY_REPLY),
    (KEY_REPLY_WITHHELD, "fr", FR_REPLY_WITHHELD),
    (KEY_GUARDRAIL_TOO_LONG, "fr", FR_GUARDRAIL_TOO_LONG),
    (KEY_GUARDRAIL_BLOCKED_TOPIC, "fr", FR_GUARDRAIL_BLOCKED_TOPIC),
    (KEY_VERIFICATION_WARN_SUFFIX, "fr", FR_VERIFICATION_WARN_SUFFIX),
    (KEY_VERIFICATION_BLOCK_FALLBACK, "fr", FR_VERIFICATION_BLOCK_FALLBACK),
    (KEY_SCOPE_REFUSAL, "fr", FR_SCOPE_REFUSAL),
    (KEY_CAPABILITY_REFUSAL, "fr", FR_CAPABILITY_REFUSAL),
    (
        KEY_COACH_SCOPE_CARVE_OUT_NUTRITION,
        "fr",
        FR_COACH_SCOPE_CARVE_OUT_NUTRITION,
    ),
    (
        KEY_COACH_SCOPE_CARVE_OUT_RECIPES,
        "fr",
        FR_COACH_SCOPE_CARVE_OUT_RECIPES,
    ),
    (KEY_THINKING_PLACEHOLDER, "fr", FR_THINKING_PLACEHOLDER),
    (KEY_UNKNOWN_COMMAND, "fr", FR_UNKNOWN_COMMAND),
    (KEY_STATUS_READING_QUESTION, "fr", FR_STATUS_READING_QUESTION),
    (KEY_STATUS_GENERATING_RESPONSE, "fr", FR_STATUS_GENERATING_RESPONSE),
    (KEY_STATUS_CALLING_TOOL, "fr", FR_STATUS_CALLING_TOOL),
    (KEY_STATUS_ERROR, "fr", FR_STATUS_ERROR),
    (KEY_COACH_PROPOSAL_WELCOME, "fr", FR_COACH_PROPOSAL_WELCOME),
    (
        KEY_COACH_PROPOSAL_WELCOME_GENERIC,
        "fr",
        FR_COACH_PROPOSAL_WELCOME_GENERIC,
    ),
    (KEY_COACH_PROPOSAL_FOOTER, "fr", FR_COACH_PROPOSAL_FOOTER),
    (KEY_REGISTRATION_APPROVED, "fr", FR_REGISTRATION_APPROVED),
    (KEY_BACKFILL_READY, "fr", FR_BACKFILL_READY),
    (KEY_BACKFILL_LIST_HEADER, "fr", FR_BACKFILL_LIST_HEADER),
    (KEY_BACKFILL_LIST_MORE, "fr", FR_BACKFILL_LIST_MORE),
    (KEY_LINK_FALLBACK_PROMPT, "fr", FR_LINK_FALLBACK_PROMPT),
    (KEY_LINK_INITIAL_PROMPT, "fr", FR_LINK_INITIAL_PROMPT),
    (KEY_LINK_EMAIL_PROMPT, "fr", FR_LINK_EMAIL_PROMPT),
    (KEY_LINK_LOGOUT_COMPLETE, "fr", FR_LINK_LOGOUT_COMPLETE),
    (KEY_LINK_CANCELLED, "fr", FR_LINK_CANCELLED),
    (KEY_LINK_GENERIC_ERROR, "fr", FR_LINK_GENERIC_ERROR),
    (KEY_LINK_SIGNUP_OFFER, "fr", FR_LINK_SIGNUP_OFFER),
    (KEY_LINK_SIGNUP_CREATED, "fr", FR_LINK_SIGNUP_CREATED),
    (KEY_LINK_SIGNUP_FAILED, "fr", FR_LINK_SIGNUP_FAILED),
    (KEY_LINK_NO_TENANT, "fr", FR_LINK_NO_TENANT),
    (KEY_LINK_EMAIL_NOT_CONFIGURED, "fr", FR_LINK_EMAIL_NOT_CONFIGURED),
    (KEY_LINK_EMAIL_SEND_FAILED, "fr", FR_LINK_EMAIL_SEND_FAILED),
    (KEY_LINK_INVALID_EMAIL, "fr", FR_LINK_INVALID_EMAIL),
    (KEY_LINK_OTP_SENT, "fr", FR_LINK_OTP_SENT),
    (KEY_LINK_TOO_MANY_ATTEMPTS, "fr", FR_LINK_TOO_MANY_ATTEMPTS),
    (KEY_LINK_INCORRECT_CODE, "fr", FR_LINK_INCORRECT_CODE),
    (KEY_LINK_VERIFICATION_ERROR, "fr", FR_LINK_VERIFICATION_ERROR),
    (KEY_LINK_IDENTITY_COLLISION, "fr", FR_LINK_IDENTITY_COLLISION),
    (KEY_LINK_OTP_PROMPT, "fr", FR_LINK_OTP_PROMPT),
    (KEY_LINK_SESSION_EXPIRED, "fr", FR_LINK_SESSION_EXPIRED),
    (KEY_LINK_SUCCESS, "fr", FR_LINK_SUCCESS),
    (KEY_ACCOUNT_PENDING, "fr", FR_ACCOUNT_PENDING),
    (KEY_ACCOUNT_SUSPENDED, "fr", FR_ACCOUNT_SUSPENDED),
    (KEY_RATE_LIMITED, "fr", FR_RATE_LIMITED),
    (KEY_QUOTA_EXCEEDED, "fr", FR_QUOTA_EXCEEDED),
    (KEY_QUOTA_WARNING, "fr", FR_QUOTA_WARNING),
    (KEY_NO_PROVIDER_CONNECTED, "fr", FR_NO_PROVIDER_CONNECTED),
    (
        KEY_NO_PROVIDER_CONNECTED_WITH_EMAIL,
        "fr",
        FR_NO_PROVIDER_CONNECTED_WITH_EMAIL,
    ),
    (KEY_CONNECT_PROMPT, "fr", FR_CONNECT_PROMPT),
    (KEY_CONNECT_BUTTON, "fr", FR_CONNECT_BUTTON),
    (KEY_CONNECT_TITLE, "fr", FR_CONNECT_TITLE),
    (KEY_PROVIDER_REAUTH_REQUIRED, "fr", FR_PROVIDER_REAUTH_REQUIRED),
    (
        KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK,
        "fr",
        FR_PROVIDER_REAUTH_REQUIRED_NO_LINK,
    ),
    (
        KEY_PROVIDER_RECONNECT_BUTTON,
        "fr",
        FR_PROVIDER_RECONNECT_BUTTON,
    ),
    (KEY_STATUS_HEADER, "fr", FR_STATUS_HEADER),
    (KEY_STATUS_PROVIDERS_NONE, "fr", FR_STATUS_PROVIDERS_NONE),
    (KEY_STATUS_PROVIDERS_LABEL, "fr", FR_STATUS_PROVIDERS_LABEL),
    (KEY_STATUS_GROUPS_LABEL, "fr", FR_STATUS_GROUPS_LABEL),
    (KEY_STATUS_CHANNEL_LABEL, "fr", FR_STATUS_CHANNEL_LABEL),
    (KEY_HELP_HEADER, "fr", FR_HELP_HEADER),
    (KEY_HELP_DOMAIN_GENERAL, "fr", FR_HELP_DOMAIN_GENERAL),
    (KEY_HELP_DOMAIN_GROUP, "fr", FR_HELP_DOMAIN_GROUP),
    (KEY_HELP_DOMAIN_COACH, "fr", FR_HELP_DOMAIN_COACH),
    (KEY_HELP_DOMAIN_DATA, "fr", FR_HELP_DOMAIN_DATA),
    (KEY_HELP_DOMAIN_PROVIDER, "fr", FR_HELP_DOMAIN_PROVIDER),
    (KEY_HELP_DOMAIN_ACCOUNT, "fr", FR_HELP_DOMAIN_ACCOUNT),
    (KEY_HELP_DOMAIN_TRAINING, "fr", FR_HELP_DOMAIN_TRAINING),
    (KEY_HELP_DOMAIN_DISCOVER, "fr", FR_HELP_DOMAIN_DISCOVER),
    (KEY_HELP_FOOTER, "fr", FR_HELP_FOOTER),
    (KEY_LOGOUT_CONFIRM_PROMPT, "fr", FR_LOGOUT_CONFIRM_PROMPT),
    (KEY_RESET_CONFIRM, "fr", FR_RESET_CONFIRM),
    (KEY_PRIVACY_STATUS_LINE, "fr", FR_PRIVACY_STATUS_LINE),
    (KEY_PRIVACY_STATUS_ENABLED, "fr", FR_PRIVACY_STATUS_ENABLED),
    (KEY_PRIVACY_STATUS_DISABLED, "fr", FR_PRIVACY_STATUS_DISABLED),
    (KEY_PRIVACY_ON_CONFIRMATION, "fr", FR_PRIVACY_ON_CONFIRMATION),
    (KEY_PRIVACY_OFF_CONFIRMATION, "fr", FR_PRIVACY_OFF_CONFIRMATION),
    (KEY_TIMEZONE_SET, "fr", FR_TIMEZONE_SET),
    (KEY_TIMEZONE_INVALID, "fr", FR_TIMEZONE_INVALID),
    (KEY_PILLARS_OPENER, "fr", FR_PILLARS_OPENER),
    (KEY_PILLARS_DM_ONLY, "fr", FR_PILLARS_DM_ONLY),
    (KEY_INTAKE_OPENER, "fr", FR_INTAKE_OPENER),
    (KEY_INTAKE_PERSONA, "fr", FR_INTAKE_PERSONA),
    (KEY_INTAKE_PARQ_INTRO, "fr", FR_INTAKE_PARQ_INTRO),
    (KEY_INTAKE_PARQ_HEART_CONDITION, "fr", FR_INTAKE_PARQ_HEART_CONDITION),
    (KEY_INTAKE_PARQ_CHEST_PAIN, "fr", FR_INTAKE_PARQ_CHEST_PAIN),
    (KEY_INTAKE_PARQ_DIZZINESS, "fr", FR_INTAKE_PARQ_DIZZINESS),
    (KEY_INTAKE_PARQ_CHRONIC_CONDITION, "fr", FR_INTAKE_PARQ_CHRONIC_CONDITION),
    (KEY_INTAKE_PARQ_MEDICATION, "fr", FR_INTAKE_PARQ_MEDICATION),
    (KEY_INTAKE_PARQ_JOINT_PROBLEM, "fr", FR_INTAKE_PARQ_JOINT_PROBLEM),
    (KEY_INTAKE_PARQ_SUPERVISED_ONLY, "fr", FR_INTAKE_PARQ_SUPERVISED_ONLY),
    (KEY_INTAKE_YESNO_HINT, "fr", FR_INTAKE_YESNO_HINT),
    (KEY_INTAKE_RETRY, "fr", FR_INTAKE_RETRY),
    (KEY_INTAKE_COMPLETE_CLEAR, "fr", FR_INTAKE_COMPLETE_CLEAR),
    (KEY_INTAKE_COMPLETE_FLAGGED, "fr", FR_INTAKE_COMPLETE_FLAGGED),
    (KEY_PILLARS_START_FAILED, "fr", FR_PILLARS_START_FAILED),
    (KEY_RESET_WALK_INTERRUPTED, "fr", FR_RESET_WALK_INTERRUPTED),
    (KEY_CALIBRATE_OPENER, "fr", FR_CALIBRATE_OPENER),
    (KEY_CALIBRATE_DM_ONLY, "fr", FR_CALIBRATE_DM_ONLY),
    (KEY_CALIBRATE_START_FAILED, "fr", FR_CALIBRATE_START_FAILED),
    (KEY_CALIBRATE_COMPLETE_HEADER, "fr", FR_CALIBRATE_COMPLETE_HEADER),
    (KEY_CALIBRATE_COMPLETE_MISSING, "fr", FR_CALIBRATE_COMPLETE_MISSING),
    (KEY_CALIBRATE_FOLLOWUP_PLAN, "fr", FR_CALIBRATE_FOLLOWUP_PLAN),
    (KEY_CALIBRATE_FOLLOWUP_NO_PLAN, "fr", FR_CALIBRATE_FOLLOWUP_NO_PLAN),
    (KEY_CALIBRATE_TOPIC_INJURY, "fr", FR_CALIBRATE_TOPIC_INJURY),
    (KEY_CALIBRATE_TOPIC_RECOVERY, "fr", FR_CALIBRATE_TOPIC_RECOVERY),
    (KEY_PLAN_GOAL_LINE, "fr", FR_PLAN_GOAL_LINE),
    (KEY_PLAN_BLOCK_LINE, "fr", FR_PLAN_BLOCK_LINE),
    (KEY_PLAN_DAY_LINE, "fr", FR_PLAN_DAY_LINE),
    (KEY_PLAN_REST, "fr", FR_PLAN_REST),
    (KEY_PLAN_TODAY, "fr", FR_PLAN_TODAY),
    (KEY_PLAN_TOMORROW, "fr", FR_PLAN_TOMORROW),
    (KEY_PLAN_WEEK_HEADER, "fr", FR_PLAN_WEEK_HEADER),
    (KEY_PLAN_NO_SESSION, "fr", FR_PLAN_NO_SESSION),
    (KEY_PLAN_NO_COVERAGE, "fr", FR_PLAN_NO_COVERAGE),
    (KEY_PLAN_RESUMES, "fr", FR_PLAN_RESUMES),
    (KEY_PLAN_EMPTY, "fr", FR_PLAN_EMPTY),
    (KEY_PLAN_STALE_GOAL, "fr", FR_PLAN_STALE_GOAL),
    (KEY_GROUP_LIST_EMPTY, "fr", FR_GROUP_LIST_EMPTY),
    (KEY_GROUP_LIST_HEADER, "fr", FR_GROUP_LIST_HEADER),
    (KEY_GROUP_LIST_ITEM, "fr", FR_GROUP_LIST_ITEM),
    (KEY_GROUP_NOT_A_MEMBER, "fr", FR_GROUP_NOT_A_MEMBER),
    (KEY_GROUP_STATUS_SUMMARY, "fr", FR_GROUP_STATUS_SUMMARY),
    (KEY_GROUP_PEER_SHARING_ON, "fr", FR_GROUP_PEER_SHARING_ON),
    (KEY_GROUP_PEER_SHARING_OFF, "fr", FR_GROUP_PEER_SHARING_OFF),
    (KEY_GROUP_MEMBERS_HEADER, "fr", FR_GROUP_MEMBERS_HEADER),
    (KEY_GROUP_MEMBERS_UNKNOWN, "fr", FR_GROUP_MEMBERS_UNKNOWN),
    (KEY_GROUP_MEMBERS_ITEM, "fr", FR_GROUP_MEMBERS_ITEM),
    (KEY_GROUP_ROLE_OWNER, "fr", FR_GROUP_ROLE_OWNER),
    (KEY_GROUP_ROLE_ADMIN, "fr", FR_GROUP_ROLE_ADMIN),
    (KEY_GROUP_ROLE_MEMBER, "fr", FR_GROUP_ROLE_MEMBER),
    (KEY_GROUP_INVITE_FORBIDDEN, "fr", FR_GROUP_INVITE_FORBIDDEN),
    (KEY_GROUP_INVITE_BODY, "fr", FR_GROUP_INVITE_BODY),
    (KEY_GROUP_INVITE_UNAVAILABLE, "fr", FR_GROUP_INVITE_UNAVAILABLE),
    (KEY_COACH_INVITE_BODY, "fr", FR_COACH_INVITE_BODY),
    (KEY_GROUP_LEAVE_PROMPT, "fr", FR_GROUP_LEAVE_PROMPT),
    (KEY_GROUP_CONSENT_USAGE, "fr", FR_GROUP_CONSENT_USAGE),
    (KEY_GROUP_RESPOND_USAGE, "fr", FR_GROUP_RESPOND_USAGE),
    (KEY_GROUP_RESPOND_MENTIONS, "fr", FR_GROUP_RESPOND_MENTIONS),
    (KEY_GROUP_RESPOND_ALL, "fr", FR_GROUP_RESPOND_ALL),
    (KEY_GROUP_RESPOND_STATUS_MENTIONS, "fr", FR_GROUP_RESPOND_STATUS_MENTIONS),
    (KEY_GROUP_COACH_DETACHED, "fr", FR_GROUP_COACH_DETACHED),
    (KEY_NOTIFICATION_CHANNEL_BODY, "fr", FR_NOTIFICATION_CHANNEL_BODY),
    (KEY_GROUP_CONSENT_UPDATED, "fr", FR_GROUP_CONSENT_UPDATED),
    (KEY_GROUP_CREATE_USAGE, "fr", FR_GROUP_CREATE_USAGE),
    (KEY_GROUP_CREATE_NO_COACH, "fr", FR_GROUP_CREATE_NO_COACH),
    (KEY_GROUP_CREATE_UNAVAILABLE, "fr", FR_GROUP_CREATE_UNAVAILABLE),
    (KEY_GROUP_CREATE_FORBIDDEN, "fr", FR_GROUP_CREATE_FORBIDDEN),
    (KEY_GROUP_CREATED, "fr", FR_GROUP_CREATED),
    (KEY_GROUP_INVITE_LABEL, "fr", FR_GROUP_INVITE_LABEL),
    (KEY_GROUP_JOIN_INVALID_CODE, "fr", FR_GROUP_JOIN_INVALID_CODE),
    (KEY_GROUP_JOIN_ALREADY_MEMBER, "fr", FR_GROUP_JOIN_ALREADY_MEMBER),
    (KEY_GROUP_JOIN_FULL, "fr", FR_GROUP_JOIN_FULL),
    (KEY_GROUP_JOINED, "fr", FR_GROUP_JOINED),
    (KEY_GROUP_JOINED_AS_COACH, "fr", FR_GROUP_JOINED_AS_COACH),
    (KEY_DISCOVER_CARD_TITLE, "fr", FR_DISCOVER_CARD_TITLE),
    (KEY_DISCOVER_ITEM, "fr", FR_DISCOVER_ITEM),
    (KEY_DISCOVER_EMPTY, "fr", FR_DISCOVER_EMPTY),
    (KEY_DISCOVER_CATALOGUE_EMPTY, "fr", FR_DISCOVER_CATALOGUE_EMPTY),
    (KEY_DISCOVER_MORE_LABEL, "fr", FR_DISCOVER_MORE_LABEL),
    (KEY_DISCOVER_INSTALL_USAGE, "fr", FR_DISCOVER_INSTALL_USAGE),
    (KEY_DISCOVER_INSTALL_UNKNOWN_HANDLE, "fr", FR_DISCOVER_INSTALL_UNKNOWN_HANDLE),
    (KEY_DISCOVER_INSTALLED, "fr", FR_DISCOVER_INSTALLED),
    (KEY_DISCOVER_INSTALL_ALREADY, "fr", FR_DISCOVER_INSTALL_ALREADY),
    (KEY_DISCOVER_ADD_LABEL, "fr", FR_DISCOVER_ADD_LABEL),
    (KEY_COACH_LIST_EMPTY, "fr", FR_COACH_LIST_EMPTY),
    (KEY_COACH_LIST_CARD_TITLE, "fr", FR_COACH_LIST_CARD_TITLE),
    (KEY_COACH_LIST_ITEM, "fr", FR_COACH_LIST_ITEM),
    (KEY_COACH_LIST_ITEM_NO_HANDLE, "fr", FR_COACH_LIST_ITEM_NO_HANDLE),
    (KEY_COACH_LIST_FOOTER, "fr", FR_COACH_LIST_FOOTER),
    (KEY_COACH_NO_DESCRIPTION, "fr", FR_COACH_NO_DESCRIPTION),
    (KEY_COACH_GROUP_UPDATED, "fr", FR_COACH_GROUP_UPDATED),
    (KEY_COACH_USER_UPDATED, "fr", FR_COACH_USER_UPDATED),
    (KEY_COACH_ASSIGN_NOT_A_MEMBER, "fr", FR_COACH_ASSIGN_NOT_A_MEMBER),
    (KEY_COACH_ASSIGN_FORBIDDEN, "fr", FR_COACH_ASSIGN_FORBIDDEN),
    (KEY_COACH_ADD_USAGE, "fr", FR_COACH_ADD_USAGE),
    (KEY_COACH_ADD_UNKNOWN, "fr", FR_COACH_ADD_UNKNOWN),
    (KEY_COACH_REMOVE_GROUP_THREAD, "fr", FR_COACH_REMOVE_GROUP_THREAD),
    (KEY_COACH_REMOVE_NOTHING, "fr", FR_COACH_REMOVE_NOTHING),
    (KEY_COACH_REMOVED, "fr", FR_COACH_REMOVED),
    (KEY_COACH_CREATE_NO_CONVERSATION, "fr", FR_COACH_CREATE_NO_CONVERSATION),
    (KEY_COACH_CREATE_EMPTY, "fr", FR_COACH_CREATE_EMPTY),
    (KEY_COACH_CREATE_USAGE, "fr", FR_COACH_CREATE_USAGE),
    (KEY_COACH_CREATE_CARD_TITLE, "fr", FR_COACH_CREATE_CARD_TITLE),
    (KEY_COACH_CREATE_PROPOSAL_BODY, "fr", FR_COACH_CREATE_PROPOSAL_BODY),
    (KEY_COACH_CREATE_CONFIRM_LABEL, "fr", FR_COACH_CREATE_CONFIRM_LABEL),
    (KEY_COACH_CREATE_DISCARD_LABEL, "fr", FR_COACH_CREATE_DISCARD_LABEL),
    (KEY_COACH_CREATE_QUOTA, "fr", FR_COACH_CREATE_QUOTA),
    (KEY_COACH_CREATE_DONE, "fr", FR_COACH_CREATE_DONE),
    (KEY_COACH_CREATE_DONE_UNBOUND, "fr", FR_COACH_CREATE_DONE_UNBOUND),
    (KEY_COACH_CREATE_DISCARDED, "fr", FR_COACH_CREATE_DISCARDED),

    // ── English ─────────────────────────────────────────────────────────
    (KEY_COMMITMENT_MET, "en", EN_COMMITMENT_MET),
    (KEY_COMMITMENT_PARTIAL, "en", EN_COMMITMENT_PARTIAL),
    (KEY_COMMITMENT_MISSED, "en", EN_COMMITMENT_MISSED),
    (KEY_COMMITMENT_ACTIVITY_ANY, "en", EN_COMMITMENT_ACTIVITY_ANY),
    (KEY_COMMITMENT_PUSH_TITLE, "en", EN_COMMITMENT_PUSH_TITLE),
    (KEY_SCOPE_REFUSAL, "en", EN_SCOPE_REFUSAL),
    (KEY_CAPABILITY_REFUSAL, "en", EN_CAPABILITY_REFUSAL),
    (
        KEY_COACH_SCOPE_CARVE_OUT_NUTRITION,
        "en",
        EN_COACH_SCOPE_CARVE_OUT_NUTRITION,
    ),
    (
        KEY_COACH_SCOPE_CARVE_OUT_RECIPES,
        "en",
        EN_COACH_SCOPE_CARVE_OUT_RECIPES,
    ),
    (KEY_THINKING_PLACEHOLDER, "en", EN_THINKING_PLACEHOLDER),
    (KEY_UNKNOWN_COMMAND, "en", EN_UNKNOWN_COMMAND),
    (KEY_STATUS_READING_QUESTION, "en", EN_STATUS_READING_QUESTION),
    (KEY_STATUS_GENERATING_RESPONSE, "en", EN_STATUS_GENERATING_RESPONSE),
    (KEY_STATUS_CALLING_TOOL, "en", EN_STATUS_CALLING_TOOL),
    (KEY_STATUS_ERROR, "en", EN_STATUS_ERROR),
    (KEY_COACH_PROPOSAL_WELCOME, "en", EN_COACH_PROPOSAL_WELCOME),
    (
        KEY_COACH_PROPOSAL_WELCOME_GENERIC,
        "en",
        EN_COACH_PROPOSAL_WELCOME_GENERIC,
    ),
    (KEY_COACH_PROPOSAL_FOOTER, "en", EN_COACH_PROPOSAL_FOOTER),
    (KEY_REGISTRATION_APPROVED, "en", EN_REGISTRATION_APPROVED),
    (KEY_BACKFILL_READY, "en", EN_BACKFILL_READY),
    (KEY_BACKFILL_LIST_HEADER, "en", EN_BACKFILL_LIST_HEADER),
    (KEY_BACKFILL_LIST_MORE, "en", EN_BACKFILL_LIST_MORE),
    (KEY_ERROR_GENERIC, "en", EN_ERROR_GENERIC),
    (KEY_GUARDIAN_DENIED, "en", EN_GUARDIAN_DENIED),
    (KEY_GUARDIAN_CONFIRM_PROMPT, "en", EN_GUARDIAN_CONFIRM_PROMPT),
    (KEY_GUARDIAN_CONFIRM_DONE, "en", EN_GUARDIAN_CONFIRM_DONE),
    (KEY_GUARDIAN_CONFIRM_FAILED, "en", EN_GUARDIAN_CONFIRM_FAILED),
    (KEY_GUARDIAN_CONFIRM_DENIED, "en", EN_GUARDIAN_CONFIRM_DENIED),
    (KEY_GUARDIAN_CONFIRM_EXPIRED, "en", EN_GUARDIAN_CONFIRM_EXPIRED),
    (KEY_GUARDIAN_CONFIRM_NOT_FOUND, "en", EN_GUARDIAN_CONFIRM_NOT_FOUND),
    (KEY_EMPTY_REPLY, "en", EN_EMPTY_REPLY),
    (KEY_REPLY_WITHHELD, "en", EN_REPLY_WITHHELD),
    (KEY_GUARDRAIL_TOO_LONG, "en", EN_GUARDRAIL_TOO_LONG),
    (KEY_GUARDRAIL_BLOCKED_TOPIC, "en", EN_GUARDRAIL_BLOCKED_TOPIC),
    (KEY_VERIFICATION_WARN_SUFFIX, "en", EN_VERIFICATION_WARN_SUFFIX),
    (KEY_VERIFICATION_BLOCK_FALLBACK, "en", EN_VERIFICATION_BLOCK_FALLBACK),
    (KEY_LINK_FALLBACK_PROMPT, "en", EN_LINK_FALLBACK_PROMPT),
    (KEY_LINK_INITIAL_PROMPT, "en", EN_LINK_INITIAL_PROMPT),
    (KEY_LINK_EMAIL_PROMPT, "en", EN_LINK_EMAIL_PROMPT),
    (KEY_LINK_LOGOUT_COMPLETE, "en", EN_LINK_LOGOUT_COMPLETE),
    (KEY_LINK_CANCELLED, "en", EN_LINK_CANCELLED),
    (KEY_LINK_GENERIC_ERROR, "en", EN_LINK_GENERIC_ERROR),
    (KEY_LINK_SIGNUP_OFFER, "en", EN_LINK_SIGNUP_OFFER),
    (KEY_LINK_SIGNUP_CREATED, "en", EN_LINK_SIGNUP_CREATED),
    (KEY_LINK_SIGNUP_FAILED, "en", EN_LINK_SIGNUP_FAILED),
    (KEY_LINK_NO_TENANT, "en", EN_LINK_NO_TENANT),
    (KEY_LINK_EMAIL_NOT_CONFIGURED, "en", EN_LINK_EMAIL_NOT_CONFIGURED),
    (KEY_LINK_EMAIL_SEND_FAILED, "en", EN_LINK_EMAIL_SEND_FAILED),
    (KEY_LINK_INVALID_EMAIL, "en", EN_LINK_INVALID_EMAIL),
    (KEY_LINK_OTP_SENT, "en", EN_LINK_OTP_SENT),
    (KEY_LINK_TOO_MANY_ATTEMPTS, "en", EN_LINK_TOO_MANY_ATTEMPTS),
    (KEY_LINK_INCORRECT_CODE, "en", EN_LINK_INCORRECT_CODE),
    (KEY_LINK_VERIFICATION_ERROR, "en", EN_LINK_VERIFICATION_ERROR),
    (KEY_LINK_IDENTITY_COLLISION, "en", EN_LINK_IDENTITY_COLLISION),
    (KEY_LINK_OTP_PROMPT, "en", EN_LINK_OTP_PROMPT),
    (KEY_LINK_SESSION_EXPIRED, "en", EN_LINK_SESSION_EXPIRED),
    (KEY_LINK_SUCCESS, "en", EN_LINK_SUCCESS),
    (KEY_ACCOUNT_PENDING, "en", EN_ACCOUNT_PENDING),
    (KEY_ACCOUNT_SUSPENDED, "en", EN_ACCOUNT_SUSPENDED),
    (KEY_RATE_LIMITED, "en", EN_RATE_LIMITED),
    (KEY_QUOTA_EXCEEDED, "en", EN_QUOTA_EXCEEDED),
    (KEY_QUOTA_WARNING, "en", EN_QUOTA_WARNING),
    (KEY_NO_PROVIDER_CONNECTED, "en", EN_NO_PROVIDER_CONNECTED),
    (
        KEY_NO_PROVIDER_CONNECTED_WITH_EMAIL,
        "en",
        EN_NO_PROVIDER_CONNECTED_WITH_EMAIL,
    ),
    (KEY_CONNECT_PROMPT, "en", EN_CONNECT_PROMPT),
    (KEY_CONNECT_BUTTON, "en", EN_CONNECT_BUTTON),
    (KEY_CONNECT_TITLE, "en", EN_CONNECT_TITLE),
    (KEY_PROVIDER_REAUTH_REQUIRED, "en", EN_PROVIDER_REAUTH_REQUIRED),
    (
        KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK,
        "en",
        EN_PROVIDER_REAUTH_REQUIRED_NO_LINK,
    ),
    (
        KEY_PROVIDER_RECONNECT_BUTTON,
        "en",
        EN_PROVIDER_RECONNECT_BUTTON,
    ),
    (KEY_STATUS_HEADER, "en", EN_STATUS_HEADER),
    (KEY_STATUS_PROVIDERS_NONE, "en", EN_STATUS_PROVIDERS_NONE),
    (KEY_STATUS_PROVIDERS_LABEL, "en", EN_STATUS_PROVIDERS_LABEL),
    (KEY_STATUS_GROUPS_LABEL, "en", EN_STATUS_GROUPS_LABEL),
    (KEY_STATUS_CHANNEL_LABEL, "en", EN_STATUS_CHANNEL_LABEL),
    (KEY_HELP_HEADER, "en", EN_HELP_HEADER),
    (KEY_HELP_DOMAIN_GENERAL, "en", EN_HELP_DOMAIN_GENERAL),
    (KEY_HELP_DOMAIN_GROUP, "en", EN_HELP_DOMAIN_GROUP),
    (KEY_HELP_DOMAIN_COACH, "en", EN_HELP_DOMAIN_COACH),
    (KEY_HELP_DOMAIN_DATA, "en", EN_HELP_DOMAIN_DATA),
    (KEY_HELP_DOMAIN_PROVIDER, "en", EN_HELP_DOMAIN_PROVIDER),
    (KEY_HELP_DOMAIN_ACCOUNT, "en", EN_HELP_DOMAIN_ACCOUNT),
    (KEY_HELP_DOMAIN_TRAINING, "en", EN_HELP_DOMAIN_TRAINING),
    (KEY_HELP_DOMAIN_DISCOVER, "en", EN_HELP_DOMAIN_DISCOVER),
    (KEY_HELP_FOOTER, "en", EN_HELP_FOOTER),
    (KEY_LOGOUT_CONFIRM_PROMPT, "en", EN_LOGOUT_CONFIRM_PROMPT),
    (KEY_RESET_CONFIRM, "en", EN_RESET_CONFIRM),
    (KEY_PRIVACY_STATUS_LINE, "en", EN_PRIVACY_STATUS_LINE),
    (KEY_PRIVACY_STATUS_ENABLED, "en", EN_PRIVACY_STATUS_ENABLED),
    (KEY_PRIVACY_STATUS_DISABLED, "en", EN_PRIVACY_STATUS_DISABLED),
    (KEY_PRIVACY_ON_CONFIRMATION, "en", EN_PRIVACY_ON_CONFIRMATION),
    (KEY_PRIVACY_OFF_CONFIRMATION, "en", EN_PRIVACY_OFF_CONFIRMATION),
    (KEY_TIMEZONE_SET, "en", EN_TIMEZONE_SET),
    (KEY_TIMEZONE_INVALID, "en", EN_TIMEZONE_INVALID),
    (KEY_PILLARS_OPENER, "en", EN_PILLARS_OPENER),
    (KEY_PILLARS_DM_ONLY, "en", EN_PILLARS_DM_ONLY),
    (KEY_INTAKE_OPENER, "en", EN_INTAKE_OPENER),
    (KEY_INTAKE_PERSONA, "en", EN_INTAKE_PERSONA),
    (KEY_INTAKE_PARQ_INTRO, "en", EN_INTAKE_PARQ_INTRO),
    (KEY_INTAKE_PARQ_HEART_CONDITION, "en", EN_INTAKE_PARQ_HEART_CONDITION),
    (KEY_INTAKE_PARQ_CHEST_PAIN, "en", EN_INTAKE_PARQ_CHEST_PAIN),
    (KEY_INTAKE_PARQ_DIZZINESS, "en", EN_INTAKE_PARQ_DIZZINESS),
    (KEY_INTAKE_PARQ_CHRONIC_CONDITION, "en", EN_INTAKE_PARQ_CHRONIC_CONDITION),
    (KEY_INTAKE_PARQ_MEDICATION, "en", EN_INTAKE_PARQ_MEDICATION),
    (KEY_INTAKE_PARQ_JOINT_PROBLEM, "en", EN_INTAKE_PARQ_JOINT_PROBLEM),
    (KEY_INTAKE_PARQ_SUPERVISED_ONLY, "en", EN_INTAKE_PARQ_SUPERVISED_ONLY),
    (KEY_INTAKE_YESNO_HINT, "en", EN_INTAKE_YESNO_HINT),
    (KEY_INTAKE_RETRY, "en", EN_INTAKE_RETRY),
    (KEY_INTAKE_COMPLETE_CLEAR, "en", EN_INTAKE_COMPLETE_CLEAR),
    (KEY_INTAKE_COMPLETE_FLAGGED, "en", EN_INTAKE_COMPLETE_FLAGGED),
    (KEY_PILLARS_START_FAILED, "en", EN_PILLARS_START_FAILED),
    (KEY_RESET_WALK_INTERRUPTED, "en", EN_RESET_WALK_INTERRUPTED),
    (KEY_CALIBRATE_OPENER, "en", EN_CALIBRATE_OPENER),
    (KEY_CALIBRATE_DM_ONLY, "en", EN_CALIBRATE_DM_ONLY),
    (KEY_CALIBRATE_START_FAILED, "en", EN_CALIBRATE_START_FAILED),
    (KEY_CALIBRATE_COMPLETE_HEADER, "en", EN_CALIBRATE_COMPLETE_HEADER),
    (KEY_CALIBRATE_COMPLETE_MISSING, "en", EN_CALIBRATE_COMPLETE_MISSING),
    (KEY_CALIBRATE_FOLLOWUP_PLAN, "en", EN_CALIBRATE_FOLLOWUP_PLAN),
    (KEY_CALIBRATE_FOLLOWUP_NO_PLAN, "en", EN_CALIBRATE_FOLLOWUP_NO_PLAN),
    (KEY_CALIBRATE_TOPIC_INJURY, "en", EN_CALIBRATE_TOPIC_INJURY),
    (KEY_CALIBRATE_TOPIC_RECOVERY, "en", EN_CALIBRATE_TOPIC_RECOVERY),
    (KEY_PLAN_GOAL_LINE, "en", EN_PLAN_GOAL_LINE),
    (KEY_PLAN_BLOCK_LINE, "en", EN_PLAN_BLOCK_LINE),
    (KEY_PLAN_DAY_LINE, "en", EN_PLAN_DAY_LINE),
    (KEY_PLAN_REST, "en", EN_PLAN_REST),
    (KEY_PLAN_TODAY, "en", EN_PLAN_TODAY),
    (KEY_PLAN_TOMORROW, "en", EN_PLAN_TOMORROW),
    (KEY_PLAN_WEEK_HEADER, "en", EN_PLAN_WEEK_HEADER),
    (KEY_PLAN_NO_SESSION, "en", EN_PLAN_NO_SESSION),
    (KEY_PLAN_NO_COVERAGE, "en", EN_PLAN_NO_COVERAGE),
    (KEY_PLAN_RESUMES, "en", EN_PLAN_RESUMES),
    (KEY_PLAN_EMPTY, "en", EN_PLAN_EMPTY),
    (KEY_PLAN_STALE_GOAL, "en", EN_PLAN_STALE_GOAL),
    (KEY_GROUP_LIST_EMPTY, "en", EN_GROUP_LIST_EMPTY),
    (KEY_GROUP_LIST_HEADER, "en", EN_GROUP_LIST_HEADER),
    (KEY_GROUP_LIST_ITEM, "en", EN_GROUP_LIST_ITEM),
    (KEY_GROUP_NOT_A_MEMBER, "en", EN_GROUP_NOT_A_MEMBER),
    (KEY_GROUP_STATUS_SUMMARY, "en", EN_GROUP_STATUS_SUMMARY),
    (KEY_GROUP_PEER_SHARING_ON, "en", EN_GROUP_PEER_SHARING_ON),
    (KEY_GROUP_PEER_SHARING_OFF, "en", EN_GROUP_PEER_SHARING_OFF),
    (KEY_GROUP_MEMBERS_HEADER, "en", EN_GROUP_MEMBERS_HEADER),
    (KEY_GROUP_MEMBERS_UNKNOWN, "en", EN_GROUP_MEMBERS_UNKNOWN),
    (KEY_GROUP_MEMBERS_ITEM, "en", EN_GROUP_MEMBERS_ITEM),
    (KEY_GROUP_ROLE_OWNER, "en", EN_GROUP_ROLE_OWNER),
    (KEY_GROUP_ROLE_ADMIN, "en", EN_GROUP_ROLE_ADMIN),
    (KEY_GROUP_ROLE_MEMBER, "en", EN_GROUP_ROLE_MEMBER),
    (KEY_GROUP_INVITE_FORBIDDEN, "en", EN_GROUP_INVITE_FORBIDDEN),
    (KEY_GROUP_INVITE_BODY, "en", EN_GROUP_INVITE_BODY),
    (KEY_GROUP_INVITE_UNAVAILABLE, "en", EN_GROUP_INVITE_UNAVAILABLE),
    (KEY_COACH_INVITE_BODY, "en", EN_COACH_INVITE_BODY),
    (KEY_GROUP_LEAVE_PROMPT, "en", EN_GROUP_LEAVE_PROMPT),
    (KEY_GROUP_CONSENT_USAGE, "en", EN_GROUP_CONSENT_USAGE),
    (KEY_GROUP_RESPOND_USAGE, "en", EN_GROUP_RESPOND_USAGE),
    (KEY_GROUP_RESPOND_MENTIONS, "en", EN_GROUP_RESPOND_MENTIONS),
    (KEY_GROUP_RESPOND_ALL, "en", EN_GROUP_RESPOND_ALL),
    (KEY_GROUP_RESPOND_STATUS_MENTIONS, "en", EN_GROUP_RESPOND_STATUS_MENTIONS),
    (KEY_GROUP_COACH_DETACHED, "en", EN_GROUP_COACH_DETACHED),
    (KEY_NOTIFICATION_CHANNEL_BODY, "en", EN_NOTIFICATION_CHANNEL_BODY),
    (KEY_GROUP_CONSENT_UPDATED, "en", EN_GROUP_CONSENT_UPDATED),
    (KEY_GROUP_CREATE_USAGE, "en", EN_GROUP_CREATE_USAGE),
    (KEY_GROUP_CREATE_NO_COACH, "en", EN_GROUP_CREATE_NO_COACH),
    (KEY_GROUP_CREATE_UNAVAILABLE, "en", EN_GROUP_CREATE_UNAVAILABLE),
    (KEY_GROUP_CREATE_FORBIDDEN, "en", EN_GROUP_CREATE_FORBIDDEN),
    (KEY_GROUP_CREATED, "en", EN_GROUP_CREATED),
    (KEY_GROUP_INVITE_LABEL, "en", EN_GROUP_INVITE_LABEL),
    (KEY_GROUP_JOIN_INVALID_CODE, "en", EN_GROUP_JOIN_INVALID_CODE),
    (KEY_GROUP_JOIN_ALREADY_MEMBER, "en", EN_GROUP_JOIN_ALREADY_MEMBER),
    (KEY_GROUP_JOIN_FULL, "en", EN_GROUP_JOIN_FULL),
    (KEY_GROUP_JOINED, "en", EN_GROUP_JOINED),
    (KEY_GROUP_JOINED_AS_COACH, "en", EN_GROUP_JOINED_AS_COACH),
    (KEY_DISCOVER_CARD_TITLE, "en", EN_DISCOVER_CARD_TITLE),
    (KEY_DISCOVER_ITEM, "en", EN_DISCOVER_ITEM),
    (KEY_DISCOVER_EMPTY, "en", EN_DISCOVER_EMPTY),
    (KEY_DISCOVER_CATALOGUE_EMPTY, "en", EN_DISCOVER_CATALOGUE_EMPTY),
    (KEY_DISCOVER_MORE_LABEL, "en", EN_DISCOVER_MORE_LABEL),
    (KEY_DISCOVER_INSTALL_USAGE, "en", EN_DISCOVER_INSTALL_USAGE),
    (KEY_DISCOVER_INSTALL_UNKNOWN_HANDLE, "en", EN_DISCOVER_INSTALL_UNKNOWN_HANDLE),
    (KEY_DISCOVER_INSTALLED, "en", EN_DISCOVER_INSTALLED),
    (KEY_DISCOVER_INSTALL_ALREADY, "en", EN_DISCOVER_INSTALL_ALREADY),
    (KEY_DISCOVER_ADD_LABEL, "en", EN_DISCOVER_ADD_LABEL),
    (KEY_COACH_LIST_EMPTY, "en", EN_COACH_LIST_EMPTY),
    (KEY_COACH_LIST_CARD_TITLE, "en", EN_COACH_LIST_CARD_TITLE),
    (KEY_COACH_LIST_ITEM, "en", EN_COACH_LIST_ITEM),
    (KEY_COACH_LIST_ITEM_NO_HANDLE, "en", EN_COACH_LIST_ITEM_NO_HANDLE),
    (KEY_COACH_LIST_FOOTER, "en", EN_COACH_LIST_FOOTER),
    (KEY_COACH_NO_DESCRIPTION, "en", EN_COACH_NO_DESCRIPTION),
    (KEY_COACH_GROUP_UPDATED, "en", EN_COACH_GROUP_UPDATED),
    (KEY_COACH_USER_UPDATED, "en", EN_COACH_USER_UPDATED),
    (KEY_COACH_ASSIGN_NOT_A_MEMBER, "en", EN_COACH_ASSIGN_NOT_A_MEMBER),
    (KEY_COACH_ASSIGN_FORBIDDEN, "en", EN_COACH_ASSIGN_FORBIDDEN),
    (KEY_COACH_ADD_USAGE, "en", EN_COACH_ADD_USAGE),
    (KEY_COACH_ADD_UNKNOWN, "en", EN_COACH_ADD_UNKNOWN),
    (KEY_COACH_REMOVE_GROUP_THREAD, "en", EN_COACH_REMOVE_GROUP_THREAD),
    (KEY_COACH_REMOVE_NOTHING, "en", EN_COACH_REMOVE_NOTHING),
    (KEY_COACH_REMOVED, "en", EN_COACH_REMOVED),
    (KEY_COACH_CREATE_NO_CONVERSATION, "en", EN_COACH_CREATE_NO_CONVERSATION),
    (KEY_COACH_CREATE_EMPTY, "en", EN_COACH_CREATE_EMPTY),
    (KEY_COACH_CREATE_USAGE, "en", EN_COACH_CREATE_USAGE),
    (KEY_COACH_CREATE_CARD_TITLE, "en", EN_COACH_CREATE_CARD_TITLE),
    (KEY_COACH_CREATE_PROPOSAL_BODY, "en", EN_COACH_CREATE_PROPOSAL_BODY),
    (KEY_COACH_CREATE_CONFIRM_LABEL, "en", EN_COACH_CREATE_CONFIRM_LABEL),
    (KEY_COACH_CREATE_DISCARD_LABEL, "en", EN_COACH_CREATE_DISCARD_LABEL),
    (KEY_COACH_CREATE_QUOTA, "en", EN_COACH_CREATE_QUOTA),
    (KEY_COACH_CREATE_DONE, "en", EN_COACH_CREATE_DONE),
    (KEY_COACH_CREATE_DONE_UNBOUND, "en", EN_COACH_CREATE_DONE_UNBOUND),
    (KEY_COACH_CREATE_DISCARDED, "en", EN_COACH_CREATE_DISCARDED),

    // ── Spanish ─────────────────────────────────────────────────────────
    (KEY_COMMITMENT_MET, "es", ES_COMMITMENT_MET),
    (KEY_COMMITMENT_PARTIAL, "es", ES_COMMITMENT_PARTIAL),
    (KEY_COMMITMENT_MISSED, "es", ES_COMMITMENT_MISSED),
    (KEY_COMMITMENT_ACTIVITY_ANY, "es", ES_COMMITMENT_ACTIVITY_ANY),
    (KEY_COMMITMENT_PUSH_TITLE, "es", ES_COMMITMENT_PUSH_TITLE),
    (KEY_SCOPE_REFUSAL, "es", ES_SCOPE_REFUSAL),
    (KEY_CAPABILITY_REFUSAL, "es", ES_CAPABILITY_REFUSAL),
    (
        KEY_COACH_SCOPE_CARVE_OUT_NUTRITION,
        "es",
        ES_COACH_SCOPE_CARVE_OUT_NUTRITION,
    ),
    (
        KEY_COACH_SCOPE_CARVE_OUT_RECIPES,
        "es",
        ES_COACH_SCOPE_CARVE_OUT_RECIPES,
    ),
    (KEY_THINKING_PLACEHOLDER, "es", ES_THINKING_PLACEHOLDER),
    (KEY_UNKNOWN_COMMAND, "es", ES_UNKNOWN_COMMAND),
    (KEY_STATUS_READING_QUESTION, "es", ES_STATUS_READING_QUESTION),
    (KEY_STATUS_GENERATING_RESPONSE, "es", ES_STATUS_GENERATING_RESPONSE),
    (KEY_STATUS_CALLING_TOOL, "es", ES_STATUS_CALLING_TOOL),
    (KEY_STATUS_ERROR, "es", ES_STATUS_ERROR),
    (KEY_COACH_PROPOSAL_WELCOME, "es", ES_COACH_PROPOSAL_WELCOME),
    (
        KEY_COACH_PROPOSAL_WELCOME_GENERIC,
        "es",
        ES_COACH_PROPOSAL_WELCOME_GENERIC,
    ),
    (KEY_COACH_PROPOSAL_FOOTER, "es", ES_COACH_PROPOSAL_FOOTER),
    (KEY_REGISTRATION_APPROVED, "es", ES_REGISTRATION_APPROVED),
    (KEY_BACKFILL_READY, "es", ES_BACKFILL_READY),
    (KEY_BACKFILL_LIST_HEADER, "es", ES_BACKFILL_LIST_HEADER),
    (KEY_BACKFILL_LIST_MORE, "es", ES_BACKFILL_LIST_MORE),
    (KEY_ERROR_GENERIC, "es", ES_ERROR_GENERIC),
    (KEY_GUARDIAN_DENIED, "es", ES_GUARDIAN_DENIED),
    (KEY_GUARDIAN_CONFIRM_PROMPT, "es", ES_GUARDIAN_CONFIRM_PROMPT),
    (KEY_GUARDIAN_CONFIRM_DONE, "es", ES_GUARDIAN_CONFIRM_DONE),
    (KEY_GUARDIAN_CONFIRM_FAILED, "es", ES_GUARDIAN_CONFIRM_FAILED),
    (KEY_GUARDIAN_CONFIRM_DENIED, "es", ES_GUARDIAN_CONFIRM_DENIED),
    (KEY_GUARDIAN_CONFIRM_EXPIRED, "es", ES_GUARDIAN_CONFIRM_EXPIRED),
    (KEY_GUARDIAN_CONFIRM_NOT_FOUND, "es", ES_GUARDIAN_CONFIRM_NOT_FOUND),
    (KEY_EMPTY_REPLY, "es", ES_EMPTY_REPLY),
    (KEY_REPLY_WITHHELD, "es", ES_REPLY_WITHHELD),
    (KEY_GUARDRAIL_TOO_LONG, "es", ES_GUARDRAIL_TOO_LONG),
    (KEY_GUARDRAIL_BLOCKED_TOPIC, "es", ES_GUARDRAIL_BLOCKED_TOPIC),
    (KEY_VERIFICATION_WARN_SUFFIX, "es", ES_VERIFICATION_WARN_SUFFIX),
    (KEY_VERIFICATION_BLOCK_FALLBACK, "es", ES_VERIFICATION_BLOCK_FALLBACK),
    (KEY_LINK_FALLBACK_PROMPT, "es", ES_LINK_FALLBACK_PROMPT),
    (KEY_LINK_INITIAL_PROMPT, "es", ES_LINK_INITIAL_PROMPT),
    (KEY_LINK_EMAIL_PROMPT, "es", ES_LINK_EMAIL_PROMPT),
    (KEY_LINK_LOGOUT_COMPLETE, "es", ES_LINK_LOGOUT_COMPLETE),
    (KEY_LINK_CANCELLED, "es", ES_LINK_CANCELLED),
    (KEY_LINK_GENERIC_ERROR, "es", ES_LINK_GENERIC_ERROR),
    (KEY_LINK_SIGNUP_OFFER, "es", ES_LINK_SIGNUP_OFFER),
    (KEY_LINK_SIGNUP_CREATED, "es", ES_LINK_SIGNUP_CREATED),
    (KEY_LINK_SIGNUP_FAILED, "es", ES_LINK_SIGNUP_FAILED),
    (KEY_LINK_NO_TENANT, "es", ES_LINK_NO_TENANT),
    (KEY_LINK_EMAIL_NOT_CONFIGURED, "es", ES_LINK_EMAIL_NOT_CONFIGURED),
    (KEY_LINK_EMAIL_SEND_FAILED, "es", ES_LINK_EMAIL_SEND_FAILED),
    (KEY_LINK_INVALID_EMAIL, "es", ES_LINK_INVALID_EMAIL),
    (KEY_LINK_OTP_SENT, "es", ES_LINK_OTP_SENT),
    (KEY_LINK_TOO_MANY_ATTEMPTS, "es", ES_LINK_TOO_MANY_ATTEMPTS),
    (KEY_LINK_INCORRECT_CODE, "es", ES_LINK_INCORRECT_CODE),
    (KEY_LINK_VERIFICATION_ERROR, "es", ES_LINK_VERIFICATION_ERROR),
    (KEY_LINK_IDENTITY_COLLISION, "es", ES_LINK_IDENTITY_COLLISION),
    (KEY_LINK_OTP_PROMPT, "es", ES_LINK_OTP_PROMPT),
    (KEY_LINK_SESSION_EXPIRED, "es", ES_LINK_SESSION_EXPIRED),
    (KEY_LINK_SUCCESS, "es", ES_LINK_SUCCESS),
    (KEY_ACCOUNT_PENDING, "es", ES_ACCOUNT_PENDING),
    (KEY_ACCOUNT_SUSPENDED, "es", ES_ACCOUNT_SUSPENDED),
    (KEY_RATE_LIMITED, "es", ES_RATE_LIMITED),
    (KEY_QUOTA_EXCEEDED, "es", ES_QUOTA_EXCEEDED),
    (KEY_QUOTA_WARNING, "es", ES_QUOTA_WARNING),
    (KEY_NO_PROVIDER_CONNECTED, "es", ES_NO_PROVIDER_CONNECTED),
    (
        KEY_NO_PROVIDER_CONNECTED_WITH_EMAIL,
        "es",
        ES_NO_PROVIDER_CONNECTED_WITH_EMAIL,
    ),
    (KEY_CONNECT_PROMPT, "es", ES_CONNECT_PROMPT),
    (KEY_CONNECT_BUTTON, "es", ES_CONNECT_BUTTON),
    (KEY_CONNECT_TITLE, "es", ES_CONNECT_TITLE),
    (KEY_PROVIDER_REAUTH_REQUIRED, "es", ES_PROVIDER_REAUTH_REQUIRED),
    (
        KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK,
        "es",
        ES_PROVIDER_REAUTH_REQUIRED_NO_LINK,
    ),
    (
        KEY_PROVIDER_RECONNECT_BUTTON,
        "es",
        ES_PROVIDER_RECONNECT_BUTTON,
    ),
    (KEY_STATUS_HEADER, "es", ES_STATUS_HEADER),
    (KEY_STATUS_PROVIDERS_NONE, "es", ES_STATUS_PROVIDERS_NONE),
    (KEY_STATUS_PROVIDERS_LABEL, "es", ES_STATUS_PROVIDERS_LABEL),
    (KEY_STATUS_GROUPS_LABEL, "es", ES_STATUS_GROUPS_LABEL),
    (KEY_STATUS_CHANNEL_LABEL, "es", ES_STATUS_CHANNEL_LABEL),
    (KEY_HELP_HEADER, "es", ES_HELP_HEADER),
    (KEY_HELP_DOMAIN_GENERAL, "es", ES_HELP_DOMAIN_GENERAL),
    (KEY_HELP_DOMAIN_GROUP, "es", ES_HELP_DOMAIN_GROUP),
    (KEY_HELP_DOMAIN_COACH, "es", ES_HELP_DOMAIN_COACH),
    (KEY_HELP_DOMAIN_DATA, "es", ES_HELP_DOMAIN_DATA),
    (KEY_HELP_DOMAIN_PROVIDER, "es", ES_HELP_DOMAIN_PROVIDER),
    (KEY_HELP_DOMAIN_ACCOUNT, "es", ES_HELP_DOMAIN_ACCOUNT),
    (KEY_HELP_DOMAIN_TRAINING, "es", ES_HELP_DOMAIN_TRAINING),
    (KEY_HELP_DOMAIN_DISCOVER, "es", ES_HELP_DOMAIN_DISCOVER),
    (KEY_HELP_FOOTER, "es", ES_HELP_FOOTER),
    (KEY_LOGOUT_CONFIRM_PROMPT, "es", ES_LOGOUT_CONFIRM_PROMPT),
    (KEY_PRIVACY_STATUS_LINE, "es", ES_PRIVACY_STATUS_LINE),
    (KEY_PRIVACY_STATUS_ENABLED, "es", ES_PRIVACY_STATUS_ENABLED),
    (KEY_PRIVACY_STATUS_DISABLED, "es", ES_PRIVACY_STATUS_DISABLED),
    (KEY_PRIVACY_ON_CONFIRMATION, "es", ES_PRIVACY_ON_CONFIRMATION),
    (KEY_PRIVACY_OFF_CONFIRMATION, "es", ES_PRIVACY_OFF_CONFIRMATION),
    (KEY_TIMEZONE_SET, "es", ES_TIMEZONE_SET),
    (KEY_TIMEZONE_INVALID, "es", ES_TIMEZONE_INVALID),
    (KEY_PILLARS_OPENER, "es", ES_PILLARS_OPENER),
    (KEY_PILLARS_DM_ONLY, "es", ES_PILLARS_DM_ONLY),
    (KEY_INTAKE_OPENER, "es", ES_INTAKE_OPENER),
    (KEY_INTAKE_PERSONA, "es", ES_INTAKE_PERSONA),
    (KEY_INTAKE_PARQ_INTRO, "es", ES_INTAKE_PARQ_INTRO),
    (KEY_INTAKE_PARQ_HEART_CONDITION, "es", ES_INTAKE_PARQ_HEART_CONDITION),
    (KEY_INTAKE_PARQ_CHEST_PAIN, "es", ES_INTAKE_PARQ_CHEST_PAIN),
    (KEY_INTAKE_PARQ_DIZZINESS, "es", ES_INTAKE_PARQ_DIZZINESS),
    (KEY_INTAKE_PARQ_CHRONIC_CONDITION, "es", ES_INTAKE_PARQ_CHRONIC_CONDITION),
    (KEY_INTAKE_PARQ_MEDICATION, "es", ES_INTAKE_PARQ_MEDICATION),
    (KEY_INTAKE_PARQ_JOINT_PROBLEM, "es", ES_INTAKE_PARQ_JOINT_PROBLEM),
    (KEY_INTAKE_PARQ_SUPERVISED_ONLY, "es", ES_INTAKE_PARQ_SUPERVISED_ONLY),
    (KEY_INTAKE_YESNO_HINT, "es", ES_INTAKE_YESNO_HINT),
    (KEY_INTAKE_RETRY, "es", ES_INTAKE_RETRY),
    (KEY_INTAKE_COMPLETE_CLEAR, "es", ES_INTAKE_COMPLETE_CLEAR),
    (KEY_INTAKE_COMPLETE_FLAGGED, "es", ES_INTAKE_COMPLETE_FLAGGED),
    (KEY_PILLARS_START_FAILED, "es", ES_PILLARS_START_FAILED),
    (KEY_RESET_WALK_INTERRUPTED, "es", ES_RESET_WALK_INTERRUPTED),
    (KEY_CALIBRATE_OPENER, "es", ES_CALIBRATE_OPENER),
    (KEY_CALIBRATE_DM_ONLY, "es", ES_CALIBRATE_DM_ONLY),
    (KEY_CALIBRATE_START_FAILED, "es", ES_CALIBRATE_START_FAILED),
    (KEY_CALIBRATE_COMPLETE_HEADER, "es", ES_CALIBRATE_COMPLETE_HEADER),
    (KEY_CALIBRATE_COMPLETE_MISSING, "es", ES_CALIBRATE_COMPLETE_MISSING),
    (KEY_CALIBRATE_FOLLOWUP_PLAN, "es", ES_CALIBRATE_FOLLOWUP_PLAN),
    (KEY_CALIBRATE_FOLLOWUP_NO_PLAN, "es", ES_CALIBRATE_FOLLOWUP_NO_PLAN),
    (KEY_CALIBRATE_TOPIC_INJURY, "es", ES_CALIBRATE_TOPIC_INJURY),
    (KEY_CALIBRATE_TOPIC_RECOVERY, "es", ES_CALIBRATE_TOPIC_RECOVERY),
    (KEY_PLAN_GOAL_LINE, "es", ES_PLAN_GOAL_LINE),
    (KEY_PLAN_BLOCK_LINE, "es", ES_PLAN_BLOCK_LINE),
    (KEY_PLAN_DAY_LINE, "es", ES_PLAN_DAY_LINE),
    (KEY_PLAN_REST, "es", ES_PLAN_REST),
    (KEY_PLAN_TODAY, "es", ES_PLAN_TODAY),
    (KEY_PLAN_TOMORROW, "es", ES_PLAN_TOMORROW),
    (KEY_PLAN_WEEK_HEADER, "es", ES_PLAN_WEEK_HEADER),
    (KEY_PLAN_NO_SESSION, "es", ES_PLAN_NO_SESSION),
    (KEY_PLAN_NO_COVERAGE, "es", ES_PLAN_NO_COVERAGE),
    (KEY_PLAN_RESUMES, "es", ES_PLAN_RESUMES),
    (KEY_PLAN_EMPTY, "es", ES_PLAN_EMPTY),
    (KEY_PLAN_STALE_GOAL, "es", ES_PLAN_STALE_GOAL),
    (KEY_GROUP_LIST_EMPTY, "es", ES_GROUP_LIST_EMPTY),
    (KEY_GROUP_LIST_HEADER, "es", ES_GROUP_LIST_HEADER),
    (KEY_GROUP_LIST_ITEM, "es", ES_GROUP_LIST_ITEM),
    (KEY_GROUP_NOT_A_MEMBER, "es", ES_GROUP_NOT_A_MEMBER),
    (KEY_GROUP_STATUS_SUMMARY, "es", ES_GROUP_STATUS_SUMMARY),
    (KEY_GROUP_PEER_SHARING_ON, "es", ES_GROUP_PEER_SHARING_ON),
    (KEY_GROUP_PEER_SHARING_OFF, "es", ES_GROUP_PEER_SHARING_OFF),
    (KEY_GROUP_MEMBERS_HEADER, "es", ES_GROUP_MEMBERS_HEADER),
    (KEY_GROUP_MEMBERS_UNKNOWN, "es", ES_GROUP_MEMBERS_UNKNOWN),
    (KEY_GROUP_MEMBERS_ITEM, "es", ES_GROUP_MEMBERS_ITEM),
    (KEY_GROUP_ROLE_OWNER, "es", ES_GROUP_ROLE_OWNER),
    (KEY_GROUP_ROLE_ADMIN, "es", ES_GROUP_ROLE_ADMIN),
    (KEY_GROUP_ROLE_MEMBER, "es", ES_GROUP_ROLE_MEMBER),
    (KEY_GROUP_INVITE_FORBIDDEN, "es", ES_GROUP_INVITE_FORBIDDEN),
    (KEY_GROUP_INVITE_BODY, "es", ES_GROUP_INVITE_BODY),
    (KEY_GROUP_INVITE_UNAVAILABLE, "es", ES_GROUP_INVITE_UNAVAILABLE),
    (KEY_COACH_INVITE_BODY, "es", ES_COACH_INVITE_BODY),
    (KEY_GROUP_LEAVE_PROMPT, "es", ES_GROUP_LEAVE_PROMPT),
    (KEY_GROUP_CONSENT_USAGE, "es", ES_GROUP_CONSENT_USAGE),
    (KEY_GROUP_RESPOND_USAGE, "es", ES_GROUP_RESPOND_USAGE),
    (KEY_GROUP_RESPOND_MENTIONS, "es", ES_GROUP_RESPOND_MENTIONS),
    (KEY_GROUP_RESPOND_ALL, "es", ES_GROUP_RESPOND_ALL),
    (KEY_GROUP_RESPOND_STATUS_MENTIONS, "es", ES_GROUP_RESPOND_STATUS_MENTIONS),
    (KEY_GROUP_COACH_DETACHED, "es", ES_GROUP_COACH_DETACHED),
    (KEY_NOTIFICATION_CHANNEL_BODY, "es", ES_NOTIFICATION_CHANNEL_BODY),
    (KEY_GROUP_CONSENT_UPDATED, "es", ES_GROUP_CONSENT_UPDATED),
    (KEY_GROUP_CREATE_USAGE, "es", ES_GROUP_CREATE_USAGE),
    (KEY_GROUP_CREATE_NO_COACH, "es", ES_GROUP_CREATE_NO_COACH),
    (KEY_GROUP_CREATE_UNAVAILABLE, "es", ES_GROUP_CREATE_UNAVAILABLE),
    (KEY_GROUP_CREATE_FORBIDDEN, "es", ES_GROUP_CREATE_FORBIDDEN),
    (KEY_GROUP_CREATED, "es", ES_GROUP_CREATED),
    (KEY_GROUP_INVITE_LABEL, "es", ES_GROUP_INVITE_LABEL),
    (KEY_GROUP_JOIN_INVALID_CODE, "es", ES_GROUP_JOIN_INVALID_CODE),
    (KEY_GROUP_JOIN_ALREADY_MEMBER, "es", ES_GROUP_JOIN_ALREADY_MEMBER),
    (KEY_GROUP_JOIN_FULL, "es", ES_GROUP_JOIN_FULL),
    (KEY_GROUP_JOINED, "es", ES_GROUP_JOINED),
    (KEY_GROUP_JOINED_AS_COACH, "es", ES_GROUP_JOINED_AS_COACH),
    (KEY_DISCOVER_CARD_TITLE, "es", ES_DISCOVER_CARD_TITLE),
    (KEY_DISCOVER_ITEM, "es", ES_DISCOVER_ITEM),
    (KEY_DISCOVER_EMPTY, "es", ES_DISCOVER_EMPTY),
    (KEY_DISCOVER_CATALOGUE_EMPTY, "es", ES_DISCOVER_CATALOGUE_EMPTY),
    (KEY_DISCOVER_MORE_LABEL, "es", ES_DISCOVER_MORE_LABEL),
    (KEY_DISCOVER_INSTALL_USAGE, "es", ES_DISCOVER_INSTALL_USAGE),
    (KEY_DISCOVER_INSTALL_UNKNOWN_HANDLE, "es", ES_DISCOVER_INSTALL_UNKNOWN_HANDLE),
    (KEY_DISCOVER_INSTALLED, "es", ES_DISCOVER_INSTALLED),
    (KEY_DISCOVER_INSTALL_ALREADY, "es", ES_DISCOVER_INSTALL_ALREADY),
    (KEY_DISCOVER_ADD_LABEL, "es", ES_DISCOVER_ADD_LABEL),
    (KEY_COACH_LIST_EMPTY, "es", ES_COACH_LIST_EMPTY),
    (KEY_COACH_LIST_CARD_TITLE, "es", ES_COACH_LIST_CARD_TITLE),
    (KEY_COACH_LIST_ITEM, "es", ES_COACH_LIST_ITEM),
    (KEY_COACH_LIST_ITEM_NO_HANDLE, "es", ES_COACH_LIST_ITEM_NO_HANDLE),
    (KEY_COACH_LIST_FOOTER, "es", ES_COACH_LIST_FOOTER),
    (KEY_COACH_NO_DESCRIPTION, "es", ES_COACH_NO_DESCRIPTION),
    (KEY_COACH_GROUP_UPDATED, "es", ES_COACH_GROUP_UPDATED),
    (KEY_COACH_USER_UPDATED, "es", ES_COACH_USER_UPDATED),
    (KEY_COACH_ASSIGN_NOT_A_MEMBER, "es", ES_COACH_ASSIGN_NOT_A_MEMBER),
    (KEY_COACH_ASSIGN_FORBIDDEN, "es", ES_COACH_ASSIGN_FORBIDDEN),
    (KEY_COACH_ADD_USAGE, "es", ES_COACH_ADD_USAGE),
    (KEY_COACH_ADD_UNKNOWN, "es", ES_COACH_ADD_UNKNOWN),
    (KEY_COACH_REMOVE_GROUP_THREAD, "es", ES_COACH_REMOVE_GROUP_THREAD),
    (KEY_COACH_REMOVE_NOTHING, "es", ES_COACH_REMOVE_NOTHING),
    (KEY_COACH_REMOVED, "es", ES_COACH_REMOVED),
    (KEY_COACH_CREATE_NO_CONVERSATION, "es", ES_COACH_CREATE_NO_CONVERSATION),
    (KEY_COACH_CREATE_EMPTY, "es", ES_COACH_CREATE_EMPTY),
    (KEY_COACH_CREATE_USAGE, "es", ES_COACH_CREATE_USAGE),
    (KEY_COACH_CREATE_CARD_TITLE, "es", ES_COACH_CREATE_CARD_TITLE),
    (KEY_COACH_CREATE_PROPOSAL_BODY, "es", ES_COACH_CREATE_PROPOSAL_BODY),
    (KEY_COACH_CREATE_CONFIRM_LABEL, "es", ES_COACH_CREATE_CONFIRM_LABEL),
    (KEY_COACH_CREATE_DISCARD_LABEL, "es", ES_COACH_CREATE_DISCARD_LABEL),
    (KEY_COACH_CREATE_QUOTA, "es", ES_COACH_CREATE_QUOTA),
    (KEY_COACH_CREATE_DONE, "es", ES_COACH_CREATE_DONE),
    (KEY_COACH_CREATE_DONE_UNBOUND, "es", ES_COACH_CREATE_DONE_UNBOUND),
    (KEY_COACH_CREATE_DISCARDED, "es", ES_COACH_CREATE_DISCARDED),
    (KEY_RESET_CONFIRM, "es", ES_RESET_CONFIRM),

    // ── German ──────────────────────────────────────────────────────────
    (KEY_COMMITMENT_MET, "de", DE_COMMITMENT_MET),
    (KEY_COMMITMENT_PARTIAL, "de", DE_COMMITMENT_PARTIAL),
    (KEY_COMMITMENT_MISSED, "de", DE_COMMITMENT_MISSED),
    (KEY_COMMITMENT_ACTIVITY_ANY, "de", DE_COMMITMENT_ACTIVITY_ANY),
    (KEY_COMMITMENT_PUSH_TITLE, "de", DE_COMMITMENT_PUSH_TITLE),
    (KEY_SCOPE_REFUSAL, "de", DE_SCOPE_REFUSAL),
    (KEY_CAPABILITY_REFUSAL, "de", DE_CAPABILITY_REFUSAL),
    (
        KEY_COACH_SCOPE_CARVE_OUT_NUTRITION,
        "de",
        DE_COACH_SCOPE_CARVE_OUT_NUTRITION,
    ),
    (
        KEY_COACH_SCOPE_CARVE_OUT_RECIPES,
        "de",
        DE_COACH_SCOPE_CARVE_OUT_RECIPES,
    ),
    (KEY_THINKING_PLACEHOLDER, "de", DE_THINKING_PLACEHOLDER),
    (KEY_UNKNOWN_COMMAND, "de", DE_UNKNOWN_COMMAND),
    (KEY_STATUS_READING_QUESTION, "de", DE_STATUS_READING_QUESTION),
    (KEY_STATUS_GENERATING_RESPONSE, "de", DE_STATUS_GENERATING_RESPONSE),
    (KEY_STATUS_CALLING_TOOL, "de", DE_STATUS_CALLING_TOOL),
    (KEY_STATUS_ERROR, "de", DE_STATUS_ERROR),
    (KEY_COACH_PROPOSAL_WELCOME, "de", DE_COACH_PROPOSAL_WELCOME),
    (
        KEY_COACH_PROPOSAL_WELCOME_GENERIC,
        "de",
        DE_COACH_PROPOSAL_WELCOME_GENERIC,
    ),
    (KEY_COACH_PROPOSAL_FOOTER, "de", DE_COACH_PROPOSAL_FOOTER),
    (KEY_REGISTRATION_APPROVED, "de", DE_REGISTRATION_APPROVED),
    (KEY_BACKFILL_READY, "de", DE_BACKFILL_READY),
    (KEY_BACKFILL_LIST_HEADER, "de", DE_BACKFILL_LIST_HEADER),
    (KEY_BACKFILL_LIST_MORE, "de", DE_BACKFILL_LIST_MORE),
    (KEY_ERROR_GENERIC, "de", DE_ERROR_GENERIC),
    (KEY_GUARDIAN_DENIED, "de", DE_GUARDIAN_DENIED),
    (KEY_GUARDIAN_CONFIRM_PROMPT, "de", DE_GUARDIAN_CONFIRM_PROMPT),
    (KEY_GUARDIAN_CONFIRM_DONE, "de", DE_GUARDIAN_CONFIRM_DONE),
    (KEY_GUARDIAN_CONFIRM_FAILED, "de", DE_GUARDIAN_CONFIRM_FAILED),
    (KEY_GUARDIAN_CONFIRM_DENIED, "de", DE_GUARDIAN_CONFIRM_DENIED),
    (KEY_GUARDIAN_CONFIRM_EXPIRED, "de", DE_GUARDIAN_CONFIRM_EXPIRED),
    (KEY_GUARDIAN_CONFIRM_NOT_FOUND, "de", DE_GUARDIAN_CONFIRM_NOT_FOUND),
    (KEY_EMPTY_REPLY, "de", DE_EMPTY_REPLY),
    (KEY_REPLY_WITHHELD, "de", DE_REPLY_WITHHELD),
    (KEY_GUARDRAIL_TOO_LONG, "de", DE_GUARDRAIL_TOO_LONG),
    (KEY_GUARDRAIL_BLOCKED_TOPIC, "de", DE_GUARDRAIL_BLOCKED_TOPIC),
    (KEY_VERIFICATION_WARN_SUFFIX, "de", DE_VERIFICATION_WARN_SUFFIX),
    (KEY_VERIFICATION_BLOCK_FALLBACK, "de", DE_VERIFICATION_BLOCK_FALLBACK),
    (KEY_LINK_FALLBACK_PROMPT, "de", DE_LINK_FALLBACK_PROMPT),
    (KEY_LINK_INITIAL_PROMPT, "de", DE_LINK_INITIAL_PROMPT),
    (KEY_LINK_EMAIL_PROMPT, "de", DE_LINK_EMAIL_PROMPT),
    (KEY_LINK_LOGOUT_COMPLETE, "de", DE_LINK_LOGOUT_COMPLETE),
    (KEY_LINK_CANCELLED, "de", DE_LINK_CANCELLED),
    (KEY_LINK_GENERIC_ERROR, "de", DE_LINK_GENERIC_ERROR),
    (KEY_LINK_SIGNUP_OFFER, "de", DE_LINK_SIGNUP_OFFER),
    (KEY_LINK_SIGNUP_CREATED, "de", DE_LINK_SIGNUP_CREATED),
    (KEY_LINK_SIGNUP_FAILED, "de", DE_LINK_SIGNUP_FAILED),
    (KEY_LINK_NO_TENANT, "de", DE_LINK_NO_TENANT),
    (KEY_LINK_EMAIL_NOT_CONFIGURED, "de", DE_LINK_EMAIL_NOT_CONFIGURED),
    (KEY_LINK_EMAIL_SEND_FAILED, "de", DE_LINK_EMAIL_SEND_FAILED),
    (KEY_LINK_INVALID_EMAIL, "de", DE_LINK_INVALID_EMAIL),
    (KEY_LINK_OTP_SENT, "de", DE_LINK_OTP_SENT),
    (KEY_LINK_TOO_MANY_ATTEMPTS, "de", DE_LINK_TOO_MANY_ATTEMPTS),
    (KEY_LINK_INCORRECT_CODE, "de", DE_LINK_INCORRECT_CODE),
    (KEY_LINK_VERIFICATION_ERROR, "de", DE_LINK_VERIFICATION_ERROR),
    (KEY_LINK_IDENTITY_COLLISION, "de", DE_LINK_IDENTITY_COLLISION),
    (KEY_LINK_OTP_PROMPT, "de", DE_LINK_OTP_PROMPT),
    (KEY_LINK_SESSION_EXPIRED, "de", DE_LINK_SESSION_EXPIRED),
    (KEY_LINK_SUCCESS, "de", DE_LINK_SUCCESS),
    (KEY_ACCOUNT_PENDING, "de", DE_ACCOUNT_PENDING),
    (KEY_ACCOUNT_SUSPENDED, "de", DE_ACCOUNT_SUSPENDED),
    (KEY_RATE_LIMITED, "de", DE_RATE_LIMITED),
    (KEY_QUOTA_EXCEEDED, "de", DE_QUOTA_EXCEEDED),
    (KEY_QUOTA_WARNING, "de", DE_QUOTA_WARNING),
    (KEY_NO_PROVIDER_CONNECTED, "de", DE_NO_PROVIDER_CONNECTED),
    (
        KEY_NO_PROVIDER_CONNECTED_WITH_EMAIL,
        "de",
        DE_NO_PROVIDER_CONNECTED_WITH_EMAIL,
    ),
    (KEY_CONNECT_PROMPT, "de", DE_CONNECT_PROMPT),
    (KEY_CONNECT_BUTTON, "de", DE_CONNECT_BUTTON),
    (KEY_CONNECT_TITLE, "de", DE_CONNECT_TITLE),
    (KEY_PROVIDER_REAUTH_REQUIRED, "de", DE_PROVIDER_REAUTH_REQUIRED),
    (
        KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK,
        "de",
        DE_PROVIDER_REAUTH_REQUIRED_NO_LINK,
    ),
    (
        KEY_PROVIDER_RECONNECT_BUTTON,
        "de",
        DE_PROVIDER_RECONNECT_BUTTON,
    ),
    (KEY_STATUS_HEADER, "de", DE_STATUS_HEADER),
    (KEY_STATUS_PROVIDERS_NONE, "de", DE_STATUS_PROVIDERS_NONE),
    (KEY_STATUS_PROVIDERS_LABEL, "de", DE_STATUS_PROVIDERS_LABEL),
    (KEY_STATUS_GROUPS_LABEL, "de", DE_STATUS_GROUPS_LABEL),
    (KEY_STATUS_CHANNEL_LABEL, "de", DE_STATUS_CHANNEL_LABEL),
    (KEY_HELP_HEADER, "de", DE_HELP_HEADER),
    (KEY_HELP_DOMAIN_GENERAL, "de", DE_HELP_DOMAIN_GENERAL),
    (KEY_HELP_DOMAIN_GROUP, "de", DE_HELP_DOMAIN_GROUP),
    (KEY_HELP_DOMAIN_COACH, "de", DE_HELP_DOMAIN_COACH),
    (KEY_HELP_DOMAIN_DATA, "de", DE_HELP_DOMAIN_DATA),
    (KEY_HELP_DOMAIN_PROVIDER, "de", DE_HELP_DOMAIN_PROVIDER),
    (KEY_HELP_DOMAIN_ACCOUNT, "de", DE_HELP_DOMAIN_ACCOUNT),
    (KEY_HELP_DOMAIN_TRAINING, "de", DE_HELP_DOMAIN_TRAINING),
    (KEY_HELP_DOMAIN_DISCOVER, "de", DE_HELP_DOMAIN_DISCOVER),
    (KEY_HELP_FOOTER, "de", DE_HELP_FOOTER),
    (KEY_LOGOUT_CONFIRM_PROMPT, "de", DE_LOGOUT_CONFIRM_PROMPT),
    (KEY_PRIVACY_STATUS_LINE, "de", DE_PRIVACY_STATUS_LINE),
    (KEY_PRIVACY_STATUS_ENABLED, "de", DE_PRIVACY_STATUS_ENABLED),
    (KEY_PRIVACY_STATUS_DISABLED, "de", DE_PRIVACY_STATUS_DISABLED),
    (KEY_PRIVACY_ON_CONFIRMATION, "de", DE_PRIVACY_ON_CONFIRMATION),
    (KEY_PRIVACY_OFF_CONFIRMATION, "de", DE_PRIVACY_OFF_CONFIRMATION),
    (KEY_TIMEZONE_SET, "de", DE_TIMEZONE_SET),
    (KEY_TIMEZONE_INVALID, "de", DE_TIMEZONE_INVALID),
    (KEY_PILLARS_OPENER, "de", DE_PILLARS_OPENER),
    (KEY_PILLARS_DM_ONLY, "de", DE_PILLARS_DM_ONLY),
    (KEY_INTAKE_OPENER, "de", DE_INTAKE_OPENER),
    (KEY_INTAKE_PERSONA, "de", DE_INTAKE_PERSONA),
    (KEY_INTAKE_PARQ_INTRO, "de", DE_INTAKE_PARQ_INTRO),
    (KEY_INTAKE_PARQ_HEART_CONDITION, "de", DE_INTAKE_PARQ_HEART_CONDITION),
    (KEY_INTAKE_PARQ_CHEST_PAIN, "de", DE_INTAKE_PARQ_CHEST_PAIN),
    (KEY_INTAKE_PARQ_DIZZINESS, "de", DE_INTAKE_PARQ_DIZZINESS),
    (KEY_INTAKE_PARQ_CHRONIC_CONDITION, "de", DE_INTAKE_PARQ_CHRONIC_CONDITION),
    (KEY_INTAKE_PARQ_MEDICATION, "de", DE_INTAKE_PARQ_MEDICATION),
    (KEY_INTAKE_PARQ_JOINT_PROBLEM, "de", DE_INTAKE_PARQ_JOINT_PROBLEM),
    (KEY_INTAKE_PARQ_SUPERVISED_ONLY, "de", DE_INTAKE_PARQ_SUPERVISED_ONLY),
    (KEY_INTAKE_YESNO_HINT, "de", DE_INTAKE_YESNO_HINT),
    (KEY_INTAKE_RETRY, "de", DE_INTAKE_RETRY),
    (KEY_INTAKE_COMPLETE_CLEAR, "de", DE_INTAKE_COMPLETE_CLEAR),
    (KEY_INTAKE_COMPLETE_FLAGGED, "de", DE_INTAKE_COMPLETE_FLAGGED),
    (KEY_PILLARS_START_FAILED, "de", DE_PILLARS_START_FAILED),
    (KEY_RESET_WALK_INTERRUPTED, "de", DE_RESET_WALK_INTERRUPTED),
    (KEY_CALIBRATE_OPENER, "de", DE_CALIBRATE_OPENER),
    (KEY_CALIBRATE_DM_ONLY, "de", DE_CALIBRATE_DM_ONLY),
    (KEY_CALIBRATE_START_FAILED, "de", DE_CALIBRATE_START_FAILED),
    (KEY_CALIBRATE_COMPLETE_HEADER, "de", DE_CALIBRATE_COMPLETE_HEADER),
    (KEY_CALIBRATE_COMPLETE_MISSING, "de", DE_CALIBRATE_COMPLETE_MISSING),
    (KEY_CALIBRATE_FOLLOWUP_PLAN, "de", DE_CALIBRATE_FOLLOWUP_PLAN),
    (KEY_CALIBRATE_FOLLOWUP_NO_PLAN, "de", DE_CALIBRATE_FOLLOWUP_NO_PLAN),
    (KEY_CALIBRATE_TOPIC_INJURY, "de", DE_CALIBRATE_TOPIC_INJURY),
    (KEY_CALIBRATE_TOPIC_RECOVERY, "de", DE_CALIBRATE_TOPIC_RECOVERY),
    (KEY_PLAN_GOAL_LINE, "de", DE_PLAN_GOAL_LINE),
    (KEY_PLAN_BLOCK_LINE, "de", DE_PLAN_BLOCK_LINE),
    (KEY_PLAN_DAY_LINE, "de", DE_PLAN_DAY_LINE),
    (KEY_PLAN_REST, "de", DE_PLAN_REST),
    (KEY_PLAN_TODAY, "de", DE_PLAN_TODAY),
    (KEY_PLAN_TOMORROW, "de", DE_PLAN_TOMORROW),
    (KEY_PLAN_WEEK_HEADER, "de", DE_PLAN_WEEK_HEADER),
    (KEY_PLAN_NO_SESSION, "de", DE_PLAN_NO_SESSION),
    (KEY_PLAN_NO_COVERAGE, "de", DE_PLAN_NO_COVERAGE),
    (KEY_PLAN_RESUMES, "de", DE_PLAN_RESUMES),
    (KEY_PLAN_EMPTY, "de", DE_PLAN_EMPTY),
    (KEY_PLAN_STALE_GOAL, "de", DE_PLAN_STALE_GOAL),
    (KEY_GROUP_LIST_EMPTY, "de", DE_GROUP_LIST_EMPTY),
    (KEY_GROUP_LIST_HEADER, "de", DE_GROUP_LIST_HEADER),
    (KEY_GROUP_LIST_ITEM, "de", DE_GROUP_LIST_ITEM),
    (KEY_GROUP_NOT_A_MEMBER, "de", DE_GROUP_NOT_A_MEMBER),
    (KEY_GROUP_STATUS_SUMMARY, "de", DE_GROUP_STATUS_SUMMARY),
    (KEY_GROUP_PEER_SHARING_ON, "de", DE_GROUP_PEER_SHARING_ON),
    (KEY_GROUP_PEER_SHARING_OFF, "de", DE_GROUP_PEER_SHARING_OFF),
    (KEY_GROUP_MEMBERS_HEADER, "de", DE_GROUP_MEMBERS_HEADER),
    (KEY_GROUP_MEMBERS_UNKNOWN, "de", DE_GROUP_MEMBERS_UNKNOWN),
    (KEY_GROUP_MEMBERS_ITEM, "de", DE_GROUP_MEMBERS_ITEM),
    (KEY_GROUP_ROLE_OWNER, "de", DE_GROUP_ROLE_OWNER),
    (KEY_GROUP_ROLE_ADMIN, "de", DE_GROUP_ROLE_ADMIN),
    (KEY_GROUP_ROLE_MEMBER, "de", DE_GROUP_ROLE_MEMBER),
    (KEY_GROUP_INVITE_FORBIDDEN, "de", DE_GROUP_INVITE_FORBIDDEN),
    (KEY_GROUP_INVITE_BODY, "de", DE_GROUP_INVITE_BODY),
    (KEY_GROUP_INVITE_UNAVAILABLE, "de", DE_GROUP_INVITE_UNAVAILABLE),
    (KEY_COACH_INVITE_BODY, "de", DE_COACH_INVITE_BODY),
    (KEY_GROUP_LEAVE_PROMPT, "de", DE_GROUP_LEAVE_PROMPT),
    (KEY_GROUP_CONSENT_USAGE, "de", DE_GROUP_CONSENT_USAGE),
    (KEY_GROUP_RESPOND_USAGE, "de", DE_GROUP_RESPOND_USAGE),
    (KEY_GROUP_RESPOND_MENTIONS, "de", DE_GROUP_RESPOND_MENTIONS),
    (KEY_GROUP_RESPOND_ALL, "de", DE_GROUP_RESPOND_ALL),
    (KEY_GROUP_RESPOND_STATUS_MENTIONS, "de", DE_GROUP_RESPOND_STATUS_MENTIONS),
    (KEY_GROUP_COACH_DETACHED, "de", DE_GROUP_COACH_DETACHED),
    (KEY_NOTIFICATION_CHANNEL_BODY, "de", DE_NOTIFICATION_CHANNEL_BODY),
    (KEY_GROUP_CONSENT_UPDATED, "de", DE_GROUP_CONSENT_UPDATED),
    (KEY_GROUP_CREATE_USAGE, "de", DE_GROUP_CREATE_USAGE),
    (KEY_GROUP_CREATE_NO_COACH, "de", DE_GROUP_CREATE_NO_COACH),
    (KEY_GROUP_CREATE_UNAVAILABLE, "de", DE_GROUP_CREATE_UNAVAILABLE),
    (KEY_GROUP_CREATE_FORBIDDEN, "de", DE_GROUP_CREATE_FORBIDDEN),
    (KEY_GROUP_CREATED, "de", DE_GROUP_CREATED),
    (KEY_GROUP_INVITE_LABEL, "de", DE_GROUP_INVITE_LABEL),
    (KEY_GROUP_JOIN_INVALID_CODE, "de", DE_GROUP_JOIN_INVALID_CODE),
    (KEY_GROUP_JOIN_ALREADY_MEMBER, "de", DE_GROUP_JOIN_ALREADY_MEMBER),
    (KEY_GROUP_JOIN_FULL, "de", DE_GROUP_JOIN_FULL),
    (KEY_GROUP_JOINED, "de", DE_GROUP_JOINED),
    (KEY_GROUP_JOINED_AS_COACH, "de", DE_GROUP_JOINED_AS_COACH),
    (KEY_DISCOVER_CARD_TITLE, "de", DE_DISCOVER_CARD_TITLE),
    (KEY_DISCOVER_ITEM, "de", DE_DISCOVER_ITEM),
    (KEY_DISCOVER_EMPTY, "de", DE_DISCOVER_EMPTY),
    (KEY_DISCOVER_CATALOGUE_EMPTY, "de", DE_DISCOVER_CATALOGUE_EMPTY),
    (KEY_DISCOVER_MORE_LABEL, "de", DE_DISCOVER_MORE_LABEL),
    (KEY_DISCOVER_INSTALL_USAGE, "de", DE_DISCOVER_INSTALL_USAGE),
    (KEY_DISCOVER_INSTALL_UNKNOWN_HANDLE, "de", DE_DISCOVER_INSTALL_UNKNOWN_HANDLE),
    (KEY_DISCOVER_INSTALLED, "de", DE_DISCOVER_INSTALLED),
    (KEY_DISCOVER_INSTALL_ALREADY, "de", DE_DISCOVER_INSTALL_ALREADY),
    (KEY_DISCOVER_ADD_LABEL, "de", DE_DISCOVER_ADD_LABEL),
    (KEY_COACH_LIST_EMPTY, "de", DE_COACH_LIST_EMPTY),
    (KEY_COACH_LIST_CARD_TITLE, "de", DE_COACH_LIST_CARD_TITLE),
    (KEY_COACH_LIST_ITEM, "de", DE_COACH_LIST_ITEM),
    (KEY_COACH_LIST_ITEM_NO_HANDLE, "de", DE_COACH_LIST_ITEM_NO_HANDLE),
    (KEY_COACH_LIST_FOOTER, "de", DE_COACH_LIST_FOOTER),
    (KEY_COACH_NO_DESCRIPTION, "de", DE_COACH_NO_DESCRIPTION),
    (KEY_COACH_GROUP_UPDATED, "de", DE_COACH_GROUP_UPDATED),
    (KEY_COACH_USER_UPDATED, "de", DE_COACH_USER_UPDATED),
    (KEY_COACH_ASSIGN_NOT_A_MEMBER, "de", DE_COACH_ASSIGN_NOT_A_MEMBER),
    (KEY_COACH_ASSIGN_FORBIDDEN, "de", DE_COACH_ASSIGN_FORBIDDEN),
    (KEY_COACH_ADD_USAGE, "de", DE_COACH_ADD_USAGE),
    (KEY_COACH_ADD_UNKNOWN, "de", DE_COACH_ADD_UNKNOWN),
    (KEY_COACH_REMOVE_GROUP_THREAD, "de", DE_COACH_REMOVE_GROUP_THREAD),
    (KEY_COACH_REMOVE_NOTHING, "de", DE_COACH_REMOVE_NOTHING),
    (KEY_COACH_REMOVED, "de", DE_COACH_REMOVED),
    (KEY_COACH_CREATE_NO_CONVERSATION, "de", DE_COACH_CREATE_NO_CONVERSATION),
    (KEY_COACH_CREATE_EMPTY, "de", DE_COACH_CREATE_EMPTY),
    (KEY_COACH_CREATE_USAGE, "de", DE_COACH_CREATE_USAGE),
    (KEY_COACH_CREATE_CARD_TITLE, "de", DE_COACH_CREATE_CARD_TITLE),
    (KEY_COACH_CREATE_PROPOSAL_BODY, "de", DE_COACH_CREATE_PROPOSAL_BODY),
    (KEY_COACH_CREATE_CONFIRM_LABEL, "de", DE_COACH_CREATE_CONFIRM_LABEL),
    (KEY_COACH_CREATE_DISCARD_LABEL, "de", DE_COACH_CREATE_DISCARD_LABEL),
    (KEY_COACH_CREATE_QUOTA, "de", DE_COACH_CREATE_QUOTA),
    (KEY_COACH_CREATE_DONE, "de", DE_COACH_CREATE_DONE),
    (KEY_COACH_CREATE_DONE_UNBOUND, "de", DE_COACH_CREATE_DONE_UNBOUND),
    (KEY_COACH_CREATE_DISCARDED, "de", DE_COACH_CREATE_DISCARDED),
    (KEY_RESET_CONFIRM, "de", DE_RESET_CONFIRM),

    // ── Portuguese ──────────────────────────────────────────────────────
    (KEY_COMMITMENT_MET, "pt", PT_COMMITMENT_MET),
    (KEY_COMMITMENT_PARTIAL, "pt", PT_COMMITMENT_PARTIAL),
    (KEY_COMMITMENT_MISSED, "pt", PT_COMMITMENT_MISSED),
    (KEY_COMMITMENT_ACTIVITY_ANY, "pt", PT_COMMITMENT_ACTIVITY_ANY),
    (KEY_COMMITMENT_PUSH_TITLE, "pt", PT_COMMITMENT_PUSH_TITLE),
    (KEY_SCOPE_REFUSAL, "pt", PT_SCOPE_REFUSAL),
    (KEY_CAPABILITY_REFUSAL, "pt", PT_CAPABILITY_REFUSAL),
    (
        KEY_COACH_SCOPE_CARVE_OUT_NUTRITION,
        "pt",
        PT_COACH_SCOPE_CARVE_OUT_NUTRITION,
    ),
    (
        KEY_COACH_SCOPE_CARVE_OUT_RECIPES,
        "pt",
        PT_COACH_SCOPE_CARVE_OUT_RECIPES,
    ),
    (KEY_THINKING_PLACEHOLDER, "pt", PT_THINKING_PLACEHOLDER),
    (KEY_UNKNOWN_COMMAND, "pt", PT_UNKNOWN_COMMAND),
    (KEY_STATUS_READING_QUESTION, "pt", PT_STATUS_READING_QUESTION),
    (KEY_STATUS_GENERATING_RESPONSE, "pt", PT_STATUS_GENERATING_RESPONSE),
    (KEY_STATUS_CALLING_TOOL, "pt", PT_STATUS_CALLING_TOOL),
    (KEY_STATUS_ERROR, "pt", PT_STATUS_ERROR),
    (KEY_COACH_PROPOSAL_WELCOME, "pt", PT_COACH_PROPOSAL_WELCOME),
    (
        KEY_COACH_PROPOSAL_WELCOME_GENERIC,
        "pt",
        PT_COACH_PROPOSAL_WELCOME_GENERIC,
    ),
    (KEY_COACH_PROPOSAL_FOOTER, "pt", PT_COACH_PROPOSAL_FOOTER),
    (KEY_REGISTRATION_APPROVED, "pt", PT_REGISTRATION_APPROVED),
    (KEY_BACKFILL_READY, "pt", PT_BACKFILL_READY),
    (KEY_BACKFILL_LIST_HEADER, "pt", PT_BACKFILL_LIST_HEADER),
    (KEY_BACKFILL_LIST_MORE, "pt", PT_BACKFILL_LIST_MORE),
    (KEY_ERROR_GENERIC, "pt", PT_ERROR_GENERIC),
    (KEY_GUARDIAN_DENIED, "pt", PT_GUARDIAN_DENIED),
    (KEY_GUARDIAN_CONFIRM_PROMPT, "pt", PT_GUARDIAN_CONFIRM_PROMPT),
    (KEY_GUARDIAN_CONFIRM_DONE, "pt", PT_GUARDIAN_CONFIRM_DONE),
    (KEY_GUARDIAN_CONFIRM_FAILED, "pt", PT_GUARDIAN_CONFIRM_FAILED),
    (KEY_GUARDIAN_CONFIRM_DENIED, "pt", PT_GUARDIAN_CONFIRM_DENIED),
    (KEY_GUARDIAN_CONFIRM_EXPIRED, "pt", PT_GUARDIAN_CONFIRM_EXPIRED),
    (KEY_GUARDIAN_CONFIRM_NOT_FOUND, "pt", PT_GUARDIAN_CONFIRM_NOT_FOUND),
    (KEY_EMPTY_REPLY, "pt", PT_EMPTY_REPLY),
    (KEY_REPLY_WITHHELD, "pt", PT_REPLY_WITHHELD),
    (KEY_GUARDRAIL_TOO_LONG, "pt", PT_GUARDRAIL_TOO_LONG),
    (KEY_GUARDRAIL_BLOCKED_TOPIC, "pt", PT_GUARDRAIL_BLOCKED_TOPIC),
    (KEY_VERIFICATION_WARN_SUFFIX, "pt", PT_VERIFICATION_WARN_SUFFIX),
    (KEY_VERIFICATION_BLOCK_FALLBACK, "pt", PT_VERIFICATION_BLOCK_FALLBACK),
    (KEY_LINK_FALLBACK_PROMPT, "pt", PT_LINK_FALLBACK_PROMPT),
    (KEY_LINK_INITIAL_PROMPT, "pt", PT_LINK_INITIAL_PROMPT),
    (KEY_LINK_EMAIL_PROMPT, "pt", PT_LINK_EMAIL_PROMPT),
    (KEY_LINK_LOGOUT_COMPLETE, "pt", PT_LINK_LOGOUT_COMPLETE),
    (KEY_LINK_CANCELLED, "pt", PT_LINK_CANCELLED),
    (KEY_LINK_GENERIC_ERROR, "pt", PT_LINK_GENERIC_ERROR),
    (KEY_LINK_SIGNUP_OFFER, "pt", PT_LINK_SIGNUP_OFFER),
    (KEY_LINK_SIGNUP_CREATED, "pt", PT_LINK_SIGNUP_CREATED),
    (KEY_LINK_SIGNUP_FAILED, "pt", PT_LINK_SIGNUP_FAILED),
    (KEY_LINK_NO_TENANT, "pt", PT_LINK_NO_TENANT),
    (KEY_LINK_EMAIL_NOT_CONFIGURED, "pt", PT_LINK_EMAIL_NOT_CONFIGURED),
    (KEY_LINK_EMAIL_SEND_FAILED, "pt", PT_LINK_EMAIL_SEND_FAILED),
    (KEY_LINK_INVALID_EMAIL, "pt", PT_LINK_INVALID_EMAIL),
    (KEY_LINK_OTP_SENT, "pt", PT_LINK_OTP_SENT),
    (KEY_LINK_TOO_MANY_ATTEMPTS, "pt", PT_LINK_TOO_MANY_ATTEMPTS),
    (KEY_LINK_INCORRECT_CODE, "pt", PT_LINK_INCORRECT_CODE),
    (KEY_LINK_VERIFICATION_ERROR, "pt", PT_LINK_VERIFICATION_ERROR),
    (KEY_LINK_IDENTITY_COLLISION, "pt", PT_LINK_IDENTITY_COLLISION),
    (KEY_LINK_OTP_PROMPT, "pt", PT_LINK_OTP_PROMPT),
    (KEY_LINK_SESSION_EXPIRED, "pt", PT_LINK_SESSION_EXPIRED),
    (KEY_LINK_SUCCESS, "pt", PT_LINK_SUCCESS),
    (KEY_ACCOUNT_PENDING, "pt", PT_ACCOUNT_PENDING),
    (KEY_ACCOUNT_SUSPENDED, "pt", PT_ACCOUNT_SUSPENDED),
    (KEY_RATE_LIMITED, "pt", PT_RATE_LIMITED),
    (KEY_QUOTA_EXCEEDED, "pt", PT_QUOTA_EXCEEDED),
    (KEY_QUOTA_WARNING, "pt", PT_QUOTA_WARNING),
    (KEY_NO_PROVIDER_CONNECTED, "pt", PT_NO_PROVIDER_CONNECTED),
    (
        KEY_NO_PROVIDER_CONNECTED_WITH_EMAIL,
        "pt",
        PT_NO_PROVIDER_CONNECTED_WITH_EMAIL,
    ),
    (KEY_CONNECT_PROMPT, "pt", PT_CONNECT_PROMPT),
    (KEY_CONNECT_BUTTON, "pt", PT_CONNECT_BUTTON),
    (KEY_CONNECT_TITLE, "pt", PT_CONNECT_TITLE),
    (KEY_PROVIDER_REAUTH_REQUIRED, "pt", PT_PROVIDER_REAUTH_REQUIRED),
    (
        KEY_PROVIDER_REAUTH_REQUIRED_NO_LINK,
        "pt",
        PT_PROVIDER_REAUTH_REQUIRED_NO_LINK,
    ),
    (
        KEY_PROVIDER_RECONNECT_BUTTON,
        "pt",
        PT_PROVIDER_RECONNECT_BUTTON,
    ),
    (KEY_STATUS_HEADER, "pt", PT_STATUS_HEADER),
    (KEY_STATUS_PROVIDERS_NONE, "pt", PT_STATUS_PROVIDERS_NONE),
    (KEY_STATUS_PROVIDERS_LABEL, "pt", PT_STATUS_PROVIDERS_LABEL),
    (KEY_STATUS_GROUPS_LABEL, "pt", PT_STATUS_GROUPS_LABEL),
    (KEY_STATUS_CHANNEL_LABEL, "pt", PT_STATUS_CHANNEL_LABEL),
    (KEY_HELP_HEADER, "pt", PT_HELP_HEADER),
    (KEY_HELP_DOMAIN_GENERAL, "pt", PT_HELP_DOMAIN_GENERAL),
    (KEY_HELP_DOMAIN_GROUP, "pt", PT_HELP_DOMAIN_GROUP),
    (KEY_HELP_DOMAIN_COACH, "pt", PT_HELP_DOMAIN_COACH),
    (KEY_HELP_DOMAIN_DATA, "pt", PT_HELP_DOMAIN_DATA),
    (KEY_HELP_DOMAIN_PROVIDER, "pt", PT_HELP_DOMAIN_PROVIDER),
    (KEY_HELP_DOMAIN_ACCOUNT, "pt", PT_HELP_DOMAIN_ACCOUNT),
    (KEY_HELP_DOMAIN_TRAINING, "pt", PT_HELP_DOMAIN_TRAINING),
    (KEY_HELP_DOMAIN_DISCOVER, "pt", PT_HELP_DOMAIN_DISCOVER),
    (KEY_HELP_FOOTER, "pt", PT_HELP_FOOTER),
    (KEY_LOGOUT_CONFIRM_PROMPT, "pt", PT_LOGOUT_CONFIRM_PROMPT),
    (KEY_PRIVACY_STATUS_LINE, "pt", PT_PRIVACY_STATUS_LINE),
    (KEY_PRIVACY_STATUS_ENABLED, "pt", PT_PRIVACY_STATUS_ENABLED),
    (KEY_PRIVACY_STATUS_DISABLED, "pt", PT_PRIVACY_STATUS_DISABLED),
    (KEY_PRIVACY_ON_CONFIRMATION, "pt", PT_PRIVACY_ON_CONFIRMATION),
    (KEY_PRIVACY_OFF_CONFIRMATION, "pt", PT_PRIVACY_OFF_CONFIRMATION),
    (KEY_TIMEZONE_SET, "pt", PT_TIMEZONE_SET),
    (KEY_TIMEZONE_INVALID, "pt", PT_TIMEZONE_INVALID),
    (KEY_PILLARS_OPENER, "pt", PT_PILLARS_OPENER),
    (KEY_PILLARS_DM_ONLY, "pt", PT_PILLARS_DM_ONLY),
    (KEY_INTAKE_OPENER, "pt", PT_INTAKE_OPENER),
    (KEY_INTAKE_PERSONA, "pt", PT_INTAKE_PERSONA),
    (KEY_INTAKE_PARQ_INTRO, "pt", PT_INTAKE_PARQ_INTRO),
    (KEY_INTAKE_PARQ_HEART_CONDITION, "pt", PT_INTAKE_PARQ_HEART_CONDITION),
    (KEY_INTAKE_PARQ_CHEST_PAIN, "pt", PT_INTAKE_PARQ_CHEST_PAIN),
    (KEY_INTAKE_PARQ_DIZZINESS, "pt", PT_INTAKE_PARQ_DIZZINESS),
    (KEY_INTAKE_PARQ_CHRONIC_CONDITION, "pt", PT_INTAKE_PARQ_CHRONIC_CONDITION),
    (KEY_INTAKE_PARQ_MEDICATION, "pt", PT_INTAKE_PARQ_MEDICATION),
    (KEY_INTAKE_PARQ_JOINT_PROBLEM, "pt", PT_INTAKE_PARQ_JOINT_PROBLEM),
    (KEY_INTAKE_PARQ_SUPERVISED_ONLY, "pt", PT_INTAKE_PARQ_SUPERVISED_ONLY),
    (KEY_INTAKE_YESNO_HINT, "pt", PT_INTAKE_YESNO_HINT),
    (KEY_INTAKE_RETRY, "pt", PT_INTAKE_RETRY),
    (KEY_INTAKE_COMPLETE_CLEAR, "pt", PT_INTAKE_COMPLETE_CLEAR),
    (KEY_INTAKE_COMPLETE_FLAGGED, "pt", PT_INTAKE_COMPLETE_FLAGGED),
    (KEY_PILLARS_START_FAILED, "pt", PT_PILLARS_START_FAILED),
    (KEY_RESET_WALK_INTERRUPTED, "pt", PT_RESET_WALK_INTERRUPTED),
    (KEY_CALIBRATE_OPENER, "pt", PT_CALIBRATE_OPENER),
    (KEY_CALIBRATE_DM_ONLY, "pt", PT_CALIBRATE_DM_ONLY),
    (KEY_CALIBRATE_START_FAILED, "pt", PT_CALIBRATE_START_FAILED),
    (KEY_CALIBRATE_COMPLETE_HEADER, "pt", PT_CALIBRATE_COMPLETE_HEADER),
    (KEY_CALIBRATE_COMPLETE_MISSING, "pt", PT_CALIBRATE_COMPLETE_MISSING),
    (KEY_CALIBRATE_FOLLOWUP_PLAN, "pt", PT_CALIBRATE_FOLLOWUP_PLAN),
    (KEY_CALIBRATE_FOLLOWUP_NO_PLAN, "pt", PT_CALIBRATE_FOLLOWUP_NO_PLAN),
    (KEY_CALIBRATE_TOPIC_INJURY, "pt", PT_CALIBRATE_TOPIC_INJURY),
    (KEY_CALIBRATE_TOPIC_RECOVERY, "pt", PT_CALIBRATE_TOPIC_RECOVERY),
    (KEY_PLAN_GOAL_LINE, "pt", PT_PLAN_GOAL_LINE),
    (KEY_PLAN_BLOCK_LINE, "pt", PT_PLAN_BLOCK_LINE),
    (KEY_PLAN_DAY_LINE, "pt", PT_PLAN_DAY_LINE),
    (KEY_PLAN_REST, "pt", PT_PLAN_REST),
    (KEY_PLAN_TODAY, "pt", PT_PLAN_TODAY),
    (KEY_PLAN_TOMORROW, "pt", PT_PLAN_TOMORROW),
    (KEY_PLAN_WEEK_HEADER, "pt", PT_PLAN_WEEK_HEADER),
    (KEY_PLAN_NO_SESSION, "pt", PT_PLAN_NO_SESSION),
    (KEY_PLAN_NO_COVERAGE, "pt", PT_PLAN_NO_COVERAGE),
    (KEY_PLAN_RESUMES, "pt", PT_PLAN_RESUMES),
    (KEY_PLAN_EMPTY, "pt", PT_PLAN_EMPTY),
    (KEY_PLAN_STALE_GOAL, "pt", PT_PLAN_STALE_GOAL),
    (KEY_GROUP_LIST_EMPTY, "pt", PT_GROUP_LIST_EMPTY),
    (KEY_GROUP_LIST_HEADER, "pt", PT_GROUP_LIST_HEADER),
    (KEY_GROUP_LIST_ITEM, "pt", PT_GROUP_LIST_ITEM),
    (KEY_GROUP_NOT_A_MEMBER, "pt", PT_GROUP_NOT_A_MEMBER),
    (KEY_GROUP_STATUS_SUMMARY, "pt", PT_GROUP_STATUS_SUMMARY),
    (KEY_GROUP_PEER_SHARING_ON, "pt", PT_GROUP_PEER_SHARING_ON),
    (KEY_GROUP_PEER_SHARING_OFF, "pt", PT_GROUP_PEER_SHARING_OFF),
    (KEY_GROUP_MEMBERS_HEADER, "pt", PT_GROUP_MEMBERS_HEADER),
    (KEY_GROUP_MEMBERS_UNKNOWN, "pt", PT_GROUP_MEMBERS_UNKNOWN),
    (KEY_GROUP_MEMBERS_ITEM, "pt", PT_GROUP_MEMBERS_ITEM),
    (KEY_GROUP_ROLE_OWNER, "pt", PT_GROUP_ROLE_OWNER),
    (KEY_GROUP_ROLE_ADMIN, "pt", PT_GROUP_ROLE_ADMIN),
    (KEY_GROUP_ROLE_MEMBER, "pt", PT_GROUP_ROLE_MEMBER),
    (KEY_GROUP_INVITE_FORBIDDEN, "pt", PT_GROUP_INVITE_FORBIDDEN),
    (KEY_GROUP_INVITE_BODY, "pt", PT_GROUP_INVITE_BODY),
    (KEY_GROUP_INVITE_UNAVAILABLE, "pt", PT_GROUP_INVITE_UNAVAILABLE),
    (KEY_COACH_INVITE_BODY, "pt", PT_COACH_INVITE_BODY),
    (KEY_GROUP_LEAVE_PROMPT, "pt", PT_GROUP_LEAVE_PROMPT),
    (KEY_GROUP_CONSENT_USAGE, "pt", PT_GROUP_CONSENT_USAGE),
    (KEY_GROUP_RESPOND_USAGE, "pt", PT_GROUP_RESPOND_USAGE),
    (KEY_GROUP_RESPOND_MENTIONS, "pt", PT_GROUP_RESPOND_MENTIONS),
    (KEY_GROUP_RESPOND_ALL, "pt", PT_GROUP_RESPOND_ALL),
    (KEY_GROUP_RESPOND_STATUS_MENTIONS, "pt", PT_GROUP_RESPOND_STATUS_MENTIONS),
    (KEY_GROUP_COACH_DETACHED, "pt", PT_GROUP_COACH_DETACHED),
    (KEY_NOTIFICATION_CHANNEL_BODY, "pt", PT_NOTIFICATION_CHANNEL_BODY),
    (KEY_GROUP_CONSENT_UPDATED, "pt", PT_GROUP_CONSENT_UPDATED),
    (KEY_GROUP_CREATE_USAGE, "pt", PT_GROUP_CREATE_USAGE),
    (KEY_GROUP_CREATE_NO_COACH, "pt", PT_GROUP_CREATE_NO_COACH),
    (KEY_GROUP_CREATE_UNAVAILABLE, "pt", PT_GROUP_CREATE_UNAVAILABLE),
    (KEY_GROUP_CREATE_FORBIDDEN, "pt", PT_GROUP_CREATE_FORBIDDEN),
    (KEY_GROUP_CREATED, "pt", PT_GROUP_CREATED),
    (KEY_GROUP_INVITE_LABEL, "pt", PT_GROUP_INVITE_LABEL),
    (KEY_GROUP_JOIN_INVALID_CODE, "pt", PT_GROUP_JOIN_INVALID_CODE),
    (KEY_GROUP_JOIN_ALREADY_MEMBER, "pt", PT_GROUP_JOIN_ALREADY_MEMBER),
    (KEY_GROUP_JOIN_FULL, "pt", PT_GROUP_JOIN_FULL),
    (KEY_GROUP_JOINED, "pt", PT_GROUP_JOINED),
    (KEY_GROUP_JOINED_AS_COACH, "pt", PT_GROUP_JOINED_AS_COACH),
    (KEY_DISCOVER_CARD_TITLE, "pt", PT_DISCOVER_CARD_TITLE),
    (KEY_DISCOVER_ITEM, "pt", PT_DISCOVER_ITEM),
    (KEY_DISCOVER_EMPTY, "pt", PT_DISCOVER_EMPTY),
    (KEY_DISCOVER_CATALOGUE_EMPTY, "pt", PT_DISCOVER_CATALOGUE_EMPTY),
    (KEY_DISCOVER_MORE_LABEL, "pt", PT_DISCOVER_MORE_LABEL),
    (KEY_DISCOVER_INSTALL_USAGE, "pt", PT_DISCOVER_INSTALL_USAGE),
    (KEY_DISCOVER_INSTALL_UNKNOWN_HANDLE, "pt", PT_DISCOVER_INSTALL_UNKNOWN_HANDLE),
    (KEY_DISCOVER_INSTALLED, "pt", PT_DISCOVER_INSTALLED),
    (KEY_DISCOVER_INSTALL_ALREADY, "pt", PT_DISCOVER_INSTALL_ALREADY),
    (KEY_DISCOVER_ADD_LABEL, "pt", PT_DISCOVER_ADD_LABEL),
    (KEY_COACH_LIST_EMPTY, "pt", PT_COACH_LIST_EMPTY),
    (KEY_COACH_LIST_CARD_TITLE, "pt", PT_COACH_LIST_CARD_TITLE),
    (KEY_COACH_LIST_ITEM, "pt", PT_COACH_LIST_ITEM),
    (KEY_COACH_LIST_ITEM_NO_HANDLE, "pt", PT_COACH_LIST_ITEM_NO_HANDLE),
    (KEY_COACH_LIST_FOOTER, "pt", PT_COACH_LIST_FOOTER),
    (KEY_COACH_NO_DESCRIPTION, "pt", PT_COACH_NO_DESCRIPTION),
    (KEY_COACH_GROUP_UPDATED, "pt", PT_COACH_GROUP_UPDATED),
    (KEY_COACH_USER_UPDATED, "pt", PT_COACH_USER_UPDATED),
    (KEY_COACH_ASSIGN_NOT_A_MEMBER, "pt", PT_COACH_ASSIGN_NOT_A_MEMBER),
    (KEY_COACH_ASSIGN_FORBIDDEN, "pt", PT_COACH_ASSIGN_FORBIDDEN),
    (KEY_COACH_ADD_USAGE, "pt", PT_COACH_ADD_USAGE),
    (KEY_COACH_ADD_UNKNOWN, "pt", PT_COACH_ADD_UNKNOWN),
    (KEY_COACH_REMOVE_GROUP_THREAD, "pt", PT_COACH_REMOVE_GROUP_THREAD),
    (KEY_COACH_REMOVE_NOTHING, "pt", PT_COACH_REMOVE_NOTHING),
    (KEY_COACH_REMOVED, "pt", PT_COACH_REMOVED),
    (KEY_COACH_CREATE_NO_CONVERSATION, "pt", PT_COACH_CREATE_NO_CONVERSATION),
    (KEY_COACH_CREATE_EMPTY, "pt", PT_COACH_CREATE_EMPTY),
    (KEY_COACH_CREATE_USAGE, "pt", PT_COACH_CREATE_USAGE),
    (KEY_COACH_CREATE_CARD_TITLE, "pt", PT_COACH_CREATE_CARD_TITLE),
    (KEY_COACH_CREATE_PROPOSAL_BODY, "pt", PT_COACH_CREATE_PROPOSAL_BODY),
    (KEY_COACH_CREATE_CONFIRM_LABEL, "pt", PT_COACH_CREATE_CONFIRM_LABEL),
    (KEY_COACH_CREATE_DISCARD_LABEL, "pt", PT_COACH_CREATE_DISCARD_LABEL),
    (KEY_COACH_CREATE_QUOTA, "pt", PT_COACH_CREATE_QUOTA),
    (KEY_COACH_CREATE_DONE, "pt", PT_COACH_CREATE_DONE),
    (KEY_COACH_CREATE_DONE_UNBOUND, "pt", PT_COACH_CREATE_DONE_UNBOUND),
    (KEY_COACH_CREATE_DISCARDED, "pt", PT_COACH_CREATE_DISCARDED),
    (KEY_RESET_CONFIRM, "pt", PT_RESET_CONFIRM),
];

/// A single localized messaging string entry in the registry.
#[derive(Debug, Clone)]
pub struct MessagingStringEntry {
    /// The raw template string (may contain `{0}`, `{1}`, … placeholders).
    pub content: String,
    /// SHA-256 hex digest of the content bytes.
    pub sha256: String,
    /// Where this entry was loaded from.
    pub source: PromptSource,
    /// When this entry was loaded or last updated.
    pub loaded_at: DateTime<Utc>,
}

/// Two-level storage: `key → locale → entry`. Nested so locale fallback is
/// a cheap pointer lookup and so admin/diagnostic code can iterate all
/// translations of a single key without a table scan.
type LocaleMap = HashMap<String, HashMap<String, MessagingStringEntry>>;

/// Thread-safe registry for user-facing messaging strings, keyed by
/// `(message_key, locale)`.
///
/// Initialized with compiled-in FR/EN/ES/DE/PT defaults. Additional
/// locales become available when the contremaitre sync downloads them
/// from the GitHub repo and calls [`MessagingStringsRegistry::update`].
pub struct MessagingStringsRegistry {
    entries: RwLock<LocaleMap>,
}

impl MessagingStringsRegistry {
    /// Create a registry populated with the compiled-in defaults.
    #[must_use]
    pub fn new() -> Self {
        let now = Utc::now();
        let mut entries: LocaleMap = HashMap::new();
        for (key, locale, content) in COMPILED_IN {
            let sha256 = compute_sha256(content.as_bytes());
            entries.entry((*key).to_owned()).or_default().insert(
                (*locale).to_owned(),
                MessagingStringEntry {
                    content: (*content).to_owned(),
                    sha256,
                    source: PromptSource::CompiledIn,
                    loaded_at: now,
                },
            );
        }
        Self {
            entries: RwLock::new(entries),
        }
    }

    /// Get the template for `(key, locale)` using the documented fallback
    /// chain: requested locale → [`DEFAULT_LOCALE`] → compiled-in default
    /// for [`DEFAULT_LOCALE`] → empty string.
    ///
    /// Returns an owned `String` because the underlying `RwLock` guard
    /// must not escape the function.
    #[must_use]
    pub fn get(&self, key: &str, locale: &str) -> String {
        let guard = self.read();
        if let Some(per_locale) = guard.get(key) {
            if let Some(entry) = per_locale.get(locale) {
                return entry.content.clone();
            }
            if locale != DEFAULT_LOCALE {
                if let Some(entry) = per_locale.get(DEFAULT_LOCALE) {
                    return entry.content.clone();
                }
            }
        }
        drop(guard);
        compiled_in_fallback(key, DEFAULT_LOCALE)
            .unwrap_or("")
            .to_owned()
    }

    /// Render a template for `(key, locale)` with positional arguments.
    ///
    /// Convenience wrapper around [`Self::get`] + [`format_template`]; keeps
    /// call sites a one-liner instead of a two-step dance.
    #[must_use]
    pub fn render(&self, key: &str, locale: &str, args: &[&str]) -> String {
        let template = self.get(key, locale);
        format_template(&template, args)
    }

    /// Get the SHA-256 hash for `(key, locale)` (used by the sync engine
    /// to skip unchanged entries during webhook hot-reloads).
    #[must_use]
    pub fn sha256(&self, key: &str, locale: &str) -> Option<String> {
        self.read()
            .get(key)
            .and_then(|per_locale| per_locale.get(locale))
            .map(|entry| entry.sha256.clone())
    }

    /// Insert or update an entry for `(key, locale)`. Called by the sync
    /// engine when a newer version of a string is downloaded.
    pub fn update(&self, key: &str, locale: &str, content: String, sha256: String) {
        self.write().entry(key.to_owned()).or_default().insert(
            locale.to_owned(),
            MessagingStringEntry {
                content,
                sha256,
                source: PromptSource::Contremaitre,
                loaded_at: Utc::now(),
            },
        );
    }

    /// List every `(key, locale, entry)` triple currently in the registry
    /// (for admin/diagnostic UIs).
    #[must_use]
    pub fn list(&self) -> Vec<(String, String, MessagingStringEntry)> {
        let guard = self.read();
        let mut out = Vec::new();
        for (key, per_locale) in guard.iter() {
            for (locale, entry) in per_locale {
                out.push((key.clone(), locale.clone(), entry.clone()));
            }
        }
        out
    }

    /// Count of distinct message keys in the registry (across all locales).
    #[must_use]
    pub fn key_count(&self) -> usize {
        self.read().len()
    }

    /// Total count of `(key, locale)` entries in the registry.
    #[must_use]
    pub fn entry_count(&self) -> usize {
        self.read().values().map(HashMap::len).sum()
    }

    fn read(&self) -> RwLockReadGuard<'_, LocaleMap> {
        self.entries.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, LocaleMap> {
        self.entries.write().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for MessagingStringsRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Look up the compiled-in default for `(key, locale)` from the `COMPILED_IN`
/// table. Returns `None` when the combination is not shipped with the binary.
///
/// Used as the final fallback when the registry itself is missing an entry,
/// which shouldn't happen for the built-in keys but keeps lookups infallible.
fn compiled_in_fallback(key: &str, locale: &str) -> Option<&'static str> {
    COMPILED_IN
        .iter()
        .find(|(k, l, _)| *k == key && *l == locale)
        .map(|(_, _, content)| *content)
}

/// Substitute positional placeholders `{0}`, `{1}`, … in `template`.
///
/// Matches each placeholder with the corresponding entry in `args`.
/// Placeholders without a corresponding argument are left literally in
/// the output. Surplus args are ignored.
///
/// Chosen over handlebars/minijinja per the 2026-04-15 audit gist decision
/// (Option B — zero new dependencies, unambiguous indexing).
#[must_use]
pub fn format_template(template: &str, args: &[&str]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.char_indices().peekable();
    while let Some((_, c)) = chars.next() {
        if c != '{' {
            out.push(c);
            continue;
        }
        // Try to parse `{N}` where N is a run of ASCII digits. If anything
        // else is between the braces, emit the opening brace literally and
        // continue — preserves any `{X}` tokens we don't own.
        let mut digits = String::new();
        let mut closed = false;
        while let Some(&(_, next)) = chars.peek() {
            if next.is_ascii_digit() {
                digits.push(next);
                chars.next();
            } else if next == '}' {
                chars.next();
                closed = true;
                break;
            } else {
                break;
            }
        }
        if closed && !digits.is_empty() {
            if let Ok(idx) = digits.parse::<usize>() {
                if let Some(value) = args.get(idx) {
                    out.push_str(value);
                    continue;
                }
            }
        }
        // Not a recognized placeholder — reconstitute the literal text so
        // the template is preserved byte-for-byte.
        out.push('{');
        out.push_str(&digits);
        if closed {
            out.push('}');
        }
    }
    out
}
