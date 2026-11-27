// ABOUTME: Hot-reloadable per-persona conformance contracts from contremaitre
// ABOUTME: Feeds dravr-colleur's persona style stage through the chat pipeline; zero hardcoded thresholds
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Per-persona output-format conformance contracts.
//!
//! Source-of-truth file: `config/persona_contracts.yaml` in
//! dravr-contremaitre, registered under
//! `manifest.json::config.persona_contracts`. Hot-reloads from
//! contremaitre through the same sync pipeline as the prompt files,
//! so calibrating "Casual hard cap is 150 words" does not require a
//! redeploy.
//!
//! ## Layered design
//!
//! - **YAML** lives in contremaitre — purely data, no Rust thresholds
//! - **[`PersonaContract`]** — per-persona ruleset parsed from the YAML
//! - **[`PersonaContractRegistry`]** — thread-safe in-memory snapshot,
//!   replaced atomically on every successful sync through an `ArcSwap`
//! - **dravr-colleur's persona style stage**, run by the chat pipeline —
//!   checks each rule of the contract's [`StyleContract`] against the LLM
//!   reply post-dispatch, logs `warn!` per violation
//!
//! Falls back to **no rules** when contremaitre hasn't synced yet — the
//! conformance stage no-ops on a startup or transient sync failure
//! rather than blocking chat. The vault doc's "Casual is the safe
//! fallback" applies here too: missing contract = least-restrictive.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use arc_swap::ArcSwap;
use dravr_colleur::StyleContract;
use dravr_tronc::dates::Timestamp;
use pierre_core::models::CoachingPersona;
use serde::{Deserialize, Serialize};

use pierre_core::errors::{AppError, AppResult};

/// Top-level contract document parsed from
/// `config/persona_contracts.yaml`.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PersonaContractsDocument {
    /// Schema version. The platform tolerates unknown future versions —
    /// a forward-compat upgrade tweaks the schema in contremaitre, the
    /// platform parses what it understands and ignores the rest.
    #[serde(default = "default_version")]
    pub version: u32,
    /// Per-persona contracts keyed by `snake_case` `CoachingPersona` slug.
    #[serde(default)]
    pub personas: HashMap<String, PersonaContract>,
    /// Universal (persona-independent) acronym glossary: `acronym ->
    /// locale -> full form`. Drives the chat pipeline's first-use
    /// acronym-expansion pass. Empty when the source document predates
    /// the glossary or contremaitre hasn't synced yet, in which case the
    /// expansion pass no-ops and only the prompt-level rule applies.
    #[serde(default)]
    pub glossary: HashMap<String, HashMap<String, String>>,
}

const fn default_version() -> u32 {
    1
}

/// Locale the acronym glossary falls back to when the active turn locale
/// has no entry for a given acronym. English is the lingua franca for the
/// sports-science terms, so a missing locale yields the English full form
/// rather than no gloss at all.
const GLOSSARY_FALLBACK_LOCALE: &str = "en";

/// Single-persona ruleset.
///
/// Every field is optional in the YAML; missing fields fall back to
/// permissive defaults. The `inherits` field is resolved at registry
/// install time — by the time a contract reaches a conformance check,
/// `inherits` is already flattened into the merged ruleset.
///
/// The output-style rules are dravr-colleur's [`StyleContract`], flattened
/// so the YAML keeps one shape: colleur owns the rules and the check that
/// reads them, the platform owns the inheritance and the notification
/// policy beside them.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PersonaContract {
    /// Slug of another persona whose rules should be merged in *before*
    /// this contract's overrides. Resolved once during registry install.
    /// Cycles are rejected at install time; missing parents fall back to
    /// no inheritance with a `warn!` log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherits: Option<String>,

    /// The persona's output-style rules, checked on every reply.
    #[serde(flatten)]
    pub style: StyleContract,

    /// Notification cadence policy. `tier_floor` is consumed by the persona
    /// notification-policy gate in `pierre-notifications`, which withholds
    /// every push above the floor, and every `digest` cadence (`daily`,
    /// `weekly`, `per_session`, `per_athlete`) by the persona digest in
    /// `pierre_services::notification_digest_scheduler`, which returns what
    /// the gate withheld. Both ship as a shadow-mode dark launch, ledgered
    /// in `feature-phases.yaml`.
    #[serde(default)]
    pub notification: NotificationPolicy,
}

/// Notification cadence per persona.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct NotificationPolicy {
    /// Lowest priority tier the user receives unsolicited. P0 = break
    /// glass for everyone; P3 = "every push" (Power-athlete request).
    /// Stored as a free-form label until canot exposes a typed enum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier_floor: Option<String>,
    /// Digest cadence: `daily`, `weekly`, `per_session` or `per_athlete`.
    /// Any other label resolves to no digest.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

/// Snapshot stored in [`PersonaContractRegistry`] alongside the
/// contracts so callers can correlate against `loaded_at` /
/// `source_sha256` when triaging.
#[derive(Debug, Clone, Default)]
pub struct PersonaContractsSnapshot {
    /// Resolved contracts after `inherits` flattening.
    pub by_slug: HashMap<String, PersonaContract>,
    /// Universal acronym glossary (`acronym -> locale -> full form`),
    /// carried verbatim from the source document — persona-independent,
    /// so it is not touched by `inherits` resolution.
    pub glossary: HashMap<String, HashMap<String, String>>,
    /// Schema version observed in the source document.
    pub version: u32,
    /// SHA-256 of the source YAML at install time.
    pub source_sha256: Option<String>,
    /// When this snapshot was installed.
    pub loaded_at: Option<Timestamp>,
}

impl PersonaContractsSnapshot {
    /// Look up the contract for a persona, falling back to Casual
    /// (which the vault doc names as the safe fallback). Returns `None`
    /// only when neither the requested persona nor `casual` is present
    /// in the registry — i.e. before the first sync lands.
    #[must_use]
    pub fn contract(&self, persona: CoachingPersona) -> Option<&PersonaContract> {
        self.by_slug
            .get(persona.as_str())
            .or_else(|| self.by_slug.get(CoachingPersona::Casual.as_str()))
    }

    /// `true` when the snapshot has not been hydrated yet (boot before
    /// first contremaitre sync, or contremaitre disabled). The
    /// conformance stage uses this to no-op cleanly.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_slug.is_empty()
    }

    /// Full form for `acronym` in `locale`, falling back to the English
    /// entry when the requested locale is absent. Returns `None` when the
    /// acronym is not catalogued at all (the expansion pass then leaves
    /// it untouched). Lookup is exact and case-sensitive — callers pass
    /// the canonical acronym spelling as stored in the glossary.
    #[must_use]
    pub fn acronym_gloss(&self, acronym: &str, locale: &str) -> Option<&str> {
        let by_locale = self.glossary.get(acronym)?;
        by_locale
            .get(locale)
            .or_else(|| by_locale.get(GLOSSARY_FALLBACK_LOCALE))
            .map(String::as_str)
    }
}

/// Thread-safe registry holding the active [`PersonaContractsSnapshot`].
///
/// Replacements via `apply_overlay` are atomic: a chat turn either sees
/// the old snapshot or the new one, never a half-applied set.
#[derive(Debug)]
pub struct PersonaContractRegistry {
    snapshot: ArcSwap<PersonaContractsSnapshot>,
}

impl Default for PersonaContractRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PersonaContractRegistry {
    /// Build an empty registry. The conformance stage no-ops until
    /// [`Self::apply_overlay`] is called with a parsed YAML document
    /// (typically the contremaitre sync engine).
    #[must_use]
    pub fn new() -> Self {
        Self {
            snapshot: ArcSwap::from_pointee(PersonaContractsSnapshot::default()),
        }
    }

    /// Return the currently installed snapshot. Cheap (`Arc::clone`).
    /// Hold the returned `Arc` for the duration of one chat turn rather
    /// than calling repeatedly, so one turn reads one snapshot. Lock-free.
    #[must_use]
    pub fn snapshot(&self) -> Arc<PersonaContractsSnapshot> {
        self.snapshot.load_full()
    }

    /// SHA-256 of the YAML source the current snapshot was built from,
    /// or `None` for the empty boot snapshot. Used by the sync engine
    /// to skip re-parsing when the manifest hash already matches.
    #[must_use]
    pub fn current_overlay_sha256(&self) -> Option<String> {
        self.snapshot.load().source_sha256.clone()
    }

    /// Parse a YAML body and atomically install it as the active
    /// snapshot.
    ///
    /// # Errors
    ///
    /// Returns [`AppError::config`] when the YAML fails to parse or
    /// when an `inherits` chain is malformed (cycle, missing parent
    /// referenced as required). Missing optional fields are tolerated;
    /// the registry's role is to surface authoring mistakes early, not
    /// to police every taste choice.
    pub fn apply_overlay(&self, yaml: &str) -> AppResult<()> {
        let doc: PersonaContractsDocument = serde_saphyr::from_str_with_options(
            yaml,
            serde_saphyr::options! { with_snippet: false },
        )
        .map_err(|e| AppError::config(format!("persona_contracts.yaml parse failed: {e}")))?;
        let PersonaContractsDocument {
            version,
            personas,
            glossary,
        } = doc;
        let resolved = resolve_inheritance(personas)?;
        let snapshot = PersonaContractsSnapshot {
            by_slug: resolved,
            glossary,
            version,
            source_sha256: Some(super::manifest::compute_sha256(yaml.as_bytes())),
            loaded_at: Some(Timestamp::now()),
        };
        self.snapshot.store(Arc::new(snapshot));
        Ok(())
    }
}

/// Flatten any `inherits: <slug>` chains into self-contained contracts.
///
/// Children OVERLAY parent rules: a child's `Some(value)` wins over the
/// parent's; a child leaving an Option/Vec at default keeps the
/// parent's value. This is what lets the YAML express
/// "Coach inherits Power-athlete plus roster framing" without copying
/// every Power-athlete rule into Coach's block.
fn resolve_inheritance(
    mut raw: HashMap<String, PersonaContract>,
) -> AppResult<HashMap<String, PersonaContract>> {
    let slugs: Vec<String> = raw.keys().cloned().collect();
    let mut resolved: HashMap<String, PersonaContract> = HashMap::new();

    for slug in slugs {
        let mut visiting = HashSet::new();
        let merged = resolve_one(&slug, &mut raw, &mut visiting)?;
        resolved.insert(slug, merged);
    }

    Ok(resolved)
}

fn resolve_one(
    slug: &str,
    raw: &mut HashMap<String, PersonaContract>,
    visiting: &mut HashSet<String>,
) -> AppResult<PersonaContract> {
    if !visiting.insert(slug.to_owned()) {
        return Err(AppError::config(format!(
            "persona_contracts.yaml has an `inherits` cycle through '{slug}'"
        )));
    }

    let Some(child) = raw.get(slug).cloned() else {
        return Err(AppError::config(format!(
            "persona_contracts.yaml references unknown parent persona '{slug}'"
        )));
    };

    let merged = match child.inherits.clone() {
        None => child,
        Some(parent_slug) => {
            let parent = resolve_one(&parent_slug, raw, visiting)?;
            overlay(parent, child)
        }
    };

    visiting.remove(slug);
    Ok(merged)
}

/// Overlay `child` onto `parent`: the style rules as colleur's
/// [`StyleContract::overlay`] resolves them, and the notification policy
/// field by field, a child `Some(_)` winning over the parent's.
fn overlay(parent: PersonaContract, child: PersonaContract) -> PersonaContract {
    PersonaContract {
        inherits: None,
        style: StyleContract::overlay(parent.style, child.style),
        notification: NotificationPolicy {
            tier_floor: child
                .notification
                .tier_floor
                .or(parent.notification.tier_floor),
            digest: child.notification.digest.or(parent.notification.digest),
        },
    }
}
