// ABOUTME: UserFact — a structured claim the harness has extracted about a user
// ABOUTME: Distilled from conversation turns and activities by the memory extraction worker
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::scope::MemoryScope;

/// Semantic category of a [`UserFact`].
///
/// Bounded to a small enum so the extraction prompt and the retrieval filter
/// can share a stable vocabulary. New kinds should be additive only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FactKind {
    /// Long-lived identity/preference (sport, goal, coach style preference).
    Preference,
    /// Physiological or training-state claim ("resting HR around 52").
    Physiology,
    /// Injury, pain, or health constraint the coach should remember.
    Injury,
    /// Goal the user committed to, with or without a deadline.
    Goal,
    /// Schedule / availability constraint ("can only run Tue/Thu/Sat").
    Schedule,
    /// Equipment or environment ("trains on a Wahoo Kickr indoors in winter").
    Equipment,
    /// Catch-all for semantically meaningful facts that don't fit elsewhere.
    Other,
}

impl FactKind {
    /// Stable string identifier used in DB serialization and extraction prompts.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Preference => "preference",
            Self::Physiology => "physiology",
            Self::Injury => "injury",
            Self::Goal => "goal",
            Self::Schedule => "schedule",
            Self::Equipment => "equipment",
            Self::Other => "other",
        }
    }

    /// Parse from the DB string form. Unknown values fall back to [`Self::Other`]
    /// so older rows don't panic during migration.
    #[must_use]
    pub fn parse_lenient(s: &str) -> Self {
        match s {
            "preference" => Self::Preference,
            "physiology" => Self::Physiology,
            "injury" => Self::Injury,
            "goal" => Self::Goal,
            "schedule" => Self::Schedule,
            "equipment" => Self::Equipment,
            _ => Self::Other,
        }
    }
}

/// A structured claim the harness has extracted about a user.
///
/// Facts are the unit of semantic memory. They are tenant-scoped, have
/// provenance back to a source message, carry a confidence score, and
/// optionally carry an embedding for vector recall.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserFact {
    /// Stable identifier for this fact.
    pub id: String,
    /// Tenant that owns the fact.
    pub tenant_id: String,
    /// User the fact is about.
    pub user_id: String,
    /// Coach that the fact was collected for, if any. `None` means the fact
    /// is user-wide across coaches.
    pub coach_id: Option<String>,
    /// Scope bucket (conversation / user / tenant).
    pub scope: MemoryScope,
    /// Semantic category.
    pub kind: FactKind,
    /// Short subject phrase — typically "you" or a named entity.
    pub subject: String,
    /// Predicate phrase ("prefers", "runs", "has").
    pub predicate: String,
    /// Object phrase — the value being asserted.
    pub object: String,
    /// Extractor confidence in `[0.0, 1.0]`.
    pub confidence: f32,
    /// ID of the message the fact was extracted from, for provenance.
    pub source_msg_id: Option<String>,
    /// Optional embedding vector for semantic recall. Stored as `Vec<f32>` at
    /// the DB boundary; both `SQLite` (BLOB) and `Postgres` (pgvector) translate
    /// transparently in the repository layer.
    pub embedding: Option<Vec<f32>>,
    /// When the fact was first recorded.
    pub created_at: DateTime<Utc>,
    /// When the fact was last touched (updated / merged / confidence decayed).
    pub updated_at: DateTime<Utc>,
}

impl UserFact {
    /// Short-hand check to see if this fact is still confident enough to be
    /// injected into the next prompt. Callers pick the threshold; this
    /// helper just avoids duplicating the comparison.
    #[must_use]
    pub fn is_confident(&self, min: f32) -> bool {
        self.confidence >= min
    }
}

/// Aggregate health snapshot for the memory extraction worker.
///
/// The extraction worker is a fire-and-forget background task, so the health
/// of the pipeline is best observed by looking at the rows it has actually
/// produced in `user_facts` rather than instrumenting the worker itself.
///
/// Counters use `u64` so JSON marshaling matches the database column shape
/// (`COUNT(*)` returns an `i64` that we widen) and all-zero fields stay
/// readable in the UI without sentinel values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserFactMetrics {
    /// Total number of facts stored for the tenant across all users/coaches.
    pub total_facts: u64,
    /// Facts inserted or updated in the last 24 hours — the "recent activity"
    /// signal that tells operators the worker is still extracting.
    pub facts_last_24h: u64,
    /// Facts inserted or updated in the last 7 days — the "healthy baseline"
    /// signal that tells operators the worker produced anything this week.
    pub facts_last_7d: u64,
    /// Distinct users the tenant has facts for. Useful when comparing with the
    /// tenant's active-user count to spot extraction gaps.
    pub distinct_users: u64,
    /// Count per `FactKind` string (e.g. `preference`, `physiology`, ...).
    /// A `BTreeMap` keeps JSON serialization order stable across calls, which
    /// makes the admin tab table render deterministic.
    pub facts_by_kind: BTreeMap<String, u64>,
    /// Timestamp of the most recently updated fact, or `None` when the tenant
    /// has no facts yet. Drives the "last activity" badge in the admin UI.
    pub newest_updated_at: Option<DateTime<Utc>>,
}

impl UserFactMetrics {
    /// Convenience constructor for the empty snapshot — no facts yet.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            total_facts: 0,
            facts_last_24h: 0,
            facts_last_7d: 0,
            distinct_users: 0,
            facts_by_kind: BTreeMap::new(),
            newest_updated_at: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{FactKind, MemoryScope, UserFact};
    use chrono::Utc;

    #[test]
    fn fact_kind_roundtrip() {
        for kind in [
            FactKind::Preference,
            FactKind::Physiology,
            FactKind::Injury,
            FactKind::Goal,
            FactKind::Schedule,
            FactKind::Equipment,
            FactKind::Other,
        ] {
            assert_eq!(FactKind::parse_lenient(kind.as_str()), kind);
        }
    }

    #[test]
    fn unknown_kind_falls_back_to_other() {
        assert_eq!(FactKind::parse_lenient("hallucinated"), FactKind::Other);
    }

    #[test]
    fn is_confident_threshold() {
        let now = Utc::now();
        let fact = UserFact {
            id: "f1".into(),
            tenant_id: "t1".into(),
            user_id: "u1".into(),
            coach_id: None,
            scope: MemoryScope::User,
            kind: FactKind::Goal,
            subject: "you".into(),
            predicate: "want".into(),
            object: "sub-3 marathon".into(),
            confidence: 0.72,
            source_msg_id: Some("m1".into()),
            embedding: None,
            created_at: now,
            updated_at: now,
        };
        assert!(fact.is_confident(0.7));
        assert!(!fact.is_confident(0.8));
    }
}
