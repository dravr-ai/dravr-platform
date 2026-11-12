// ABOUTME: Hot-reloadable training catalogue registry — flavours, season skeletons, workout templates, selection table
// ABOUTME: Seeded from the pinned dravr-contremaitre training tables; the sync overlays one entry per (kind, slug)
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
//! The registry is seeded at construction from the pinned
//! `dravr_contremaitre::training` tables — contremaitre's `training/` tree,
//! compiled into that crate — with [`PromptSource::CompiledIn`], so
//! the agent has a full bank before the first contremaitre sync lands and
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
use dravr_contremaitre::training;
use pierre_core::models::periodization::{
    Flavour, Generated, InputDimension, InstanceRequest, PhaseKind, SelectionRow, SelectionTable,
    SkeletonTemplate, SportFamily, SportMix, UnresolvedReference, WorkoutFilter, WorkoutPurpose,
    WorkoutTemplate,
};
use pierre_core::models::SportType;
use tracing::error;

use super::errors::ContremaitreError;
use super::manifest::compute_sha256;
use super::registry::PromptSource;

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
    /// Entries seeded from the pinned dravr-contremaitre tables.
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
#[derive(Debug)]
pub struct TrainingCatalogueRegistry {
    /// The live set the agent reads.
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
            (CatalogueKind::Flavour, training::FLAVOURS),
            (CatalogueKind::Skeleton, training::SKELETONS),
            (CatalogueKind::Workout, training::WORKOUTS),
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
            training::SELECTION,
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
    ///
    /// A slug that is no longer a file — a legacy slug a family absorbed, as
    /// an alias or as a preset key — resolves to that family, handed back as
    /// the session the old file described: the preset's steps, sport and
    /// duration, generated with the alias's pinned values. A caller that
    /// wants the family itself, with its presets, asks
    /// [`Self::resolve_workout`].
    #[must_use]
    pub fn workout(&self, slug: &str) -> Option<WorkoutTemplate> {
        let resolved = self.resolve_workout(slug)?;
        if resolved.request.preset.is_none() {
            return Some(resolved.template);
        }
        let generated = resolved.template.generate(&resolved.request, None);
        Some(session_view(&resolved.template, generated))
    }

    /// A slug resolved against the bank: a file stem first, then a family's
    /// legacy alias, then a family's preset key.
    #[must_use]
    pub fn resolve_workout(&self, slug: &str) -> Option<ResolvedWorkout> {
        let guard = self.read();
        if let Some(entry) = guard.get(&(CatalogueKind::Workout, slug.to_owned())) {
            if let CatalogueItem::Workout(w) = &entry.item {
                return Some(ResolvedWorkout {
                    template: (**w).clone(),
                    request: InstanceRequest::default(),
                });
            }
        }
        let mut families: Vec<&WorkoutTemplate> = guard
            .values()
            .filter_map(|e| match &e.item {
                CatalogueItem::Workout(w) if w.is_family() => Some(&**w),
                _ => None,
            })
            .collect();
        families.sort_by(|a, b| a.slug.cmp(&b.slug));
        families.into_iter().find_map(|family| {
            let (_, alias) = family.resolve_preset(slug)?;
            Some(ResolvedWorkout {
                template: family.clone(),
                request: InstanceRequest {
                    preset: Some(slug.to_owned()),
                    sport: alias.and_then(|a| a.sport.clone()),
                    ..InstanceRequest::default()
                },
            })
        })
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
    ///   (and the sport, for an open-water skeleton). A flavour is walked
    ///   again for each sport family the selection table lets it be applied
    ///   to, its non-strength purposes needing a carrier of that sport; those
    ///   reports read `<purpose> for <family>`.
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

        let selection = guard.values().find_map(|e| match &e.item {
            CatalogueItem::Selection(table) => Some(&**table),
            _ => None,
        });

        for entry in guard.values() {
            match &entry.item {
                CatalogueItem::Flavour(flavour) => {
                    out.extend(flavour.unresolved_references(evidence_exists));
                    out.extend(
                        flavour
                            .unresolved_purposes(&|phase, purpose| carried(phase, purpose, None)),
                    );
                    // Per sport the flavour can be applied to (carnet#1465): a run
                    // template alone does not answer for a cyclist's endurance day.
                    // Strength purposes are carried by the gym sport for every sport,
                    // so they stay sport-blind.
                    for family in flavour_sports(&flavour.id, selection) {
                        let sport = family.sport();
                        out.extend(
                            flavour
                                .unresolved_purposes(&|phase, purpose| {
                                    purpose.is_strength()
                                        || carried(phase, purpose, Some(sport.clone()))
                                })
                                .into_iter()
                                .map(|mut unresolved| {
                                    unresolved.reference =
                                        format!("{} for {family}", unresolved.reference);
                                    unresolved
                                }),
                        );
                    }
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

/// A workout slug resolved against the bank.
#[derive(Debug, Clone)]
pub struct ResolvedWorkout {
    /// The template the slug resolved to — a family when the slug was one of
    /// its legacy aliases or preset keys.
    pub template: WorkoutTemplate,
    /// What to generate for the slug: its preset and the alias's pinned
    /// sport; empty when the slug is the template's own.
    pub request: InstanceRequest,
}

/// A family handed back as the single session one generated instance of it
/// describes.
///
/// The view carries the instance's steps, sport and duration, named after
/// its preset and held at that preset's readiness floor, under the family's
/// purpose: what a legacy slug meant before its file was absorbed, and the
/// one session a caller prescribes from a family.
#[must_use]
pub fn session_view(family: &WorkoutTemplate, generated: Generated) -> WorkoutTemplate {
    let preset = generated.preset.as_deref();
    let mut view = family.clone();
    if let Some(named) = preset.and_then(|key| family.preset(key)) {
        view.name = format!("{} — {}", family.name, named.name);
    }
    view.fit.readiness_min = family.effective_fit(preset).readiness_min;
    view.structure = generated.steps;
    view.duration_minutes = generated.duration_minutes;
    view.sport = generated.sport;
    view
}

/// The sport families a flavour can be applied to, read from the selection
/// table: the bike when a cycling or triathlon row exists that does not
/// exclude it, the run when a running or triathlon row does. With no such
/// row (no table, or a table without a sport-mix dimension), both.
///
/// LIMITATION(registre#1498): `flavour_sports` never returns `SportFamily::Swim`;
/// no swim form carries the quality purposes, so swim coverage is sport-blind.
fn flavour_sports(id: &str, selection: Option<&SelectionTable>) -> Vec<SportFamily> {
    let rows: Vec<&SelectionRow> = selection
        .map(|table| {
            table
                .rows
                .iter()
                .filter(|row| row.input == InputDimension::SportMix)
                .collect()
        })
        .unwrap_or_default();
    let admits = |mixes: [SportMix; 2]| {
        let matching: Vec<&SelectionRow> = rows
            .iter()
            .copied()
            .filter(|row| mixes.iter().any(|mix| row.value == mix.as_str()))
            .collect();
        matching.is_empty()
            || matching
                .iter()
                .any(|row| !row.exclude.iter().any(|exclusion| exclusion.id == id))
    };
    let mut sports = Vec::new();
    if admits([SportMix::Cycling, SportMix::Triathlon]) {
        sports.push(SportFamily::Ride);
    }
    if admits([SportMix::Running, SportMix::Triathlon]) {
        sports.push(SportFamily::Run);
    }
    sports
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_core::models::periodization::{EvidenceTier, FlavourExclusion, ReadinessLevel};

    /// A two-preset family whose slugs no catalogue file carries, so the
    /// compiled-in bank cannot answer for it.
    const FAMILY: &str = r#"
id = "4b9c97a8-91d6-51bc-99d1-5454f2f8269b"
slug = "fixture_vo2max_short"
name = "VO2max — short intervals"
sport = "ride"
sport_variants = ["ride", "run"]
intensity_distribution = "vo2max"
purpose = "vo2max_short"
evidence_tier = "grey"
caveat = "Fixture."

[params]
sets = { min = 1, max = 5, default = 3 }
reps = { min = 6, max = 30, default = 13 }
work_seconds = { min = 15, max = 60, default = 30 }
rest_seconds = { min = 10, max = 30, default = 15 }
set_rest_seconds = { min = 120, max = 180, default = 180 }
rpe = { min = 8, max = 9 }

[params.intensity]
ride = "100-110% MAP"
run = "100-120% vVO2max"

[progression]
order = ["add_rep", "add_set", "raise_intensity"]

[fit]
phases = ["build", "peak"]
readiness_min = "p2"
max_per_week = 2
min_spacing_hours = 48

[frame.ride]
warmup = { seconds = 900, target = "Z2", label = "Warm-up" }
cooldown = { seconds = 600, target = "Z1", label = "Cool-down" }

[frame.run]
warmup = { seconds = 900, target = "Z2", label = "Warm-up" }
cooldown = { seconds = 600, target = "Z1", label = "Cool-down" }

[aliases]
fixture_alias_30_15 = { preset = "fx_30_15", sport = "ride" }
fixture_alias_30_30 = { preset = "fx_30_30", sport = "run" }

[[presets]]
key = "fx_30_15"
name = "30/15 (Rønnestad)"
shape = "intervals"
params = { reps = { min = 10, max = 13, default = 13 }, sets = { min = 2, max = 4, default = 3 }, work_seconds = { min = 30, max = 30, default = 30 }, rest_seconds = { min = 15, max = 15, default = 15 } }

[presets.ride]
badge = { status = "trialled", basis = "Rønnestad 2015 [RCT]" }
cycle = [
  { label = "30 s hard", target = "100-110% MAP", pacing = "best_sustainable" },
  { label = "15 s float", target = "50% MAP" },
]

[presets.run]
badge = { status = "transposed" }
cycle = [
  { label = "30 s fast", target = "100-105% vVO2max" },
  { label = "15 s jog", target = "50% vVO2max" },
]

[[presets]]
key = "fx_30_30"
name = "30/30 (Billat)"
shape = "intervals"
readiness_min = "p3"
params = { reps = { min = 10, max = 25, default = 15 }, sets = { min = 1, max = 3, default = 1 }, work_seconds = { min = 30, max = 30, default = 30 }, rest_seconds = { min = 30, max = 30, default = 30 } }

[presets.ride]
badge = { status = "transposed" }
cycle = [
  { label = "30 s hard", target = "115-125% MAP" },
  { label = "30 s float", target = "50% MAP" },
]

[presets.run]
badge = { status = "trialled", acute_only = true, basis = "Billat 2000 [acute]" }
cycle = [
  { label = "30 s fast", target = "100-105% vVO2max" },
  { label = "30 s jog", target = "50% vVO2max" },
]
"#;

    fn registry_with_family() -> TrainingCatalogueRegistry {
        let registry = TrainingCatalogueRegistry::new();
        let item = parse_catalogue_file(CatalogueKind::Workout, FAMILY).unwrap();
        registry.update(
            CatalogueKind::Workout,
            "fixture_vo2max_short",
            item,
            compute_sha256(FAMILY.as_bytes()),
        );
        registry
    }

    #[test]
    fn a_legacy_alias_resolves_to_its_family_with_the_pinned_sport() {
        let registry = registry_with_family();
        let resolved = registry.resolve_workout("fixture_alias_30_30").unwrap();
        assert_eq!(resolved.template.slug, "fixture_vo2max_short");
        assert_eq!(
            resolved.request.preset.as_deref(),
            Some("fixture_alias_30_30"),
            "the alias is what the generator resolves, pins and all"
        );
        assert_eq!(resolved.request.sport, Some(SportType::Run));
        let by_key = registry.resolve_workout("fx_30_15").unwrap();
        assert_eq!(by_key.template.slug, "fixture_vo2max_short");
        assert_eq!(by_key.request.preset.as_deref(), Some("fx_30_15"));
        assert_eq!(by_key.request.sport, None, "a preset key pins no sport");
        let own = registry.resolve_workout("fixture_vo2max_short").unwrap();
        assert_eq!(
            own.request,
            InstanceRequest::default(),
            "the family's own slug asks for nothing"
        );
        assert!(registry.resolve_workout("fixture_no_such_slug").is_none());
    }

    #[test]
    fn a_legacy_slug_reads_back_as_the_session_its_file_described() {
        let registry = registry_with_family();
        let view = registry.workout("fixture_alias_30_30").unwrap();
        assert_eq!(view.slug, "fixture_vo2max_short");
        assert_eq!(view.name, "VO2max — short intervals — 30/30 (Billat)");
        assert_eq!(view.sport, SportType::Run, "the alias pins the run form");
        assert_eq!(
            view.fit.readiness_min,
            ReadinessLevel::P3,
            "the preset raises the family's p2 floor"
        );
        // Warm-up, one set of (work, float), cool-down; 15 × 60 s of reps.
        assert_eq!(view.structure.len(), 4, "{:?}", view.structure);
        assert_eq!(view.structure[1].target_zone, "100-105% vVO2max");
        assert_eq!(view.structure[1].repeat, 15);
        assert_eq!(view.duration_minutes, 15 + 15 + 10);
        let family = registry.workout("fixture_vo2max_short").unwrap();
        assert_eq!(family.name, "VO2max — short intervals");
        assert_eq!(family.fit.readiness_min, ReadinessLevel::P2);
    }

    /// A sport-mix row excluding `excluded`.
    fn sport_mix_row(mix: SportMix, excluded: &[&str]) -> SelectionRow {
        SelectionRow {
            input: InputDimension::SportMix,
            value: mix.as_str().to_owned(),
            prefer: Vec::new(),
            exclude: excluded
                .iter()
                .map(|id| FlavourExclusion {
                    id: (*id).to_owned(),
                    reason: "fixture".to_owned(),
                })
                .collect(),
            tier: EvidenceTier::Grey,
            evidence_refs: Vec::new(),
            note: None,
        }
    }

    #[test]
    fn a_flavour_reaches_each_sport_a_sport_mix_row_leaves_it() {
        assert_eq!(
            flavour_sports("any", None),
            [SportFamily::Ride, SportFamily::Run],
            "with no table, nothing rules a sport out"
        );
        let table = SelectionTable {
            rows: vec![
                sport_mix_row(SportMix::Cycling, &["trail", "cycling_out"]),
                sport_mix_row(SportMix::Triathlon, &["trail"]),
                sport_mix_row(SportMix::Running, &["road_out"]),
            ],
        };
        assert_eq!(
            flavour_sports("trail", Some(&table)),
            [SportFamily::Run],
            "excluded by every row that puts an athlete on the bike"
        );
        assert_eq!(
            flavour_sports("cycling_out", Some(&table)),
            [SportFamily::Ride, SportFamily::Run],
            "the triathlon row still applies it to the bike"
        );
        assert_eq!(
            flavour_sports("road_out", Some(&table)),
            [SportFamily::Ride, SportFamily::Run],
            "the triathlon row still applies it to the run"
        );
        let no_cycling_rows = SelectionTable {
            rows: vec![sport_mix_row(SportMix::Running, &["trail"])],
        };
        assert_eq!(
            flavour_sports("trail", Some(&no_cycling_rows)),
            [SportFamily::Ride],
            "a sport no row speaks to stays admitted; the running row rules the run out"
        );
    }
}
