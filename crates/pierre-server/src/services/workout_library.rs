// ABOUTME: Endurance Phase 5 workout library — loads compiled-in cornerstone TOML templates + persisted user-authored rows
// ABOUTME: Produces WorkoutTemplate instances that the prescribe path serialises and pushes to providers
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;
use std::sync::OnceLock;

use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::WorkoutTemplate;

const TEMPLATE_LONG_RUN_Z2: &[u8] =
    include_bytes!("../../../../workout_templates/long_run_z2.toml");
const TEMPLATE_THRESHOLD_4X8: &[u8] =
    include_bytes!("../../../../workout_templates/threshold_4x8.toml");
const TEMPLATE_VO2_5X3: &[u8] = include_bytes!("../../../../workout_templates/vo2_5x3.toml");
const TEMPLATE_RECOVERY_30MIN: &[u8] =
    include_bytes!("../../../../workout_templates/recovery_30min.toml");
const TEMPLATE_TEMPO_PROGRESSION: &[u8] =
    include_bytes!("../../../../workout_templates/tempo_progression.toml");
const TEMPLATE_SWEET_SPOT_2X20: &[u8] =
    include_bytes!("../../../../workout_templates/sweet_spot_2x20.toml");

/// Slugs of the six Endurance cornerstone templates compiled into the binary.
pub const CORNERSTONE_SLUGS: &[&str] = &[
    "long_run_z2",
    "threshold_4x8",
    "vo2_5x3",
    "recovery_30min",
    "tempo_progression",
    "sweet_spot_2x20",
];

static COMPILED_LIBRARY: OnceLock<HashMap<&'static str, WorkoutTemplate>> = OnceLock::new();

fn compiled_library() -> &'static HashMap<&'static str, WorkoutTemplate> {
    COMPILED_LIBRARY.get_or_init(|| {
        let entries: &[(&'static str, &[u8])] = &[
            ("long_run_z2", TEMPLATE_LONG_RUN_Z2),
            ("threshold_4x8", TEMPLATE_THRESHOLD_4X8),
            ("vo2_5x3", TEMPLATE_VO2_5X3),
            ("recovery_30min", TEMPLATE_RECOVERY_30MIN),
            ("tempo_progression", TEMPLATE_TEMPO_PROGRESSION),
            ("sweet_spot_2x20", TEMPLATE_SWEET_SPOT_2X20),
        ];
        let mut map = HashMap::with_capacity(entries.len());
        for (slug, bytes) in entries {
            match WorkoutTemplate::from_toml(bytes) {
                Ok(template) => {
                    map.insert(*slug, template);
                }
                Err(err) => {
                    tracing::error!(
                        slug = %slug,
                        error = %err,
                        "Endurance cornerstone template failed to parse — compiled-in TOML is malformed"
                    );
                }
            }
        }
        map
    })
}

/// List the six compiled-in cornerstone workout templates.
#[must_use]
pub fn cornerstone_templates() -> Vec<WorkoutTemplate> {
    let lib = compiled_library();
    CORNERSTONE_SLUGS
        .iter()
        .filter_map(|slug| lib.get(*slug).cloned())
        .collect()
}

/// Look up a compiled-in template by slug.
#[must_use]
pub fn cornerstone_by_slug(slug: &str) -> Option<WorkoutTemplate> {
    compiled_library().get(slug).cloned()
}

/// Same as [`cornerstone_by_slug`] but returns an [`AppError::not_found`]
/// when the slug is unknown — convenient for the prescribe path.
///
/// # Errors
///
/// Returns [`AppError`] when no compiled-in template matches `slug`.
pub fn require_cornerstone(slug: &str) -> AppResult<WorkoutTemplate> {
    cornerstone_by_slug(slug).ok_or_else(|| {
        AppError::not_found(format!(
            "no Endurance cornerstone workout template with slug '{slug}'"
        ))
    })
}
