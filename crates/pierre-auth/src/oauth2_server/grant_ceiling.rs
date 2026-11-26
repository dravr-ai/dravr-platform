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
//!
//! ## A step-up child (carnet#1544 slice 4b)
//!
//! A grant with a live step-up child carries the child's own ceiling
//! ([`GrantCeiling::child`]): its picked streams, at its tier, window and
//! budgets, and its step-up terms. A call runs under exactly one of the two,
//! chosen by what the tool reads ([`GrantCeiling::for_streams`]): the
//! parent's when it covers every stream, else the child's when that does.
//! So a child adds streams and never widens the parent's own: the parent's
//! streams keep its T1 tier and 90-day window, and only the child's picked
//! streams reach T2, for two years back, under the child's budgets.
//!
//! T2 itself opens for an outside client only through [`Self::opens_t2`]:
//! the child's step-up is `owner_verified` and its host declared T2 on both
//! carriers — its CIMD at consent, and the per-request claim of the call
//! ([`Self::with_request_host_tier`]), the stricter of the two winning.

use dravr_barrage::{Stream, Tier};
use dravr_tronc::dates::{CivilDate, Timestamp, Zone};
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::StepUpTerms;

use super::authorization_details::{AccessMode, Budgets, Representation, ResolvedDetails};

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
    /// Whether the grant is consumed by the first call that reads under it
    pub single_use: bool,
    /// The step-up a child grant was approved with; `None` on a parent
    pub step_up: Option<StepUpTerms>,
    /// The tier the call's own per-request Host Profile claim declares (owl
    /// §6.2 carrier B); `None` when the call carries none
    pub request_host_tier: Option<Tier>,
    /// The grant's live step-up child, which a call reading only its streams
    /// runs under instead ([`Self::for_streams`])
    pub child: Option<Box<GrantCeiling>>,
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
        // A picking preset reads the streams the owner picked, each among its
        // own; every other preset its own.
        let streams = if preset.picks_streams() {
            details
                .picked_streams()
                .into_iter()
                .filter(|stream| preset.streams().contains(stream))
                .collect()
        } else {
            preset
                .streams()
                .iter()
                .copied()
                .filter(|stream| {
                    details.representation() == Representation::SummaryAndRaw
                        || !preset.raw_streams().contains(stream)
                })
                .collect()
        };
        Ok(Self {
            client_grant_id: client_grant_id.to_owned(),
            streams,
            max_tier: details.max_tier(),
            since,
            first_day: zone.date_at(since),
            budgets: details.budgets(),
            single_use: details.access_mode() == AccessMode::SingleUse,
            step_up: None,
            request_host_tier: None,
            child: None,
        })
    }

    /// This ceiling with `child`, the ceiling of its live step-up child
    /// approved with `terms`.
    #[must_use]
    pub fn with_child(mut self, mut child: Self, terms: StepUpTerms) -> Self {
        child.step_up = Some(terms);
        child.child = None;
        self.child = Some(Box::new(child));
        self
    }

    /// This ceiling, its child's included, for a call whose per-request Host
    /// Profile claim declares `tier` (`None`: it declares none).
    #[must_use]
    pub fn with_request_host_tier(mut self, tier: Option<Tier>) -> Self {
        self.request_host_tier = tier;
        if let Some(child) = &mut self.child {
            child.request_host_tier = tier;
        }
        self
    }

    /// The ceiling a call reading `streams` runs under: this grant's own
    /// (without its child) when it covers every one of them at its tier, else
    /// its child's when that covers every one, else this grant's own, which
    /// then refuses the call. A call reading no stream runs under this
    /// grant's own.
    #[must_use]
    pub fn for_streams(&self, streams: &[Stream]) -> Self {
        let covers = |ceiling: &Self| {
            streams
                .iter()
                .all(|stream| ceiling.holds(*stream) && ceiling.reaches(stream.tier()))
        };
        let own = Self {
            child: None,
            ..self.clone()
        };
        if streams.is_empty() || covers(&own) {
            return own;
        }
        match self.child.as_deref() {
            Some(child) if covers(child) => child.clone(),
            _ => own,
        }
    }

    /// Whether a step-up could cure a call reading `streams` this ceiling
    /// refuses: each is a stream a one-off question may pick (owl §3.1, the
    /// summary streams of tiers T0 to T2) and this grant's own does not
    /// cover it. A T3 stream never qualifies.
    #[must_use]
    pub fn step_up_cures(&self, streams: &[Stream]) -> bool {
        let own = self.for_streams(&[]);
        let one_off = dravr_barrage::Preset::OneOffQuestion.streams();
        !streams.is_empty()
            && streams.iter().all(|stream| one_off.contains(stream))
            && streams
                .iter()
                .any(|stream| !own.holds(*stream) || !own.reaches(stream.tier()))
    }

    /// Whether a T2 `stream` opens to an outside client under this ceiling:
    /// it is a step-up child's that holds `stream` at T2, the athlete stepped
    /// up `owner_verified`, and its host declared T2 both at consent and on
    /// this call (the stricter carrier wins: a call declaring less, or
    /// nothing, opens nothing). The athlete's `sensitive_t2` switch is asked
    /// besides, by the tier gate.
    #[must_use]
    pub fn opens_t2(&self, stream: Stream) -> bool {
        self.step_up.as_ref().is_some_and(StepUpTerms::opens_t2)
            && self.request_host_tier == Some(Tier::T2)
            && stream.tier() == Tier::T2
            && self.holds(stream)
            && self.reaches(Tier::T2)
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

    fn one_off(streams: &[&str]) -> ResolvedDetails {
        use crate::oauth2_server::authorization_details::{parse_authorization_details, resolve};
        let picked: Vec<_> = streams
            .iter()
            .map(|name| serde_json::json!({"name": name}))
            .collect();
        let raw = serde_json::json!([
            {"type": "https://pdpp.dev/data-access", "source": {"id": MCP},
             "selection_preset": "one_off_question", "streams": picked},
            {"type": "https://dravr.ai/pdpp/health-profile/v0/health-constraints",
             "applies_to": 0, "critical": true, "max_tier": "T2"},
        ]);
        approved(
            &resolve(
                &parse_authorization_details(&raw.to_string()).unwrap(),
                &[MCP],
            )
            .unwrap(),
        )
    }

    fn terms(acr: &str, tier: &str) -> StepUpTerms {
        StepUpTerms {
            parent_grant_id: "g".to_owned(),
            auth_time: Timestamp::now(),
            acr: acr.to_owned(),
            host_profile_basis: pierre_core::models::HostProfileBasis::DeclaredCimd,
            host_profile_tier: tier.to_owned(),
        }
    }

    fn with_labs_child(acr: &str, tier: &str) -> GrantCeiling {
        let parent = GrantCeiling::of_details(
            "g",
            &approved(&preset_defaults(Preset::TrainingCoach, MCP)),
            &Zone::utc(),
        )
        .unwrap();
        let child =
            GrantCeiling::of_details("c", &one_off(&["labs.summary"]), &Zone::utc()).unwrap();
        parent.with_child(child, terms(acr, tier))
    }

    #[test]
    fn a_call_runs_under_the_parent_unless_only_the_child_covers_it() {
        let ceiling = with_labs_child(pierre_core::models::OWNER_VERIFIED_ACR, "T2");
        let sleep = ceiling.for_streams(&[Stream::Sleep]);
        assert_eq!(
            sleep.client_grant_id, "g",
            "bug: a parent stream ran under the child"
        );
        assert!(
            sleep.child.is_none(),
            "bug: the effective ceiling kept a child"
        );
        let labs = ceiling.for_streams(&[Stream::Labs]);
        assert_eq!(
            labs.client_grant_id, "c",
            "bug: the child's stream ran under the parent"
        );
        assert!(labs.single_use, "bug: a one-off child is not single-use");
        assert_eq!(
            labs.budgets.total_rows,
            Some(500),
            "bug: the child's budget was lost"
        );
        let both = ceiling.for_streams(&[Stream::Labs, Stream::Workout]);
        assert_eq!(
            both.client_grant_id, "g",
            "bug: a call reading beyond either grant ran under the child"
        );
        assert!(
            !both.holds(Stream::Labs),
            "bug: the union widened the parent"
        );
    }

    #[test]
    fn t2_opens_only_with_owner_verified_a_t2_host_and_its_per_request_claim() {
        let open = with_labs_child(pierre_core::models::OWNER_VERIFIED_ACR, "T2")
            .with_request_host_tier(Some(Tier::T2))
            .for_streams(&[Stream::Labs]);
        assert!(
            open.opens_t2(Stream::Labs),
            "bug: every condition held and T2 stayed shut"
        );
        assert!(
            !open.opens_t2(Stream::ClinicalRaw),
            "bug: an unpicked T2 stream opened"
        );
        let no_claim = with_labs_child(pierre_core::models::OWNER_VERIFIED_ACR, "T2")
            .for_streams(&[Stream::Labs]);
        assert!(
            !no_claim.opens_t2(Stream::Labs),
            "bug: no per-request claim opened T2"
        );
        let t1_claim = with_labs_child(pierre_core::models::OWNER_VERIFIED_ACR, "T2")
            .with_request_host_tier(Some(Tier::T1))
            .for_streams(&[Stream::Labs]);
        assert!(
            !t1_claim.opens_t2(Stream::Labs),
            "bug: a T1 claim opened T2"
        );
        let weak = with_labs_child("password", "T2")
            .with_request_host_tier(Some(Tier::T2))
            .for_streams(&[Stream::Labs]);
        assert!(!weak.opens_t2(Stream::Labs), "bug: a weaker acr opened T2");
        let t1_host = with_labs_child(pierre_core::models::OWNER_VERIFIED_ACR, "T1")
            .with_request_host_tier(Some(Tier::T2))
            .for_streams(&[Stream::Labs]);
        assert!(
            !t1_host.opens_t2(Stream::Labs),
            "bug: a T1 CIMD host opened T2"
        );
        let parent = with_labs_child(pierre_core::models::OWNER_VERIFIED_ACR, "T2")
            .with_request_host_tier(Some(Tier::T2));
        assert!(
            !parent.opens_t2(Stream::Labs),
            "bug: the parent itself opened T2"
        );
    }

    #[test]
    fn only_a_one_off_stream_beyond_the_parent_is_curable_by_step_up() {
        let parent = GrantCeiling::of_details(
            "g",
            &approved(&preset_defaults(Preset::TrainingCoach, MCP)),
            &Zone::utc(),
        )
        .unwrap();
        assert!(
            parent.step_up_cures(&[Stream::Labs]),
            "bug: labs not curable"
        );
        assert!(
            !parent.step_up_cures(&[Stream::Sleep]),
            "bug: a held stream curable"
        );
        assert!(
            !parent.step_up_cures(&[Stream::RestrictedNotes]),
            "bug: T3 curable by step-up"
        );
        assert!(
            !parent.step_up_cures(&[Stream::ClinicalRaw]),
            "bug: a raw T2 curable"
        );
        assert!(
            !parent.step_up_cures(&[]),
            "bug: a call reading nothing curable"
        );
    }
}
