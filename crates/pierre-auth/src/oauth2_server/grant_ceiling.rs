// ABOUTME: What a call made under a detailed consent grant may reach: its streams, tier, data window and budgets
// ABOUTME: Resolved once at authentication from the grant's stored document, carried by AuthResult into dispatch

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The grant ceiling (owl PDPP §3, plan slice 3 item 6).
//!
//! A delegated token minted under a grant whose `authorization_details` name a
//! preset reads only what that preset grants: its streams (without the raw
//! ones on a `summary` grant), up to its `max_tier`, no row dated before the
//! window's `since`, within its budgets. [`GrantCeiling`] is that reach,
//! resolved from the grant's resolved document when the token is admitted and
//! carried with the call wherever it runs. A plain grant, and every credential
//! that is not a delegated token, has none: it reads what it read before.
//!
//! A document that does not read, or whose window was never frozen, is a
//! refusal ([`GrantCeiling::from_document`] errs), never a plain grant: a
//! ceiling that cannot be resolved must not widen the call.

use dravr_barrage::{Stream, Tier};
use dravr_tronc::dates::{CivilDate, Timestamp, Zone};
use pierre_core::errors::{AppError, AppResult};

use super::authorization_details::{Budgets, Representation, ResolvedDetails};

/// What a call made under a detailed consent grant may reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantCeiling {
    /// The consent grant the call is made under (`oauth_client_grants.id`)
    pub client_grant_id: String,
    /// The streams the grant reads: the preset's, without its raw streams on
    /// a summary grant
    pub streams: Vec<Stream>,
    /// The highest tier a call under the grant may read
    pub max_tier: Tier,
    /// The earliest moment a row the grant reads may be dated: the athlete's
    /// local midnight starting [`Self::first_day`], in UTC
    pub since: Timestamp,
    /// The first of the athlete's days a daily row the grant reads may be
    /// dated, in their zone
    pub first_day: CivilDate,
    /// The grant's request and row budgets
    pub budgets: Budgets,
}

impl GrantCeiling {
    /// The ceiling of grant `client_grant_id`, approved for `details`, for an
    /// athlete in `zone`.
    ///
    /// # Errors
    /// An internal error when the details' window was never frozen: a grant
    /// stores its details only once the athlete approved them.
    pub fn of_details(
        client_grant_id: &str,
        details: &ResolvedDetails,
        zone: &Zone,
    ) -> AppResult<Self> {
        let since = details.since().ok_or_else(|| {
            AppError::internal("A grant's authorization details carry no frozen window")
        })?;
        let preset = details.preset();
        let streams = preset
            .streams()
            .iter()
            .copied()
            .filter(|stream| {
                details.representation() == Representation::SummaryAndRaw
                    || !preset.raw_streams().contains(stream)
            })
            .collect();
        Ok(Self {
            client_grant_id: client_grant_id.to_owned(),
            streams,
            max_tier: details.max_tier(),
            since,
            first_day: zone.date_at(since),
            budgets: details.budgets(),
        })
    }

    /// The ceiling of grant `client_grant_id`, from the document it stored,
    /// for an athlete in `zone`.
    ///
    /// # Errors
    /// An internal error when the document does not read as resolved details
    /// this server wrote, or carries no frozen window.
    pub fn from_document(client_grant_id: &str, document: &str, zone: &Zone) -> AppResult<Self> {
        Self::of_details(
            client_grant_id,
            &ResolvedDetails::from_document(document)?,
            zone,
        )
    }

    /// Whether a row dated `at` falls inside the grant's window.
    #[must_use]
    pub fn admits_moment(&self, at: Timestamp) -> bool {
        at >= self.since
    }

    /// Whether a daily row of the athlete's `day` falls inside the window.
    #[must_use]
    pub fn admits_day(&self, day: CivilDate) -> bool {
        day >= self.first_day
    }

    /// Whether the grant reads `stream`.
    #[must_use]
    pub fn holds(&self, stream: Stream) -> bool {
        self.streams.contains(&stream)
    }

    /// Whether the grant reads data of `tier`.
    #[must_use]
    pub fn reaches(&self, tier: Tier) -> bool {
        tier <= self.max_tier
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::oauth2_server::authorization_details::{preset_defaults, Preset, ResolvedDetails};
    use dravr_tronc::dates::Elapsed;
    const MCP: &str = "https://mcp.example.test/mcp";

    fn approved(details: &ResolvedDetails) -> ResolvedDetails {
        details.freeze(Timestamp::now(), &Zone::utc()).unwrap().0
    }

    #[test]
    fn a_training_coach_grant_holds_its_preset_streams_raw_included() {
        let ceiling = GrantCeiling::of_details(
            "g",
            &approved(&preset_defaults(Preset::TrainingCoach, MCP)),
            &Zone::utc(),
        )
        .unwrap();
        assert!(
            ceiling.holds(Stream::Workout) && ceiling.holds(Stream::WorkoutRaw),
            "bug: a training grant lost its sessions or their samples"
        );
        assert!(
            !ceiling.holds(Stream::WorkoutRoute),
            "a preset never names a route"
        );
        assert!(
            !ceiling.holds(Stream::CardioRaw),
            "bug: a training grant reads another preset's raw stream"
        );
        assert!(
            ceiling.reaches(Tier::T1) && !ceiling.reaches(Tier::T2),
            "bug: a training grant's tier ceiling is not T1"
        );
        assert_eq!(
            ceiling.budgets.requests_per_hour, 120,
            "bug: the ceiling lost the grant's hourly budget"
        );
    }

    #[test]
    fn a_summary_grant_holds_no_raw_stream() {
        let ceiling = GrantCeiling::of_details(
            "g",
            &approved(&preset_defaults(Preset::SleepCoach, MCP)),
            &Zone::utc(),
        )
        .unwrap();
        assert!(
            ceiling.holds(Stream::Sleep),
            "bug: a sleep grant lost its nights"
        );
        assert!(
            !ceiling.holds(Stream::Workout) && !ceiling.holds(Stream::WorkoutRaw),
            "bug: a sleep grant reads sessions"
        );
    }

    #[test]
    fn an_unfrozen_or_unreadable_document_is_no_ceiling_and_no_plain_grant() {
        let unfrozen = preset_defaults(Preset::TrainingCoach, MCP);
        assert!(
            GrantCeiling::of_details("g", &unfrozen, &Zone::utc()).is_err(),
            "bug: an unfrozen document resolved to a ceiling"
        );
        assert!(
            GrantCeiling::from_document("g", "not a document", &Zone::utc()).is_err(),
            "bug: an unreadable document resolved to a ceiling"
        );
        let document = approved(&unfrozen).to_document().unwrap();
        let ceiling = GrantCeiling::from_document("g", &document, &Zone::utc()).unwrap();
        assert_eq!(
            ceiling.client_grant_id, "g",
            "bug: the ceiling names another grant"
        );
    }

    #[test]
    fn the_window_admits_rows_from_its_first_local_day_on() {
        let zone = Zone::from_name("America/Montreal").unwrap();
        let details = preset_defaults(Preset::TrainingCoach, MCP)
            .freeze(Timestamp::now(), &zone)
            .unwrap()
            .0;
        let ceiling = GrantCeiling::of_details("g", &details, &zone).unwrap();
        assert_eq!(
            zone.start_of_day(ceiling.first_day).unwrap(),
            ceiling.since,
            "bug: the window's first day and its moment disagree"
        );
        assert!(
            ceiling.admits_day(ceiling.first_day),
            "bug: the window's first day is outside it"
        );
        assert!(
            !ceiling.admits_day(ceiling.first_day.checked_sub_days(1).unwrap()),
            "bug: the day before the window is inside it"
        );
        assert!(
            ceiling.admits_moment(ceiling.since),
            "bug: the window's first moment is outside it"
        );
        assert!(
            !ceiling.admits_moment(ceiling.since - Elapsed::seconds(1)),
            "bug: the moment before the window is inside it"
        );
    }
}
