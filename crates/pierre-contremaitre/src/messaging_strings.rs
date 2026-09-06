// ABOUTME: Hot-reloadable locale-aware registry for every user-facing string, server- and client-rendered
// ABOUTME: Seeded from the five embedded translation.json catalogue files; contremaitre overlays them at runtime
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
//
// file-size-ok: one `KEY_*` literal per server-rendered catalogue key — data the
// Tier 1b gate reads from this file, not logic; the registry itself is ~250 lines.

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
//! ## Where the strings live
//!
//! In the five `packages/i18n/src/locales/<locale>/translation.json` files —
//! the same files the web and mobile apps import. This module embeds them at
//! build time (`include_str!`), flattens the nested objects to dotted keys and
//! seeds the registry, so the server, the web app and the phone can never
//! disagree about what a key says. Every key ships in all five locales: Tier 1b
//! (`scripts/ci/check-contremaitre-sync.sh`) and `contremaitre_test.rs` both
//! fail otherwise. The `KEY_*` constants below name the server-rendered subset
//! — positional `{0}` placeholders, filled by [`format_template`]; a
//! client-rendered key uses i18next's `{{name}}` and is never read here.
//!
//! ## Locale model
//!
//! Lookups follow the chain:
//!
//! 1. `(key, requested_locale)` — exact match
//! 2. `(key, DEFAULT_LOCALE)` — fall back to the default locale (`"fr"`)
//! 3. Empty string
//!
//! Contremaitre overlays the embedded text at runtime through a sparse
//! per-locale bundle, `strings/<locale>.json`, holding only the keys to
//! override in the catalogue's own nested shape; the manifest lists each
//! bundle and the next sync applies it ([`MessagingStringsRegistry::apply_bundle`]).
//! `GET /api/i18n/{locale}` serves the overlaid registry to the clients, so
//! a fix pushed to contremaitre reaches every surface without a deploy.
//!
//! ## Templating
//!
//! Values may contain positional placeholders (`{0}`, `{1}`, …) that
//! callers fill in via [`format_template`] (Option B from the 2026-04-15
//! audit gist — zero new dependencies, unambiguous indexing).

use std::collections::HashMap;
use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use chrono::{DateTime, Utc};
use serde_json::Value;
use tracing::{error, warn};

use pierre_core::models::SUPPORTED_LOCALES;

use super::manifest::compute_sha256;
use super::registry::PromptSource;

/// Default locale used when a caller does not specify one.
///
/// Also used when the requested locale is missing from the registry. It is the
/// first entry of [`SUPPORTED_LOCALES`] rather than its own literal, so the
/// default follows the one list instead of drifting from it — currently
/// French, because the majority user base is francophone.
pub const DEFAULT_LOCALE: &str = SUPPORTED_LOCALES[0];

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
/// Key: the turn was cut short before producing anything.
///
/// Its wall-clock ceiling elapsed, or the instance running it shut down.
/// Closes the status placeholder that would otherwise stay open forever.
/// No format placeholders.
pub const KEY_TURN_INTERRUPTED: &str = "messaging.turn_interrupted";
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
/// Key: the language this turn is conducted in.
///
/// Appended to the tail of every system prompt, on every surface, rendered
/// from the locale the turn already resolved — the same value that selects the
/// coach prompt, the refusals above, the acronym glosses and the guardrail
/// disclaimer. Each locale's text is authored in its own language, so the
/// directive is itself an instance of what it asks for.
pub const KEY_TURN_LANGUAGE: &str = "messaging.turn.language";
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

/// Key: title of the app-push completion notice. No placeholders.
///
/// Raised when the conversation is an in-app one, which has no outbound
/// channel to send on: the notice itself is a persisted turn in the thread and
/// this notification is what tells the athlete it landed.
pub const KEY_BACKFILL_PUSH_TITLE: &str = "messaging.backfill.push_title";

/// Key: body of that same notification. `{0}` = count of activities loaded.
///
/// Deliberately a one-line count pointing at the chat rather than the activity
/// list, which is already the persisted turn and does not fit a lock screen.
pub const KEY_BACKFILL_PUSH_BODY: &str = "messaging.backfill.push_body";

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

/// Key: the sender is past the plan's limit and inside the burst allowance.
///
/// Distinct from [`KEY_QUOTA_WARNING`] because the two say different things and
/// one template cannot say both. The warning renders "{used} of {limit}", which
/// is nonsense once `used` exceeds `limit` — live on 2026-09-02 an athlete was
/// told *"tu as utilisé 670828 de 500000 sur ton forfait"* four turns running
/// (registre#251). The burst line names the limit and the reset, and never
/// prints the comparison.
///
/// Also distinct from [`KEY_QUOTA_EXCEEDED`], which is for a turn that was
/// REFUSED. A burst turn ran and the athlete got their answer.
pub const KEY_QUOTA_BURST: &str = "messaging.quota_burst";

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

/// Key: a provider needs reconnecting on a turn the athlete's OTHER
/// connections already answered.
///
/// [`KEY_PROVIDER_REAUTH_REQUIRED`] says the coach could not retrieve the data,
/// which contradicts an answer that just delivered it. A multi-source athlete
/// whose watch token died still gets their history from a healthy connection,
/// so this copy stands beside that answer: what you asked for is above, `{0}` is
/// disconnected, reconnect it to see everything. `{0}` = provider display name;
/// `{1}` = one-time hosted-login URL.
pub const KEY_PROVIDER_REAUTH_SERVED: &str = "messaging.provider.reauth_served";

/// The same standing as [`KEY_PROVIDER_REAUTH_SERVED`], said without a link.
///
/// Minting can fail on a served turn exactly as it can on a blank one, and the
/// answer above it is real either way. Naming the disconnected provider is the
/// part that survives without a URL. `{0}` = provider display name, and there is
/// no URL placeholder.
pub const KEY_PROVIDER_REAUTH_SERVED_NO_LINK: &str = "messaging.provider.reauth_served_no_link";

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
/// Key: `/pillars` opener for a walk started in a shared room.
///
/// Says out loud what the athlete just chose — the exchange is visible to the
/// room and theirs alone to answer — and points them at a direct message for
/// the two pillars a room walk never covers (stress/mind and recovery habits).
pub const KEY_PILLARS_OPENER_ROOM: &str = "commands.pillars.opener_room";
/// Key: `/pillars` re-screen refusal in a shared room.
///
/// `full` and the DM-only pillars (mental, substances) supersede facts a room
/// walk can never re-ask, so those arguments are refused there — before any
/// expiry runs — and the athlete is pointed at a direct message.
pub const KEY_PILLARS_ARG_DM_ONLY: &str = "commands.pillars.arg_dm_only";

/// Posted in a shared room when a command was answered in the caller's DM.
///
/// Carries no answer, only its whereabouts: a room may hold several athletes,
/// so the reply itself stays private. Without this the room shows a command
/// and no response at all.
pub const KEY_SLASH_ANSWERED_PRIVATELY: &str = "commands.answered_privately";
/// Key: `/pillars` failure when the walk could not be activated on the
/// conversation.
pub const KEY_PILLARS_START_FAILED: &str = "commands.pillars.start_failed";
/// Key: `/reset` note appended when the reset ended an in-progress profile walk.
pub const KEY_RESET_WALK_INTERRUPTED: &str = "commands.reset.walk_interrupted";
/// Key: the word a freshly opened conversation is named after.
///
/// Sits before its date and time. Shared with the clients' own "+" button,
/// which stamps the same shape; the server needs it for a thread it forges
/// itself.
pub const KEY_NEW_CONVERSATION_TITLE_PREFIX: &str = "chat.newConversationTitlePrefix";

// ── memory fact sentences ──────────────────────────────────────────────────
// One key per `PredicateCode`; `{0}` is the athlete's own words (the object).
// Rendered once, on the server, in the athlete's locale — for the memory
// screen, the recall tool and the coach dossier alike.
/// Key: memory fact sentence for the `training_for` predicate code. Args: `{0}` object.
pub const KEY_MEMORY_PREDICATE_TRAINING_FOR: &str = "messaging.memory.predicate.training_for";
/// Key: memory fact sentence for the `working_toward` predicate code. Args: `{0}` object.
pub const KEY_MEMORY_PREDICATE_WORKING_TOWARD: &str = "messaging.memory.predicate.working_toward";
/// Key: memory fact sentence for the `target_race` predicate code. Args: `{0}` object.
pub const KEY_MEMORY_PREDICATE_TARGET_RACE: &str = "messaging.memory.predicate.target_race";
/// Key: memory fact sentence for the `prefer` predicate code. Args: `{0}` object.
pub const KEY_MEMORY_PREDICATE_PREFER: &str = "messaging.memory.predicate.prefer";
/// Key: memory fact sentence for the `avoid` predicate code. Args: `{0}` object.
pub const KEY_MEMORY_PREDICATE_AVOID: &str = "messaging.memory.predicate.avoid";
/// Key: memory fact sentence for the `primarily_train` predicate code. Args: `{0}` object.
pub const KEY_MEMORY_PREDICATE_PRIMARILY_TRAIN: &str = "messaging.memory.predicate.primarily_train";
/// Key: memory fact sentence for the `have_baseline` predicate code. Args: `{0}` object.
pub const KEY_MEMORY_PREDICATE_HAVE_BASELINE: &str = "messaging.memory.predicate.have_baseline";
/// Key: memory fact sentence for the `have` predicate code. Args: `{0}` object.
pub const KEY_MEMORY_PREDICATE_HAVE: &str = "messaging.memory.predicate.have";
/// Key: memory fact sentence for the `recovering_from` predicate code. Args: `{0}` object.
pub const KEY_MEMORY_PREDICATE_RECOVERING_FROM: &str = "messaging.memory.predicate.recovering_from";
/// Key: memory fact sentence for the `can_train_on` predicate code. Args: `{0}` object.
pub const KEY_MEMORY_PREDICATE_CAN_TRAIN_ON: &str = "messaging.memory.predicate.can_train_on";
/// Key: memory fact sentence for the `cannot_train_on` predicate code. Args: `{0}` object.
pub const KEY_MEMORY_PREDICATE_CANNOT_TRAIN_ON: &str = "messaging.memory.predicate.cannot_train_on";
/// Key: memory fact sentence for the `need_session_on` predicate code. Args: `{0}` object.
pub const KEY_MEMORY_PREDICATE_NEED_SESSION_ON: &str = "messaging.memory.predicate.need_session_on";
/// Key: memory fact sentence for the `unavailable` predicate code. Args: `{0}` object.
pub const KEY_MEMORY_PREDICATE_UNAVAILABLE: &str = "messaging.memory.predicate.unavailable";
/// Key: memory fact sentence for the `own` predicate code. Args: `{0}` object.
pub const KEY_MEMORY_PREDICATE_OWN: &str = "messaging.memory.predicate.own";
/// Key: memory fact sentence for the `train_on` predicate code. Args: `{0}` object.
pub const KEY_MEMORY_PREDICATE_TRAIN_ON: &str = "messaging.memory.predicate.train_on";
/// Key: memory fact sentence for the `train_because` predicate code. Args: `{0}` object.
pub const KEY_MEMORY_PREDICATE_TRAIN_BECAUSE: &str = "messaging.memory.predicate.train_because";
/// Key: memory fact sentence for the `parq_yes` predicate code. Args: `{0}` object.
pub const KEY_MEMORY_PREDICATE_PARQ_YES: &str = "messaging.memory.predicate.parq_yes";
/// Key: memory fact sentence for the `flagged` predicate code. Args: `{0}` object.
pub const KEY_MEMORY_PREDICATE_FLAGGED: &str = "messaging.memory.predicate.flagged";
/// Key: memory fact sentence for the `states` predicate code. Args: `{0}` object.
pub const KEY_MEMORY_PREDICATE_STATES: &str = "messaging.memory.predicate.states";

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
/// Key: `/calibrate` opener for an interview started in a shared room.
///
/// The private opener plus the visibility contract: the answers appear here,
/// they are the caller's alone to give, and the room reads along.
pub const KEY_CALIBRATE_OPENER_ROOM: &str = "commands.calibrate.opener_room";
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
/// Key: `/plan` current-phase line. `{0}` = the phase kind, `{1}` = the hours suffix.
pub const KEY_PLAN_PHASE_LINE: &str = "commands.plan.phase_line";
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
/// Key: `/plan share` header opening the reply posted to a messaging room.
/// `{0}` = the caller's display name, HTML-escaped by the handler because the
/// reply is rich text.
pub const KEY_PLAN_SHARED_HEADER: &str = "commands.plan.shared_header";

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

/// Key: notice appended to a coach reply after the tenant-isolation stage
/// redacted a section citing an athlete outside the coach's roster.
pub const KEY_PERSONA_ISOLATION_REDACTED: &str = "persona.isolation.redacted";

/// Key: one-line summary of the Casual persona.
pub const KEY_PERSONA_SUMMARY_CASUAL: &str = "persona.summary.casual";

/// Key: one-line summary of the Enthusiast persona.
pub const KEY_PERSONA_SUMMARY_ENTHUSIAST: &str = "persona.summary.enthusiast";

/// Key: one-line summary of the Power-athlete persona.
pub const KEY_PERSONA_SUMMARY_POWER_ATHLETE: &str = "persona.summary.power_athlete";

/// Key: one-line summary of the Coach persona.
pub const KEY_PERSONA_SUMMARY_COACH: &str = "persona.summary.coach";

/// Key: contract rule — reply word cap. `{0}` = the cap.
pub const KEY_PERSONA_RULE_MAX_WORDS: &str = "persona.rule.max_words";

/// Key: contract rule — lists at or above a threshold collapse to prose.
/// `{0}` = the threshold.
pub const KEY_PERSONA_RULE_NO_LONG_LISTS: &str = "persona.rule.no_long_lists";

/// Key: contract rule — no framework citations or technical acronyms.
pub const KEY_PERSONA_RULE_NO_CITATIONS: &str = "persona.rule.no_citations";

/// Key: contract rule — replies are prose, never structured label/value blocks.
pub const KEY_PERSONA_RULE_PROSE_ONLY: &str = "persona.rule.prose_only";

/// Key: contract rule — numbers are rounded for readability.
pub const KEY_PERSONA_RULE_ROUNDED_NUMBERS: &str = "persona.rule.rounded_numbers";

/// Key: contract rule — no tool-call narration in replies.
pub const KEY_PERSONA_RULE_NO_NARRATION: &str = "persona.rule.no_narration";

/// Key: contract rule — acronyms are glossed at first occurrence.
pub const KEY_PERSONA_RULE_ACRONYMS_GLOSSED: &str = "persona.rule.acronyms_glossed";

/// Key: contract rule — structured blocks stay short. `{0}` = the line cap.
pub const KEY_PERSONA_RULE_SHORT_BLOCKS: &str = "persona.rule.short_blocks";

/// Key: contract rule — framework citation on every quantified claim.
pub const KEY_PERSONA_RULE_CITATIONS_REQUIRED: &str = "persona.rule.citations_required";

/// Key: contract rule — line-by-line activity blocks.
pub const KEY_PERSONA_RULE_LINE_BY_LINE: &str = "persona.rule.line_by_line";

/// Key: contract rule — no conversational softeners.
pub const KEY_PERSONA_RULE_NO_SOFTENERS: &str = "persona.rule.no_softeners";

/// Key: contract rule — exact numbers, never rounded.
pub const KEY_PERSONA_RULE_EXACT_NUMBERS: &str = "persona.rule.exact_numbers";

/// Key: contract rule — P0–P3 readiness ladder on every verdict.
pub const KEY_PERSONA_RULE_P0_P3_LADDER: &str = "persona.rule.p0_p3_ladder";

/// Key: contract rule — every data block names the athlete it belongs to.
pub const KEY_PERSONA_RULE_ATHLETE_ATTRIBUTION: &str = "persona.rule.athlete_attribution";

/// Key: contract rule — athlete citations are verified against the coach's roster.
pub const KEY_PERSONA_RULE_ROSTER_VERIFIED: &str = "persona.rule.roster_verified";

/// Key: enforcement badge when the effective contract runs `strict_mode`.
pub const KEY_PERSONA_ENFORCEMENT_VERIFIED: &str = "persona.enforcement.verified";

/// Key: enforcement badge when contract violations are advisory (log-only).
pub const KEY_PERSONA_ENFORCEMENT_ADVISORY: &str = "persona.enforcement.advisory";

/// Key: title of the weekly persona-digest notification — the roll-up of
/// pushes the persona notification policy held back.
pub const KEY_NOTIFICATION_DIGEST_TITLE: &str = "notifications.digest.title";

/// Key: body of the weekly persona-digest notification. `{0}` = how many
/// notifications were held for the digest since the previous one.
pub const KEY_NOTIFICATION_DIGEST_BODY: &str = "notifications.digest.body";

// ── Notification event keys ───────────────────────────────────────────────
//
// A notification row stores the event that happened plus its parameters;
// `pierre_notifications::NotificationEvent` names these keys and
// `pierre_services::notification_text` renders them, so the same row reads
// French for a French athlete and English for an English one. Each `{n}` is
// the parameter the event declares in that position.

/// Key: a provider delivered a new activity. No format placeholders.
pub const KEY_NOTIFICATION_ACTIVITY_SYNCED_TITLE: &str =
    "notifications.event.activity_synced.title";
/// Key: the activity's line. `{0}` type, `{1}` distance, `{2}` duration.
pub const KEY_NOTIFICATION_ACTIVITY_SYNCED_BODY: &str = "notifications.event.activity_synced.body";
/// Key: acute training load crossed its threshold. No format placeholders.
pub const KEY_NOTIFICATION_TRAINING_LOAD_ALERT_TITLE: &str =
    "notifications.event.training_load_alert.title";
/// Key: the load reading. `{0}` = the ATL value.
pub const KEY_NOTIFICATION_TRAINING_LOAD_ALERT_BODY: &str =
    "notifications.event.training_load_alert.body";
/// Key: the recovery score dropped. No format placeholders.
pub const KEY_NOTIFICATION_LOW_RECOVERY_SCORE_TITLE: &str =
    "notifications.event.low_recovery_score.title";
/// Key: the recovery reading. `{0}` = the score out of 100.
pub const KEY_NOTIFICATION_LOW_RECOVERY_SCORE_BODY: &str =
    "notifications.event.low_recovery_score.body";
/// Key: the stress trend suggests fatigue. No format placeholders.
pub const KEY_NOTIFICATION_OVERTRAINING_WARNING_TITLE: &str =
    "notifications.event.overtraining_warning.title";
/// Key: what the trend shows. No format placeholders.
pub const KEY_NOTIFICATION_OVERTRAINING_WARNING_BODY: &str =
    "notifications.event.overtraining_warning.body";
/// Key: a personal record was detected. No format placeholders.
pub const KEY_NOTIFICATION_PERSONAL_RECORD_TITLE: &str =
    "notifications.event.personal_record.title";
/// Key: the record itself. `{0}` distance label, `{1}` time.
pub const KEY_NOTIFICATION_PERSONAL_RECORD_BODY: &str = "notifications.event.personal_record.body";
/// Key: a cumulative milestone was reached. No format placeholders.
pub const KEY_NOTIFICATION_MILESTONE_REACHED_TITLE: &str =
    "notifications.event.milestone_reached.title";
/// Key: the milestone itself. `{0}` value, `{1}` unit.
pub const KEY_NOTIFICATION_MILESTONE_REACHED_BODY: &str =
    "notifications.event.milestone_reached.body";
/// Key: a fitness metric improved. No format placeholders.
pub const KEY_NOTIFICATION_FITNESS_IMPROVEMENT_TITLE: &str =
    "notifications.event.fitness_improvement.title";
/// Key: which metric moved. `{0}` metric name, `{1}` new value.
pub const KEY_NOTIFICATION_FITNESS_IMPROVEMENT_BODY: &str =
    "notifications.event.fitness_improvement.body";
/// Key: a coach wrote to the athlete. No format placeholders.
pub const KEY_NOTIFICATION_COACH_MESSAGE_TITLE: &str = "notifications.event.coach_message.title";
/// Key: who wrote. `{0}` = the coach's name.
pub const KEY_NOTIFICATION_COACH_MESSAGE_BODY: &str = "notifications.event.coach_message.body";
/// Key: a coach revised the training plan. No format placeholders.
pub const KEY_NOTIFICATION_PLAN_UPDATED_TITLE: &str = "notifications.event.plan_updated.title";
/// Key: who revised it. `{0}` = the coach's name.
pub const KEY_NOTIFICATION_PLAN_UPDATED_BODY: &str = "notifications.event.plan_updated.body";
/// Key: a coach left a note on an activity. No format placeholders.
pub const KEY_NOTIFICATION_COACH_FEEDBACK_TITLE: &str = "notifications.event.coach_feedback.title";
/// Key: whose note, on what. `{0}` coach name, `{1}` activity type.
pub const KEY_NOTIFICATION_COACH_FEEDBACK_BODY: &str = "notifications.event.coach_feedback.body";
/// Key: a provider sync failed. `{0}` = the provider's name.
pub const KEY_NOTIFICATION_SYNC_FAILURE_TITLE: &str = "notifications.event.sync_failure.title";
/// Key: why it failed. `{0}` = the error summary.
pub const KEY_NOTIFICATION_SYNC_FAILURE_BODY: &str = "notifications.event.sync_failure.body";
/// Key: several sync failures collapsed into one feed row. `{0}` = how many.
pub const KEY_NOTIFICATION_SYNC_FAILURE_COLLAPSED_TITLE: &str =
    "notifications.event.sync_failure.collapsed_title";
/// Key: the collapsed group's body. `{0}` = how many syncs failed.
pub const KEY_NOTIFICATION_SYNC_FAILURE_COLLAPSED_BODY: &str =
    "notifications.event.sync_failure.collapsed_body";

/// Key: the "reply to your coach" action button. No format placeholders.
pub const KEY_NOTIFICATION_ACTION_REPLY: &str = "notifications.action.reply";
/// Key: the "reconnect this provider" action button. No format placeholders.
pub const KEY_NOTIFICATION_ACTION_RECONNECT: &str = "notifications.action.reconnect";

/// The catalogue, embedded at build time.
///
/// The same five files both clients import. Nested JSON, flattened to dotted
/// keys on load; the registry test asserts the seeded locales are exactly
/// `pierre_core::models::SUPPORTED_LOCALES`.
const CATALOGUE: [(&str, &str); 5] = [
    (
        "fr",
        include_str!("../../../packages/i18n/src/locales/fr/translation.json"),
    ),
    (
        "en",
        include_str!("../../../packages/i18n/src/locales/en/translation.json"),
    ),
    (
        "es",
        include_str!("../../../packages/i18n/src/locales/es/translation.json"),
    ),
    (
        "de",
        include_str!("../../../packages/i18n/src/locales/de/translation.json"),
    ),
    (
        "pt",
        include_str!("../../../packages/i18n/src/locales/pt/translation.json"),
    ),
];

/// Walk a nested catalogue object, collecting `(dotted key, text)` for every leaf.
fn flatten(prefix: &str, value: &Value, out: &mut Vec<(String, String)>) {
    match value {
        Value::Object(map) => {
            for (name, child) in map {
                let key = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}.{name}")
                };
                flatten(&key, child, out);
            }
        }
        Value::String(text) => out.push((prefix.to_owned(), text.clone())),
        other => error!(key = prefix, value = %other, "catalogue leaf is not a string; skipped"),
    }
}

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
    /// Digest of the last override bundle applied per locale, so the sync
    /// engine can skip a bundle the manifest still lists at the same hash.
    bundle_shas: RwLock<HashMap<String, String>>,
}

impl MessagingStringsRegistry {
    /// Create a registry seeded from the embedded catalogue.
    #[must_use]
    pub fn new() -> Self {
        let now = Utc::now();
        let mut entries: LocaleMap = HashMap::new();
        for (locale, json) in CATALOGUE {
            let tree: Value = match serde_json::from_str(json) {
                Ok(tree) => tree,
                Err(error) => {
                    // Tier 1b and the registry test both parse these files, so
                    // a malformed one cannot reach a build; skipping keeps the
                    // other locales serving instead of taking the server down.
                    error!(locale, %error, "embedded catalogue failed to parse; locale not seeded");
                    continue;
                }
            };
            let mut leaves = Vec::new();
            flatten("", &tree, &mut leaves);
            for (key, content) in leaves {
                let sha256 = compute_sha256(content.as_bytes());
                entries.entry(key).or_default().insert(
                    locale.to_owned(),
                    MessagingStringEntry {
                        content,
                        sha256,
                        source: PromptSource::CompiledIn,
                        loaded_at: now,
                    },
                );
            }
        }
        Self {
            entries: RwLock::new(entries),
            bundle_shas: RwLock::new(HashMap::new()),
        }
    }

    /// Apply a sparse override bundle for `locale`.
    ///
    /// `json` is a nested object in the catalogue's own shape; every leaf
    /// replaces the registry's text for that key in that locale. Returns the
    /// number of keys applied. A key the catalogue does not hold is applied
    /// too and logged, so a typo shows up in the sync log instead of
    /// vanishing. Records `bundle_sha256` so an unchanged bundle is skipped
    /// on the next sync.
    ///
    /// # Errors
    ///
    /// Returns the parse error when `json` is not valid JSON; nothing is
    /// applied in that case and the previous strings stay live.
    pub fn apply_bundle(
        &self,
        locale: &str,
        json: &str,
        bundle_sha256: &str,
    ) -> Result<usize, serde_json::Error> {
        let tree: Value = serde_json::from_str(json)?;
        let mut leaves = Vec::new();
        flatten("", &tree, &mut leaves);
        let now = Utc::now();
        let applied = leaves.len();
        {
            let mut guard = self.write();
            for (key, content) in leaves {
                if !guard.contains_key(&key) {
                    warn!(
                        key,
                        locale, "override bundle names a key the catalogue does not hold"
                    );
                }
                let sha256 = compute_sha256(content.as_bytes());
                guard.entry(key).or_default().insert(
                    locale.to_owned(),
                    MessagingStringEntry {
                        content,
                        sha256,
                        source: PromptSource::Contremaitre,
                        loaded_at: now,
                    },
                );
            }
        }
        self.bundle_shas
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(locale.to_owned(), bundle_sha256.to_owned());
        Ok(applied)
    }

    /// Digest of the override bundle last applied for `locale`, if any.
    #[must_use]
    pub fn bundle_sha256(&self, locale: &str) -> Option<String> {
        self.bundle_shas
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(locale)
            .cloned()
    }

    /// Get the template for `(key, locale)` using the documented fallback
    /// chain: requested locale → [`DEFAULT_LOCALE`] → empty string.
    ///
    /// Returns an owned `String` because the underlying `RwLock` guard
    /// must not escape the function.
    #[must_use]
    pub fn get(&self, key: &str, locale: &str) -> String {
        let guard = self.read();
        let Some(per_locale) = guard.get(key) else {
            return String::new();
        };
        if let Some(entry) = per_locale.get(locale) {
            return entry.content.clone();
        }
        per_locale
            .get(DEFAULT_LOCALE)
            .map(|entry| entry.content.clone())
            .unwrap_or_default()
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
