// ABOUTME: The one row-level merge of stored activity copies into workouts, and the two gates a read applies first
// ABOUTME: Home, the gear ledger, the places pass and the activity aggregator merge and gate stored rows through it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Workouts from stored rows.
//!
//! The store holds one row per copy of an activity: the Strava API's, the
//! session mirror's, Garmin's push, the athlete's own export. A count
//! counts workouts, never rows, so every reader that sums or lists stored
//! activities merges the copies first, and keeps each copy's row beside the
//! merged session: a row's key, place and segment read are the row's, not
//! the session's ([`Workout`]). The merge is [`merge_duplicates`] under
//! [`DedupConfig::from_env`], the one the chat turn's activity list runs.
//!
//! A read gates its rows before it merges them, so a withheld copy never
//! fills a field of a served one (carnet#724):
//! - [`served_rows`] is what the athlete's own surfaces serve: every row in
//!   the app, and over an external transport none of a provider whose terms
//!   keep its data inside Dravr;
//! - [`filter_activity_rows`] is what a model may read: the item filter
//!   every tool applies ([`ai_scope::filter_activities`]), row by row.

use std::cmp::Reverse;
use std::iter;
use std::ptr;

use pierre_core::ai_policy::ProviderTerms;
use pierre_core::constants::oauth_providers::garmin_recorded;
use pierre_core::models::{Activity, PlaceRef};
use pierre_database::repositories::{sport_type_string, CachedActivityRow};
use pierre_providers::ai_scope;
use pierre_providers::deduplication::{is_canonical_copy, merge_duplicates, DedupConfig};

/// The sport bucket of an activity whose sport has no spelling.
const UNNAMED_SPORT: &str = "other";

/// One workout the store holds: the merged session, and the stored rows of
/// the copies it was merged from.
#[derive(Debug)]
pub struct Workout<'rows> {
    /// The merged session: the merger's canonical copy, with the fields it
    /// lacked filled from the other recordings.
    pub session: Activity,
    /// The stored row of the canonical copy.
    pub canonical: &'rows CachedActivityRow,
    /// The stored rows of the other copies merged into it.
    pub copies: Vec<&'rows CachedActivityRow>,
}

impl<'rows> Workout<'rows> {
    /// Every stored row of the workout: the canonical copy's, then the
    /// others'.
    pub fn rows(&self) -> impl Iterator<Item = &'rows CachedActivityRow> + '_ {
        iter::once(self.canonical).chain(self.copies.iter().copied())
    }

    /// The place the workout belongs to: the canonical row's, else the
    /// first other copy's that has one.
    #[must_use]
    pub fn place(&self) -> Option<&'rows PlaceRef> {
        self.rows().find_map(|row| row.place.as_ref())
    }

    /// Whether a detail read of any copy read the workout's segment efforts.
    #[must_use]
    pub fn segments_read(&self) -> bool {
        self.rows().any(|row| row.segment_read)
    }

    /// Whether the workout carries Garmin-sourced data: the merged session
    /// or any of its copies does, since its figures may come from any of
    /// them.
    #[must_use]
    pub fn carries_garmin_data(&self) -> bool {
        iter::once(&self.session)
            .chain(self.rows().map(|row| &row.activity))
            .any(|activity| garmin_recorded(activity.source(), activity.device_name()))
    }
}

/// The workouts `rows` holds, newest first.
///
/// Every copy of one workout is merged into one session through [`merge_duplicates`] under
/// [`DedupConfig::from_env`], exactly as the chat turn's activity list
/// merges them, each with the rows it was merged from.
#[must_use]
pub fn workouts(rows: &[CachedActivityRow]) -> Vec<Workout<'_>> {
    // The merger consumes its input and rewrites the canonical copy, while
    // each row's stored key, place and reads are still needed.
    let recordings: Vec<Activity> = rows.iter().map(|row| row.activity.clone()).collect();
    let (mut sessions, report) = merge_duplicates(recordings, &DedupConfig::from_env());
    // A merged workout sits where its canonical copy was read, which need not
    // be where its newest copy was; the list is newest first by the session.
    sessions.sort_by_key(|session| Reverse(session.start_date()));
    sessions
        .into_iter()
        .filter_map(|session| {
            let canonical = rows
                .iter()
                .find(|row| is_canonical_copy(&row.activity, &session))?;
            // A group's members name the canonical copy first: it is the
            // workout's canonical row, never one of its other copies.
            let copies = report
                .groups
                .iter()
                .find(|group| group.canonical_id == session.id())
                .into_iter()
                .flat_map(|group| {
                    rows.iter().filter(move |row| {
                        !ptr::eq(*row, canonical)
                            && group.fragment_ids.iter().any(|id| id == row.activity.id())
                            && group.window_start <= row.activity.start_date()
                            && row.activity.start_date() <= group.window_end
                    })
                })
                .collect();
            Some(Workout {
                session,
                canonical,
                copies,
            })
        })
        .collect()
}

/// The stored rows the athlete's own surfaces serve.
///
/// Over an external transport (an API key), a provider whose terms keep its data inside
/// Dravr's own surfaces is dropped before any merge, so a withheld copy of a
/// workout can never fill the fields of a served one (carnet#724). The
/// athlete's own app sees every row.
#[must_use]
pub fn served_rows(
    terms: &dyn ProviderTerms,
    rows: Vec<CachedActivityRow>,
) -> Vec<CachedActivityRow> {
    ai_scope::retain_served_here(terms, rows, |row| {
        (row.provider.as_str(), row.activity.source())
    })
}

/// The stored rows a model may read.
///
/// The item filter every tool reads activities through
/// ([`ai_scope::filter_activities`]), applied row by row: a withheld activity drops
/// its row, and a kept or reduced one replaces the row's activity while the
/// row keeps its own key, place and reads. Withheld items are tallied in the
/// enclosing [`ai_scope::tallied`] scope exactly as the item filter tallies
/// them.
#[must_use]
pub fn filter_activity_rows(
    terms: &dyn ProviderTerms,
    rows: Vec<CachedActivityRow>,
) -> Vec<CachedActivityRow> {
    rows.into_iter()
        .filter_map(|mut row| {
            let activity = ai_scope::filter_activities(terms, vec![row.activity])
                .into_iter()
                .next()?;
            row.activity = activity;
            Some(row)
        })
        .collect()
}

/// The sport an activity is counted under: the store's own `sport_type`
/// spelling, the key Home's volume weeks and the recent list's sport filter
/// carry, and the one a count by sport groups on.
#[must_use]
pub fn sport_bucket(activity: &Activity) -> String {
    sport_type_string(activity).unwrap_or_else(|| UNNAMED_SPORT.to_owned())
}
