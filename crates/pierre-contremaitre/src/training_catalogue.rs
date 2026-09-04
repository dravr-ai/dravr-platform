// ABOUTME: Hot-reloadable training catalogue registry — flavours, season skeletons, workout templates, selection table
// ABOUTME: Seeded from the compiled-in training_catalogue/ mirror; contremaitre overlays one entry per (kind, slug)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Training catalogue registry
//!
//! The catalogue counterpart to [`super::PromptRegistry`]: the four file
//! shapes of contremaitre's `training/` tree, parsed and validated by the
//! periodization kernel, keyed by `(kind, slug)` where the slug is the file
//! stem (`polarized-classic`, `marathon-linear`, `vo2max_4x8`) and the one
//! selection table sits under [`SELECTION_SLUG`].
//!
//! The registry is seeded at construction from the generated
//! [`super::training_catalogue_embedded`] tables — the byte-for-byte mirror
//! of the tree at the platform root — with [`PromptSource::CompiledIn`], so
//! the coach has a full bank before the first contremaitre sync lands and
//! whenever the store is unreachable. A sync overlays entries with
//! [`PromptSource::Contremaitre`]; removing an overlaid entry reverts it to
//! the compiled-in one rather than leaving a hole.
//!
//! Every entry passed its own file's `validate` before it got in. The rules
//! that span files — a selection row naming a flavour with no file, a
//! purpose a flavour or skeleton asks for that no workout carries, an
//! `evidence_refs` path with no proposition behind it — are
//! [`TrainingCatalogueRegistry::unresolved_references`], which the sync
//! runs after every pass that changed something.

use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::{PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use chrono::{DateTime, Utc};
use pierre_core::models::periodization::{
    Flavour, PhaseKind, SelectionTable, SkeletonTemplate, UnresolvedReference, WorkoutFilter,
    WorkoutPurpose, WorkoutTemplate,
};
use pierre_core::models::SportType;
use tracing::error;

use super::errors::ContremaitreError;
use super::manifest::compute_sha256;
use super::registry::PromptSource;
use super::training_catalogue_embedded::{
    EMBEDDED_FLAVOURS, EMBEDDED_SELECTION, EMBEDDED_SKELETONS, EMBEDDED_WORKOUTS,
};

/// The slug the single selection table is registered under — there is one
/// `training/selection.yaml`, so its key is the shape's own name.
pub const SELECTION_SLUG: &str = "selection";

/// The four file shapes of the catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CatalogueKind {
    /// `training/flavours/<id>.yaml`
    Flavour,
    /// `training/skeletons/<id>.yaml`
    Skeleton,
    /// `training/workouts/<slug>.toml`
    Workout,
    /// `training/selection.yaml`
    Selection,
}

impl CatalogueKind {
    /// Every kind, in the order the manifest's `training` section lists them.
    pub const ALL: &[Self] = &[
        Self::Flavour,
        Self::Skeleton,
        Self::Workout,
        Self::Selection,
    ];

    /// The manifest section and directory name of the kind.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Flavour => "flavours",
            Self::Skeleton => "skeletons",
            Self::Workout => "workouts",
            Self::Selection => "selection",
        }
    }
}

impl fmt::Display for CatalogueKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One parsed catalogue file. Boxed because the four payloads differ by an
/// order of magnitude in size and the registry stores them side by side.
#[derive(Debug, Clone, PartialEq)]
pub enum CatalogueItem {
    /// A training-intensity-distribution model.
    Flavour(Box<Flavour>),
    /// A season skeleton for an event class.
    Skeleton(Box<SkeletonTemplate>),
    /// A workout template with its parameter ranges.
    Workout(Box<WorkoutTemplate>),
    /// The profile-to-flavour selection table.
    Selection(Box<SelectionTable>),
}

impl CatalogueItem {
    /// The shape this item is.
    #[must_use]
    pub const fn kind(&self) -> CatalogueKind {
        match self {
            Self::Flavour(_) => CatalogueKind::Flavour,
            Self::Skeleton(_) => CatalogueKind::Skeleton,
            Self::Workout(_) => CatalogueKind::Workout,
            Self::Selection(_) => CatalogueKind::Selection,
        }
    }
}

/// A single catalogue entry in the registry.
#[derive(Debug, Clone)]
pub struct CatalogueEntry {
    /// The parsed, validated file.
    pub item: CatalogueItem,
    /// SHA-256 hex digest of the file text it was parsed from.
    pub sha256: String,
    /// Where this entry was loaded from.
    pub source: PromptSource,
    /// When this entry was loaded or last updated.
    pub loaded_at: DateTime<Utc>,
}

/// Counts over the live registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CatalogueStats {
    /// Number of flavours.
    pub flavours: usize,
    /// Number of season skeletons.
    pub skeletons: usize,
    /// Number of workout templates.
    pub workouts: usize,
    /// Rows in the selection table, zero when there is none.
    pub selection_rows: usize,
    /// Entries loaded from the compiled-in mirror.
    pub compiled_in_count: usize,
    /// Entries overlaid from contremaitre.
    pub contremaitre_count: usize,
}

impl fmt::Display for CatalogueStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} flavours + {} skeletons + {} workouts + {} selection rows ({} compiled-in, {} contremaitre)",
            self.flavours,
            self.skeletons,
            self.workouts,
            self.selection_rows,
            self.compiled_in_count,
            self.contremaitre_count
        )
    }
}

type EntryMap = HashMap<(CatalogueKind, String), CatalogueEntry>;

/// Thread-safe registry for the training catalogue.
///
/// Seeded with the compiled-in mirror; entries are overlaid by the
/// contremaitre sync and revert to the seed when the overlay is removed.
pub struct TrainingCatalogueRegistry {
    /// The live set the coach reads.
    entries: RwLock<EntryMap>,
    /// The seed, kept so a removed overlay falls back to it.
    compiled_in: EntryMap,
}

impl TrainingCatalogueRegistry {
    /// Create a registry populated with every compiled-in catalogue file.
    ///
    /// A file that fails to parse or validate is logged at `error!` and
    /// left out, so the registry always constructs; the seed test in
    /// `training_catalogue_test.rs` pins the full count so that error
    /// cannot pass CI.
    #[must_use]
    pub fn new() -> Self {
        let now = Utc::now();
        let mut compiled_in = HashMap::new();
        let tables: [(CatalogueKind, &[(&str, &str)]); 3] = [
            (CatalogueKind::Flavour, EMBEDDED_FLAVOURS),
            (CatalogueKind::Skeleton, EMBEDDED_SKELETONS),
            (CatalogueKind::Workout, EMBEDDED_WORKOUTS),
        ];
        for (kind, table) in tables {
            for (slug, text) in table {
                seed_entry(&mut compiled_in, kind, slug, text, now);
            }
        }
        seed_entry(
            &mut compiled_in,
            CatalogueKind::Selection,
            SELECTION_SLUG,
            EMBEDDED_SELECTION,
            now,
        );
        Self {
            entries: RwLock::new(compiled_in.clone()),
            compiled_in,
        }
    }

    /// Return the current SHA-256 hash for an entry, if any.
    #[must_use]
    pub fn sha256(&self, kind: CatalogueKind, slug: &str) -> Option<String> {
        self.read()
            .get(&(kind, slug.to_owned()))
            .map(|e| e.sha256.clone())
    }

    /// Insert or overlay an entry with content synced from contremaitre.
    pub fn update(&self, kind: CatalogueKind, slug: &str, item: CatalogueItem, sha256: String) {
        self.write().insert(
            (kind, slug.to_owned()),
            CatalogueEntry {
                item,
                sha256,
                source: PromptSource::Contremaitre,
                loaded_at: Utc::now(),
            },
        );
    }

    /// Remove a contremaitre overlay.
    ///
    /// Reverts the slot to its compiled-in entry when one exists, else
    /// drops it. Returns `true` when the live set changed — the slot held
    /// an overlay, or an entry with no compiled-in fallback.
    #[must_use]
    pub fn remove(&self, kind: CatalogueKind, slug: &str) -> bool {
        let key = (kind, slug.to_owned());
        let mut entries = self.write();
        match self.compiled_in.get(&key) {
            Some(seed) => {
                let overlaid = entries
                    .get(&key)
                    .is_some_and(|e| e.source == PromptSource::Contremaitre);
                if overlaid {
                    entries.insert(key, seed.clone());
                }
                overlaid
            }
            None => entries.remove(&key).is_some(),
        }
    }

    /// Every flavour, sorted by id.
    #[must_use]
    pub fn flavours(&self) -> Vec<Flavour> {
        let mut out: Vec<Flavour> = self
            .read()
            .values()
            .filter_map(|e| match &e.item {
                CatalogueItem::Flavour(f) => Some((**f).clone()),
                _ => None,
            })
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// One flavour by id.
    #[must_use]
    pub fn flavour(&self, id: &str) -> Option<Flavour> {
        match &self
            .read()
            .get(&(CatalogueKind::Flavour, id.to_owned()))?
            .item
        {
            CatalogueItem::Flavour(f) => Some((**f).clone()),
            _ => None,
        }
    }

    /// Every season skeleton, sorted by id.
    #[must_use]
    pub fn skeletons(&self) -> Vec<SkeletonTemplate> {
        let mut out: Vec<SkeletonTemplate> = self
            .read()
            .values()
            .filter_map(|e| match &e.item {
                CatalogueItem::Skeleton(s) => Some((**s).clone()),
                _ => None,
            })
            .collect();
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// One skeleton by id.
    #[must_use]
    pub fn skeleton(&self, id: &str) -> Option<SkeletonTemplate> {
        match &self
            .read()
            .get(&(CatalogueKind::Skeleton, id.to_owned()))?
            .item
        {
            CatalogueItem::Skeleton(s) => Some((**s).clone()),
            _ => None,
        }
    }

    /// Every workout template, sorted by slug.
    #[must_use]
    pub fn workouts(&self) -> Vec<WorkoutTemplate> {
        let mut out = self.collect_workouts(|_| true);
        out.sort_by(|a, b| a.slug.cmp(&b.slug));
        out
    }

    /// One workout template by slug.
    #[must_use]
    pub fn workout(&self, slug: &str) -> Option<WorkoutTemplate> {
        match &self
            .read()
            .get(&(CatalogueKind::Workout, slug.to_owned()))?
            .item
        {
            CatalogueItem::Workout(w) => Some((**w).clone()),
            _ => None,
        }
    }

    /// The templates that satisfy `filter`, sorted by `(purpose, slug)` so
    /// the bank reads grouped by what a session is for.
    #[must_use]
    pub fn workouts_matching(&self, filter: &WorkoutFilter) -> Vec<WorkoutTemplate> {
        let mut out = self.collect_workouts(|w| filter.matches(w));
        out.sort_by(|a, b| a.purpose.cmp(&b.purpose).then_with(|| a.slug.cmp(&b.slug)));
        out
    }

    /// The selection table, when the catalogue carries one.
    #[must_use]
    pub fn selection(&self) -> Option<SelectionTable> {
        match &self
            .read()
            .get(&(CatalogueKind::Selection, SELECTION_SLUG.to_owned()))?
            .item
        {
            CatalogueItem::Selection(s) => Some((**s).clone()),
            _ => None,
        }
    }

    /// Counts over the live registry.
    #[must_use]
    pub fn stats(&self) -> CatalogueStats {
        let guard = self.read();
        let mut stats = CatalogueStats {
            flavours: 0,
            skeletons: 0,
            workouts: 0,
            selection_rows: 0,
            compiled_in_count: 0,
            contremaitre_count: 0,
        };
        for entry in guard.values() {
            match &entry.item {
                CatalogueItem::Flavour(_) => stats.flavours += 1,
                CatalogueItem::Skeleton(_) => stats.skeletons += 1,
                CatalogueItem::Workout(_) => stats.workouts += 1,
                CatalogueItem::Selection(table) => stats.selection_rows += table.rows.len(),
            }
            match entry.source {
                PromptSource::CompiledIn => stats.compiled_in_count += 1,
                PromptSource::Contremaitre => stats.contremaitre_count += 1,
            }
        }
        stats
    }

    /// Every reference the live catalogue makes that nothing answers for.
    ///
    /// Three kinds, each as an [`UnresolvedReference`] naming the owner
    /// file and the key the reference sits under:
    ///
    /// - an `evidence_refs` path `evidence_exists(category, slug)` denies —
    ///   the kernel's per-file walk, applied to every entry;
    /// - a selection row's `prefer` / `exclude` id with no flavour entry
    ///   (`rows[i].prefer[j].id`) — the kernel's `unresolved_flavours` walk
    ///   answered from the loaded flavours;
    /// - a purpose a flavour's session mix or readiness ladder, or a
    ///   skeleton's key sessions or strength block, asks for that no
    ///   workout carries — the kernel's `unresolved_purposes` walks, each
    ///   answered by one [`WorkoutFilter`] match over the loaded bank, so a
    ///   purpose named for a phase needs a carrier that fits that phase
    ///   (and the sport, for an open-water skeleton).
    ///
    /// Sorted by `(owner, key, reference)` so two runs over the same
    /// catalogue report the same list.
    pub fn unresolved_references(
        &self,
        evidence_exists: &dyn Fn(&str, &str) -> bool,
    ) -> Vec<UnresolvedReference> {
        let guard = self.read();
        let mut out = Vec::new();

        let flavour_ids: BTreeSet<&str> = guard
            .values()
            .filter_map(|e| match &e.item {
                CatalogueItem::Flavour(f) => Some(f.id.as_str()),
                _ => None,
            })
            .collect();
        let bank: Vec<&WorkoutTemplate> = guard
            .values()
            .filter_map(|e| match &e.item {
                CatalogueItem::Workout(w) => Some(&**w),
                _ => None,
            })
            .collect();
        let carried =
            |phase: Option<PhaseKind>, purpose: WorkoutPurpose, sport: Option<SportType>| {
                let filter = WorkoutFilter {
                    purpose: Some(purpose),
                    phase,
                    sport,
                };
                bank.iter().any(|w| filter.matches(w))
            };

        for entry in guard.values() {
            match &entry.item {
                CatalogueItem::Flavour(flavour) => {
                    out.extend(flavour.unresolved_references(evidence_exists));
                    out.extend(
                        flavour
                            .unresolved_purposes(&|phase, purpose| carried(phase, purpose, None)),
                    );
                }
                CatalogueItem::Skeleton(skeleton) => {
                    out.extend(skeleton.unresolved_references(evidence_exists));
                    out.extend(skeleton.unresolved_purposes(&|phase, purpose, sport| {
                        carried(phase, purpose, sport.cloned())
                    }));
                }
                CatalogueItem::Workout(workout) => {
                    out.extend(workout.unresolved_references(evidence_exists));
                }
                CatalogueItem::Selection(table) => {
                    out.extend(table.unresolved_references(evidence_exists));
                    out.extend(table.unresolved_flavours(&|id| flavour_ids.contains(id)));
                }
            }
        }

        out.sort_by(|a, b| {
            a.owner
                .cmp(&b.owner)
                .then_with(|| a.key.cmp(&b.key))
                .then_with(|| a.reference.cmp(&b.reference))
        });
        out
    }

    fn collect_workouts(&self, keep: impl Fn(&WorkoutTemplate) -> bool) -> Vec<WorkoutTemplate> {
        self.read()
            .values()
            .filter_map(|e| match &e.item {
                CatalogueItem::Workout(w) if keep(w) => Some((**w).clone()),
                _ => None,
            })
            .collect()
    }

    fn read(&self) -> RwLockReadGuard<'_, EntryMap> {
        self.entries.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, EntryMap> {
        self.entries.write().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Default for TrainingCatalogueRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Parse one compiled-in file into `map`, or log why it was left out.
fn seed_entry(map: &mut EntryMap, kind: CatalogueKind, slug: &str, text: &str, now: DateTime<Utc>) {
    match parse_catalogue_file(kind, text) {
        Ok(item) => {
            map.insert(
                (kind, slug.to_owned()),
                CatalogueEntry {
                    item,
                    sha256: compute_sha256(text.as_bytes()),
                    source: PromptSource::CompiledIn,
                    loaded_at: now,
                },
            );
        }
        Err(e) => error!(
            kind = kind.as_str(),
            slug,
            error = %e,
            "compiled-in training catalogue file rejected — the seed leaves it out"
        ),
    }
}

/// Parse one catalogue file of the given kind through the kernel, which
/// validates it before handing it back.
///
/// The kernel's error already names the file by its id or slug when the
/// text parsed far enough to have one, and the field path otherwise; the
/// message is prefixed with the kind so the sync's warning reads whole.
///
/// # Errors
///
/// Returns [`ContremaitreError::ManifestParse`] when the text is not a
/// well-formed document of that shape or breaks one of its invariants.
pub fn parse_catalogue_file(
    kind: CatalogueKind,
    contents: &str,
) -> Result<CatalogueItem, ContremaitreError> {
    let parsed = match kind {
        CatalogueKind::Flavour => {
            Flavour::from_yaml(contents).map(|f| CatalogueItem::Flavour(Box::new(f)))
        }
        CatalogueKind::Skeleton => {
            SkeletonTemplate::from_yaml(contents).map(|s| CatalogueItem::Skeleton(Box::new(s)))
        }
        CatalogueKind::Workout => {
            WorkoutTemplate::from_toml(contents).map(|w| CatalogueItem::Workout(Box::new(w)))
        }
        CatalogueKind::Selection => {
            SelectionTable::from_yaml(contents).map(|t| CatalogueItem::Selection(Box::new(t)))
        }
    };
    parsed.map_err(|e| ContremaitreError::ManifestParse(format!("training {kind}: {e}")))
}
