// ABOUTME: Hot-reloadable per-persona conformance contracts from contremaitre
// ABOUTME: Powers the chat-pipeline persona_conformance stage; zero hardcoded thresholds
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Per-persona output-format conformance contracts.
//!
//! Source-of-truth file: `config/persona_contracts.yaml` in
//! dravr-contremaitre, registered under
//! `manifest.json::config.persona_contracts`. Hot-reloads from
//! contremaitre over the same GCS-backed pipeline as the prompt files,
//! so calibrating "Casual hard cap is 150 words" does not require a
//! redeploy.
//!
//! ## Layered design
//!
//! - **YAML** lives in contremaitre — purely data, no Rust thresholds
//! - **[`PersonaContract`]** — per-persona ruleset parsed from the YAML
//! - **[`PersonaContractRegistry`]** — thread-safe in-memory snapshot,
//!   replaced atomically on every successful sync; `RwLock` guards the
//!   pointer swap
//! - **`persona_conformance` chat-pipeline stage** (separate module) —
//!   runs each rule against the LLM reply post-dispatch, logs `warn!`
//!   per violation
//!
//! Falls back to **no rules** when contremaitre hasn't synced yet — the
//! conformance stage no-ops on a startup or transient sync failure
//! rather than blocking chat. The vault doc's "Casual is the safe
//! fallback" applies here too: missing contract = least-restrictive.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use chrono::{DateTime, Utc};
use pierre_core::models::CoachingPersona;
use serde::{Deserialize, Serialize};

use crate::errors::{AppError, AppResult};

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
}

const fn default_version() -> u32 {
    1
}

/// Single-persona ruleset.
///
/// Every field is optional in the YAML; missing fields fall back to
/// permissive defaults. The `inherits` field is resolved at registry
/// install time — by the time a contract reaches a conformance check,
/// `inherits` is already flattened into the merged ruleset.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct PersonaContract {
    /// Slug of another persona whose rules should be merged in *before*
    /// this contract's overrides. Resolved once during registry install.
    /// Cycles are rejected at install time; missing parents fall back to
    /// no inheritance with a `warn!` log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inherits: Option<String>,

    /// Hard maximum word count for the user-facing reply (entire body,
    /// including the verification suffix and any localized denial). The
    /// vault doc's "Casual hard 150-word cap" lives here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_words: Option<usize>,

    /// When `true`, surfacing any framework name from `framework_allowlist`
    /// — or any of the canonical framework labels Coggan/Banister/etc. —
    /// is a violation. Used by Casual to keep replies acronym-free.
    #[serde(default)]
    pub forbid_framework_citations: bool,

    /// When `true`, the reply must NOT contain a structured "label:
    /// value" line-by-line block. Casual replies are prose-only.
    #[serde(default)]
    pub forbid_line_by_line_blocks: bool,

    /// When set, lists with this many bullets or more must be collapsed
    /// to prose. `Some(3)` matches the vault doc's "lists of 3+ items
    /// collapse to prose" Casual rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub forbid_lists_at_or_above_count: Option<usize>,

    /// When `true`, numbers like `74.6 km` must be rounded (`75 km`).
    /// Implementation reports any 4+ significant digit decimal as a
    /// candidate violation; tolerance for false positives is high since
    /// the rule is advisory in non-strict mode.
    #[serde(default)]
    pub round_numbers_required: bool,

    /// When `true`, the assistant MUST NOT narrate tool calls
    /// ("Let me check your training history…"). Strings in
    /// [`TOOL_NARRATION_PHRASES`] flag the reply.
    #[serde(default)]
    pub forbid_tool_call_narration: bool,

    /// Acronyms whose first appearance must carry a parenthetical gloss
    /// — e.g. `CTL (your fitness baseline)`. A bare `CTL` triggers a
    /// violation. Empty list disables the check.
    #[serde(default)]
    pub forbid_acronyms_unglossed: Vec<String>,

    /// Catches first-use occurrences of any acronym (the platform's
    /// general acronym list, not the explicit `forbid_acronyms_unglossed`
    /// allowlist). Used by Enthusiast where the rule is "glossed once on
    /// first use" rather than "never".
    #[serde(default)]
    pub forbid_acronyms_first_use_unglossed: bool,

    /// Maximum number of lines per structured block. Used by
    /// Enthusiast's "small structured blocks (3–5 lines, no bullets)
    /// for per-activity summaries" rule.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_block_max_lines: Option<usize>,

    /// When `true`, every numeric claim must carry one of the
    /// frameworks listed in `framework_allowlist`. Power-athlete only.
    #[serde(default)]
    pub require_framework_citation_per_numeric: bool,

    /// Allowed framework labels (case-insensitive substring match) for
    /// `require_framework_citation_per_numeric`. Defaulting to empty
    /// disables the citation-presence check by definition.
    #[serde(default)]
    pub framework_allowlist: Vec<String>,

    /// When `true`, the reply must contain at least one structured
    /// `label: value` block (the inverse of `forbid_line_by_line_blocks`).
    #[serde(default)]
    pub require_line_by_line_block: bool,

    /// Conversational softeners that are forbidden in Power-athlete
    /// replies ("nice work", "I think", "around"). Case-insensitive
    /// substring match.
    #[serde(default)]
    pub forbid_softeners: Vec<String>,

    /// When `true`, vague modifiers ("approximately", "around", "~")
    /// next to numbers are violations. The implementation flags `~`,
    /// `≈`, "approximately", "around" within 10 chars of a digit.
    #[serde(default)]
    pub require_exact_numbers: bool,

    /// When `true`, replies issuing a Go / Modify / Skip verdict must
    /// include the literal P0 / P1 / P2 / P3 ladder anchor.
    #[serde(default)]
    pub require_p0_p3_ladder: bool,

    /// When `true`, replies referencing a specific athlete must prefix
    /// the data block with `<display_name> · <last4uuid>` so the coach
    /// can keep multiple athletes straight in scrollback.
    #[serde(default)]
    pub require_athlete_id_prefix: bool,

    /// When `true`, the conformance stage refuses to surface data
    /// outside the coach's roster (treat as tenant-isolation violation).
    #[serde(default)]
    pub require_tenant_isolation: bool,

    /// Notification cadence policy. Carried through the registry but
    /// not yet wired into canot's push tier metadata.
    #[serde(default)]
    pub notification: NotificationPolicy,

    /// When `true`, conformance violations BLOCK the reply and
    /// re-prompt the LLM with a "fix this" delta. When `false`,
    /// violations are advisory `warn!` logs only. Per-persona opt-in.
    #[serde(default)]
    pub strict_mode: bool,
}

/// Notification cadence per persona.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct NotificationPolicy {
    /// Lowest priority tier the user receives unsolicited. P0 = break
    /// glass for everyone; P3 = "every push" (Power-athlete request).
    /// Stored as a free-form label until canot exposes a typed enum.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier_floor: Option<String>,
    /// Digest cadence: `daily`, `weekly`, `per_session`, `per_athlete`,
    /// or any future label canot exposes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

/// Phrases that count as tool-call narration in conformance checks.
///
/// Compiled-in because they describe assistant *output style*, not
/// product policy — moving them into YAML would risk a contremaitre
/// edit silently weakening the rule across all personas.
pub const TOOL_NARRATION_PHRASES: &[&str] = &[
    "let me check",
    "i'll look up",
    "let me look",
    "let me pull",
    "let me fetch",
    "i'll fetch",
    "calling the tool",
    "running the query",
];

/// Snapshot stored in [`PersonaContractRegistry`] alongside the
/// contracts so callers can correlate against `loaded_at` /
/// `source_sha256` when triaging.
#[derive(Debug, Clone, Default)]
pub struct PersonaContractsSnapshot {
    /// Resolved contracts after `inherits` flattening.
    pub by_slug: HashMap<String, PersonaContract>,
    /// Schema version observed in the source document.
    pub version: u32,
    /// SHA-256 of the source YAML at install time.
    pub source_sha256: Option<String>,
    /// When this snapshot was installed.
    pub loaded_at: Option<DateTime<Utc>>,
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
}

/// Thread-safe registry holding the active [`PersonaContractsSnapshot`].
///
/// Replacements via `apply_overlay` are atomic: a chat turn either sees
/// the old snapshot or the new one, never a half-applied set.
#[derive(Debug)]
pub struct PersonaContractRegistry {
    snapshot: RwLock<Arc<PersonaContractsSnapshot>>,
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
            snapshot: RwLock::new(Arc::new(PersonaContractsSnapshot::default())),
        }
    }

    /// Return the currently installed snapshot. Cheap (`Arc::clone`).
    /// Hold the returned `Arc` for the duration of one chat turn rather
    /// than calling repeatedly — the snapshot is immutable, so reads
    /// are lock-free in practice.
    #[must_use]
    pub fn snapshot(&self) -> Arc<PersonaContractsSnapshot> {
        Arc::clone(&self.read())
    }

    /// SHA-256 of the YAML source the current snapshot was built from,
    /// or `None` for the empty boot snapshot. Used by the sync engine
    /// to skip re-parsing when the manifest hash already matches.
    #[must_use]
    pub fn current_overlay_sha256(&self) -> Option<String> {
        self.read().source_sha256.clone()
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
        let doc: PersonaContractsDocument = serde_yaml::from_str(yaml)
            .map_err(|e| AppError::config(format!("persona_contracts.yaml parse failed: {e}")))?;
        let resolved = resolve_inheritance(doc.personas)?;
        let snapshot = PersonaContractsSnapshot {
            by_slug: resolved,
            version: doc.version,
            source_sha256: Some(super::contremaitre::manifest::compute_sha256(
                yaml.as_bytes(),
            )),
            loaded_at: Some(Utc::now()),
        };
        *self.write() = Arc::new(snapshot);
        Ok(())
    }

    fn read(&self) -> RwLockReadGuard<'_, Arc<PersonaContractsSnapshot>> {
        self.snapshot.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, Arc<PersonaContractsSnapshot>> {
        self.snapshot
            .write()
            .unwrap_or_else(PoisonError::into_inner)
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

/// Overlay `child` onto `parent` field-by-field. Convention: a child
/// `Some(_)` or non-empty `Vec`/`bool=true` wins; a child default
/// (`None`, `Vec::default()`, `false`) keeps the parent value.
fn overlay(parent: PersonaContract, child: PersonaContract) -> PersonaContract {
    PersonaContract {
        inherits: None,
        max_words: child.max_words.or(parent.max_words),
        forbid_framework_citations: child.forbid_framework_citations
            || parent.forbid_framework_citations,
        forbid_line_by_line_blocks: child.forbid_line_by_line_blocks
            || parent.forbid_line_by_line_blocks,
        forbid_lists_at_or_above_count: child
            .forbid_lists_at_or_above_count
            .or(parent.forbid_lists_at_or_above_count),
        round_numbers_required: child.round_numbers_required || parent.round_numbers_required,
        forbid_tool_call_narration: child.forbid_tool_call_narration
            || parent.forbid_tool_call_narration,
        forbid_acronyms_unglossed: vec_overlay(
            parent.forbid_acronyms_unglossed,
            child.forbid_acronyms_unglossed,
        ),
        forbid_acronyms_first_use_unglossed: child.forbid_acronyms_first_use_unglossed
            || parent.forbid_acronyms_first_use_unglossed,
        structured_block_max_lines: child
            .structured_block_max_lines
            .or(parent.structured_block_max_lines),
        require_framework_citation_per_numeric: child.require_framework_citation_per_numeric
            || parent.require_framework_citation_per_numeric,
        framework_allowlist: vec_overlay(parent.framework_allowlist, child.framework_allowlist),
        require_line_by_line_block: child.require_line_by_line_block
            || parent.require_line_by_line_block,
        forbid_softeners: vec_overlay(parent.forbid_softeners, child.forbid_softeners),
        require_exact_numbers: child.require_exact_numbers || parent.require_exact_numbers,
        require_p0_p3_ladder: child.require_p0_p3_ladder || parent.require_p0_p3_ladder,
        require_athlete_id_prefix: child.require_athlete_id_prefix
            || parent.require_athlete_id_prefix,
        require_tenant_isolation: child.require_tenant_isolation || parent.require_tenant_isolation,
        notification: NotificationPolicy {
            tier_floor: child
                .notification
                .tier_floor
                .or(parent.notification.tier_floor),
            digest: child.notification.digest.or(parent.notification.digest),
        },
        strict_mode: child.strict_mode || parent.strict_mode,
    }
}

/// Child's vec wins when non-empty; otherwise parent's. Lets a child
/// declare `forbid_softeners: []` to clear inherited entries — but the
/// far more common pattern (in the YAML committed today) is an empty
/// child vec meaning "I don't override; keep the parent's".
fn vec_overlay(parent: Vec<String>, child: Vec<String>) -> Vec<String> {
    if child.is_empty() {
        parent
    } else {
        child
    }
}
