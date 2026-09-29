// ABOUTME: Pins the seeded training catalogue — counts, slugs, the KB numbers of named files, every file through the kernel
// ABOUTME: Cross-file rules: no dangling evidence ref, every named session has a carrier fitting its phase, the seed mirrors the pinned tables
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The registry is seeded from `dravr_contremaitre::training`, the tables the
//! pinned dravr-contremaitre rev compiles from its `training/` tree. A file
//! that fails to parse is logged and left out rather than failing the boot,
//! so the exact counts here are what stops a broken file from reaching a
//! release: 10 flavours, 12 skeletons, 37 workouts, one selection table.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::{BTreeSet, HashSet};

use dravr_contremaitre::{evidence, training};
use pierre_contremaitre::manifest::compute_sha256;
use pierre_contremaitre::training_catalogue::{
    CatalogueItem, CatalogueKind, TrainingCatalogueRegistry, SELECTION_SLUG,
};
use pierre_core::models::periodization::{
    evidence_ref_parts, Contraindication, EventClass, EvidenceTier, Flavour, FlavourFamily,
    Measurement, Modifier, PhaseKind, ReadinessLevel, RelativeIntensity, SelectionTable,
    SkeletonTemplate, WorkoutFilter, WorkoutPurpose, WorkoutTemplate,
};
use pierre_core::models::SportType;

/// The 37 workout slugs: the 33 of the Phase 1 bank (spec §9) and the four
/// trail sessions the ultra-trail flavour names.
const WORKOUT_SLUGS: [&str; 37] = [
    "back_to_back_long",
    "billat_30_30",
    "brick",
    "complex_training",
    "core_mobility",
    "double_threshold_day",
    "downhill_repeats",
    "endurance",
    "hill_sprints",
    "long_run_z2",
    "over_under",
    "plyo_basic",
    "race_pace_long",
    "recovery_30min",
    "repeated_sprint",
    "simulation",
    "sprint_interval",
    "strength_aa",
    "strength_maint",
    "strength_max",
    "strides",
    "sweet_spot_2x20",
    "swim_css",
    "swim_dryland",
    "swim_usrpt",
    "tempo",
    "tempo_progression",
    "threshold_4x8",
    "threshold_short",
    "trail_race_simulation",
    "uphill_power_hike",
    "vo2_5x3",
    "vo2max_30_15",
    "vo2max_4x8",
    "vo2max_hills",
    "vo2max_tmax",
    "vo2max_varied",
];

/// The ten flavour ids: the nine of spec §3.3 and the mountain ultra-trail one.
const FLAVOUR_IDS: [&str; 10] = [
    "hvlit-foundation",
    "norwegian-singles-subthreshold",
    "norwegian-threshold-density",
    "polarized-classic",
    "pyramidal-base",
    "pyramidal-long-course",
    "pyramidal-to-polarized",
    "pyramidal-ultra-trail",
    "race-specific",
    "time-crunched-threshold",
];

/// The twelve skeleton ids (spec §9).
const SKELETON_IDS: [&str; 12] = [
    "crit",
    "half-iron",
    "half-marathon",
    "ironman",
    "marathon-linear",
    "no-race-foundation",
    "open-water-swim",
    "road-race-gran-fondo",
    "run-5k-10k",
    "sprint-olympic-tri",
    "time-trial",
    "ultra",
];

/// Files the seed carries: 10 + 12 + 37 + the selection table.
const SEED_FILE_COUNT: usize = 60;

/// The pinned table for one file shape: `(stem, text)` per file.
fn table(kind: CatalogueKind) -> &'static [(&'static str, &'static str)] {
    match kind {
        CatalogueKind::Flavour => training::FLAVOURS,
        CatalogueKind::Skeleton => training::SKELETONS,
        CatalogueKind::Workout => training::WORKOUTS,
        CatalogueKind::Selection => panic!("the selection table is one document, not a table"),
    }
}

/// The file stems the pinned table carries for `kind`, sorted.
fn stems(kind: CatalogueKind) -> BTreeSet<String> {
    table(kind)
        .iter()
        .map(|(stem, _)| (*stem).to_owned())
        .collect()
}

fn as_set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

/// Every `(category, slug)` the pinned evidence corpus answers for, keyed the
/// way `evidence_ref_parts` splits a ref.
fn evidence_keys() -> HashSet<(String, String)> {
    evidence::SPORTS_SCIENCE
        .iter()
        .filter_map(|(key, _)| {
            let path = format!("evidence/sports_science/{key}");
            evidence_ref_parts(&path).map(|(category, slug)| (category.to_owned(), slug.to_owned()))
        })
        .collect()
}

// ============================================================================
// Counts and slugs
// ============================================================================

#[test]
fn the_seed_carries_the_whole_catalogue() {
    let registry = TrainingCatalogueRegistry::new();
    let stats = registry.stats();
    assert_eq!(stats.flavours, 10, "{stats}");
    assert_eq!(stats.skeletons, 12, "{stats}");
    assert_eq!(stats.workouts, 37, "{stats}");
    assert!(stats.selection_rows >= 43, "{stats}");
    assert_eq!(stats.compiled_in_count, SEED_FILE_COUNT, "{stats}");
    assert_eq!(stats.contremaitre_count, 0, "{stats}");

    let workouts: BTreeSet<String> = registry.workouts().into_iter().map(|w| w.slug).collect();
    assert_eq!(workouts, as_set(&WORKOUT_SLUGS));
    let flavours: BTreeSet<String> = registry.flavours().into_iter().map(|f| f.id).collect();
    assert_eq!(flavours, as_set(&FLAVOUR_IDS));
    let skeletons: BTreeSet<String> = registry.skeletons().into_iter().map(|s| s.id).collect();
    assert_eq!(skeletons, as_set(&SKELETON_IDS));
    assert!(
        registry.selection().is_some(),
        "the selection table is seeded"
    );
}

#[test]
fn the_seed_lists_are_sorted() {
    let registry = TrainingCatalogueRegistry::new();
    let workouts: Vec<String> = registry.workouts().into_iter().map(|w| w.slug).collect();
    let mut sorted = workouts.clone();
    sorted.sort();
    assert_eq!(workouts, sorted, "workouts() sorts by slug");
    let flavours: Vec<String> = registry.flavours().into_iter().map(|f| f.id).collect();
    let mut sorted = flavours.clone();
    sorted.sort();
    assert_eq!(flavours, sorted, "flavours() sorts by id");
}

// ============================================================================
// Named values from the knowledge base
// ============================================================================

#[test]
fn vo2max_4x8_carries_seiler_2013() {
    let registry = TrainingCatalogueRegistry::new();
    let workout = registry.workout("vo2max_4x8").expect("vo2max_4x8 seeded");
    assert_eq!(workout.purpose, WorkoutPurpose::Vo2maxLong);
    assert_eq!(workout.params.work_seconds.as_ref().unwrap().default, 480);
    assert!(
        workout.fit.phases.contains(&PhaseKind::Build)
            && workout.fit.phases.contains(&PhaseKind::Peak),
        "4 x 8 fits build and peak: {:?}",
        workout.fit.phases
    );
    assert_eq!(workout.fit.readiness_min, ReadinessLevel::P2);
    assert!(workout.is_compiled_in, "a catalogue workout is read-only");
    for sport in [SportType::Ride, SportType::Run] {
        let anchor = workout
            .params
            .intensity
            .get(&sport)
            .unwrap_or_else(|| panic!("vo2max_4x8 names an anchor for {sport:?}"));
        assert!(
            RelativeIntensity::parse(anchor).is_some(),
            "{sport:?} anchor {anchor:?} is in the intensity grammar"
        );
    }
}

#[test]
fn polarized_classic_carries_the_tid_table() {
    let registry = TrainingCatalogueRegistry::new();
    let flavour = registry.flavour("polarized-classic").expect("seeded");
    let base = &flavour.tid_targets[&PhaseKind::Base];
    assert!(
        (base.z1.min - 0.80).abs() < f32::EPSILON,
        "base z1 min {}",
        base.z1.min
    );
    assert!(
        (base.z1.max - 0.90).abs() < f32::EPSILON,
        "base z1 max {}",
        base.z1.max
    );
    assert_eq!(flavour.hard_sessions_per_week.max, 2);
}

#[test]
fn hvlit_fits_under_four_hours() {
    let registry = TrainingCatalogueRegistry::new();
    let flavour = registry.flavour("hvlit-foundation").expect("seeded");
    assert!(
        flavour.prerequisites.min_hours_per_week < 4.0,
        "hvlit is the under-four-hours flavour, got {}",
        flavour.prerequisites.min_hours_per_week
    );
}

#[test]
fn norwegian_threshold_density_needs_lactate_and_is_not_for_novices() {
    let registry = TrainingCatalogueRegistry::new();
    let flavour = registry
        .flavour("norwegian-threshold-density")
        .expect("seeded");
    assert_eq!(
        flavour.prerequisites.measurement[0],
        vec![Measurement::Lactate]
    );
    assert!(flavour
        .contraindications
        .contains(&Contraindication::NoviceFirstSeason));
}

#[test]
fn norwegian_singles_is_grey_with_a_caveat() {
    let registry = TrainingCatalogueRegistry::new();
    let flavour = registry
        .flavour("norwegian-singles-subthreshold")
        .expect("seeded");
    assert_eq!(flavour.evidence_tier, EvidenceTier::Grey);
    let caveat = flavour
        .caveat
        .as_deref()
        .expect("a grey flavour states its caveat");
    assert!(caveat.contains("no peer-reviewed evidence"), "{caveat}");
}

#[test]
fn taper_days_follow_the_event_table() {
    let registry = TrainingCatalogueRegistry::new();
    let marathon = registry.skeleton("marathon-linear").expect("seeded");
    assert!(marathon.taper.as_ref().unwrap().days.min >= 14);
    let short = registry.skeleton("run-5k-10k").expect("seeded");
    assert!(short.taper.as_ref().unwrap().days.max <= 10);
    let foundation = registry.skeleton("no-race-foundation").expect("seeded");
    assert!(foundation.taper.is_none(), "no race, no taper");
    assert_eq!(foundation.event_classes, vec![EventClass::NoRace]);
}

#[test]
fn no_skeleton_drops_its_taper_or_peak() {
    let registry = TrainingCatalogueRegistry::new();
    for skeleton in registry.skeletons() {
        assert!(
            !skeleton
                .drop_order
                .iter()
                .any(|kind| matches!(kind, PhaseKind::Taper | PhaseKind::Peak)),
            "skeleton '{}' drop_order {:?} names taper or peak",
            skeleton.id,
            skeleton.drop_order
        );
    }
}

// ============================================================================
// Every file on disk, through the kernel
// ============================================================================

#[test]
fn every_pinned_file_passes_the_kernel() {
    let mut parsed = 0;
    for (id, text) in training::FLAVOURS {
        let flavour =
            Flavour::from_yaml(text).unwrap_or_else(|e| panic!("flavours/{id}.yaml: {e}"));
        assert_eq!(flavour.id, *id, "a flavour's id is its file stem");
        parsed += 1;
    }
    for (id, text) in training::SKELETONS {
        let skeleton = SkeletonTemplate::from_yaml(text)
            .unwrap_or_else(|e| panic!("skeletons/{id}.yaml: {e}"));
        assert_eq!(skeleton.id, *id, "a skeleton's id is its file stem");
        parsed += 1;
    }
    for (slug, text) in training::WORKOUTS {
        let workout = WorkoutTemplate::from_toml(text)
            .unwrap_or_else(|e| panic!("workouts/{slug}.toml: {e}"));
        assert_eq!(workout.slug, *slug, "a workout's slug is its file stem");
        parsed += 1;
    }
    let table = SelectionTable::from_yaml(training::SELECTION)
        .unwrap_or_else(|e| panic!("selection.yaml: {e}"));
    assert!(
        table.rows.len() >= 43,
        "{} selection rows",
        table.rows.len()
    );
    parsed += 1;
    assert_eq!(parsed, SEED_FILE_COUNT);
}

#[test]
fn every_evidence_ref_resolves_against_the_pinned_corpus() {
    let keys = evidence_keys();
    assert!(
        keys.len() >= 100,
        "the pinned corpus holds {} propositions",
        keys.len()
    );
    assert!(
        !keys.iter().any(|(_, slug)| slug == "README"),
        "README.md is not a proposition"
    );
    let registry = TrainingCatalogueRegistry::new();
    let exists =
        |category: &str, slug: &str| keys.contains(&(category.to_owned(), slug.to_owned()));
    let unresolved = registry.unresolved_references(&exists);
    assert!(
        unresolved.is_empty(),
        "dangling references:\n{}",
        unresolved
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("\n")
    );
}

// ============================================================================
// The seed mirrors the pinned tables
// ============================================================================

#[test]
fn the_seed_slugs_equal_the_pinned_table_per_kind() {
    let registry = TrainingCatalogueRegistry::new();
    let flavours: BTreeSet<String> = registry.flavours().into_iter().map(|f| f.id).collect();
    assert_eq!(flavours, stems(CatalogueKind::Flavour));
    let skeletons: BTreeSet<String> = registry.skeletons().into_iter().map(|s| s.id).collect();
    assert_eq!(skeletons, stems(CatalogueKind::Skeleton));
    let workouts: BTreeSet<String> = registry.workouts().into_iter().map(|w| w.slug).collect();
    assert_eq!(workouts, stems(CatalogueKind::Workout));
}

#[test]
fn a_seeded_sha_is_the_sha_of_the_pinned_file() {
    let registry = TrainingCatalogueRegistry::new();
    let mut checked = 0;
    for kind in [
        CatalogueKind::Flavour,
        CatalogueKind::Skeleton,
        CatalogueKind::Workout,
    ] {
        for (slug, text) in table(kind) {
            assert_eq!(
                registry.sha256(kind, slug).as_deref(),
                Some(compute_sha256(text.as_bytes()).as_str()),
                "{kind:?}/{slug}"
            );
            checked += 1;
        }
    }
    assert_eq!(
        registry.sha256(CatalogueKind::Selection, SELECTION_SLUG),
        Some(compute_sha256(training::SELECTION.as_bytes()))
    );
    checked += 1;
    assert_eq!(checked, SEED_FILE_COUNT);
    assert!(registry
        .sha256(CatalogueKind::Workout, "no_such_workout")
        .is_none());
}

// ============================================================================
// Overlay and revert
// ============================================================================

#[test]
fn update_overlays_and_remove_reverts_to_the_compiled_in_entry() {
    let registry = TrainingCatalogueRegistry::new();
    let seeded = registry.workout("vo2max_4x8").expect("seeded");
    let mut overlay = seeded.clone();
    overlay.duration_minutes = 70;
    registry.update(
        CatalogueKind::Workout,
        "vo2max_4x8",
        CatalogueItem::Workout(Box::new(overlay.clone())),
        "overlay-sha".to_owned(),
    );
    assert_eq!(registry.workout("vo2max_4x8").unwrap(), overlay);
    assert_eq!(
        registry
            .sha256(CatalogueKind::Workout, "vo2max_4x8")
            .as_deref(),
        Some("overlay-sha")
    );
    let stats = registry.stats();
    assert_eq!(
        (stats.workouts, stats.contremaitre_count),
        (37, 1),
        "{stats}"
    );

    assert!(
        registry.remove(CatalogueKind::Workout, "vo2max_4x8"),
        "the overlay was live"
    );
    assert_eq!(
        registry.workout("vo2max_4x8").unwrap(),
        seeded,
        "reverted to the seed"
    );
    let stats = registry.stats();
    assert_eq!(
        (stats.compiled_in_count, stats.contremaitre_count),
        (SEED_FILE_COUNT, 0),
        "{stats}"
    );
    assert!(
        !registry.remove(CatalogueKind::Workout, "vo2max_4x8"),
        "removing a seed-only slot changes nothing"
    );
}

#[test]
fn a_slug_with_no_seed_is_dropped_on_remove() {
    let registry = TrainingCatalogueRegistry::new();
    let mut extra = registry.workout("endurance").expect("seeded");
    extra.slug = "endurance_hot_fix".to_owned();
    registry.update(
        CatalogueKind::Workout,
        "endurance_hot_fix",
        CatalogueItem::Workout(Box::new(extra)),
        "x".to_owned(),
    );
    assert_eq!(registry.stats().workouts, 38);
    assert!(registry.workout("endurance_hot_fix").is_some());
    assert!(registry.remove(CatalogueKind::Workout, "endurance_hot_fix"));
    assert!(registry.workout("endurance_hot_fix").is_none());
    assert_eq!(registry.stats().workouts, 37);
    assert!(!registry.remove(CatalogueKind::Workout, "endurance_hot_fix"));
}

// ============================================================================
// WorkoutFilter
// ============================================================================

#[test]
fn an_empty_phase_list_matches_every_phase() {
    let registry = TrainingCatalogueRegistry::new();
    let mut any_phase = registry.workout("vo2max_4x8").expect("seeded");
    assert!(
        !any_phase.fit.phases.contains(&PhaseKind::Taper),
        "the seeded 4 x 8 does not fit a taper"
    );
    let taper = WorkoutFilter {
        phase: Some(PhaseKind::Taper),
        ..WorkoutFilter::default()
    };
    assert!(!taper.matches(&any_phase));
    any_phase.fit.phases.clear();
    assert!(
        taper.matches(&any_phase),
        "empty fit.phases means any phase"
    );
    assert!(
        WorkoutFilter::default().matches(&any_phase),
        "no criteria matches everything"
    );
}

#[test]
fn a_variant_sport_matches_and_a_foreign_one_does_not() {
    let registry = TrainingCatalogueRegistry::new();
    let workout = registry.workout("vo2max_4x8").expect("seeded");
    assert_eq!(workout.sport, SportType::Ride);
    assert!(workout.sport_variants.contains(&SportType::Run));
    let run = WorkoutFilter {
        sport: Some(SportType::Run),
        ..WorkoutFilter::default()
    };
    assert!(run.matches(&workout), "a variant sport matches");
    let swim = WorkoutFilter {
        sport: Some(SportType::Swim),
        ..WorkoutFilter::default()
    };
    assert!(!swim.matches(&workout));
    let wrong_purpose = WorkoutFilter {
        purpose: Some(WorkoutPurpose::Recovery),
        sport: Some(SportType::Run),
        phase: None,
    };
    assert!(
        !wrong_purpose.matches(&workout),
        "every stated criterion must hold"
    );
}

#[test]
fn workouts_matching_groups_by_purpose_then_slug() {
    let registry = TrainingCatalogueRegistry::new();
    let vo2 = registry.workouts_matching(&WorkoutFilter {
        purpose: Some(WorkoutPurpose::Vo2maxLong),
        ..WorkoutFilter::default()
    });
    let slugs: Vec<&str> = vo2.iter().map(|w| w.slug.as_str()).collect();
    assert_eq!(
        slugs,
        ["vo2_5x3", "vo2max_4x8", "vo2max_tmax", "vo2max_varied"]
    );

    let taper = registry.workouts_matching(&WorkoutFilter {
        phase: Some(PhaseKind::Taper),
        ..WorkoutFilter::default()
    });
    assert!(taper
        .iter()
        .all(|w| w.fit.phases.contains(&PhaseKind::Taper)));
    assert!(taper.iter().any(|w| w.slug == "race_pace_long"));
    assert!(taper.iter().all(|w| w.slug != "vo2max_4x8"));
    let keys: Vec<(WorkoutPurpose, &str)> =
        taper.iter().map(|w| (w.purpose, w.slug.as_str())).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    assert_eq!(keys, sorted, "sorted by (purpose, slug)");
}

// ============================================================================
// The phase-aware carrier rule
// ============================================================================

#[test]
fn every_flavour_session_has_a_carrier_fitting_its_phase() {
    let registry = TrainingCatalogueRegistry::new();
    for flavour in registry.flavours() {
        for (phase, weights) in &flavour.session_mix {
            for purpose in weights.keys() {
                let filter = WorkoutFilter {
                    purpose: Some(*purpose),
                    phase: Some(*phase),
                    sport: None,
                };
                assert!(
                    !registry.workouts_matching(&filter).is_empty(),
                    "flavour '{}' session_mix.{phase}.{purpose}: no workout with purpose {purpose} fits phase {phase}",
                    flavour.id
                );
            }
        }
    }
}

#[test]
fn every_skeleton_key_session_has_a_carrier_fitting_its_phase_and_sport() {
    let registry = TrainingCatalogueRegistry::new();
    for skeleton in registry.skeletons() {
        let required_sport =
            (skeleton.event_classes == [EventClass::OpenWaterSwim]).then_some(SportType::Swim);
        for (i, phase) in skeleton.phases.iter().enumerate() {
            for (j, purpose) in phase.key_sessions.iter().enumerate() {
                let filter = WorkoutFilter {
                    purpose: Some(*purpose),
                    phase: Some(phase.kind),
                    sport: required_sport.clone(),
                };
                assert!(
                    !registry.workouts_matching(&filter).is_empty(),
                    "skeleton '{}' phases[{i}].key_sessions[{j}]: no workout with purpose {purpose} fits phase {}{}",
                    skeleton.id,
                    phase.kind,
                    required_sport
                        .as_ref()
                        .map_or_else(String::new, |s| format!(" for sport {s:?}"))
                );
            }
        }
    }
}

#[test]
fn the_open_water_skeleton_is_carried_by_swim_templates() {
    let registry = TrainingCatalogueRegistry::new();
    let skeleton = registry.skeleton("open-water-swim").expect("seeded");
    assert_eq!(skeleton.event_classes, vec![EventClass::OpenWaterSwim]);
    let swim_purposes: BTreeSet<WorkoutPurpose> = skeleton
        .phases
        .iter()
        .flat_map(|phase| phase.key_sessions.iter().copied())
        .collect();
    assert!(swim_purposes.contains(&WorkoutPurpose::RaceSpecific));
    for purpose in swim_purposes {
        let carriers = registry.workouts_matching(&WorkoutFilter {
            purpose: Some(purpose),
            phase: None,
            sport: Some(SportType::Swim),
        });
        assert!(
            carriers
                .iter()
                .all(|w| w.sport == SportType::Swim || w.sport_variants.contains(&SportType::Swim)),
            "{purpose}: every carrier lists swim as its sport or a variant"
        );
        assert!(!carriers.is_empty(), "{purpose}: no swim carrier");
    }
}

/// Every citation the catalogue makes, by the kind of file that makes it.
///
/// Measured from the tree: one `evidence_refs` key per flavour, skeleton and
/// workout file and per selection row, 356 references in all.
const CATALOGUE_EVIDENCE_REFS: [(&str, usize); 4] = [
    ("flavour ", 42),
    ("skeleton ", 171),
    ("workout ", 70),
    ("selection table", 73),
];

/// The positive control for [`every_evidence_ref_resolves_against_the_fixtures`].
///
/// That test's assertion is negative — the list of dangling references must be
/// empty — so a checker that always returned nothing would pass it identically
/// and the catalogue's citations would go unpoliced. Deny every proposition and
/// the checker must report all of them, counted per owner kind.
///
/// Counting per kind is what makes this a control rather than a formality.
/// `unresolved_references` walks four independent arms — flavours, skeletons,
/// workouts, the selection table — and an assertion on the total alone, or one
/// that hunts for any single reference that fires, stays green when three of
/// the four go silent. These four numbers name which one did.
#[test]
fn the_reference_check_reports_every_citation_when_none_resolve() {
    let keys = evidence_keys();
    let registry = TrainingCatalogueRegistry::new();

    // The baseline the sibling test pins: with the real fixtures, nothing
    // dangles. Without it the counts below could be met by references that are
    // broken in the shipped catalogue rather than by the denial.
    let resolves =
        |category: &str, slug: &str| keys.contains(&(category.to_owned(), slug.to_owned()));
    assert!(
        registry.unresolved_references(&resolves).is_empty(),
        "the control starts from a catalogue whose references all resolve"
    );

    let denied = registry.unresolved_references(&|_: &str, _: &str| false);
    let counted: usize = CATALOGUE_EVIDENCE_REFS.iter().map(|(_, n)| n).sum();
    assert_eq!(
        denied.len(),
        counted,
        "every citation must be reported when none resolve"
    );
    for (owner_kind, expected) in CATALOGUE_EVIDENCE_REFS {
        let reported = denied
            .iter()
            .filter(|u| u.owner.starts_with(owner_kind))
            .count();
        assert_eq!(
            reported, expected,
            "{owner_kind}citations went unreported — that arm of the walk is silent"
        );
    }
}

/// The long-course pyramidal is the flavour the Ironman scenario picks *because
/// of* its durability block, and until now nothing asserted the block existed.
///
/// `recommend_plan_flavour_tool_test`'s Ironman case is named for it and says
/// so in an assertion message, but the payload carries no modifiers — it emits
/// `id`, `label`, `score` and `reasons` — so the claim could only ever be made
/// here. Deleting `durability_block` from the flavour file left every test in
/// the repository green, which is the state this closes.
#[test]
fn the_long_course_pyramidal_carries_the_durability_block() {
    let registry = TrainingCatalogueRegistry::new();
    let flavour = registry
        .flavour("pyramidal-long-course")
        .expect("seeded from the embedded catalogue");
    assert!(
        flavour.modifiers.contains(&Modifier::DurabilityBlock),
        "a race over six hours is what this flavour exists for: {:?}",
        flavour.modifiers
    );
    assert!(
        flavour.modifiers.contains(&Modifier::FuellingProgression),
        "and fuelling is the other half of that demand: {:?}",
        flavour.modifiers
    );
    // The sibling it is chosen over shares the architecture and not the
    // emphasis — which is the whole reason the Ironman scenario must land on
    // this one rather than on `pyramidal-base`.
    let base = registry.flavour("pyramidal-base").expect("seeded");
    assert_eq!(
        base.family, flavour.family,
        "same architecture, or the scenario is not about the modifiers"
    );
    assert!(
        !base.modifiers.contains(&Modifier::DurabilityBlock),
        "and the plain pyramidal does not carry the block: {:?}",
        base.modifiers
    );
}

/// The ultra-trail pyramidal is the long-course one for a pure runner: the same
/// durability block, and no brick in any phase, which is the reason it exists.
///
/// Its four trail sessions are catalogue workouts rather than a coach
/// package's, so any agent that lays this flavour can prescribe them; each is a
/// run whose purpose the flavour's session mix asks for.
#[test]
fn the_ultra_trail_pyramidal_carries_the_durability_block_and_no_brick() {
    let registry = TrainingCatalogueRegistry::new();
    let flavour = registry
        .flavour("pyramidal-ultra-trail")
        .expect("seeded from the embedded catalogue");
    assert!(
        flavour.modifiers.contains(&Modifier::DurabilityBlock),
        "a mountain ultra is a durability event: {:?}",
        flavour.modifiers
    );
    for (phase, weights) in &flavour.session_mix {
        assert!(
            !weights.contains_key(&WorkoutPurpose::Brick),
            "session_mix.{phase} names a brick a runner has no bike for: {weights:?}"
        );
    }
    // The sibling it replaces for runners does carry one, or the assertion
    // above pins nothing.
    let long_course = registry.flavour("pyramidal-long-course").expect("seeded");
    assert!(
        long_course
            .session_mix
            .values()
            .any(|weights| weights.contains_key(&WorkoutPurpose::Brick)),
        "pyramidal-long-course builds a brick: {:?}",
        long_course.session_mix
    );

    for slug in [
        "back_to_back_long",
        "downhill_repeats",
        "trail_race_simulation",
        "uphill_power_hike",
    ] {
        let workout = registry
            .workout(slug)
            .unwrap_or_else(|| panic!("{slug} is a catalogue workout"));
        assert!(
            workout.is_compiled_in,
            "{slug} is read-only catalogue content"
        );
        assert_eq!(workout.sport, SportType::Run, "{slug} is a run");
        assert!(
            flavour
                .session_mix
                .values()
                .any(|weights| weights.contains_key(&workout.purpose)),
            "{slug}'s purpose {} is one the trail flavour asks for",
            workout.purpose
        );
    }
}

/// The Ironman skeleton pins its base phase to pyramidal, the way the 5 km
/// skeleton pins its build to polarized.
///
/// The tool test asserts the 5 km case and not this one; the asymmetry was an
/// omission rather than a decision, and "gets a pyramidal base" is half of
/// what that scenario's name claims.
#[test]
fn the_ironman_skeleton_pins_its_base_to_pyramidal() {
    let registry = TrainingCatalogueRegistry::new();
    let skeleton = registry.skeleton("ironman").expect("seeded");
    let base = skeleton
        .phases
        .iter()
        .find(|p| p.kind == PhaseKind::Base)
        .expect("an ironman season has a base phase");
    assert_eq!(
        base.flavour_override,
        Some(FlavourFamily::Pyramidal),
        "the long-course base is pyramidal whatever the season flavour: {:?}",
        base.flavour_override
    );
}
