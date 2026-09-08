// ABOUTME: Reads a coach package's training artefacts — flavour.yaml, skeleton.yaml, workouts/*.toml — beside the prompt
// ABOUTME: Each file goes through the dravr-cageux kernel, which validates it; the result is what the seeder stores per coach
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Coach package artefacts
//!
//! A coach directory — `<category>/<slug>/` — carries its prompt as
//! `<locale>.md` files. Beside them a coach may ship the second authorship
//! tier of the training catalogue:
//!
//! ```text
//! <category>/<slug>/
//!   en.md              the prompt (required)
//!   flavour.yaml       the house flavour — one, its `id` is what the rule pins
//!   skeleton.yaml      the coach's season skeleton — one
//!   workouts/
//!     <slug>.toml      workout templates, the file stem equal to `slug`
//! ```
//!
//! Every file is parsed by the same kernel the Dravr catalogue is, so a
//! package obeys every invariant `training/` does. A file the kernel refuses
//! is an error naming the file; the seeder logs it and skips the whole
//! package rather than storing half of one.

use std::fs;
use std::path::{Path, PathBuf};

use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{ArtefactKind, PackageArtefact};

/// The house flavour's filename inside a coach directory.
pub const FLAVOUR_FILE: &str = "flavour.yaml";
/// The skeleton's filename inside a coach directory.
pub const SKELETON_FILE: &str = "skeleton.yaml";
/// The directory of workout templates inside a coach directory.
pub const WORKOUTS_DIR: &str = "workouts";

/// Read every package artefact a coach directory carries.
///
/// Returns an empty list for a directory with only prompt files — most
/// coaches carry no package — and the artefacts ordered flavour, skeleton,
/// then workouts by file name, so two scans of one checkout list them the
/// same way.
///
/// # Errors
///
/// Returns [`ErrorCode::StorageError`] when a file cannot be read and
/// [`ErrorCode::InvalidFormat`] when the kernel refuses one or a workout
/// file's stem differs from the `slug` it declares — the same rule the
/// catalogue's `training/workouts/` holds to, so a template is found under
/// the name its file carries.
pub fn read_package_artefacts(coach_dir: &Path) -> AppResult<Vec<PackageArtefact>> {
    let mut out = Vec::new();
    let flavour = coach_dir.join(FLAVOUR_FILE);
    if flavour.is_file() {
        out.push(read_artefact(ArtefactKind::Flavour, &flavour)?);
    }
    let skeleton = coach_dir.join(SKELETON_FILE);
    if skeleton.is_file() {
        out.push(read_artefact(ArtefactKind::Skeleton, &skeleton)?);
    }
    for path in workout_files(&coach_dir.join(WORKOUTS_DIR))? {
        let artefact = read_artefact(ArtefactKind::Workout, &path)?;
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if stem != artefact.slug {
            return Err(AppError::new(
                ErrorCode::InvalidFormat,
                format!(
                    "workout template {} declares slug '{}' but its file is named '{stem}'",
                    path.display(),
                    artefact.slug
                ),
            ));
        }
        out.push(artefact);
    }
    Ok(out)
}

/// `true` when the directory carries any package file — what the seeder asks
/// before deciding whether a coach with no rows needs a write at all.
#[must_use]
pub fn has_package_files(coach_dir: &Path) -> bool {
    coach_dir.join(FLAVOUR_FILE).is_file()
        || coach_dir.join(SKELETON_FILE).is_file()
        || coach_dir.join(WORKOUTS_DIR).is_dir()
}

fn read_artefact(kind: ArtefactKind, path: &Path) -> AppResult<PackageArtefact> {
    let content = fs::read_to_string(path).map_err(|e| {
        AppError::new(
            ErrorCode::StorageError,
            format!("Failed to read package file {}: {e}", path.display()),
        )
    })?;
    PackageArtefact::parse(kind, &content).map_err(|e| {
        AppError::new(
            ErrorCode::InvalidFormat,
            format!("package {kind} {} rejected: {e}", path.display()),
        )
    })
}

/// The `*.toml` files under `dir`, sorted by name; empty when `dir` is absent.
fn workout_files(dir: &Path) -> AppResult<Vec<PathBuf>> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let entries = fs::read_dir(dir).map_err(|e| {
        AppError::new(
            ErrorCode::StorageError,
            format!("Failed to read {}: {e}", dir.display()),
        )
    })?;
    let mut files: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|ext| ext == "toml"))
        .collect();
    files.sort();
    Ok(files)
}
