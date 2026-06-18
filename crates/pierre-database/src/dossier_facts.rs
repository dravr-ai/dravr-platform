// ABOUTME: Groups pierre-memory UserFacts into the Dossier's pillar/north-star/medical buckets
// ABOUTME: Shared by the SQLite and Postgres compose_dossier impls so they stay identical
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Utc};
use pierre_core::models::{DossierFact, Pillar};
use pierre_memory::{FactKind, UserFact};

/// Upper bound on facts pulled for the per-user context bundle. Higher than the
/// flat-recall limit because facts now fan across six pillars + North Star +
/// medical; the renderer's token budget is the real gate downstream.
pub const FACT_BUNDLE_LIMIT: i64 = 40;

/// The Dossier's pillar-context buckets, grouped from a flat fact list.
pub struct DossierFactBuckets {
    /// Pillar-tagged facts keyed by pillar.
    pub pillars: BTreeMap<Pillar, Vec<DossierFact>>,
    /// North Star (life-motivation) facts.
    pub north_star: Vec<DossierFact>,
    /// Medical / PAR-Q flags.
    pub medical: Vec<DossierFact>,
}

/// Fallback pillar for a fact recorded before pillar tagging (`pillar` is
/// `None`). Fuelling, sleep, resilience, and connection are not inferable from
/// an untagged legacy row, so untagged facts land on the broadest fitness
/// pillar rather than guessing — this just keeps them visible in the bundle.
/// New facts carry an explicit pillar set by their caller.
const FALLBACK_PILLAR: Pillar = Pillar::TrainingAndMovement;

/// Project a single `UserFact` into a `DossierFact`, computing staleness
/// against `now`.
fn project(fact: &UserFact, now: DateTime<Utc>) -> DossierFact {
    DossierFact {
        kind: fact.kind.as_str().to_owned(),
        subject: fact.subject.clone(),
        predicate: fact.predicate.clone(),
        object: fact.object.clone(),
        confidence: fact.confidence,
        source: fact.source.as_str().to_owned(),
        updated_at: fact.updated_at,
        valid_until: fact.valid_until,
        stale: fact.valid_until.is_some_and(|vu| vu < now),
    }
}

/// Group a flat fact list into the Dossier's pillar / north-star / medical
/// buckets. North Star and medical facts are routed to their own buckets;
/// every other fact lands in its pillar (or the kind's fallback pillar).
///
/// Facts are de-duplicated by id, so callers may safely concatenate several
/// fact reads (e.g. a recency-bounded general fetch unioned with by-kind
/// fetches that guarantee safety-critical kinds reach the renderer).
pub fn group_facts(facts: &[UserFact], now: DateTime<Utc>) -> DossierFactBuckets {
    let mut buckets = DossierFactBuckets {
        pillars: BTreeMap::new(),
        north_star: Vec::new(),
        medical: Vec::new(),
    };
    let mut seen: HashSet<&str> = HashSet::new();
    for fact in facts {
        if !seen.insert(fact.id.as_str()) {
            continue;
        }
        let projected = project(fact, now);
        match fact.kind {
            FactKind::NorthStar => buckets.north_star.push(projected),
            FactKind::Medical => buckets.medical.push(projected),
            _ => {
                let pillar = fact.pillar.unwrap_or(FALLBACK_PILLAR);
                buckets.pillars.entry(pillar).or_default().push(projected);
            }
        }
    }
    buckets
}
