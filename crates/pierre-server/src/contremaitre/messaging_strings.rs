// ABOUTME: Hot-reloadable locale-aware registry for user-facing messaging strings
// ABOUTME: Compiled-in FR/EN/ES/DE/PT defaults; extra locales layer on via contremaitre
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Messaging Strings Registry
//!
//! Holds short user-facing strings that are sent back to Telegram, WhatsApp,
//! Discord, Slack, and other messaging channels. This registry covers:
//!
//! - Chat-pipeline fallbacks (generic error, empty reply, guardrail rewrites,
//!   Tier 5.5 verification warnings)
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
/// Key: LLM returned an empty reply, reformulation request.
pub const KEY_EMPTY_REPLY: &str = "messaging.empty_reply";
/// Key: text-guardrails rejected an over-long response.
pub const KEY_GUARDRAIL_TOO_LONG: &str = "messaging.guardrail.too_long";
/// Key: text-guardrails rejected a blocked-topic response.
pub const KEY_GUARDRAIL_BLOCKED_TOPIC: &str = "messaging.guardrail.blocked_topic";
/// Key: Tier 5.5 `Warn` fallback suffix appended below the LLM reply.
pub const KEY_VERIFICATION_WARN_SUFFIX: &str = "messaging.verification.warn_suffix";
/// Key: Tier 5.5 `Block` fallback that fully replaces the LLM reply.
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
/// Key: logout confirmation sent after the channel link is torn down.
pub const KEY_LINK_LOGOUT_COMPLETE: &str = "messaging.link.logout_complete";
/// Key: user typed `cancel` during the OTP flow.
pub const KEY_LINK_CANCELLED: &str = "messaging.link.cancelled";
/// Key: generic "something went wrong" for unexpected DB/infra errors.
pub const KEY_LINK_GENERIC_ERROR: &str = "messaging.link.generic_error";
/// Key: email-entry matches no Dravr account. `{0}` = registration URL.
pub const KEY_LINK_NO_ACCOUNT: &str = "messaging.link.no_account";
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

/// Key: a connected fitness provider needs to be (re)authenticated.
///
/// Surfaced deterministically by the chat pipeline's auth-recovery
/// short-circuit when the underlying scrape returns
/// `AppError::ProviderAuthRequired`. `{0}` = provider display name
/// (e.g. `Garmin Connect`, `Strava`); `{1}` = one-time hosted-login URL.
pub const KEY_PROVIDER_REAUTH_REQUIRED: &str = "messaging.provider.reauth_required";

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
/// Key: `/help` closing line inviting the user to chat.
pub const KEY_HELP_FOOTER: &str = "commands.help.footer";

// ── /logout command keys ──────────────────────────────────────────────────

/// Key: `/logout` confirmation prompt. `{0}` = channel type.
pub const KEY_LOGOUT_CONFIRM_PROMPT: &str = "commands.logout.confirm_prompt";

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
/// Key: `/group invite` rejection when the user lacks admin rights.
pub const KEY_GROUP_INVITE_FORBIDDEN: &str = "commands.group.invite_forbidden";
/// Key: `/group invite` success body.
/// `{0}` = group name, `{1}` = invite code (URL), `{2}` = invite code (display).
pub const KEY_GROUP_INVITE_BODY: &str = "commands.group.invite_body";
/// Key: `/group invite` unavailable when the groups feature is disabled.
pub const KEY_GROUP_INVITE_UNAVAILABLE: &str = "commands.group.invite_unavailable";
/// Key: `/group leave` confirmation prompt. `{0}` = group name.
pub const KEY_GROUP_LEAVE_PROMPT: &str = "commands.group.leave_prompt";
/// Key: `/group consent` usage hint when the argument is missing or invalid.
pub const KEY_GROUP_CONSENT_USAGE: &str = "commands.group.consent_usage";
/// Key: `/group consent` confirmation. `{0}` = on/off (peer-sharing localized),
/// `{1}` = group name.
pub const KEY_GROUP_CONSENT_UPDATED: &str = "commands.group.consent_updated";

// ── /coach command keys ───────────────────────────────────────────────────

/// Key: `/coach` empty list.
pub const KEY_COACH_LIST_EMPTY: &str = "commands.coach.list_empty";
/// Key: `/coach` list card title.
pub const KEY_COACH_LIST_CARD_TITLE: &str = "commands.coach.list_card_title";
/// Key: `/coach` list item. `{0}` = title, `{1}` = category, `{2}` = description.
pub const KEY_COACH_LIST_ITEM: &str = "commands.coach.list_item";
/// Key: `/coach` description fallback when the coach has no description set.
pub const KEY_COACH_NO_DESCRIPTION: &str = "commands.coach.no_description";
/// Key: `/coach select` auto-created a new group. `{0}` = group name, `{1}` = coach.
pub const KEY_COACH_GROUP_CREATED: &str = "commands.coach.group_created";
/// Key: `/coach select` result when group creation is unavailable. `{0}` = coach.
pub const KEY_COACH_GROUP_CREATION_UNAVAILABLE: &str = "commands.coach.group_creation_unavailable";
/// Key: `/coach select` / `/coach assign` success. `{0}` = coach, `{1}` = group.
pub const KEY_COACH_GROUP_UPDATED: &str = "commands.coach.group_updated";
/// Key: `/coach select` success in a DM (user-scoped coach). `{0}` = coach title.
/// Distinct from [`KEY_COACH_GROUP_UPDATED`] so DM replies don't mention any "group".
pub const KEY_COACH_USER_UPDATED: &str = "commands.coach.user_updated";
/// Key: `/coach select` prompt when the user manages multiple groups.
/// `{0}` = group count, `{1}` = coach title.
pub const KEY_COACH_MULTI_GROUP_PROMPT: &str = "commands.coach.multi_group_prompt";
/// Key: `/coach select` multi-group card title.
pub const KEY_COACH_MULTI_GROUP_CARD_TITLE: &str = "commands.coach.multi_group_card_title";
/// Key: `/coach select` multi-group card item. `{0}` = name, `{1}` = members.
pub const KEY_COACH_MULTI_GROUP_ITEM: &str = "commands.coach.multi_group_item";
/// Key: `/coach assign` rejection when the user is not a group member.
pub const KEY_COACH_ASSIGN_NOT_A_MEMBER: &str = "commands.coach.assign_not_a_member";
/// Key: `/coach assign` rejection when the user lacks admin rights.
pub const KEY_COACH_ASSIGN_FORBIDDEN: &str = "commands.coach.assign_forbidden";

// ── Compiled-in defaults: French (DEFAULT_LOCALE) ────────────────────────

/// French default for [`KEY_ERROR_GENERIC`]. `{0}` = 8-char correlation id.
pub const FR_ERROR_GENERIC: &str = "Dravr est temporairement indisponible. L'équipe a été notifiée — réessaie dans quelques minutes. (ref: {0})";
/// French default for [`KEY_EMPTY_REPLY`].
pub const FR_EMPTY_REPLY: &str =
    "Hmm, je n'ai pas réussi à formuler une réponse. Peux-tu reformuler ta question?";
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

pub(crate) const FR_LINK_FALLBACK_PROMPT: &str = "Pour discuter avec Dravr, relie d'abord ton compte. Ouvre l'app web Dravr pour connecter ce canal.";
pub(crate) const FR_LINK_INITIAL_PROMPT: &str = "Salut ! Pour discuter avec Dravr, relie d'abord ton compte :\n{0}\n\nCe lien expire dans 10 minutes.";
pub(crate) const FR_LINK_LOGOUT_COMPLETE: &str =
    "Tu es déconnecté de Dravr. Envoie un message à tout moment pour relier ton compte.";
pub(crate) const FR_LINK_CANCELLED: &str =
    "Liaison annulée. Envoie un message à tout moment pour recommencer.";
pub(crate) const FR_LINK_GENERIC_ERROR: &str =
    "Quelque chose a mal tourné. Réessaie dans un moment.";
pub(crate) const FR_LINK_NO_ACCOUNT: &str = "Aucun compte Dravr trouvé avec cette adresse. Tu peux t'inscrire sur {0} puis réessayer, ou tape « cancel » pour arrêter.";
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

pub(crate) const FR_PROVIDER_REAUTH_REQUIRED: &str = "La connexion à {0} a expiré — je ne peux pas récupérer tes données pour le moment. Reconnecte ton compte ici (lien valide 20 minutes) :\n\n{1}\n\nUne fois reconnecté, repose-moi ta question.";

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
pub(crate) const FR_HELP_FOOTER: &str = "\nOu écris-moi simplement pour discuter avec ton coach.";

pub(crate) const FR_LOGOUT_CONFIRM_PROMPT: &str = "Ceci va délier ton compte {0} de Dravr.\nIl faudra le relier pour réutiliser la messagerie.\n\nTape « logout » pour confirmer.";

pub(crate) const FR_PRIVACY_STATUS_LINE: &str = "Le consentement aux statistiques anonymes est actuellement <b>{0}</b>.\n\nUtilise <code>/privacy on</code> pour l'activer ou <code>/privacy off</code> pour le désactiver.";
pub(crate) const FR_PRIVACY_STATUS_ENABLED: &str = "activé";
pub(crate) const FR_PRIVACY_STATUS_DISABLED: &str = "désactivé";
pub(crate) const FR_PRIVACY_ON_CONFIRMATION: &str = "Le consentement aux statistiques anonymes est maintenant <b>activé</b>. Merci de nous aider à améliorer Dravr !\n\nUtilise <code>/privacy off</code> pour te retirer à tout moment.";
pub(crate) const FR_PRIVACY_OFF_CONFIRMATION: &str = "Le consentement aux statistiques anonymes est maintenant <b>désactivé</b>. Aucune donnée d'usage anonyme ne sera collectée.\n\nUtilise <code>/privacy on</code> pour te réinscrire à tout moment.";

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
pub(crate) const FR_GROUP_INVITE_FORBIDDEN: &str =
    "Seuls les admins et les propriétaires peuvent générer des liens d'invitation.";
pub(crate) const FR_GROUP_INVITE_BODY: &str =
    "Lien d'invitation pour {0} :\nhttps://app.dravr.ai/groups/join/{1}\n\nCode : {2}\nValide 7 jours.";
pub(crate) const FR_GROUP_INVITE_UNAVAILABLE: &str =
    "Les invitations de groupe ne sont pas disponibles.";
pub(crate) const FR_GROUP_LEAVE_PROMPT: &str =
    "Veux-tu vraiment quitter {0} ?\nTape « YES » pour confirmer.";
pub(crate) const FR_GROUP_CONSENT_USAGE: &str = "Usage : /group consent yes  ou  /group consent no";
pub(crate) const FR_GROUP_CONSENT_UPDATED: &str =
    "Le partage de tes données avec les autres membres de {1} est maintenant {0}.";

pub(crate) const FR_COACH_LIST_EMPTY: &str =
    "Aucun coach disponible. Demande à ton admin d'ajouter des coachs à ton espace.";
pub(crate) const FR_COACH_LIST_CARD_TITLE: &str = "Choisis un coach";
pub(crate) const FR_COACH_LIST_ITEM: &str = "• {0} [{1}]\n  {2}\n";
pub(crate) const FR_COACH_NO_DESCRIPTION: &str = "Sans description";
pub(crate) const FR_COACH_GROUP_CREATED: &str =
    "Groupe « {0} » créé avec le coach {1}. Les membres peuvent rejoindre via /group invite.";
pub(crate) const FR_COACH_GROUP_CREATION_UNAVAILABLE: &str =
    "Coach sélectionné : {0}. La création de groupe n'est pas disponible.";
pub(crate) const FR_COACH_GROUP_UPDATED: &str = "Coach mis à jour sur {0} pour le groupe {1}.";
pub(crate) const FR_COACH_USER_UPDATED: &str = "Coach sélectionné : {0}.";
pub(crate) const FR_COACH_MULTI_GROUP_PROMPT: &str =
    "Tu gères {0} groupes. Lequel doit utiliser {1} ?\n";
pub(crate) const FR_COACH_MULTI_GROUP_CARD_TITLE: &str = "Choisis un groupe";
pub(crate) const FR_COACH_MULTI_GROUP_ITEM: &str = "• {0} ({1} membres)";
pub(crate) const FR_COACH_ASSIGN_NOT_A_MEMBER: &str = "Tu n'es pas membre de ce groupe";
pub(crate) const FR_COACH_ASSIGN_FORBIDDEN: &str =
    "Seuls les admins et propriétaires du groupe peuvent changer le coach.";

// ── Compiled-in defaults: English ─────────────────────────────────────────

/// English default for [`KEY_ERROR_GENERIC`]. `{0}` = 8-char correlation id.
pub const EN_ERROR_GENERIC: &str = "Dravr is temporarily unavailable. The team has been notified — please try again in a few minutes. (ref: {0})";
/// English default for [`KEY_EMPTY_REPLY`].
pub const EN_EMPTY_REPLY: &str =
    "Hmm, I couldn't put a reply together. Can you rephrase your question?";
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

pub(crate) const EN_LINK_FALLBACK_PROMPT: &str = "To chat with Dravr, please link your account first. Visit the Dravr web app to connect this channel.";
pub(crate) const EN_LINK_INITIAL_PROMPT: &str =
    "Hi! To chat with Dravr, link your account first:\n{0}\n\nThis link expires in 10 minutes.";
pub(crate) const EN_LINK_LOGOUT_COMPLETE: &str =
    "You've been logged out from Dravr. Send a message anytime to link your account again.";
pub(crate) const EN_LINK_CANCELLED: &str =
    "Linking cancelled. Send a message anytime to start again.";
pub(crate) const EN_LINK_GENERIC_ERROR: &str = "Something went wrong. Please try again later.";
pub(crate) const EN_LINK_NO_ACCOUNT: &str = "No Dravr account found with that email. You can register at {0} and then try again, or type \"cancel\" to stop.";
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

pub(crate) const EN_PROVIDER_REAUTH_REQUIRED: &str = "Your {0} connection has expired — I can't fetch your data right now. Reconnect here (link valid for 20 minutes):\n\n{1}\n\nOnce reconnected, ask me again.";

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
pub(crate) const EN_HELP_FOOTER: &str = "\nOr just send a message to chat with your coach.";

pub(crate) const EN_LOGOUT_CONFIRM_PROMPT: &str = "This will unlink your {0} account from Dravr.\nYou will need to re-link to use messaging again.\n\nType \"logout\" to confirm.";

pub(crate) const EN_PRIVACY_STATUS_LINE: &str = "Analytics consent is currently <b>{0}</b>.\n\nUse <code>/privacy on</code> to enable or <code>/privacy off</code> to disable anonymous analytics.";
pub(crate) const EN_PRIVACY_STATUS_ENABLED: &str = "enabled";
pub(crate) const EN_PRIVACY_STATUS_DISABLED: &str = "disabled";
pub(crate) const EN_PRIVACY_ON_CONFIRMATION: &str = "Analytics consent has been <b>enabled</b>. Thank you for helping us improve Dravr!\n\nUse <code>/privacy off</code> to opt out at any time.";
pub(crate) const EN_PRIVACY_OFF_CONFIRMATION: &str = "Analytics consent has been <b>disabled</b>. No anonymous usage data will be collected.\n\nUse <code>/privacy on</code> to opt back in at any time.";

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
pub(crate) const EN_GROUP_INVITE_FORBIDDEN: &str =
    "Only admins and owners can generate invite links.";
pub(crate) const EN_GROUP_INVITE_BODY: &str =
    "Invite link for {0}:\nhttps://app.dravr.ai/groups/join/{1}\n\nCode: {2}\nValid for 7 days.";
pub(crate) const EN_GROUP_INVITE_UNAVAILABLE: &str = "Group invites are not available.";
pub(crate) const EN_GROUP_LEAVE_PROMPT: &str =
    "Are you sure you want to leave {0}?\nType \"YES\" to confirm.";
pub(crate) const EN_GROUP_CONSENT_USAGE: &str = "Usage: /group consent yes  or  /group consent no";
pub(crate) const EN_GROUP_CONSENT_UPDATED: &str =
    "Sharing your data with the other members of {1} is now {0}.";

pub(crate) const EN_COACH_LIST_EMPTY: &str =
    "No coaches available. Ask your admin to add coaches to your workspace.";
pub(crate) const EN_COACH_LIST_CARD_TITLE: &str = "Choose a coach";
pub(crate) const EN_COACH_LIST_ITEM: &str = "• {0} [{1}]\n  {2}\n";
pub(crate) const EN_COACH_NO_DESCRIPTION: &str = "No description";
pub(crate) const EN_COACH_GROUP_CREATED: &str =
    "Created group \"{0}\" with coach {1}. Members can join via /group invite.";
pub(crate) const EN_COACH_GROUP_CREATION_UNAVAILABLE: &str =
    "Selected coach: {0}. Group creation is not available.";
pub(crate) const EN_COACH_GROUP_UPDATED: &str = "Coach updated to {0} for group {1}.";
pub(crate) const EN_COACH_USER_UPDATED: &str = "Coach selected: {0}.";
pub(crate) const EN_COACH_MULTI_GROUP_PROMPT: &str =
    "You manage {0} groups. Which one should use {1}?\n";
pub(crate) const EN_COACH_MULTI_GROUP_CARD_TITLE: &str = "Choose a group";
pub(crate) const EN_COACH_MULTI_GROUP_ITEM: &str = "• {0} ({1} members)";
pub(crate) const EN_COACH_ASSIGN_NOT_A_MEMBER: &str = "You are not a member of this group";
pub(crate) const EN_COACH_ASSIGN_FORBIDDEN: &str =
    "Only group admins and owners can change the coach.";

// ── Compiled-in defaults: Spanish ─────────────────────────────────────────

pub(crate) const ES_ERROR_GENERIC: &str = "Dravr no está disponible temporalmente. El equipo ha sido notificado — inténtalo de nuevo en unos minutos. (ref: {0})";
pub(crate) const ES_EMPTY_REPLY: &str =
    "Hmm, no pude armar una respuesta. ¿Puedes reformular tu pregunta?";
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

pub(crate) const ES_LINK_FALLBACK_PROMPT: &str = "Para hablar con Dravr, primero vincula tu cuenta. Abre la app web de Dravr para conectar este canal.";
pub(crate) const ES_LINK_INITIAL_PROMPT: &str = "¡Hola! Para hablar con Dravr, vincula primero tu cuenta:\n{0}\n\nEste enlace expira en 10 minutos.";
pub(crate) const ES_LINK_LOGOUT_COMPLETE: &str = "Has cerrado sesión en Dravr. Envía un mensaje cuando quieras para volver a vincular tu cuenta.";
pub(crate) const ES_LINK_CANCELLED: &str =
    "Vinculación cancelada. Envía un mensaje cuando quieras para empezar de nuevo.";
pub(crate) const ES_LINK_GENERIC_ERROR: &str = "Algo salió mal. Inténtalo de nuevo más tarde.";
pub(crate) const ES_LINK_NO_ACCOUNT: &str = "No se encontró ninguna cuenta de Dravr con ese correo. Puedes registrarte en {0} y volver a intentarlo, o escribe «cancel» para detener.";
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

pub(crate) const ES_PROVIDER_REAUTH_REQUIRED: &str = "Tu conexión con {0} ha expirado — no puedo recuperar tus datos en este momento. Vuelve a conectar tu cuenta aquí (enlace válido durante 20 minutos):\n\n{1}\n\nUna vez reconectado, vuelve a preguntarme.";

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
pub(crate) const ES_HELP_FOOTER: &str = "\nO simplemente escríbeme para conversar con tu coach.";

pub(crate) const ES_LOGOUT_CONFIRM_PROMPT: &str = "Esto desvinculará tu cuenta de {0} de Dravr.\nTendrás que volver a vincularla para usar la mensajería.\n\nEscribe «logout» para confirmar.";

pub(crate) const ES_PRIVACY_STATUS_LINE: &str = "El consentimiento de analíticas está actualmente <b>{0}</b>.\n\nUsa <code>/privacy on</code> para activarlo o <code>/privacy off</code> para desactivarlo.";
pub(crate) const ES_PRIVACY_STATUS_ENABLED: &str = "activado";
pub(crate) const ES_PRIVACY_STATUS_DISABLED: &str = "desactivado";
pub(crate) const ES_PRIVACY_ON_CONFIRMATION: &str = "El consentimiento de analíticas está ahora <b>activado</b>. ¡Gracias por ayudarnos a mejorar Dravr!\n\nUsa <code>/privacy off</code> para darte de baja en cualquier momento.";
pub(crate) const ES_PRIVACY_OFF_CONFIRMATION: &str = "El consentimiento de analíticas está ahora <b>desactivado</b>. No se recogerán datos de uso anónimos.\n\nUsa <code>/privacy on</code> para volver a activarlo cuando quieras.";

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
pub(crate) const ES_GROUP_INVITE_FORBIDDEN: &str =
    "Solo los administradores y propietarios pueden generar enlaces de invitación.";
pub(crate) const ES_GROUP_INVITE_BODY: &str =
    "Enlace de invitación para {0}:\nhttps://app.dravr.ai/groups/join/{1}\n\nCódigo: {2}\nVálido 7 días.";
pub(crate) const ES_GROUP_INVITE_UNAVAILABLE: &str =
    "Las invitaciones de grupo no están disponibles.";
pub(crate) const ES_GROUP_LEAVE_PROMPT: &str =
    "¿Seguro que quieres salir de {0}?\nEscribe «YES» para confirmar.";
pub(crate) const ES_GROUP_CONSENT_USAGE: &str = "Uso: /group consent yes  o  /group consent no";
pub(crate) const ES_GROUP_CONSENT_UPDATED: &str =
    "Compartir tus datos con los demás miembros de {1} está ahora {0}.";

pub(crate) const ES_COACH_LIST_EMPTY: &str =
    "No hay coaches disponibles. Pide a tu admin que añada coaches a tu espacio.";
pub(crate) const ES_COACH_LIST_CARD_TITLE: &str = "Elige un coach";
pub(crate) const ES_COACH_LIST_ITEM: &str = "• {0} [{1}]\n  {2}\n";
pub(crate) const ES_COACH_NO_DESCRIPTION: &str = "Sin descripción";
pub(crate) const ES_COACH_GROUP_CREATED: &str =
    "Grupo «{0}» creado con el coach {1}. Los miembros pueden unirse con /group invite.";
pub(crate) const ES_COACH_GROUP_CREATION_UNAVAILABLE: &str =
    "Coach seleccionado: {0}. La creación de grupos no está disponible.";
pub(crate) const ES_COACH_GROUP_UPDATED: &str = "Coach actualizado a {0} para el grupo {1}.";
pub(crate) const ES_COACH_USER_UPDATED: &str = "Coach seleccionado: {0}.";
pub(crate) const ES_COACH_MULTI_GROUP_PROMPT: &str =
    "Gestionas {0} grupos. ¿Cuál debería usar {1}?\n";
pub(crate) const ES_COACH_MULTI_GROUP_CARD_TITLE: &str = "Elige un grupo";
pub(crate) const ES_COACH_MULTI_GROUP_ITEM: &str = "• {0} ({1} miembros)";
pub(crate) const ES_COACH_ASSIGN_NOT_A_MEMBER: &str = "No eres miembro de este grupo";
pub(crate) const ES_COACH_ASSIGN_FORBIDDEN: &str =
    "Solo los administradores y propietarios del grupo pueden cambiar el coach.";

// ── Compiled-in defaults: German ──────────────────────────────────────────

pub(crate) const DE_ERROR_GENERIC: &str = "Dravr ist vorübergehend nicht verfügbar. Das Team wurde benachrichtigt — versuch es in ein paar Minuten erneut. (Ref: {0})";
pub(crate) const DE_EMPTY_REPLY: &str =
    "Hmm, ich konnte keine Antwort formulieren. Kannst du deine Frage umformulieren?";
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

pub(crate) const DE_LINK_FALLBACK_PROMPT: &str = "Um mit Dravr zu chatten, verknüpfe zuerst dein Konto. Öffne die Dravr-Web-App, um diesen Kanal zu verbinden.";
pub(crate) const DE_LINK_INITIAL_PROMPT: &str = "Hallo! Um mit Dravr zu chatten, verknüpfe zuerst dein Konto:\n{0}\n\nDieser Link läuft in 10 Minuten ab.";
pub(crate) const DE_LINK_LOGOUT_COMPLETE: &str = "Du bist von Dravr abgemeldet. Schreib jederzeit eine Nachricht, um dein Konto erneut zu verknüpfen.";
pub(crate) const DE_LINK_CANCELLED: &str =
    "Verknüpfung abgebrochen. Schreib jederzeit eine Nachricht, um neu zu beginnen.";
pub(crate) const DE_LINK_GENERIC_ERROR: &str =
    "Etwas ist schiefgelaufen. Bitte versuch es später erneut.";
pub(crate) const DE_LINK_NO_ACCOUNT: &str = "Kein Dravr-Konto mit dieser E-Mail gefunden. Du kannst dich unter {0} registrieren und es erneut versuchen, oder tippe „cancel\", um abzubrechen.";
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

pub(crate) const DE_PROVIDER_REAUTH_REQUIRED: &str = "Deine Verbindung zu {0} ist abgelaufen — ich kann deine Daten gerade nicht abrufen. Verbinde dein Konto hier neu (Link 20 Minuten gültig):\n\n{1}\n\nFrag mich nach der erneuten Verbindung noch einmal.";

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
pub(crate) const DE_HELP_FOOTER: &str =
    "\nOder schreib mir einfach, um mit deinem Coach zu chatten.";

pub(crate) const DE_LOGOUT_CONFIRM_PROMPT: &str = "Damit wird dein {0}-Konto von Dravr entkoppelt.\nDu musst es neu verknüpfen, um die Messaging-Funktion wieder zu nutzen.\n\nTippe „logout\", um zu bestätigen.";

pub(crate) const DE_PRIVACY_STATUS_LINE: &str = "Die Einwilligung zu anonymen Statistiken ist derzeit <b>{0}</b>.\n\nNutze <code>/privacy on</code> zum Aktivieren oder <code>/privacy off</code> zum Deaktivieren.";
pub(crate) const DE_PRIVACY_STATUS_ENABLED: &str = "aktiviert";
pub(crate) const DE_PRIVACY_STATUS_DISABLED: &str = "deaktiviert";
pub(crate) const DE_PRIVACY_ON_CONFIRMATION: &str = "Die Einwilligung zu anonymen Statistiken ist jetzt <b>aktiviert</b>. Danke, dass du hilfst, Dravr zu verbessern!\n\nNutze <code>/privacy off</code>, um dich jederzeit abzumelden.";
pub(crate) const DE_PRIVACY_OFF_CONFIRMATION: &str = "Die Einwilligung zu anonymen Statistiken ist jetzt <b>deaktiviert</b>. Es werden keine anonymen Nutzungsdaten erhoben.\n\nNutze <code>/privacy on</code>, um dich jederzeit wieder anzumelden.";

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
pub(crate) const DE_GROUP_INVITE_FORBIDDEN: &str =
    "Nur Admins und Eigentümer können Einladungslinks erstellen.";
pub(crate) const DE_GROUP_INVITE_BODY: &str =
    "Einladungslink für {0}:\nhttps://app.dravr.ai/groups/join/{1}\n\nCode: {2}\n7 Tage gültig.";
pub(crate) const DE_GROUP_INVITE_UNAVAILABLE: &str = "Gruppeneinladungen sind nicht verfügbar.";
pub(crate) const DE_GROUP_LEAVE_PROMPT: &str =
    "Willst du {0} wirklich verlassen?\nTippe „YES\", um zu bestätigen.";
pub(crate) const DE_GROUP_CONSENT_USAGE: &str =
    "Verwendung: /group consent yes  oder  /group consent no";
pub(crate) const DE_GROUP_CONSENT_UPDATED: &str =
    "Das Teilen deiner Daten mit den anderen Mitgliedern von {1} ist jetzt {0}.";

pub(crate) const DE_COACH_LIST_EMPTY: &str =
    "Keine Coaches verfügbar. Bitte deinen Admin, Coaches zu deinem Workspace hinzuzufügen.";
pub(crate) const DE_COACH_LIST_CARD_TITLE: &str = "Coach wählen";
pub(crate) const DE_COACH_LIST_ITEM: &str = "• {0} [{1}]\n  {2}\n";
pub(crate) const DE_COACH_NO_DESCRIPTION: &str = "Keine Beschreibung";
pub(crate) const DE_COACH_GROUP_CREATED: &str =
    "Gruppe „{0}\" mit Coach {1} erstellt. Mitglieder können über /group invite beitreten.";
pub(crate) const DE_COACH_GROUP_CREATION_UNAVAILABLE: &str =
    "Ausgewählter Coach: {0}. Gruppenerstellung ist nicht verfügbar.";
pub(crate) const DE_COACH_GROUP_UPDATED: &str = "Coach auf {0} für Gruppe {1} aktualisiert.";
pub(crate) const DE_COACH_USER_UPDATED: &str = "Coach gewählt: {0}.";
pub(crate) const DE_COACH_MULTI_GROUP_PROMPT: &str =
    "Du verwaltest {0} Gruppen. Welche soll {1} nutzen?\n";
pub(crate) const DE_COACH_MULTI_GROUP_CARD_TITLE: &str = "Gruppe wählen";
pub(crate) const DE_COACH_MULTI_GROUP_ITEM: &str = "• {0} ({1} Mitglieder)";
pub(crate) const DE_COACH_ASSIGN_NOT_A_MEMBER: &str = "Du bist kein Mitglied dieser Gruppe";
pub(crate) const DE_COACH_ASSIGN_FORBIDDEN: &str =
    "Nur Gruppen-Admins und Eigentümer können den Coach wechseln.";

// ── Compiled-in defaults: Portuguese ──────────────────────────────────────

pub(crate) const PT_ERROR_GENERIC: &str = "O Dravr está temporariamente indisponível. A equipa foi notificada — tenta de novo em alguns minutos. (ref: {0})";
pub(crate) const PT_EMPTY_REPLY: &str =
    "Hmm, não consegui formular uma resposta. Podes reformular a tua pergunta?";
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

pub(crate) const PT_LINK_FALLBACK_PROMPT: &str = "Para falar com o Dravr, liga primeiro a tua conta. Abre a app web do Dravr para ligar este canal.";
pub(crate) const PT_LINK_INITIAL_PROMPT: &str = "Olá! Para falar com o Dravr, liga primeiro a tua conta:\n{0}\n\nEste link expira em 10 minutos.";
pub(crate) const PT_LINK_LOGOUT_COMPLETE: &str = "Saíste da sessão do Dravr. Envia uma mensagem a qualquer momento para ligar novamente a tua conta.";
pub(crate) const PT_LINK_CANCELLED: &str =
    "Ligação cancelada. Envia uma mensagem a qualquer momento para começar de novo.";
pub(crate) const PT_LINK_GENERIC_ERROR: &str = "Algo correu mal. Tenta de novo mais tarde.";
pub(crate) const PT_LINK_NO_ACCOUNT: &str = "Nenhuma conta Dravr encontrada com esse e-mail. Podes registar-te em {0} e voltar a tentar, ou escreve «cancel» para parar.";
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

pub(crate) const PT_PROVIDER_REAUTH_REQUIRED: &str = "A tua ligação ao {0} expirou — não consigo aceder aos teus dados de momento. Liga novamente a tua conta aqui (link válido por 20 minutos):\n\n{1}\n\nDepois de te reconectares, volta a perguntar-me.";

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
pub(crate) const PT_HELP_FOOTER: &str = "\nOu escreve-me para conversar com o teu coach.";

pub(crate) const PT_LOGOUT_CONFIRM_PROMPT: &str = "Isto vai desvincular a tua conta {0} do Dravr.\nVais precisar de voltar a ligá-la para usar o messaging.\n\nEscreve «logout» para confirmar.";

pub(crate) const PT_PRIVACY_STATUS_LINE: &str = "O consentimento de estatísticas anónimas está atualmente <b>{0}</b>.\n\nUsa <code>/privacy on</code> para ativar ou <code>/privacy off</code> para desativar.";
pub(crate) const PT_PRIVACY_STATUS_ENABLED: &str = "ativado";
pub(crate) const PT_PRIVACY_STATUS_DISABLED: &str = "desativado";
pub(crate) const PT_PRIVACY_ON_CONFIRMATION: &str = "O consentimento de estatísticas anónimas está agora <b>ativado</b>. Obrigado por ajudares a melhorar o Dravr!\n\nUsa <code>/privacy off</code> para cancelar a qualquer momento.";
pub(crate) const PT_PRIVACY_OFF_CONFIRMATION: &str = "O consentimento de estatísticas anónimas está agora <b>desativado</b>. Não serão recolhidos dados de uso anónimos.\n\nUsa <code>/privacy on</code> para voltar a ativar a qualquer momento.";

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
pub(crate) const PT_GROUP_INVITE_FORBIDDEN: &str =
    "Apenas administradores e proprietários podem gerar links de convite.";
pub(crate) const PT_GROUP_INVITE_BODY: &str =
    "Link de convite para {0}:\nhttps://app.dravr.ai/groups/join/{1}\n\nCódigo: {2}\nVálido 7 dias.";
pub(crate) const PT_GROUP_INVITE_UNAVAILABLE: &str = "Convites de grupo não estão disponíveis.";
pub(crate) const PT_GROUP_LEAVE_PROMPT: &str =
    "Tens a certeza que queres sair de {0}?\nEscreve «YES» para confirmar.";
pub(crate) const PT_GROUP_CONSENT_USAGE: &str = "Uso: /group consent yes  ou  /group consent no";
pub(crate) const PT_GROUP_CONSENT_UPDATED: &str =
    "Partilhar os teus dados com os outros membros de {1} está agora {0}.";

pub(crate) const PT_COACH_LIST_EMPTY: &str =
    "Nenhum coach disponível. Pede ao teu admin para adicionar coaches ao teu espaço.";
pub(crate) const PT_COACH_LIST_CARD_TITLE: &str = "Escolhe um coach";
pub(crate) const PT_COACH_LIST_ITEM: &str = "• {0} [{1}]\n  {2}\n";
pub(crate) const PT_COACH_NO_DESCRIPTION: &str = "Sem descrição";
pub(crate) const PT_COACH_GROUP_CREATED: &str =
    "Grupo «{0}» criado com o coach {1}. Os membros podem juntar-se via /group invite.";
pub(crate) const PT_COACH_GROUP_CREATION_UNAVAILABLE: &str =
    "Coach selecionado: {0}. A criação de grupos não está disponível.";
pub(crate) const PT_COACH_GROUP_UPDATED: &str = "Coach atualizado para {0} no grupo {1}.";
pub(crate) const PT_COACH_USER_UPDATED: &str = "Treinador selecionado: {0}.";
pub(crate) const PT_COACH_MULTI_GROUP_PROMPT: &str = "Geres {0} grupos. Qual deve usar {1}?\n";
pub(crate) const PT_COACH_MULTI_GROUP_CARD_TITLE: &str = "Escolhe um grupo";
pub(crate) const PT_COACH_MULTI_GROUP_ITEM: &str = "• {0} ({1} membros)";
pub(crate) const PT_COACH_ASSIGN_NOT_A_MEMBER: &str = "Não és membro deste grupo";
pub(crate) const PT_COACH_ASSIGN_FORBIDDEN: &str =
    "Apenas administradores e proprietários do grupo podem mudar o coach.";

/// Compiled-in `(key, locale, content)` triples loaded into the registry
/// at construction. Any new locale added here automatically becomes
/// available as a fallback target without code changes at call sites.
#[rustfmt::skip]
const COMPILED_IN: &[(&str, &str, &str)] = &[
    // ── French (DEFAULT_LOCALE) ─────────────────────────────────────────
    (KEY_ERROR_GENERIC, "fr", FR_ERROR_GENERIC),
    (KEY_EMPTY_REPLY, "fr", FR_EMPTY_REPLY),
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
    (KEY_LINK_FALLBACK_PROMPT, "fr", FR_LINK_FALLBACK_PROMPT),
    (KEY_LINK_INITIAL_PROMPT, "fr", FR_LINK_INITIAL_PROMPT),
    (KEY_LINK_LOGOUT_COMPLETE, "fr", FR_LINK_LOGOUT_COMPLETE),
    (KEY_LINK_CANCELLED, "fr", FR_LINK_CANCELLED),
    (KEY_LINK_GENERIC_ERROR, "fr", FR_LINK_GENERIC_ERROR),
    (KEY_LINK_NO_ACCOUNT, "fr", FR_LINK_NO_ACCOUNT),
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
    (KEY_PROVIDER_REAUTH_REQUIRED, "fr", FR_PROVIDER_REAUTH_REQUIRED),
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
    (KEY_HELP_FOOTER, "fr", FR_HELP_FOOTER),
    (KEY_LOGOUT_CONFIRM_PROMPT, "fr", FR_LOGOUT_CONFIRM_PROMPT),
    (KEY_PRIVACY_STATUS_LINE, "fr", FR_PRIVACY_STATUS_LINE),
    (KEY_PRIVACY_STATUS_ENABLED, "fr", FR_PRIVACY_STATUS_ENABLED),
    (KEY_PRIVACY_STATUS_DISABLED, "fr", FR_PRIVACY_STATUS_DISABLED),
    (KEY_PRIVACY_ON_CONFIRMATION, "fr", FR_PRIVACY_ON_CONFIRMATION),
    (KEY_PRIVACY_OFF_CONFIRMATION, "fr", FR_PRIVACY_OFF_CONFIRMATION),
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
    (KEY_GROUP_INVITE_FORBIDDEN, "fr", FR_GROUP_INVITE_FORBIDDEN),
    (KEY_GROUP_INVITE_BODY, "fr", FR_GROUP_INVITE_BODY),
    (KEY_GROUP_INVITE_UNAVAILABLE, "fr", FR_GROUP_INVITE_UNAVAILABLE),
    (KEY_GROUP_LEAVE_PROMPT, "fr", FR_GROUP_LEAVE_PROMPT),
    (KEY_GROUP_CONSENT_USAGE, "fr", FR_GROUP_CONSENT_USAGE),
    (KEY_GROUP_CONSENT_UPDATED, "fr", FR_GROUP_CONSENT_UPDATED),
    (KEY_COACH_LIST_EMPTY, "fr", FR_COACH_LIST_EMPTY),
    (KEY_COACH_LIST_CARD_TITLE, "fr", FR_COACH_LIST_CARD_TITLE),
    (KEY_COACH_LIST_ITEM, "fr", FR_COACH_LIST_ITEM),
    (KEY_COACH_NO_DESCRIPTION, "fr", FR_COACH_NO_DESCRIPTION),
    (KEY_COACH_GROUP_CREATED, "fr", FR_COACH_GROUP_CREATED),
    (KEY_COACH_GROUP_CREATION_UNAVAILABLE, "fr", FR_COACH_GROUP_CREATION_UNAVAILABLE),
    (KEY_COACH_GROUP_UPDATED, "fr", FR_COACH_GROUP_UPDATED),
    (KEY_COACH_USER_UPDATED, "fr", FR_COACH_USER_UPDATED),
    (KEY_COACH_MULTI_GROUP_PROMPT, "fr", FR_COACH_MULTI_GROUP_PROMPT),
    (KEY_COACH_MULTI_GROUP_CARD_TITLE, "fr", FR_COACH_MULTI_GROUP_CARD_TITLE),
    (KEY_COACH_MULTI_GROUP_ITEM, "fr", FR_COACH_MULTI_GROUP_ITEM),
    (KEY_COACH_ASSIGN_NOT_A_MEMBER, "fr", FR_COACH_ASSIGN_NOT_A_MEMBER),
    (KEY_COACH_ASSIGN_FORBIDDEN, "fr", FR_COACH_ASSIGN_FORBIDDEN),

    // ── English ─────────────────────────────────────────────────────────
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
    (KEY_ERROR_GENERIC, "en", EN_ERROR_GENERIC),
    (KEY_EMPTY_REPLY, "en", EN_EMPTY_REPLY),
    (KEY_GUARDRAIL_TOO_LONG, "en", EN_GUARDRAIL_TOO_LONG),
    (KEY_GUARDRAIL_BLOCKED_TOPIC, "en", EN_GUARDRAIL_BLOCKED_TOPIC),
    (KEY_VERIFICATION_WARN_SUFFIX, "en", EN_VERIFICATION_WARN_SUFFIX),
    (KEY_VERIFICATION_BLOCK_FALLBACK, "en", EN_VERIFICATION_BLOCK_FALLBACK),
    (KEY_LINK_FALLBACK_PROMPT, "en", EN_LINK_FALLBACK_PROMPT),
    (KEY_LINK_INITIAL_PROMPT, "en", EN_LINK_INITIAL_PROMPT),
    (KEY_LINK_LOGOUT_COMPLETE, "en", EN_LINK_LOGOUT_COMPLETE),
    (KEY_LINK_CANCELLED, "en", EN_LINK_CANCELLED),
    (KEY_LINK_GENERIC_ERROR, "en", EN_LINK_GENERIC_ERROR),
    (KEY_LINK_NO_ACCOUNT, "en", EN_LINK_NO_ACCOUNT),
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
    (KEY_PROVIDER_REAUTH_REQUIRED, "en", EN_PROVIDER_REAUTH_REQUIRED),
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
    (KEY_HELP_FOOTER, "en", EN_HELP_FOOTER),
    (KEY_LOGOUT_CONFIRM_PROMPT, "en", EN_LOGOUT_CONFIRM_PROMPT),
    (KEY_PRIVACY_STATUS_LINE, "en", EN_PRIVACY_STATUS_LINE),
    (KEY_PRIVACY_STATUS_ENABLED, "en", EN_PRIVACY_STATUS_ENABLED),
    (KEY_PRIVACY_STATUS_DISABLED, "en", EN_PRIVACY_STATUS_DISABLED),
    (KEY_PRIVACY_ON_CONFIRMATION, "en", EN_PRIVACY_ON_CONFIRMATION),
    (KEY_PRIVACY_OFF_CONFIRMATION, "en", EN_PRIVACY_OFF_CONFIRMATION),
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
    (KEY_GROUP_INVITE_FORBIDDEN, "en", EN_GROUP_INVITE_FORBIDDEN),
    (KEY_GROUP_INVITE_BODY, "en", EN_GROUP_INVITE_BODY),
    (KEY_GROUP_INVITE_UNAVAILABLE, "en", EN_GROUP_INVITE_UNAVAILABLE),
    (KEY_GROUP_LEAVE_PROMPT, "en", EN_GROUP_LEAVE_PROMPT),
    (KEY_GROUP_CONSENT_USAGE, "en", EN_GROUP_CONSENT_USAGE),
    (KEY_GROUP_CONSENT_UPDATED, "en", EN_GROUP_CONSENT_UPDATED),
    (KEY_COACH_LIST_EMPTY, "en", EN_COACH_LIST_EMPTY),
    (KEY_COACH_LIST_CARD_TITLE, "en", EN_COACH_LIST_CARD_TITLE),
    (KEY_COACH_LIST_ITEM, "en", EN_COACH_LIST_ITEM),
    (KEY_COACH_NO_DESCRIPTION, "en", EN_COACH_NO_DESCRIPTION),
    (KEY_COACH_GROUP_CREATED, "en", EN_COACH_GROUP_CREATED),
    (KEY_COACH_GROUP_CREATION_UNAVAILABLE, "en", EN_COACH_GROUP_CREATION_UNAVAILABLE),
    (KEY_COACH_GROUP_UPDATED, "en", EN_COACH_GROUP_UPDATED),
    (KEY_COACH_USER_UPDATED, "en", EN_COACH_USER_UPDATED),
    (KEY_COACH_MULTI_GROUP_PROMPT, "en", EN_COACH_MULTI_GROUP_PROMPT),
    (KEY_COACH_MULTI_GROUP_CARD_TITLE, "en", EN_COACH_MULTI_GROUP_CARD_TITLE),
    (KEY_COACH_MULTI_GROUP_ITEM, "en", EN_COACH_MULTI_GROUP_ITEM),
    (KEY_COACH_ASSIGN_NOT_A_MEMBER, "en", EN_COACH_ASSIGN_NOT_A_MEMBER),
    (KEY_COACH_ASSIGN_FORBIDDEN, "en", EN_COACH_ASSIGN_FORBIDDEN),

    // ── Spanish ─────────────────────────────────────────────────────────
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
    (KEY_ERROR_GENERIC, "es", ES_ERROR_GENERIC),
    (KEY_EMPTY_REPLY, "es", ES_EMPTY_REPLY),
    (KEY_GUARDRAIL_TOO_LONG, "es", ES_GUARDRAIL_TOO_LONG),
    (KEY_GUARDRAIL_BLOCKED_TOPIC, "es", ES_GUARDRAIL_BLOCKED_TOPIC),
    (KEY_VERIFICATION_WARN_SUFFIX, "es", ES_VERIFICATION_WARN_SUFFIX),
    (KEY_VERIFICATION_BLOCK_FALLBACK, "es", ES_VERIFICATION_BLOCK_FALLBACK),
    (KEY_LINK_FALLBACK_PROMPT, "es", ES_LINK_FALLBACK_PROMPT),
    (KEY_LINK_INITIAL_PROMPT, "es", ES_LINK_INITIAL_PROMPT),
    (KEY_LINK_LOGOUT_COMPLETE, "es", ES_LINK_LOGOUT_COMPLETE),
    (KEY_LINK_CANCELLED, "es", ES_LINK_CANCELLED),
    (KEY_LINK_GENERIC_ERROR, "es", ES_LINK_GENERIC_ERROR),
    (KEY_LINK_NO_ACCOUNT, "es", ES_LINK_NO_ACCOUNT),
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
    (KEY_PROVIDER_REAUTH_REQUIRED, "es", ES_PROVIDER_REAUTH_REQUIRED),
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
    (KEY_HELP_FOOTER, "es", ES_HELP_FOOTER),
    (KEY_LOGOUT_CONFIRM_PROMPT, "es", ES_LOGOUT_CONFIRM_PROMPT),
    (KEY_PRIVACY_STATUS_LINE, "es", ES_PRIVACY_STATUS_LINE),
    (KEY_PRIVACY_STATUS_ENABLED, "es", ES_PRIVACY_STATUS_ENABLED),
    (KEY_PRIVACY_STATUS_DISABLED, "es", ES_PRIVACY_STATUS_DISABLED),
    (KEY_PRIVACY_ON_CONFIRMATION, "es", ES_PRIVACY_ON_CONFIRMATION),
    (KEY_PRIVACY_OFF_CONFIRMATION, "es", ES_PRIVACY_OFF_CONFIRMATION),
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
    (KEY_GROUP_INVITE_FORBIDDEN, "es", ES_GROUP_INVITE_FORBIDDEN),
    (KEY_GROUP_INVITE_BODY, "es", ES_GROUP_INVITE_BODY),
    (KEY_GROUP_INVITE_UNAVAILABLE, "es", ES_GROUP_INVITE_UNAVAILABLE),
    (KEY_GROUP_LEAVE_PROMPT, "es", ES_GROUP_LEAVE_PROMPT),
    (KEY_GROUP_CONSENT_USAGE, "es", ES_GROUP_CONSENT_USAGE),
    (KEY_GROUP_CONSENT_UPDATED, "es", ES_GROUP_CONSENT_UPDATED),
    (KEY_COACH_LIST_EMPTY, "es", ES_COACH_LIST_EMPTY),
    (KEY_COACH_LIST_CARD_TITLE, "es", ES_COACH_LIST_CARD_TITLE),
    (KEY_COACH_LIST_ITEM, "es", ES_COACH_LIST_ITEM),
    (KEY_COACH_NO_DESCRIPTION, "es", ES_COACH_NO_DESCRIPTION),
    (KEY_COACH_GROUP_CREATED, "es", ES_COACH_GROUP_CREATED),
    (KEY_COACH_GROUP_CREATION_UNAVAILABLE, "es", ES_COACH_GROUP_CREATION_UNAVAILABLE),
    (KEY_COACH_GROUP_UPDATED, "es", ES_COACH_GROUP_UPDATED),
    (KEY_COACH_USER_UPDATED, "es", ES_COACH_USER_UPDATED),
    (KEY_COACH_MULTI_GROUP_PROMPT, "es", ES_COACH_MULTI_GROUP_PROMPT),
    (KEY_COACH_MULTI_GROUP_CARD_TITLE, "es", ES_COACH_MULTI_GROUP_CARD_TITLE),
    (KEY_COACH_MULTI_GROUP_ITEM, "es", ES_COACH_MULTI_GROUP_ITEM),
    (KEY_COACH_ASSIGN_NOT_A_MEMBER, "es", ES_COACH_ASSIGN_NOT_A_MEMBER),
    (KEY_COACH_ASSIGN_FORBIDDEN, "es", ES_COACH_ASSIGN_FORBIDDEN),

    // ── German ──────────────────────────────────────────────────────────
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
    (KEY_ERROR_GENERIC, "de", DE_ERROR_GENERIC),
    (KEY_EMPTY_REPLY, "de", DE_EMPTY_REPLY),
    (KEY_GUARDRAIL_TOO_LONG, "de", DE_GUARDRAIL_TOO_LONG),
    (KEY_GUARDRAIL_BLOCKED_TOPIC, "de", DE_GUARDRAIL_BLOCKED_TOPIC),
    (KEY_VERIFICATION_WARN_SUFFIX, "de", DE_VERIFICATION_WARN_SUFFIX),
    (KEY_VERIFICATION_BLOCK_FALLBACK, "de", DE_VERIFICATION_BLOCK_FALLBACK),
    (KEY_LINK_FALLBACK_PROMPT, "de", DE_LINK_FALLBACK_PROMPT),
    (KEY_LINK_INITIAL_PROMPT, "de", DE_LINK_INITIAL_PROMPT),
    (KEY_LINK_LOGOUT_COMPLETE, "de", DE_LINK_LOGOUT_COMPLETE),
    (KEY_LINK_CANCELLED, "de", DE_LINK_CANCELLED),
    (KEY_LINK_GENERIC_ERROR, "de", DE_LINK_GENERIC_ERROR),
    (KEY_LINK_NO_ACCOUNT, "de", DE_LINK_NO_ACCOUNT),
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
    (KEY_PROVIDER_REAUTH_REQUIRED, "de", DE_PROVIDER_REAUTH_REQUIRED),
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
    (KEY_HELP_FOOTER, "de", DE_HELP_FOOTER),
    (KEY_LOGOUT_CONFIRM_PROMPT, "de", DE_LOGOUT_CONFIRM_PROMPT),
    (KEY_PRIVACY_STATUS_LINE, "de", DE_PRIVACY_STATUS_LINE),
    (KEY_PRIVACY_STATUS_ENABLED, "de", DE_PRIVACY_STATUS_ENABLED),
    (KEY_PRIVACY_STATUS_DISABLED, "de", DE_PRIVACY_STATUS_DISABLED),
    (KEY_PRIVACY_ON_CONFIRMATION, "de", DE_PRIVACY_ON_CONFIRMATION),
    (KEY_PRIVACY_OFF_CONFIRMATION, "de", DE_PRIVACY_OFF_CONFIRMATION),
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
    (KEY_GROUP_INVITE_FORBIDDEN, "de", DE_GROUP_INVITE_FORBIDDEN),
    (KEY_GROUP_INVITE_BODY, "de", DE_GROUP_INVITE_BODY),
    (KEY_GROUP_INVITE_UNAVAILABLE, "de", DE_GROUP_INVITE_UNAVAILABLE),
    (KEY_GROUP_LEAVE_PROMPT, "de", DE_GROUP_LEAVE_PROMPT),
    (KEY_GROUP_CONSENT_USAGE, "de", DE_GROUP_CONSENT_USAGE),
    (KEY_GROUP_CONSENT_UPDATED, "de", DE_GROUP_CONSENT_UPDATED),
    (KEY_COACH_LIST_EMPTY, "de", DE_COACH_LIST_EMPTY),
    (KEY_COACH_LIST_CARD_TITLE, "de", DE_COACH_LIST_CARD_TITLE),
    (KEY_COACH_LIST_ITEM, "de", DE_COACH_LIST_ITEM),
    (KEY_COACH_NO_DESCRIPTION, "de", DE_COACH_NO_DESCRIPTION),
    (KEY_COACH_GROUP_CREATED, "de", DE_COACH_GROUP_CREATED),
    (KEY_COACH_GROUP_CREATION_UNAVAILABLE, "de", DE_COACH_GROUP_CREATION_UNAVAILABLE),
    (KEY_COACH_GROUP_UPDATED, "de", DE_COACH_GROUP_UPDATED),
    (KEY_COACH_USER_UPDATED, "de", DE_COACH_USER_UPDATED),
    (KEY_COACH_MULTI_GROUP_PROMPT, "de", DE_COACH_MULTI_GROUP_PROMPT),
    (KEY_COACH_MULTI_GROUP_CARD_TITLE, "de", DE_COACH_MULTI_GROUP_CARD_TITLE),
    (KEY_COACH_MULTI_GROUP_ITEM, "de", DE_COACH_MULTI_GROUP_ITEM),
    (KEY_COACH_ASSIGN_NOT_A_MEMBER, "de", DE_COACH_ASSIGN_NOT_A_MEMBER),
    (KEY_COACH_ASSIGN_FORBIDDEN, "de", DE_COACH_ASSIGN_FORBIDDEN),

    // ── Portuguese ──────────────────────────────────────────────────────
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
    (KEY_ERROR_GENERIC, "pt", PT_ERROR_GENERIC),
    (KEY_EMPTY_REPLY, "pt", PT_EMPTY_REPLY),
    (KEY_GUARDRAIL_TOO_LONG, "pt", PT_GUARDRAIL_TOO_LONG),
    (KEY_GUARDRAIL_BLOCKED_TOPIC, "pt", PT_GUARDRAIL_BLOCKED_TOPIC),
    (KEY_VERIFICATION_WARN_SUFFIX, "pt", PT_VERIFICATION_WARN_SUFFIX),
    (KEY_VERIFICATION_BLOCK_FALLBACK, "pt", PT_VERIFICATION_BLOCK_FALLBACK),
    (KEY_LINK_FALLBACK_PROMPT, "pt", PT_LINK_FALLBACK_PROMPT),
    (KEY_LINK_INITIAL_PROMPT, "pt", PT_LINK_INITIAL_PROMPT),
    (KEY_LINK_LOGOUT_COMPLETE, "pt", PT_LINK_LOGOUT_COMPLETE),
    (KEY_LINK_CANCELLED, "pt", PT_LINK_CANCELLED),
    (KEY_LINK_GENERIC_ERROR, "pt", PT_LINK_GENERIC_ERROR),
    (KEY_LINK_NO_ACCOUNT, "pt", PT_LINK_NO_ACCOUNT),
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
    (KEY_PROVIDER_REAUTH_REQUIRED, "pt", PT_PROVIDER_REAUTH_REQUIRED),
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
    (KEY_HELP_FOOTER, "pt", PT_HELP_FOOTER),
    (KEY_LOGOUT_CONFIRM_PROMPT, "pt", PT_LOGOUT_CONFIRM_PROMPT),
    (KEY_PRIVACY_STATUS_LINE, "pt", PT_PRIVACY_STATUS_LINE),
    (KEY_PRIVACY_STATUS_ENABLED, "pt", PT_PRIVACY_STATUS_ENABLED),
    (KEY_PRIVACY_STATUS_DISABLED, "pt", PT_PRIVACY_STATUS_DISABLED),
    (KEY_PRIVACY_ON_CONFIRMATION, "pt", PT_PRIVACY_ON_CONFIRMATION),
    (KEY_PRIVACY_OFF_CONFIRMATION, "pt", PT_PRIVACY_OFF_CONFIRMATION),
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
    (KEY_GROUP_INVITE_FORBIDDEN, "pt", PT_GROUP_INVITE_FORBIDDEN),
    (KEY_GROUP_INVITE_BODY, "pt", PT_GROUP_INVITE_BODY),
    (KEY_GROUP_INVITE_UNAVAILABLE, "pt", PT_GROUP_INVITE_UNAVAILABLE),
    (KEY_GROUP_LEAVE_PROMPT, "pt", PT_GROUP_LEAVE_PROMPT),
    (KEY_GROUP_CONSENT_USAGE, "pt", PT_GROUP_CONSENT_USAGE),
    (KEY_GROUP_CONSENT_UPDATED, "pt", PT_GROUP_CONSENT_UPDATED),
    (KEY_COACH_LIST_EMPTY, "pt", PT_COACH_LIST_EMPTY),
    (KEY_COACH_LIST_CARD_TITLE, "pt", PT_COACH_LIST_CARD_TITLE),
    (KEY_COACH_LIST_ITEM, "pt", PT_COACH_LIST_ITEM),
    (KEY_COACH_NO_DESCRIPTION, "pt", PT_COACH_NO_DESCRIPTION),
    (KEY_COACH_GROUP_CREATED, "pt", PT_COACH_GROUP_CREATED),
    (KEY_COACH_GROUP_CREATION_UNAVAILABLE, "pt", PT_COACH_GROUP_CREATION_UNAVAILABLE),
    (KEY_COACH_GROUP_UPDATED, "pt", PT_COACH_GROUP_UPDATED),
    (KEY_COACH_USER_UPDATED, "pt", PT_COACH_USER_UPDATED),
    (KEY_COACH_MULTI_GROUP_PROMPT, "pt", PT_COACH_MULTI_GROUP_PROMPT),
    (KEY_COACH_MULTI_GROUP_CARD_TITLE, "pt", PT_COACH_MULTI_GROUP_CARD_TITLE),
    (KEY_COACH_MULTI_GROUP_ITEM, "pt", PT_COACH_MULTI_GROUP_ITEM),
    (KEY_COACH_ASSIGN_NOT_A_MEMBER, "pt", PT_COACH_ASSIGN_NOT_A_MEMBER),
    (KEY_COACH_ASSIGN_FORBIDDEN, "pt", PT_COACH_ASSIGN_FORBIDDEN),
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
