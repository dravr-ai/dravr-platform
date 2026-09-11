// ABOUTME: Resolves the training catalogue through an agent package — package over catalogue over compiled-in, by slug
// ABOUTME: Loads the package the plan's agent carries when its listing is published, and answers every catalogue read with provenance
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Package-over-catalogue resolution
//!
//! Every place that reads the training catalogue for a plan — the flavour a
//! save names, the template slugs its days name, the compliance rail's
//! re-read, the prompt's template list, the selection rule — reads through
//! [`PackagedCatalogue`], a view over the process-wide
//! [`TrainingCatalogueRegistry`] with the agent's package laid on top. A slug
//! resolves in the package first, then in the catalogue (contremaitre's
//! overlay over the compiled-in mirror, which the registry already orders),
//! and the answer says which tier it came from so the saved day can record
//! it.
//!
//! [`load_agent_package`] is the gate. A package is lent to a plan only when
//! the agent that carries it — the origin, when the plan names an installed
//! copy — has a **published** store listing. A draft or rejected package
//! resolves nothing, so the review the plan calls for is the review the
//! `PublishStatus` transitions already run; the seeder publishes catalogue
//! agents straight away, which is the same rule with the review done in
//! contremaitre.
//!
//! Resolution is per call from the database rather than cached on the
//! registry: a package changes on a seed or an approve/unpublish, and a
//! read that goes to the rows sees every instance's write at once.

use pierre_contremaitre::{EvidenceRegistry, TrainingCatalogueRegistry};
use pierre_core::errors::AppResult;
use pierre_core::models::periodization::{
    Flavour, SelectionTable, SkeletonTemplate, UnresolvedReference, WorkoutFilter, WorkoutTemplate,
};
use pierre_core::models::{Agent, AgentArtefact, ArtefactKind, ParsedArtefact, TenantId};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::TemplateSource;
use serde::Serialize;
use tracing::warn;
use uuid::Uuid;

/// An agent's package, parsed: what the resolver lays over the catalogue.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AgentPackage {
    /// The agent row the artefacts belong to — the origin, never a copy.
    pub agent_id: String,
    /// The house flavour, when the package ships one.
    pub flavour: Option<Flavour>,
    /// The agent's season skeleton, when the package ships one.
    pub skeleton: Option<SkeletonTemplate>,
    /// The package's workout templates, in `(kind, slug)` row order.
    pub workouts: Vec<WorkoutTemplate>,
}

impl AgentPackage {
    /// Parse stored rows into a package.
    ///
    /// A row the kernel now refuses — accepted by an older kernel, refused by
    /// a stricter one — is logged and skipped, so one stale artefact never
    /// hides the rest of the package.
    #[must_use]
    pub fn from_rows(agent_id: &str, rows: &[AgentArtefact]) -> Self {
        let mut package = Self {
            agent_id: agent_id.to_owned(),
            ..Self::default()
        };
        for row in rows {
            match row.parse() {
                Ok(ParsedArtefact::Flavour(f)) => package.flavour = Some(*f),
                Ok(ParsedArtefact::Skeleton(s)) => package.skeleton = Some(*s),
                Ok(ParsedArtefact::Workout(w)) => package.workouts.push(*w),
                Err(e) => warn!(
                    agent_id,
                    kind = %row.kind,
                    slug = %row.slug,
                    error = %e,
                    "stored package artefact no longer parses; skipped"
                ),
            }
        }
        package
    }

    /// `true` when the package carries nothing.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.flavour.is_none() && self.skeleton.is_none() && self.workouts.is_empty()
    }

    /// The id of the house flavour — what the selection rule pins.
    #[must_use]
    pub fn house_flavour(&self) -> Option<&str> {
        self.flavour.as_ref().map(|f| f.id.as_str())
    }

    /// Every artefact's cross-file references that nothing answers: an
    /// `evidence_refs` path with no proposition behind it, and a purpose a
    /// flavour or skeleton asks for that neither the package's workouts nor
    /// the catalogue's carry. What the review surface shows.
    ///
    /// Sorted by `(owner, key, reference)` like the registry's own report.
    pub fn unresolved_references(
        &self,
        catalogue: &TrainingCatalogueRegistry,
        evidence_exists: &dyn Fn(&str, &str) -> bool,
    ) -> Vec<UnresolvedReference> {
        let carried = |filter: WorkoutFilter| {
            self.workouts.iter().any(|w| filter.matches(w))
                || !catalogue.workouts_matching(&filter).is_empty()
        };
        let mut out = Vec::new();
        if let Some(flavour) = &self.flavour {
            out.extend(flavour.unresolved_references(evidence_exists));
            out.extend(flavour.unresolved_purposes(&|phase, purpose| {
                carried(WorkoutFilter {
                    purpose: Some(purpose),
                    phase,
                    sport: None,
                })
            }));
        }
        if let Some(skeleton) = &self.skeleton {
            out.extend(skeleton.unresolved_references(evidence_exists));
            out.extend(skeleton.unresolved_purposes(&|phase, purpose, sport| {
                carried(WorkoutFilter {
                    purpose: Some(purpose),
                    phase,
                    sport: sport.cloned(),
                })
            }));
        }
        for workout in &self.workouts {
            out.extend(workout.unresolved_references(evidence_exists));
        }
        out.sort_by(|a, b| {
            a.owner
                .cmp(&b.owner)
                .then_with(|| a.key.cmp(&b.key))
                .then_with(|| a.reference.cmp(&b.reference))
        });
        out
    }
}

/// Which tier answered a catalogue read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CatalogueTier {
    /// The agent package.
    Package,
    /// The Dravr catalogue — contremaitre's overlay or the compiled-in mirror.
    Catalogue,
}

impl CatalogueTier {
    /// The provenance a saved day records for a template found in this tier.
    #[must_use]
    pub const fn template_source(self) -> TemplateSource {
        match self {
            Self::Package => TemplateSource::Package,
            Self::Catalogue => TemplateSource::Catalogue,
        }
    }
}

/// The catalogue as one agent's athletes see it.
///
/// The package first, then the registry. Built per call by
/// [`PackagedCatalogue::new`]; a plan with no agent, or an agent with no
/// published package, reads the registry alone.
pub struct PackagedCatalogue<'a> {
    catalogue: &'a TrainingCatalogueRegistry,
    package: Option<AgentPackage>,
}

impl<'a> PackagedCatalogue<'a> {
    /// A view over `catalogue` with `package` laid on top.
    #[must_use]
    pub const fn new(
        catalogue: &'a TrainingCatalogueRegistry,
        package: Option<AgentPackage>,
    ) -> Self {
        Self { catalogue, package }
    }

    /// The registry alone — a read with no agent in hand.
    #[must_use]
    pub const fn catalogue_only(catalogue: &'a TrainingCatalogueRegistry) -> Self {
        Self::new(catalogue, None)
    }

    /// The package this view lays over the catalogue, when it has one.
    #[must_use]
    pub const fn package(&self) -> Option<&AgentPackage> {
        self.package.as_ref()
    }

    /// The house flavour the package pins, when it has one.
    #[must_use]
    pub fn house_flavour(&self) -> Option<&str> {
        self.package.as_ref().and_then(AgentPackage::house_flavour)
    }

    /// The flavour with this id: the package's when its house flavour has
    /// that id, else the catalogue's.
    #[must_use]
    pub fn flavour(&self, id: &str) -> Option<(Flavour, CatalogueTier)> {
        if let Some(f) = self.package_flavour(id) {
            return Some((f.clone(), CatalogueTier::Package));
        }
        self.catalogue
            .flavour(id)
            .map(|f| (f, CatalogueTier::Catalogue))
    }

    /// Every flavour, the house flavour replacing a catalogue flavour of the
    /// same id and otherwise added, sorted by id.
    #[must_use]
    pub fn flavours(&self) -> Vec<Flavour> {
        let mut out = self.catalogue.flavours();
        if let Some(house) = self.package.as_ref().and_then(|p| p.flavour.as_ref()) {
            out.retain(|f| f.id != house.id);
            out.push(house.clone());
            out.sort_by(|a, b| a.id.cmp(&b.id));
        }
        out
    }

    /// The skeleton with this id: the package's when it carries that id, else
    /// the catalogue's.
    #[must_use]
    pub fn skeleton(&self, id: &str) -> Option<SkeletonTemplate> {
        if let Some(own) = self
            .package
            .as_ref()
            .and_then(|p| p.skeleton.as_ref())
            .filter(|s| s.id == id)
        {
            return Some(own.clone());
        }
        self.catalogue.skeleton(id)
    }

    /// Every skeleton, the package's replacing a catalogue skeleton of the
    /// same id and otherwise added, sorted by id.
    #[must_use]
    pub fn skeletons(&self) -> Vec<SkeletonTemplate> {
        let mut out = self.catalogue.skeletons();
        if let Some(own) = self.package.as_ref().and_then(|p| p.skeleton.as_ref()) {
            out.retain(|s| s.id != own.id);
            out.push(own.clone());
            out.sort_by(|a, b| a.id.cmp(&b.id));
        }
        out
    }

    /// The selection table — the catalogue's; a package carries none.
    #[must_use]
    pub fn selection(&self) -> Option<SelectionTable> {
        self.catalogue.selection()
    }

    /// The template with this slug: the package's, else the catalogue's.
    #[must_use]
    pub fn workout(&self, slug: &str) -> Option<(WorkoutTemplate, CatalogueTier)> {
        if let Some(w) = self.package_workout(slug) {
            return Some((w.clone(), CatalogueTier::Package));
        }
        self.catalogue
            .workout(slug)
            .map(|w| (w, CatalogueTier::Catalogue))
    }

    /// Every template the filter admits, package templates first (a package
    /// template shadows a catalogue template of the same slug), then the
    /// catalogue's in its own `(purpose, slug)` order.
    #[must_use]
    pub fn workouts_matching(&self, filter: &WorkoutFilter) -> Vec<WorkoutTemplate> {
        let mut out: Vec<WorkoutTemplate> = self
            .package
            .as_ref()
            .map(|p| {
                p.workouts
                    .iter()
                    .filter(|w| filter.matches(w))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        for w in self.catalogue.workouts_matching(filter) {
            if self.package_workout(&w.slug).is_none() {
                out.push(w);
            }
        }
        out
    }

    fn package_flavour(&self, id: &str) -> Option<&Flavour> {
        self.package
            .as_ref()
            .and_then(|p| p.flavour.as_ref())
            .filter(|f| f.id == id)
    }

    fn package_workout(&self, slug: &str) -> Option<&WorkoutTemplate> {
        self.package
            .as_ref()
            .and_then(|p| p.workouts.iter().find(|w| w.slug == slug))
    }
}

/// Load the package the plan's agent lends its athletes, or `None` when
/// there is no agent, the agent carries no package, or the package is not
/// published.
///
/// `agent_id` is the agent row the plan or conversation names — an
/// installed copy or the origin; the package is read off the origin, whose
/// listing is the one the store reviews. `user_id` is the athlete, so a
/// agent the athlete cannot see resolves nothing.
///
/// # Errors
///
/// Returns the repository error when the agent, its listing or its rows
/// cannot be read.
pub async fn load_agent_package(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    agent_id: Option<&str>,
) -> AppResult<Option<AgentPackage>> {
    let Some(agent_id) = agent_id else {
        return Ok(None);
    };
    let Some(origin) = origin_agent(repos, tenant, user_id, agent_id).await? else {
        return Ok(None);
    };
    let origin_id = origin.id.to_string();
    let published = repos
        .store_listings
        .get_listing(&origin_id)
        .await?
        .is_some_and(|listing| listing.publish_status.is_published());
    if !published {
        return Ok(None);
    }
    let rows = repos
        .agent_artefacts
        .list_agent_artefacts(&origin.tenant_id, &origin_id)
        .await?;
    if rows.is_empty() {
        return Ok(None);
    }
    let package = AgentPackage::from_rows(&origin_id, &rows);
    Ok((!package.is_empty()).then_some(package))
}

/// The agent that authored the package: the row itself, or the row it was
/// forked from when the athlete uses an installed copy.
async fn origin_agent(
    repos: &RepositoryRegistry,
    tenant: TenantId,
    user_id: Uuid,
    agent_id: &str,
) -> AppResult<Option<Agent>> {
    let Some(agent) = repos.agents.get_by_id(agent_id, user_id, tenant).await? else {
        return Ok(None);
    };
    match agent.forked_from {
        Some(origin) => {
            repos
                .agents
                .get_by_id(&origin.to_string(), user_id, tenant)
                .await
        }
        None => Ok(Some(agent)),
    }
}

/// One artefact as the store review shows it: what it is, and what in it
/// nothing answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArtefactReview {
    /// `flavour`, `skeleton` or `workout`.
    pub kind: ArtefactKind,
    /// The item's own id or slug.
    pub slug: String,
    /// SHA-256 of the stored text.
    pub sha256: String,
    /// `true` when the artefact cites no `evidence_refs` at all — the
    /// kernel admits that only for a grey-practice or agent-judgement tier
    /// with a caveat, and the reviewer is told rather than the file refused.
    pub cites_no_evidence: bool,
    /// The artefact's references nothing answers: an evidence path with no
    /// proposition behind it, a purpose no template carries. Rendered as
    /// `key → reference`.
    pub unresolved: Vec<UnresolvedRef>,
    /// The stored text no longer parses — the message. A reviewer sees the
    /// row rather than a package that silently lost a file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
}

/// One unresolved reference on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UnresolvedRef {
    /// The key the reference sits under (`evidence_refs[2]`,
    /// `session_mix.build.race_specific`).
    pub key: String,
    /// The reference as written.
    pub reference: String,
}

/// The package as the store review shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackageReview {
    /// Every stored artefact, `(kind, slug)` order.
    pub artefacts: Vec<ArtefactReview>,
    /// `false` when the evidence registry was empty at review time — no
    /// contremaitre sync yet — so an evidence path could not be checked and
    /// none is flagged. The reviewer is told which case they are in.
    pub evidence_checked: bool,
}

/// Review an agent's stored artefacts against the catalogue and the evidence
/// corpus.
///
/// Evidence paths are checked against the registry when it holds anything;
/// an empty registry (contremaitre not synced) checks none and says so
/// through `evidence_checked`, the way the catalogue sync skips its own
/// report until the corpus is loaded. Purposes are checked against the
/// package's own workouts and the catalogue's together.
#[must_use]
pub fn review_package(
    rows: &[AgentArtefact],
    catalogue: &TrainingCatalogueRegistry,
    evidence: &EvidenceRegistry,
) -> PackageReview {
    let evidence_checked = !evidence.is_empty();
    let exists =
        |category: &str, slug: &str| !evidence_checked || evidence.sha256(category, slug).is_some();
    let package = AgentPackage::from_rows(rows.first().map_or("", |r| r.agent_id.as_str()), rows);
    let unresolved = package.unresolved_references(catalogue, &exists);
    let artefacts = rows
        .iter()
        .map(|row| {
            let (cites_no_evidence, parse_error) = match row.parse() {
                Ok(parsed) => (parsed.evidence_refs().is_empty(), None),
                Err(e) => (false, Some(e.to_string())),
            };
            let owner = owner_label(row);
            ArtefactReview {
                kind: row.kind,
                slug: row.slug.clone(),
                sha256: row.sha256.clone(),
                cites_no_evidence,
                unresolved: unresolved
                    .iter()
                    .filter(|u| u.owner == owner)
                    .map(|u| UnresolvedRef {
                        key: u.key.clone(),
                        reference: u.reference.clone(),
                    })
                    .collect(),
                parse_error,
            }
        })
        .collect();
    PackageReview {
        artefacts,
        evidence_checked,
    }
}

/// The `owner` string the kernel puts on an artefact's unresolved
/// references — `flavour 'id'`, `skeleton 'id'`, `workout 'slug'`.
fn owner_label(row: &AgentArtefact) -> String {
    let noun = match row.kind {
        ArtefactKind::Flavour => "flavour",
        ArtefactKind::Skeleton => "skeleton",
        ArtefactKind::Workout => "workout",
    };
    format!("{noun} '{}'", row.slug)
}
