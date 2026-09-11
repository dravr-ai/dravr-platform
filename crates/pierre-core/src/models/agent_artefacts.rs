// ABOUTME: An agent package's training artefacts — the flavour, skeleton and workout files shipped beside an agent's prompt
// ABOUTME: Stored as the source text the kernel validated, keyed by (agent, kind, slug); parsed back into kernel types on read
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Agent package artefacts
//!
//! The second authorship tier of the training catalogue. The first is the
//! Dravr catalogue in contremaitre's `training/` tree, shared by every agent;
//! the third is an athlete's own `workout_templates` row. Between them sits
//! the agent package: a `flavour.yaml`, a `skeleton.yaml` and `workouts/*.toml`
//! laid beside the agent's `<locale>.md`, parsed by the same kernel the
//! catalogue is, and stored on the agent row they belong to.
//!
//! A package's flavour is its *house flavour*: the one the selection rule
//! pins for that agent's athletes when they can run it, and the one that
//! overrides a catalogue flavour of the same id. Resolution is by slug,
//! package over catalogue over compiled-in, and only an agent whose store
//! listing is published lends its package to a plan — the `PublishStatus`
//! gate is what makes a package a reviewed artefact rather than a file.
//!
//! The row keeps the file's text rather than a decomposed shape so the
//! kernel stays the single parser: what the seeder validated is exactly what
//! the resolver re-parses, and a kernel that grows a field needs no
//! migration here.

use chrono::{DateTime, Utc};
use dravr_cageux::periodization::{CatalogueError, Flavour, SkeletonTemplate, WorkoutTemplate};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;

/// The three file shapes a package may carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtefactKind {
    /// `flavour.yaml` — the house flavour.
    Flavour,
    /// `skeleton.yaml` — the agent's season skeleton.
    Skeleton,
    /// `workouts/<slug>.toml` — a workout template.
    Workout,
}

impl ArtefactKind {
    /// The database and wire spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Flavour => "flavour",
            Self::Skeleton => "skeleton",
            Self::Workout => "workout",
        }
    }

    /// Parse the database spelling; anything else is `None` so a row a
    /// newer binary wrote is skipped, never misread as another kind.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "flavour" => Some(Self::Flavour),
            "skeleton" => Some(Self::Skeleton),
            "workout" => Some(Self::Workout),
            _ => None,
        }
    }
}

impl fmt::Display for ArtefactKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One artefact as the package ships it.
///
/// The kind, the slug the kernel read out of the text (a flavour's or
/// skeleton's `id`, a workout's `slug`), and the text itself — what the
/// parser produces and the seeder stores.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageArtefact {
    /// Which shape the file is.
    pub kind: ArtefactKind,
    /// The item's own id or slug, the key it resolves under.
    pub slug: String,
    /// The file text, exactly as the kernel validated it.
    pub content: String,
    /// SHA-256 hex digest of `content`.
    pub sha256: String,
}

impl PackageArtefact {
    /// Parse `content` as `kind` through the kernel and key the artefact by
    /// the id or slug the text declares.
    ///
    /// # Errors
    ///
    /// Returns the kernel's [`CatalogueError`] when the text is not a
    /// well-formed document of that shape or breaks one of its invariants.
    pub fn parse(kind: ArtefactKind, content: &str) -> Result<Self, CatalogueError> {
        let slug = match ParsedArtefact::parse(kind, content)? {
            ParsedArtefact::Flavour(f) => f.id,
            ParsedArtefact::Skeleton(s) => s.id,
            ParsedArtefact::Workout(w) => w.slug,
        };
        Ok(Self {
            kind,
            slug,
            content: content.to_owned(),
            sha256: sha256_hex(content),
        })
    }
}

/// A stored artefact: [`PackageArtefact`] plus the row it lives in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentArtefact {
    /// Row id.
    pub id: String,
    /// The agent row the package belongs to.
    pub agent_id: String,
    /// The agent's tenant, copied so every query carries it.
    pub tenant_id: String,
    /// Which shape the file is.
    pub kind: ArtefactKind,
    /// The item's own id or slug.
    pub slug: String,
    /// The file text.
    pub content: String,
    /// SHA-256 hex digest of `content`.
    pub sha256: String,
    /// When the row was first written.
    pub created_at: DateTime<Utc>,
    /// When the row was last rewritten.
    pub updated_at: DateTime<Utc>,
}

impl AgentArtefact {
    /// Re-parse the stored text through the kernel.
    ///
    /// # Errors
    ///
    /// Returns the kernel's [`CatalogueError`] — which cannot happen for a
    /// row the seeder wrote from a text the same kernel accepted, but can for
    /// a row an older kernel accepted and a stricter one now refuses; the
    /// resolver logs and skips such a row rather than dropping the package.
    pub fn parse(&self) -> Result<ParsedArtefact, CatalogueError> {
        ParsedArtefact::parse(self.kind, &self.content)
    }
}

/// An artefact's text parsed into the kernel type of its kind.
#[derive(Debug, Clone, PartialEq)]
pub enum ParsedArtefact {
    /// A training-intensity-distribution model.
    Flavour(Box<Flavour>),
    /// A season skeleton.
    Skeleton(Box<SkeletonTemplate>),
    /// A workout template.
    Workout(Box<WorkoutTemplate>),
}

impl ParsedArtefact {
    /// Parse `content` as `kind`; each kernel parser validates before it
    /// hands the value back.
    ///
    /// # Errors
    ///
    /// Returns the kernel's [`CatalogueError`].
    pub fn parse(kind: ArtefactKind, content: &str) -> Result<Self, CatalogueError> {
        Ok(match kind {
            ArtefactKind::Flavour => Self::Flavour(Box::new(Flavour::from_yaml(content)?)),
            ArtefactKind::Skeleton => {
                Self::Skeleton(Box::new(SkeletonTemplate::from_yaml(content)?))
            }
            ArtefactKind::Workout => Self::Workout(Box::new(WorkoutTemplate::from_toml(content)?)),
        })
    }

    /// The evidence paths the artefact cites.
    #[must_use]
    pub fn evidence_refs(&self) -> &[String] {
        match self {
            Self::Flavour(f) => &f.evidence_refs,
            Self::Skeleton(s) => &s.evidence_refs,
            Self::Workout(w) => &w.evidence_refs,
        }
    }
}

/// SHA-256 of a text, lowercase hex — the digest every artefact row carries.
#[must_use]
pub fn sha256_hex(content: &str) -> String {
    let digest = Sha256::digest(content.as_bytes());
    format!("{digest:x}")
}
