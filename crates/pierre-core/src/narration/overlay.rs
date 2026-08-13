// ABOUTME: Runtime vocabulary overlay — operator-supplied table additions
// ABOUTME: Split out of narration/mod.rs so each file stays legible
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::{Arc, LazyLock, RwLock};

use super::fold::fold_separators;

/// Shortest folded pattern the overlay accepts, in characters.
///
/// This catches truncation and fat-finger entries, not semantic over-match.
/// Length cannot tell the two apart: `openai` (6) and `raw xml` (7) are
/// legitimate vocabulary while `de mon` (6) would scrub half of every French
/// reply. Judging that is review's job — the authored file lives in a
/// reviewed repo. The floor was 10 while the overlay only carried
/// hand-written deltas; the full vocabulary contains real entries below it.
pub(super) const OVERLAY_MIN_FOLDED_CHARS: usize = 4;

/// Most entries one overlay class accepts — a runaway-file backstop far
/// above any plausible vocabulary size (the compiled-in tables sit under
/// 200 entries each).
pub(super) const OVERLAY_MAX_ENTRIES: usize = 500;

/// Typed narration-vocabulary overlay.
///
/// The YAML lives in dravr-contremaitre (`config/narration.yaml`) and is
/// parsed by `pierre-contremaitre`, which owns the platform's YAML tooling —
/// this leaf crate only receives the typed lists.
///
/// Semantics are **additive**: entries extend the compiled-in tables, never
/// replace them, so a new phrasing mutation observed in production can start
/// matching on the next sync (≤60s) without a deploy — the incident cadence
/// that motivated this (2026-07-22 → 07-23 → 07-24 → 08-11 was four
/// deploy-gated pattern iterations). Each successful apply replaces the
/// *previous overlay* wholesale, so removing a bad overlay entry is just a
/// contremaitre edit too.
#[derive(Debug, Clone, Default)]
pub struct NarrationVocabOverlay {
    /// Extends [`CAPABILITY_FAILURE_PATTERNS`]: replay scrub AND the
    /// outbound [`contains_capability_failure`] boundary detector.
    pub capability_failure: Vec<String>,
    /// Extends [`INTERNAL_NARRATION_PATTERNS`]: outbound scrub and replay.
    pub internal_narration: Vec<String>,
    /// Extends the identity vocabulary on the REPLAY path only
    /// ([`scrub_replayed_narration`]). Deliberately NOT the outbound
    /// withhold: [`identity_leak_match`] carries negation-lookbehind and
    /// class/locale/index telemetry semantics that plain strings cannot
    /// express, so the withhold contract stays compiled-in.
    pub identity: Vec<String>,
}

/// Entry counts a successful apply installed, for the sync log line.
#[derive(Debug, Clone, Copy)]
pub struct NarrationOverlayCounts {
    /// Installed `capability_failure` entries.
    pub capability_failure: usize,
    /// Installed `internal_narration` entries.
    pub internal_narration: usize,
    /// Installed replay-only `identity` entries.
    pub identity: usize,
}

/// One immutable, pre-folded overlay generation.
#[derive(Default)]
pub(super) struct NarrationVocabSnapshot {
    pub(super) capability: Vec<String>,
    pub(super) internal: Vec<String>,
    pub(super) identity: Vec<String>,
    sha256: Option<String>,
}

/// Registry holding the live overlay snapshot.
///
/// Mirrors the `GLOBAL_PRICING_REGISTRY` / `NOTIFY_ROUTING_PROVIDER` house
/// pattern: the static lives in the leaf crate beside its consumers (the
/// matchers below), the writer is the contremaitre sync engine in a higher
/// crate. Swap is atomic (`Arc` behind an `RwLock`); a failed apply leaves
/// the previous snapshot untouched (last-good-wins, like every other
/// contremaitre overlay).
pub struct NarrationVocabRegistry {
    /// Current overlay generation. `Arc<RwLock>`-free: the registry itself
    /// is a process-wide static, so the lock alone suffices.
    snapshot: RwLock<Arc<NarrationVocabSnapshot>>,
}

impl NarrationVocabRegistry {
    fn new() -> Self {
        Self {
            snapshot: RwLock::new(Arc::new(NarrationVocabSnapshot::default())),
        }
    }

    /// Validate, fold, and atomically install `overlay`, recording `sha256`
    /// (of the downloaded file) for the sync engine's skip check.
    ///
    /// # Errors
    ///
    /// Rejects the WHOLE overlay — keeping the previous snapshot live — when
    /// any class exceeds [`OVERLAY_MAX_ENTRIES`] or any entry folds below
    /// [`OVERLAY_MIN_FOLDED_CHARS`] characters: a half-installed vocabulary
    /// would be harder to reason about than a rejected one.
    pub fn apply_overlay(
        &self,
        overlay: &NarrationVocabOverlay,
        sha256: String,
    ) -> Result<NarrationOverlayCounts, String> {
        let capability = fold_overlay_entries("capability_failure", &overlay.capability_failure)?;
        let internal = fold_overlay_entries("internal_narration", &overlay.internal_narration)?;
        let identity = fold_overlay_entries("identity", &overlay.identity)?;

        let counts = NarrationOverlayCounts {
            capability_failure: capability.len(),
            internal_narration: internal.len(),
            identity: identity.len(),
        };
        let next = Arc::new(NarrationVocabSnapshot {
            capability,
            internal,
            identity,
            sha256: Some(sha256),
        });
        self.snapshot.write().map_or_else(
            |_| Err("narration vocabulary lock poisoned; overlay not installed".to_owned()),
            |mut guard| {
                *guard = next;
                Ok(counts)
            },
        )
    }

    /// SHA-256 of the currently installed overlay file, or `None` before the
    /// first successful apply. The sync engine compares this against the
    /// manifest entry to skip an unchanged file.
    #[must_use]
    pub fn current_overlay_sha256(&self) -> Option<String> {
        self.snapshot.read().ok().and_then(|s| s.sha256.clone())
    }

    /// Whether the already-folded sentence matches an overlay entry of the
    /// given class. Read-lock per call: the scrub walks sentences through
    /// `fn`-pointer matchers, so there is no seam to thread a snapshot
    /// through — and an uncontended read lock is nanoseconds against the
    /// milliseconds a chat turn costs.
    pub(super) fn matches(
        &self,
        folded: &str,
        class: impl Fn(&NarrationVocabSnapshot) -> &[String],
    ) -> bool {
        self.snapshot
            .read()
            .is_ok_and(|s| class(&s).iter().any(|p| folded.contains(p.as_str())))
    }
}

/// Fold one overlay class, rejecting entries that would over-match.
pub(super) fn fold_overlay_entries(class: &str, entries: &[String]) -> Result<Vec<String>, String> {
    if entries.len() > OVERLAY_MAX_ENTRIES {
        return Err(format!(
            "narration overlay class `{class}` has {} entries (max {OVERLAY_MAX_ENTRIES})",
            entries.len()
        ));
    }
    entries
        .iter()
        .map(|raw| {
            let folded = fold_separators(raw);
            if folded.chars().count() < OVERLAY_MIN_FOLDED_CHARS {
                Err(format!(
                    "narration overlay class `{class}` entry `{raw}` folds to fewer than \
                     {OVERLAY_MIN_FOLDED_CHARS} characters and would over-match"
                ))
            } else {
                Ok(folded)
            }
        })
        .collect()
}

/// Process-wide narration-vocabulary overlay, seeded empty; the contremaitre
/// sync engine installs downloaded generations via
/// [`NarrationVocabRegistry::apply_overlay`].
pub static GLOBAL_NARRATION_VOCAB: LazyLock<NarrationVocabRegistry> =
    LazyLock::new(NarrationVocabRegistry::new);
