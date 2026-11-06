// ABOUTME: Whether a read may show the wellness of the person it reads, and the one wording for when it may not
// ABOUTME: Always on a caller's own read; on a coach's read, under the athlete's coach_wellness_read grant, on any transport

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A coached athlete's wellness, on a coach's read
//!
//! A coach reads a coached athlete's training under their
//! `coach_sharing_consent`. Their wellness — what [`GrantNamed`] lists — takes
//! a second grant, `coach_wellness_read`, which only the athlete turns on, in
//! Dravr's own app
//! (carnet#1387). The grant holds on every transport: a coach's own AI client
//! (MCP, A2A, the public connector) reads under it as Dravr's app does, with
//! no consent of its own (carnet#1424), and each provider's transport terms
//! still decide which of its rows leave Dravr. The plan tools' readiness
//! reads under the same grant (`PlanScope::wellness_withheld`).
//!
//! [`ReadSubject::wellness_withheld`] answers that once per read, and every
//! place a read would show wellness asks it: a [`DataClass::Wellness`] tool
//! is refused before its body runs ([`wellness_class_refusal`]), a training
//! read refuses its wellness inputs ([`wellness_refusal`]) and leaves the
//! wellness fields of the athlete's physiology out ([`readable_physiology`],
//! [`readable_anchors`]). Each says why in [`WellnessWithheld::note`], one
//! wording: the grant the athlete has not given, and where they give it —
//! never that a value is missing, which it may not be.
//!
//! Under the grant a coach reads only what it names: the athlete's sleep,
//! HRV, resting heart rate, weight, readiness and daily notes, as the consent
//! words them ([`GrantNamed`], the one list every wording quotes through
//! [`wellness_data`]). Each stored row a wellness read merges is cut down to
//! that list before the merge (`named`, `readable_wellness`); the athlete's
//! own read is whole.
//!
//! [`DataClass::Wellness`]: super::DataClass::Wellness

#[cfg(feature = "tools-config")]
use pierre_core::models::UserPhysiologicalProfile;
#[cfg(feature = "tools-config")]
use pierre_services::stored_health::Anchors;

use super::{refusal, ReadSubject};
#[cfg(any(feature = "tools-analytics", feature = "tools-config"))]
use crate::context::scoped_read_subject;
use crate::protocol::types::UniversalResponse;

#[cfg(feature = "tools-data")]
mod named;
#[cfg(feature = "tools-data")]
pub(crate) use named::{readable_wellness, unnamed_refusal, unnamed_wellness_note};

/// What an athlete's wellness grant shares with their coach, as the consent
/// they agreed to names it, in its order.
///
/// The one list a coach's read of the athlete's wellness is cut down to, and
/// the one every wording of it quotes ([`wellness_data`]). Naming more here
/// widens what every coach reads under grants already given, so an entry
/// lands only once the consent copy names it too, in all five locales
/// (`delegation.wellnessApps*`), and the grants given under the narrower copy
/// are asked again.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GrantNamed {
    /// Their sleep: how long, in which stages, how well.
    Sleep,
    /// Their heart-rate variability.
    Hrv,
    /// Their resting heart rate.
    RestingHeartRate,
    /// Their body weight.
    Weight,
    /// Their readiness: the recovery and readiness scores their sources give.
    Readiness,
    /// The notes they write in their source's daily log.
    DailyNotes,
}

impl GrantNamed {
    /// Every entry, in the consent's order.
    const ALL: [Self; 6] = [
        Self::Sleep,
        Self::Hrv,
        Self::RestingHeartRate,
        Self::Weight,
        Self::Readiness,
        Self::DailyNotes,
    ];

    /// The entry in the consent's words.
    const fn words(self) -> &'static str {
        match self {
            Self::Sleep => "sleep",
            Self::Hrv => "HRV",
            Self::RestingHeartRate => "resting heart rate",
            Self::Weight => "weight",
            Self::Readiness => "readiness",
            Self::DailyNotes => "daily notes",
        }
    }
}

/// What a coach's read calls an athlete's wellness: [`GrantNamed`] in the
/// consent's words, "sleep, HRV, resting heart rate, weight, readiness and
/// daily notes".
#[must_use]
pub fn wellness_data() -> String {
    let words: Vec<&str> = GrantNamed::ALL.iter().map(|named| named.words()).collect();
    match words.split_last() {
        Some((last, rest)) if !rest.is_empty() => format!("{} and {last}", rest.join(", ")),
        _ => words.concat(),
    }
}

/// Where an athlete shares their wellness with their coach: in their own
/// Dravr app, the one place a grant is taken.
pub const WELLNESS_GRANT_WHERE: &str =
    "in their own Dravr app, under Group info → What your human coach can do";

/// Why a coach's read of a coached athlete leaves their wellness out: the
/// athlete has not granted their coach `coach_wellness_read`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WellnessWithheld {
    /// The athlete's roster name.
    pub(crate) athlete: String,
}

impl WellnessWithheld {
    /// The `reason` a refusal of this carries.
    pub(crate) const REASON: &str = "wellness_grant_needed";

    /// Why the athlete's wellness is not read, for the model to relay: the
    /// grant they have not given and where they give it. Never that a value
    /// is missing.
    #[must_use]
    pub(crate) fn note(&self) -> String {
        let athlete = &self.athlete;
        let wellness = wellness_data();
        format!(
            "{athlete} has not shared their wellness ({wellness}) with you, so none of it is \
             read here. Only {athlete} can share it, {WELLNESS_GRANT_WHERE}."
        )
    }
}

impl ReadSubject {
    /// Why this read leaves the athlete's wellness out, or `None` when it may
    /// read it: under the athlete's `coach_wellness_read`, as the gates read
    /// it for this call, whichever transport the call came over.
    #[must_use]
    pub(crate) fn wellness_withheld(&self) -> Option<WellnessWithheld> {
        (!self.access.consents.wellness_read).then(|| WellnessWithheld {
            athlete: self.access.display_name.clone(),
        })
    }
}

/// The refusal of `tool_name`, a read of the athlete's wellness, on a coach's
/// read that may not read it: before anything of theirs is read.
#[must_use]
pub(crate) fn wellness_class_refusal(
    tool_name: &str,
    withheld: &WellnessWithheld,
) -> UniversalResponse {
    refusal(
        WellnessWithheld::REASON,
        format!(
            "'{tool_name}' reads the athlete's wellness. {} Say so rather than estimating it.",
            withheld.note()
        ),
    )
}

/// Why the in-flight read leaves the wellness of the person it reads out, or
/// `None` when it may read it: always on a caller's own read
/// ([`scoped_read_subject`] is `None`), and on a coach's read as
/// [`ReadSubject::wellness_withheld`] decides.
#[cfg(any(feature = "tools-analytics", feature = "tools-config"))]
#[must_use]
pub(crate) fn wellness_withheld() -> Option<WellnessWithheld> {
    scoped_read_subject().and_then(|subject| subject.wellness_withheld())
}

/// Whether the in-flight read may read the wellness of the person it reads:
/// what [`GrantNamed`] lists.
///
/// Every place a read leaves wellness out asks here or at
/// [`wellness_withheld`] — the refused wellness inputs (`wellness_refusal`),
/// the physiology a read shows (`readable_physiology`), the anchors it looks
/// up (`readable_anchors`) and the heart-rate reading of `calculate_metrics`,
/// which the resting heart rate is solved back from — so the class opens in
/// one place. The heart-rate zones of `calculate_personalized_zones` are
/// training: the saved resting heart rate cuts them on any read, and only its
/// echoed value asks here (Phil, 2026-11-06).
#[cfg(feature = "tools-config")]
#[must_use]
pub(crate) fn wellness_readable() -> bool {
    wellness_withheld().is_none()
}

/// What a call asked for that reads the athlete's wellness.
#[cfg(feature = "tools-analytics")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WellnessInput<'call> {
    /// An argument that adds their wellness to a read of their training
    /// (`sleep_provider`).
    Argument(&'call str),
    /// A mode, `argument` set to `value`, that reads their wellness
    /// (`recommendation_type` `nutrition`, which scales from their body
    /// weight and reads their medical flag).
    Mode {
        /// The argument that picks the mode.
        argument: &'call str,
        /// The mode it picked.
        value: &'call str,
    },
}

/// The refusal of `asked`, which would make `tool_name` read the coached
/// athlete's wellness on a read of their training that may not read it
/// (`withheld` says why).
#[cfg(feature = "tools-analytics")]
#[must_use]
pub(crate) fn wellness_refusal(
    tool_name: &str,
    asked: WellnessInput<'_>,
    withheld: &WellnessWithheld,
) -> UniversalResponse {
    let note = withheld.note();
    let message = match asked {
        WellnessInput::Argument(argument) => format!(
            "'{tool_name}' with `{argument}` reads the athlete's wellness. {note} Leave \
             `{argument}` out to read their training alone."
        ),
        WellnessInput::Mode { argument, value } => format!(
            "'{tool_name}' with `{argument}` '{value}' reads the athlete's wellness. {note} Ask \
             for another `{argument}` to read their training."
        ),
    };
    refusal(WellnessWithheld::REASON, message)
}

/// The athlete's physiology as the in-flight read may show it: the whole
/// profile when it may read their wellness ([`wellness_readable`]); else
/// their training physiology alone ([`without_wellness`]).
///
/// For what a read outputs or offers as the athlete's own number. A value
/// read only to compute a training output (resting heart rate placing a
/// session in the heart-rate zones) is read from the stored profile as it
/// is, and only the training output leaves.
#[cfg(feature = "tools-config")]
#[must_use]
pub(crate) fn readable_physiology(
    profile: Option<UserPhysiologicalProfile>,
) -> Option<UserPhysiologicalProfile> {
    if wellness_readable() {
        profile
    } else {
        profile.map(without_wellness)
    }
}

/// The anchors of `wanted` the in-flight read may look up among the
/// athlete's measurements: all of them when it may read their wellness;
/// else `VO2max` alone — a resting heart rate and a weigh-in are their
/// wellness rows.
#[cfg(feature = "tools-config")]
#[must_use]
pub(crate) fn readable_anchors(wanted: Anchors) -> Anchors {
    if wellness_readable() {
        wanted
    } else {
        Anchors {
            vo2_max: wanted.vo2_max,
            ..Anchors::default()
        }
    }
}

/// `profile`'s training physiology, as carnet#1387 splits it: FTP, threshold
/// pace and heart rate, maximum heart rate, lactate threshold, the zones, CP,
/// W′, CS and D′, `VO2max`, age and sex, with where each came from. Its
/// wellness — resting heart rate and body weight, and where each came from —
/// is left out.
#[cfg(feature = "tools-config")]
fn without_wellness(mut profile: UserPhysiologicalProfile) -> UserPhysiologicalProfile {
    profile.resting_hr = None;
    profile.weight = None;
    profile.anchor_provenance.resting_hr = None;
    profile.anchor_provenance.weight = None;
    profile
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant_needed() -> WellnessWithheld {
        WellnessWithheld {
            athlete: "Mira".to_owned(),
        }
    }

    #[test]
    fn the_list_reads_as_the_consent_words_it() {
        assert_eq!(
            wellness_data(),
            "sleep, HRV, resting heart rate, weight, readiness and daily notes",
            "bug: the named list is the consent copy's, in its order"
        );
    }

    #[test]
    fn the_grant_note_names_the_athlete_and_where_they_share_it() {
        let note = grant_needed().note();
        assert!(
            note.starts_with(
                "Mira has not shared their wellness (sleep, HRV, resting heart rate, weight, \
                 readiness and daily notes)"
            ),
            "bug: the note says whose grant is missing, and what it would share: {note}"
        );
        assert!(
            note.contains("Only Mira can share it")
                && note.contains("Group info → What your human coach can do"),
            "bug: the note says who turns it on, and where: {note}"
        );
    }

    #[test]
    fn the_note_never_says_a_value_is_missing_nor_that_a_transport_reads_none() {
        let note = grant_needed().note().to_lowercase();
        for missing in ["not saved", "no data", "none was", "not stored", "missing"] {
            assert!(
                !note.contains(missing),
                "bug: a withheld read never reads as an absent value ({missing}): {note}"
            );
        }
        for transport in ["mcp", "a2a", "external client", "own app only"] {
            assert!(
                !note.contains(transport),
                "bug: the grant decides on every transport, so the note names none ({transport}): \
                 {note}"
            );
        }
    }

    #[test]
    fn a_wellness_tool_refusal_says_why_and_tells_the_model_not_to_estimate() {
        let refused = wellness_class_refusal("get_sleep_sessions", &grant_needed());
        assert!(!refused.success, "a withheld wellness read is refused");
        assert_eq!(
            refused.result.as_ref().unwrap()["reason"],
            "wellness_grant_needed",
            "the refusal carries the reason a test branches on"
        );
        let message = refused.error.unwrap();
        assert!(
            message.starts_with("'get_sleep_sessions' reads the athlete's wellness. Mira has"),
            "bug: the refusal names the tool and the grant: {message}"
        );
        assert!(
            message.ends_with("Say so rather than estimating it."),
            "bug: the refusal tells the model not to estimate: {message}"
        );
    }

    #[cfg(feature = "tools-analytics")]
    #[test]
    fn a_wellness_input_refusal_names_the_grant_and_how_to_read_the_training_alone() {
        let argument = wellness_refusal(
            "analyze_training_load",
            WellnessInput::Argument("sleep_provider"),
            &grant_needed(),
        );
        assert_eq!(
            argument.result.as_ref().unwrap()["reason"],
            "wellness_grant_needed",
            "the refusal carries the reason a test branches on"
        );
        let message = argument.error.unwrap();
        assert!(
            message.starts_with("'analyze_training_load' with `sleep_provider` reads")
                && message.contains("Mira has not shared their wellness")
                && message.contains("Leave `sleep_provider` out"),
            "bug: an argument's refusal names it, the grant, and says to leave it out: {message}"
        );
        let mode = wellness_refusal(
            "generate_recommendations",
            WellnessInput::Mode {
                argument: "recommendation_type",
                value: "nutrition",
            },
            &grant_needed(),
        )
        .error
        .unwrap();
        assert!(
            mode.contains("`recommendation_type` 'nutrition' reads")
                && mode.contains("resting heart rate, weight")
                && mode.contains("Only Mira can share it")
                && mode.contains("Ask for another `recommendation_type`"),
            "bug: a mode's refusal names the mode, why, and says to pick another: {mode}"
        );
    }

    #[cfg(feature = "tools-config")]
    mod physiology {
        use chrono::NaiveDate;
        use pierre_core::models::{
            AnchorOrigin, AnchorProvenance, AnchorStamp, PhysiologicalSex, SportType,
        };
        use uuid::Uuid;

        use super::super::*;

        fn stamp() -> AnchorStamp {
            AnchorStamp {
                origin: AnchorOrigin::Promoted,
                as_of: NaiveDate::from_ymd_opt(2026, 11, 1),
            }
        }

        /// An athlete who saved every number, each anchor with its origin.
        fn full_profile() -> UserPhysiologicalProfile {
            let mut profile = UserPhysiologicalProfile::new(Uuid::new_v4(), SportType::Run);
            profile.resting_hr = Some(46);
            profile.weight = Some(61.5);
            profile.max_hr = Some(191);
            profile.threshold_hr = Some(172);
            profile.vo2_max = Some(58.0);
            profile.lactate_threshold_percentage = Some(0.86);
            profile.ftp_watts = Some(248);
            profile.threshold_pace_sec_per_km = Some(232.0);
            profile.age = Some(31);
            profile.sex = Some(PhysiologicalSex::Female);
            profile.anchor_provenance = AnchorProvenance {
                resting_hr: Some(stamp()),
                vo2_max: Some(stamp()),
                weight: Some(stamp()),
            };
            profile
        }

        #[test]
        fn the_training_physiology_leaves_out_resting_heart_rate_and_weight() {
            let projected = without_wellness(full_profile());
            assert_eq!(projected.resting_hr, None, "bug: resting HR is wellness");
            assert_eq!(projected.weight, None, "bug: body weight is wellness");
            assert_eq!(
                projected.anchor_provenance.resting_hr, None,
                "bug: where the resting HR came from says one was saved"
            );
            assert_eq!(
                projected.anchor_provenance.weight, None,
                "bug: where the weight came from says one was saved"
            );
            let shown = serde_json::to_value(&projected).unwrap();
            assert!(
                shown["resting_hr"].is_null() && shown["weight"].is_null(),
                "bug: the serialized profile carries no wellness value: {shown}"
            );
        }

        #[test]
        fn the_training_physiology_keeps_every_training_number() {
            let projected = without_wellness(full_profile());
            assert_eq!(projected.max_hr, Some(191), "max HR is training");
            assert_eq!(
                projected.threshold_hr,
                Some(172),
                "threshold HR is training"
            );
            assert_eq!(projected.vo2_max, Some(58.0), "VO2max is training");
            assert_eq!(
                projected.lactate_threshold_percentage,
                Some(0.86),
                "LT% is training"
            );
            assert_eq!(projected.ftp_watts, Some(248), "FTP is training");
            assert_eq!(
                projected.threshold_pace_sec_per_km,
                Some(232.0),
                "threshold pace is training"
            );
            assert_eq!(projected.age, Some(31), "age is training");
            assert_eq!(
                projected.sex,
                Some(PhysiologicalSex::Female),
                "sex is training"
            );
            assert!(
                projected.anchor_provenance.vo2_max.is_some(),
                "where the VO2max came from stays with it"
            );
        }

        #[test]
        fn a_callers_own_read_shows_their_whole_physiology() {
            let shown = readable_physiology(Some(full_profile())).unwrap();
            assert_eq!(
                (shown.resting_hr, shown.weight),
                (Some(46), Some(61.5)),
                "bug: outside a coach's read, nothing of the caller's own is left out"
            );
            let wanted = Anchors {
                resting_hr: true,
                vo2_max: true,
                weight: true,
            };
            assert_eq!(
                readable_anchors(wanted),
                wanted,
                "bug: a caller's own read looks up every anchor it wants"
            );
        }
    }
}
