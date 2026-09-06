// ABOUTME: Memory primitives for the coaching harness — facts, compaction, sessions, notes, followups
// ABOUTME: Pure types crate. Persistence lives in pierre-database; orchestration in pierre-server.
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![deny(unsafe_code)]

//! # Pierre Memory
//!
//! The `pierre-memory` crate defines the primitive types used by the coaching
//! harness to remember what happened across turns, across conversations, and
//! across channels. It intentionally has no persistence logic and no LLM
//! dependencies — those live downstream.
//!
//! ## Primitives
//!
//! - [`MemoryScope`] — the isolation boundary a memory belongs to
//! - [`FactKind`] — the semantic category of a user fact
//! - [`UserFact`] — a structured claim the harness has extracted about a user
//! - [`CoachNote`] — a note authored by the coach persona itself
//! - [`CoachFollowup`] — a promised future check-in ("you said you'd ask about
//!   Alice's Achilles pain tomorrow")
//! - [`CompactionBlock`] — a summary of earlier conversation turns that replaces
//!   the raw history when the context window fills up
//! - [`CoachSession`] — a long-lived container above `chat_conversation` that
//!   binds a user + coach pair across channels
//! - [`ClaimVerdict`] — the output of the bullshit detector pipeline
//!   for a single claim emitted by a coach persona
//! - [`Playbook`] — a learned `trigger -> intervention` coaching pattern,
//!   reinforced by automatic outcome labels; [`PendingAdvice`] tracks
//!   in-flight advice awaiting its observed outcome
//! - [`Commitment`] — a countable, time-boxed promise the athlete made and the
//!   coach confirmed ("three easy runs this week"), counted against real
//!   activity data once its window closes

/// Claim verdicts from the verification pipeline.
pub mod claims;
/// Athlete commitments: countable, time-boxed promises swept against real data.
pub mod commitments;
/// Summaries that replace earlier conversation turns in long sessions.
pub mod compaction;
/// Structured user facts extracted from conversation turns.
pub mod facts;
/// Coach-authored promised future check-ins.
pub mod followups;
/// Notes the coach persona authored about a user.
pub mod notes;
/// Procedural coaching playbooks reinforced by automatic outcome labels.
pub mod playbooks;
/// Isolation boundary for every memory record.
pub mod scope;
/// Long-lived coaching sessions that span conversations and channels.
pub mod sessions;
/// Coach-authored training plans: outline (macrocycle) + weekly microcycles.
pub mod training_plans;

pub use claims::{ClaimCategory, ClaimStatus, ClaimVerdict, EvidenceStrength, VerdictLayer};
pub use commitments::{
    Commitment, CommitmentOutcome, CommitmentStatus, MAX_STATEMENT_LEN, MAX_TARGET_SESSIONS,
    MAX_WINDOW_DAYS, MIN_TARGET_SESSIONS,
};
pub use compaction::CompactionBlock;
pub use facts::{FactKind, FactSource, PredicateCode, UserFact, UserFactMetrics};
pub use followups::{CoachFollowup, FollowupStatus};
pub use notes::CoachNote;
pub use playbooks::{
    sanitize_sport_slug, wilson_lower_bound_95, AdviceStatus, ArchetypePrior, Band, Intervention,
    InterventionKind, LabelSource, MetricBaseline, OutcomeLabel, OutcomeMetric, PendingAdvice,
    Playbook, TriggerKind, TriggerPattern, MAX_SPORT_SLUG_LEN,
};
pub use scope::MemoryScope;
pub use sessions::{CoachSession, SessionStatus};
pub use training_plans::{
    parse_plan_date, FlavourSelection, GoalRace, PlanPhase, PlanStatus, PlanWeek, PlannedDay,
    RacePriority, SelectedBy, TemplateParams, TrainingPlan, WeekStatus, MAX_DAYS_PER_WEEK,
};
