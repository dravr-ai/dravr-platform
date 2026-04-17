// ABOUTME: Hot-reloadable locale-aware registry for user-facing messaging strings
// ABOUTME: Compiled-in French + English defaults; extra locales layer on via contremaitre
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Messaging Strings Registry
//!
//! Holds short user-facing strings that are sent back to Telegram, WhatsApp,
//! Discord, Slack, and other messaging channels — things like "Pierre is
//! temporarily unavailable", guardrail fallbacks, and the Tier 5.5
//! verification warning suffix.
//!
//! These are distinct from [`super::PromptRegistry`] which holds the system
//! prompts sent *to* the LLM. This registry holds strings shown *to the user*.
//!
//! ## Locale model
//!
//! Each string is stored per BCP-47 locale code (`"fr"`, `"en"`, `"es"`, …).
//! Lookups follow the chain:
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

/// Default locale used when a caller does not specify one or when the
/// requested locale is missing from the registry. Currently French because
/// the majority user base is francophone; change this constant (and add the
/// corresponding compiled-in defaults) when that shifts.
pub const DEFAULT_LOCALE: &str = "fr";

/// Key: LLM dispatch failed, user-facing apology with correlation short_id.
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

// ── Compiled-in defaults: French (DEFAULT_LOCALE) ────────────────────────

/// French default for [`KEY_ERROR_GENERIC`]. `{0}` = 8-char correlation id.
pub const FR_ERROR_GENERIC: &str = "Pierre est temporairement indisponible. L'équipe a été notifiée — réessaie dans quelques minutes. (ref: {0})";
/// French default for [`KEY_EMPTY_REPLY`].
pub const FR_EMPTY_REPLY: &str =
    "Hmm, je n'ai pas réussi à formuler une réponse. Peux-tu reformuler ta question?";
/// French default for [`KEY_GUARDRAIL_TOO_LONG`].
pub const FR_GUARDRAIL_TOO_LONG: &str = "J'ai une réponse plus longue prête, mais elle dépasse la limite de longueur configurée. Veux-tu que je te la résume plus brièvement?";
/// French default for [`KEY_GUARDRAIL_BLOCKED_TOPIC`].
pub const FR_GUARDRAIL_BLOCKED_TOPIC: &str = "Je préfère ne pas aborder ce sujet ici. Restons concentrés sur ton entraînement et ta récupération. Y a-t-il quelque chose de précis sur lequel je peux t'aider?";
/// French default for [`KEY_VERIFICATION_WARN_SUFFIX`]. `{0}` = problem count.
///
/// The caller joins this suffix to the main reply with `\n\n---\n` so the
/// separator stays in Rust and the externalized string is self-contained —
/// friendlier for translators and for GitHub rendering of the raw markdown.
pub const FR_VERIFICATION_WARN_SUFFIX: &str = "⚠️ Attention — je ne suis pas tout à fait sûr de {0} affirmation(s) ci-dessus. Demande-moi de les étayer si tu veux voir les sources.";
/// French default for [`KEY_VERIFICATION_BLOCK_FALLBACK`].
pub const FR_VERIFICATION_BLOCK_FALLBACK: &str = "J'ai commencé à répondre, mais quelques-unes des affirmations que j'allais faire ne correspondaient pas aux sources que je considère fiables. Laisse-moi reformuler — peux-tu me reposer la question avec un peu plus de contexte sur ce que tu cherches à comprendre?";

// ── Compiled-in defaults: English ─────────────────────────────────────────

/// English default for [`KEY_ERROR_GENERIC`]. `{0}` = 8-char correlation id.
pub const EN_ERROR_GENERIC: &str = "Pierre is temporarily unavailable. The team has been notified — please try again in a few minutes. (ref: {0})";
/// English default for [`KEY_EMPTY_REPLY`].
pub const EN_EMPTY_REPLY: &str =
    "Hmm, I couldn't put a reply together. Can you rephrase your question?";
/// English default for [`KEY_GUARDRAIL_TOO_LONG`].
pub const EN_GUARDRAIL_TOO_LONG: &str = "I have a longer response prepared but it exceeds the configured length cap. Want me to break it into a shorter summary?";
/// English default for [`KEY_GUARDRAIL_BLOCKED_TOPIC`].
pub const EN_GUARDRAIL_BLOCKED_TOPIC: &str = "I'd rather not get into that here. Let's stay focused on your training and recovery. Is there something specific I can help with?";
/// English default for [`KEY_VERIFICATION_WARN_SUFFIX`]. `{0}` = problem count.
pub const EN_VERIFICATION_WARN_SUFFIX: &str = "⚠️ Heads up — I'm not fully confident in {0} claim(s) above. Ask me to back them up if you want the evidence.";
/// English default for [`KEY_VERIFICATION_BLOCK_FALLBACK`].
pub const EN_VERIFICATION_BLOCK_FALLBACK: &str = "I started to answer, but a couple of the claims I was about to make didn't match the evidence I trust. Let me reword that — can you ask me again with a bit more context on what you're trying to figure out?";

/// Compiled-in `(key, locale, content)` triples loaded into the registry
/// at construction. Any new locale added here automatically becomes
/// available as a fallback target without code changes at call sites.
const COMPILED_IN: &[(&str, &str, &str)] = &[
    // French (DEFAULT_LOCALE)
    (KEY_ERROR_GENERIC, "fr", FR_ERROR_GENERIC),
    (KEY_EMPTY_REPLY, "fr", FR_EMPTY_REPLY),
    (KEY_GUARDRAIL_TOO_LONG, "fr", FR_GUARDRAIL_TOO_LONG),
    (
        KEY_GUARDRAIL_BLOCKED_TOPIC,
        "fr",
        FR_GUARDRAIL_BLOCKED_TOPIC,
    ),
    (
        KEY_VERIFICATION_WARN_SUFFIX,
        "fr",
        FR_VERIFICATION_WARN_SUFFIX,
    ),
    (
        KEY_VERIFICATION_BLOCK_FALLBACK,
        "fr",
        FR_VERIFICATION_BLOCK_FALLBACK,
    ),
    // English
    (KEY_ERROR_GENERIC, "en", EN_ERROR_GENERIC),
    (KEY_EMPTY_REPLY, "en", EN_EMPTY_REPLY),
    (KEY_GUARDRAIL_TOO_LONG, "en", EN_GUARDRAIL_TOO_LONG),
    (
        KEY_GUARDRAIL_BLOCKED_TOPIC,
        "en",
        EN_GUARDRAIL_BLOCKED_TOPIC,
    ),
    (
        KEY_VERIFICATION_WARN_SUFFIX,
        "en",
        EN_VERIFICATION_WARN_SUFFIX,
    ),
    (
        KEY_VERIFICATION_BLOCK_FALLBACK,
        "en",
        EN_VERIFICATION_BLOCK_FALLBACK,
    ),
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
/// Initialized with compiled-in French and English defaults. Additional
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

/// Substitute positional placeholders `{0}`, `{1}`, … in `template` with the
/// matching entries in `args`. Placeholders without a corresponding argument
/// are left literally in the output. Surplus args are ignored.
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
