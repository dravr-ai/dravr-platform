// ABOUTME: Pins the seeded training catalogue — counts, slugs, the KB numbers of named files, every file through the kernel
// ABOUTME: Cross-file rules: no dangling evidence ref, every named session has a carrier fitting its phase, the embedded table mirrors disk
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The registry is seeded from `training_catalogue/` at build time. A file
//! that fails to parse is logged and left out rather than failing the boot,
//! so the exact counts here are what stops a broken file from reaching a
//! release: 8 flavours, 12 skeletons, 33 workouts, one selection table.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use pierre_contremaitre::manifest::compute_sha256;
use pierre_contremaitre::training_catalogue::{
    CatalogueItem, CatalogueKind, TrainingCatalogueRegistry, SELECTION_SLUG,
};
use pierre_core::models::periodization::{
    evidence_ref_parts, Contraindication, EventClass, EvidenceTier, Flavour, Measurement,
    PhaseKind, ReadinessLevel, RelativeIntensity, SelectionTable, SkeletonTemplate, WorkoutFilter,
    WorkoutPurpose, WorkoutTemplate,
};
use pierre_core::models::SportType;

/// The 33 workout slugs of the Phase 1 bank (spec §9).
const WORKOUT_SLUGS: [&str; 33] = [
    "billat_30_30",
    "brick",
    "complex_training",
    "core_mobility",
    "double_threshold_day",
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
    "vo2_5x3",
    "vo2max_30_15",
    "vo2max_4x8",
    "vo2max_hills",
    "vo2max_tmax",
    "vo2max_varied",
];

/// The eight flavour ids (spec §3.3).
const FLAVOUR_IDS: [&str; 8] = [
    "hvlit-foundation",
    "norwegian-singles-subthreshold",
    "norwegian-threshold-density",
    "polarized-classic",
    "pyramidal-base",
    "pyramidal-to-polarized",
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

/// Files the seed carries: 8 + 12 + 33 + the selection table.
const SEED_FILE_COUNT: usize = 54;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn catalogue_dir() -> PathBuf {
    repo_root().join("training_catalogue")
}

fn evidence_dir() -> PathBuf {
    repo_root().join("crates/pierre-evals/fixtures/sports_science")
}

/// The file stems under `dir` carrying `extension`, sorted.
fn stems(dir: &Path, extension: &str) -> BTreeSet<String> {
    fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == extension))
        .map(|path| path.file_stem().unwrap().to_string_lossy().into_owned())
        .collect()
}

fn as_set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|id| (*id).to_owned()).collect()
}

/// Every `(category, slug)` the evidence fixtures answer for, keyed the way
/// `evidence_ref_parts` splits a ref — so `README.md` never counts.
fn evidence_keys() -> HashSet<(String, String)> {
    let mut keys = HashSet::new();
    for category in fs::read_dir(evidence_dir()).unwrap() {
        let category = category.unwrap().path();
        if !category.is_dir() {
            continue;
        }
        let category_name = category.file_name().unwrap().to_string_lossy().into_owned();
        for file in fs::read_dir(&category).unwrap() {
            let file = file.unwrap().file_name().to_string_lossy().into_owned();
            let path = format!("evidence/sports_science/{category_name}/{file}");
            if let Some((category, slug)) = evidence_ref_parts(&path) {
                keys.insert((category.to_owned(), slug.to_owned()));
            }
        }
    }
    keys
}

// ============================================================================
// Counts and slugs
// ============================================================================

#[test]
fn the_seed_carries_the_whole_catalogue() {
    let registry = TrainingCatalogueRegistry::new();
    let stats = registry.stats();
    assert_eq!(stats.flavours, 8, "{stats}");
    assert_eq!(stats.skeletons, 12, "{stats}");
    assert_eq!(stats.workouts, 33, "{stats}");
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
fn the_tree_holds_only_the_four_shapes() {
    let top: BTreeSet<String> = fs::read_dir(catalogue_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        top,
        as_set(&["flavours", "selection.yaml", "skeletons", "workouts"])
    );
    for (dir, extension) in [
        ("flavours", "yaml"),
        ("skeletons", "yaml"),
        ("workouts", "toml"),
    ] {
        let stray: Vec<String> = fs::read_dir(catalogue_dir().join(dir))
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_none_or(|ext| ext != extension))
            .map(|p| p.display().to_string())
            .collect();
        assert!(
            stray.is_empty(),
            "{dir}/ holds files that are not .{extension}: {stray:?}"
        );
    }
}

#[test]
fn every_file_on_disk_passes_the_kernel() {
    let dir = catalogue_dir();
    let mut parsed = 0;
    for id in stems(&dir.join("flavours"), "yaml") {
        let text = fs::read_to_string(dir.join("flavours").join(format!("{id}.yaml"))).unwrap();
        let flavour =
            Flavour::from_yaml(&text).unwrap_or_else(|e| panic!("flavours/{id}.yaml: {e}"));
        assert_eq!(flavour.id, id, "a flavour's id is its file stem");
        parsed += 1;
    }
    for id in stems(&dir.join("skeletons"), "yaml") {
        let text = fs::read_to_string(dir.join("skeletons").join(format!("{id}.yaml"))).unwrap();
        let skeleton = SkeletonTemplate::from_yaml(&text)
            .unwrap_or_else(|e| panic!("skeletons/{id}.yaml: {e}"));
        assert_eq!(skeleton.id, id, "a skeleton's id is its file stem");
        parsed += 1;
    }
    for slug in stems(&dir.join("workouts"), "toml") {
        let text = fs::read_to_string(dir.join("workouts").join(format!("{slug}.toml"))).unwrap();
        let workout = WorkoutTemplate::from_toml(&text)
            .unwrap_or_else(|e| panic!("workouts/{slug}.toml: {e}"));
        assert_eq!(workout.slug, slug, "a workout's slug is its file stem");
        parsed += 1;
    }
    let selection = fs::read_to_string(dir.join("selection.yaml")).unwrap();
    let table =
        SelectionTable::from_yaml(&selection).unwrap_or_else(|e| panic!("selection.yaml: {e}"));
    assert!(
        table.rows.len() >= 43,
        "{} selection rows",
        table.rows.len()
    );
    parsed += 1;
    assert_eq!(parsed, SEED_FILE_COUNT);
}

#[test]
fn every_evidence_ref_resolves_against_the_fixtures() {
    let keys = evidence_keys();
    assert!(
        keys.len() >= 100,
        "the fixture walk found {} propositions",
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
// The embedded table and the seed mirror the directory
// ============================================================================

#[test]
fn the_embedded_table_lists_exactly_the_files_on_disk() {
    let table = fs::read_to_string(
        repo_root().join("crates/pierre-contremaitre/src/training_catalogue_embedded.rs"),
    )
    .unwrap();
    let marker = "include_str!(\"../../../training_catalogue/";
    let embedded: BTreeSet<String> = table
        .lines()
        .filter_map(|line| line.split_once(marker))
        .map(|(_, rest)| rest.split('"').next().unwrap().to_owned())
        .collect();

    let dir = catalogue_dir();
    let mut on_disk = BTreeSet::new();
    for id in stems(&dir.join("flavours"), "yaml") {
        on_disk.insert(format!("flavours/{id}.yaml"));
    }
    for id in stems(&dir.join("skeletons"), "yaml") {
        on_disk.insert(format!("skeletons/{id}.yaml"));
    }
    for slug in stems(&dir.join("workouts"), "toml") {
        on_disk.insert(format!("workouts/{slug}.toml"));
    }
    on_disk.insert("selection.yaml".to_owned());
    assert_eq!(
        embedded, on_disk,
        "regenerate with scripts/ci/sync-contremaitre-fallback.sh"
    );
    assert_eq!(embedded.len(), SEED_FILE_COUNT);
}

#[test]
fn the_seed_slugs_equal_the_directory_listing_per_kind() {
    let registry = TrainingCatalogueRegistry::new();
    let dir = catalogue_dir();
    let flavours: BTreeSet<String> = registry.flavours().into_iter().map(|f| f.id).collect();
    assert_eq!(flavours, stems(&dir.join("flavours"), "yaml"));
    let skeletons: BTreeSet<String> = registry.skeletons().into_iter().map(|s| s.id).collect();
    assert_eq!(skeletons, stems(&dir.join("skeletons"), "yaml"));
    let workouts: BTreeSet<String> = registry.workouts().into_iter().map(|w| w.slug).collect();
    assert_eq!(workouts, stems(&dir.join("workouts"), "toml"));
}

#[test]
fn a_seeded_sha_is_the_sha_of_the_file_on_disk() {
    let registry = TrainingCatalogueRegistry::new();
    let dir = catalogue_dir();
    let mut checked = 0;
    for (kind, sub, extension) in [
        (CatalogueKind::Flavour, "flavours", "yaml"),
        (CatalogueKind::Skeleton, "skeletons", "yaml"),
        (CatalogueKind::Workout, "workouts", "toml"),
    ] {
        for slug in stems(&dir.join(sub), extension) {
            let bytes = fs::read(dir.join(sub).join(format!("{slug}.{extension}"))).unwrap();
            assert_eq!(
                registry.sha256(kind, &slug).as_deref(),
                Some(compute_sha256(&bytes).as_str()),
                "{sub}/{slug}.{extension}"
            );
            checked += 1;
        }
    }
    let selection = fs::read(dir.join("selection.yaml")).unwrap();
    assert_eq!(
        registry.sha256(CatalogueKind::Selection, SELECTION_SLUG),
        Some(compute_sha256(&selection))
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
        (33, 1),
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
    assert_eq!(registry.stats().workouts, 34);
    assert!(registry.workout("endurance_hot_fix").is_some());
    assert!(registry.remove(CatalogueKind::Workout, "endurance_hot_fix"));
    assert!(registry.workout("endurance_hot_fix").is_none());
    assert_eq!(registry.stats().workouts, 33);
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
