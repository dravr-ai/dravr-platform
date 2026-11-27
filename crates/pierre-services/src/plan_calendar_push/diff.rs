// ABOUTME: The plan's wanted calendar entries against Dravr's push ledger, key by key, and the entries the athlete edited on the provider
// ABOUTME: The one diff the calendar block, the publish preview and « Envoyer » read; the provider check is the send review's (carnet#1841)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::{HashMap, HashSet};

use dravr_tronc::dates::CivilDate;
use pierre_core::errors::AppResult;
use pierre_core::models::{
    CalendarEventRef, CalendarEventSource, PlannedSession, PlannedSessionKind, PrescribedWorkout,
};
use pierre_providers::ai_scope;
use pierre_providers::core::FitnessProvider;
use serde::Serialize;
use tracing::warn;

use super::{detail_edited, session_hash, stamped_after, DesiredEntry, PushPreview};

/// What a push would do with one calendar key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyChange {
    /// The plan wants the entry and the ledger has no live row for it.
    Create,
    /// The plan wants the entry and its content differs from the live row's.
    Update,
    /// The live row already carries the content the plan wants.
    Unchanged,
    /// The ledger holds a live row no entry of the plan wants any more: the
    /// push would take it off the calendar.
    Withdraw,
}

/// One calendar key a push would consider, judged from the ledger alone.
#[derive(Debug, Clone, PartialEq)]
pub struct KeyDiff {
    /// The entry's key.
    pub external_id: String,
    /// The entry's date: a week note's is its week's first day.
    pub date: CivilDate,
    /// A session or a week note.
    pub kind: PlannedSessionKind,
    /// What the push would do with it.
    pub change: KeyChange,
    /// The entry as Dravr last wrote it, from the live row's payload; `None`
    /// when the ledger holds no live row for the key, or its payload does
    /// not read as an entry.
    pub before: Option<PlannedSession>,
    /// The entry as the plan wants it now; `None` for a withdrawal.
    pub after: Option<PlannedSession>,
}

impl KeyDiff {
    /// Whether a push would change the calendar for this key.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        self.change != KeyChange::Unchanged
    }
}

/// The plan's wanted entries against the ledger's live plan rows, key by
/// key: the one diff the calendar block, the publish's preview and the
/// athlete's « Envoyer » count all read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LedgerDiff {
    /// Every key the plan wants, in the plan's order, then every live row
    /// no entry wants, in the ledger's order.
    pub keys: Vec<KeyDiff>,
}

impl LedgerDiff {
    /// The counts a push would change, folded from the keys.
    #[must_use]
    pub fn preview(&self) -> PushPreview {
        let mut preview = PushPreview::default();
        for key in &self.keys {
            match key.change {
                KeyChange::Create => preview.create += 1,
                KeyChange::Update => preview.update += 1,
                KeyChange::Unchanged => preview.unchanged += 1,
                KeyChange::Withdraw => preview.remove += 1,
            }
        }
        preview
    }

    /// The keys a push would change: created, updated or withdrawn.
    pub fn pending(&self) -> impl Iterator<Item = &KeyDiff> {
        self.keys.iter().filter(|key| key.is_pending())
    }
}

/// The entry a live ledger row last wrote, read from its payload; `None`
/// (logged) when the payload does not read as one.
fn written_entry(row: &PrescribedWorkout) -> Option<PlannedSession> {
    serde_json::from_str(&row.payload_json)
        .map_err(|e| {
            warn!(
                prescription_id = %row.id,
                error = %e,
                "a live plan row's payload does not read as a calendar entry"
            );
        })
        .ok()
}

/// What kind of entry a live plan row holds: its payload's, else its source's.
fn row_kind(row: &PrescribedWorkout, before: Option<&PlannedSession>) -> PlannedSessionKind {
    before.map_or_else(
        || {
            if row.source == CalendarEventSource::PlanWeekNote {
                PlannedSessionKind::WeekNote
            } else {
                PlannedSessionKind::Workout
            }
        },
        |entry| entry.kind,
    )
}

/// Diff desired entries against the ledger's live plan rows, key by key.
///
/// Rows that are not plan rows (a single prescription) are neither wanted
/// nor unwanted by the plan, and never appear.
///
/// # Errors
///
/// Returns an error when an entry cannot be hashed, which no well-formed
/// session produces.
pub fn diff_against_ledger(
    desired: &[DesiredEntry],
    live_rows: &[PrescribedWorkout],
) -> AppResult<LedgerDiff> {
    let plan_rows: Vec<&PrescribedWorkout> = live_rows
        .iter()
        .filter(|row| row.source.is_plan() && row.external_id.is_some())
        .collect();
    let live_by_key: HashMap<&str, &PrescribedWorkout> = plan_rows
        .iter()
        .filter_map(|row| row.external_id.as_deref().map(|key| (key, *row)))
        .collect();
    let mut keys = Vec::with_capacity(desired.len());
    let mut wanted: HashSet<&str> = HashSet::new();
    for entry in desired {
        let key = entry.session.external_id.as_str();
        wanted.insert(key);
        let hash = session_hash(&entry.session)?;
        let row = live_by_key.get(key).copied();
        let change = match row {
            None => KeyChange::Create,
            Some(row) if row.payload_hash.as_deref() == Some(hash.as_str()) => KeyChange::Unchanged,
            Some(_) => KeyChange::Update,
        };
        keys.push(KeyDiff {
            external_id: key.to_owned(),
            date: entry.session.date,
            kind: entry.session.kind,
            change,
            before: row.and_then(written_entry),
            after: Some(entry.session.clone()),
        });
    }
    // A key two live rows hold is one entry, as the push reads it: the row
    // `live_by_key` kept for it.
    let mut withdrawn: HashSet<&str> = HashSet::new();
    for row in plan_rows {
        let Some(key) = row.external_id.as_deref() else {
            continue;
        };
        if wanted.contains(key) || !withdrawn.insert(key) {
            continue;
        }
        let row = live_by_key.get(key).copied().unwrap_or(row);
        let before = written_entry(row);
        keys.push(KeyDiff {
            external_id: key.to_owned(),
            date: row.prescribed_for_date,
            kind: row_kind(row, before.as_ref()),
            change: KeyChange::Withdraw,
            before,
            after: None,
        });
    }
    Ok(LedgerDiff { keys })
}

/// The keys of `pending` a push would leave alone because the athlete
/// changed their entry on the provider after Dravr wrote it (carnet#1841).
///
/// These are the send review's « Modifiée sur Intervals.icu ». Only an update
/// can be left alone so; a withdrawal is taken off whatever the athlete did.
///
/// Reads `calendar`'s listing over the pending dates once, and an entry in
/// detail only where the listing carries no change stamp, as the push does.
///
/// # Errors
///
/// Returns the provider's error when its calendar cannot be listed at all.
pub async fn edited_on_calendar(
    calendar: &dyn FitnessProvider,
    live_rows: &[PrescribedWorkout],
    pending: &[KeyDiff],
) -> AppResult<Vec<String>> {
    let updates: Vec<(&KeyDiff, &PrescribedWorkout)> = pending
        .iter()
        .filter(|key| key.change == KeyChange::Update)
        .filter_map(|key| {
            live_rows
                .iter()
                .find(|row| row.external_id.as_deref() == Some(key.external_id.as_str()))
                .map(|row| (key, row))
        })
        .collect();
    let (Some(from), Some(to)) = (
        updates.iter().map(|(key, _)| key.date).min(),
        updates.iter().map(|(key, _)| key.date).max(),
    ) else {
        return Ok(Vec::new());
    };
    let observed: HashMap<String, CalendarEventRef> =
        ai_scope::for_display(calendar.list_calendar_events(from, to))
            .await?
            .into_iter()
            .map(|event| (event.provider_event_id.clone(), event))
            .collect();
    let mut edited = Vec::new();
    for (key, row) in updates {
        let Some(seen) = row
            .provider_event_id
            .as_deref()
            .and_then(|id| observed.get(id))
        else {
            continue;
        };
        let changed = stamped_after(row, seen)
            || (seen.updated_at.is_none() && detail_edited(calendar, row, seen).await);
        if changed {
            edited.push(key.external_id.clone());
        }
    }
    Ok(edited)
}
