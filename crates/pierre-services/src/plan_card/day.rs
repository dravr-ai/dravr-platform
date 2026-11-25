// ABOUTME: One day of the plan card — the session as the card shows it, its evidence, the coach's marks and call, its place on its date
// ABOUTME: The card's days and the coach's family editor preview both project a stored day through day_card

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_contremaitre::evidence_strings::{self, EvidenceChipKind, EvidenceNoteKind};
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
use pierre_core::models::periodization::serde_num::whole_u32_opt;
use pierre_core::models::periodization::{SportFamily, WorkoutPurpose, WorkoutStep};
use pierre_core::models::FuelingProtocol;
use pierre_memory::training_plans::{PlannedDay, TemplateParams, TemplateSource};
use serde::{Deserialize, Serialize};

use crate::medical_flag::FiguresWithheld;
use crate::plan_calendar_push;
use crate::plan_fueling::FuelingDisclosure;

/// One day of a week: one entry of its date, two sessions on a date being
/// two days.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DayCard {
    /// Civil date, `YYYY-MM-DD`.
    pub date: String,
    /// Sport label.
    pub sport: String,
    /// The session in the agent's words.
    pub workout: String,
    /// Planned duration in minutes, when stated.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration_min: Option<u32>,
    /// Intensity label as the agent wrote it.
    pub intensity: String,
    /// `true` for a rest day.
    pub rest: bool,
    /// The session's structure, when the day carries one.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<WorkoutStep>,
    /// What to take in, when prescribed and this athlete may see the rates.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fueling: Option<FuelingProtocol>,
    /// In place of `fueling`, for a day that stores a protocol while a
    /// medical flag is on file: why the rates are absent and who sets them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fueling_withheld: Option<FiguresWithheld>,
    /// The template the day instantiates, when it does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_slug: Option<String>,
    /// Which tier the template came from.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_source: Option<TemplateSource>,
    /// The warning the card marks a day with when the envelope check found the
    /// session outside its family's evidence, as stored with the day, in
    /// `locale`'s words (`messaging.evidence.outside`); the reply that saved it
    /// carried the notice saying why (carnet#1495). A card frozen in a reply's
    /// blocks keeps the language of the turn that saved it, so the clients
    /// word the warning from [`DayCard::outside`] and print this only when they
    /// hold no words for it. Absent when the check found nothing, and when the
    /// string is missing, as for the chip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outside_evidence: Option<String>,
    /// Whether the envelope check found the session outside its family's
    /// evidence, so a client words the warning (`messaging.evidence.outside`)
    /// in its reader's language: `outside_evidence` is worded in the language
    /// of the turn that saved the card, frozen with it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub outside: bool,
    /// Where the day's evidence sits when it is weaker than a trial in this
    /// sport without a qualifier: one chip, the weakest evidence winning
    /// (carnet#1495). Absent otherwise, and on a day no family measured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub evidence: Option<DayEvidence>,
    /// "Counts as <purpose>" in `locale`'s words, when an edit relabelled the
    /// session: what a client prints when it cannot word
    /// [`DayCard::counts_as_purpose`]. Absent when none did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counts_as: Option<String>,
    /// The purpose an edit relabelled the session to
    /// (`messaging.envelope.purpose.<purpose>`), so a client words "counts
    /// as" in its reader's language. Absent when none did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counts_as_purpose: Option<WorkoutPurpose>,
    /// The family preset the day instantiates, when it names one: what the
    /// coach's family editor opens on (carnet#1515).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_preset: Option<String>,
    /// The values the day fills its preset with, when it states any of the
    /// six the editor shows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub template_params: Option<TemplateValues>,
    /// The day's sport as a family preset reads it, when it is one of the
    /// three a preset is written for.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family_sport: Option<SportFamily>,
    /// The session the coach's edits replaced, when the coach edited this day
    /// of their draft (carnet#1714).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edited: Option<DayEdit>,
    /// The day's position among the entries on its date, 0-based, a rest
    /// entry included: with the date, how the coach's editor addresses the
    /// session (carnet#1716).
    pub position: u32,
    /// `true` when the coach added this session to its date in their draft
    /// (carnet#1716): restoring it removes it. Absent otherwise.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub added: bool,
    /// `true` when the coach marked this session as their call (carnet#1814),
    /// « C'est mon choix »: they saw where its evidence stands and keep it.
    /// Unlike `edited` and `added` it is the day's own, so a live plan's day
    /// carries it too. Absent otherwise.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub coach_call: bool,
}

/// The session a coach's edits replaced on a day of their draft: what the
/// card marks an edited day with and what restoring the day puts back
/// (carnet#1714). Only a draft's days carry one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DayEdit {
    /// The replaced session's workout, as the day wrote it.
    pub workout: String,
    /// Whether the replaced day was a rest day.
    pub rest: bool,
}

/// The six values a family day states on its preset.
///
/// What the card shows of the day's `template_params` and what the coach's
/// family editor sends (carnet#1515). The pacing and anchor a day may also
/// state stay the agent's; the card does not show them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplateValues {
    /// Sets.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole_u32_opt"
    )]
    pub sets: Option<u32>,
    /// Reps per set.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole_u32_opt"
    )]
    pub reps: Option<u32>,
    /// Work bout, seconds.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole_u32_opt"
    )]
    pub work_seconds: Option<u32>,
    /// Rest between reps, seconds.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole_u32_opt"
    )]
    pub rest_seconds: Option<u32>,
    /// Rest between sets, seconds.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole_u32_opt"
    )]
    pub set_rest_seconds: Option<u32>,
    /// Whole-session duration, minutes.
    #[serde(
        skip_serializing_if = "Option::is_none",
        deserialize_with = "whole_u32_opt"
    )]
    pub duration_minutes: Option<u32>,
}

impl TemplateValues {
    /// The six values `params` states, or `None` when it states none of them.
    #[must_use]
    pub fn of(params: &TemplateParams) -> Option<Self> {
        let values = Self {
            sets: params.sets,
            reps: params.reps,
            work_seconds: params.work_seconds,
            rest_seconds: params.rest_seconds,
            set_rest_seconds: params.set_rest_seconds,
            duration_minutes: params.duration_minutes,
        };
        (values != Self::default()).then_some(values)
    }

    /// The values as a day stores them, its pacing and anchor left to the
    /// preset.
    #[must_use]
    pub const fn template_params(self) -> TemplateParams {
        TemplateParams {
            sets: self.sets,
            reps: self.reps,
            work_seconds: self.work_seconds,
            rest_seconds: self.rest_seconds,
            duration_minutes: self.duration_minutes,
            set_rest_seconds: self.set_rest_seconds,
            pacing: None,
            anchor: None,
        }
    }
}

/// The evidence chip a day carries, by name for the client to word and in
/// `locale`'s words for one that cannot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DayEvidence {
    /// The chip by name, which a client words as `messaging.evidence.<kind>`
    /// in its reader's language.
    pub kind: EvidenceChipKind,
    /// The chip (`messaging.evidence.*`), in the language of the turn that
    /// saved the card: what a client prints when it cannot word `kind`.
    pub label: String,
    /// The studies the badge rests on, as cited ("Rønnestad 2015 [RCT]"):
    /// what the chip opens. Absent when the badge cites none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub basis: Option<String>,
    /// What the chip opens on in place of `basis` when the badge cites no
    /// study: why none is, for its status (convention, or a trial run in
    /// another sport). Absent when `basis` is present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// `note` by name, which a client words as `messaging.evidence.<kind>`.
    /// Absent when `note` is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_kind: Option<EvidenceNoteKind>,
}

/// One stored day as the card shows it, its outside-the-evidence warning,
/// evidence chip and relabel line in `locale`'s words from the findings, badge
/// and purpose the save stored.
///
/// The card's every day goes through here, and so does the day the coach's
/// family editor previews before saving it (carnet#1515), so the chip the
/// preview shows is the one the card shows once the day is saved. The card
/// reads as the first entry on its date; the week's card places each of its
/// days among its date's entries ([`DayCard::position`]).
#[must_use]
pub fn day_card(
    day: &PlannedDay,
    fueling: &FuelingDisclosure,
    registry: &MessagingStringsRegistry,
    locale: &str,
) -> DayCard {
    let (fueling, fueling_withheld) = fueling.day_fueling(day);
    let envelope = day.envelope.as_ref();
    let evidence = envelope
        .and_then(|envelope| envelope.badge.as_ref())
        .and_then(|badge| {
            let kind = evidence_strings::evidence_chip_kind(badge)?;
            evidence_strings::evidence_chip_label(registry, locale, badge).map(|label| {
                let note = evidence_strings::evidence_note(registry, locale, badge);
                DayEvidence {
                    kind,
                    label,
                    basis: badge.basis.clone(),
                    note_kind: note
                        .as_ref()
                        .and_then(|_| evidence_strings::evidence_note_kind(badge)),
                    note,
                }
            })
        });
    let relabelled = envelope.and_then(|envelope| envelope.relabelled_purpose);
    let counts_as =
        relabelled.and_then(|purpose| evidence_strings::counts_as_line(registry, locale, purpose));
    let outside = envelope.is_some_and(|envelope| !envelope.findings.is_empty());
    let outside_evidence = outside
        .then(|| evidence_strings::outside_evidence_label(registry, locale))
        .flatten();
    DayCard {
        date: day.date.clone(),
        sport: day.sport.clone(),
        workout: day.workout.clone(),
        duration_min: day.duration_min,
        intensity: day.intensity.clone(),
        rest: day.is_rest(),
        steps: day.steps.clone(),
        fueling,
        fueling_withheld,
        template_slug: day.template_slug.clone(),
        template_source: day.template_source,
        outside_evidence,
        outside,
        evidence,
        counts_as,
        counts_as_purpose: relabelled,
        template_preset: day.template_preset.clone(),
        template_params: day.template_params.as_ref().and_then(TemplateValues::of),
        family_sport: SportFamily::of(&plan_calendar_push::plan_sport(&day.sport)),
        edited: day.coach_original.as_deref().map(|original| DayEdit {
            workout: original.workout.clone(),
            rest: original.is_rest(),
        }),
        position: 0,
        added: day.coach_added,
        coach_call: day.coach_call.is_some(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_contremaitre::evidence_strings::KEY_EVIDENCE_OUTSIDE;

    /// A day the envelope check found something on is marked on its card with
    /// the warning in the reader's words; a day it found nothing on, or never
    /// measured, carries no mark at all (carnet#1495, carnet#1552).
    #[test]
    fn only_a_day_with_an_envelope_finding_is_marked_outside_the_evidence() {
        let day = |envelope: serde_json::Value| -> PlannedDay {
            serde_json::from_value(serde_json::json!({
                "date": "2026-07-14", "sport": "ride", "workout": "20/40",
                "template_slug": "vo2max_short", "envelope": envelope
            }))
            .expect("a stored day")
        };
        let fueling = FuelingDisclosure::from_gate(None);
        let strings = MessagingStringsRegistry::new();
        let finding = serde_json::json!({
            "code": "cycle_mean_floor", "action": "warn", "message": "English, for the model."
        });

        let outside = day_card(
            &day(serde_json::json!({ "findings": [finding] })),
            &fueling,
            &strings,
            "en",
        );
        assert_eq!(
            outside.outside_evidence.as_deref(),
            Some("Outside the evidence"),
            "bug: a stored finding marks the day with the catalogue's warning"
        );
        assert_eq!(
            serde_json::to_value(&outside).unwrap()["outside_evidence"],
            "Outside the evidence",
            "bug: the warning reaches the wire as the label the clients print"
        );
        assert_eq!(
            serde_json::to_value(&outside).unwrap()["outside"],
            true,
            "bug: the warning also travels as a flag a client words in its own language"
        );
        assert_eq!(
            day_card(
                &day(serde_json::json!({ "findings": [finding] })),
                &fueling,
                &strings,
                "fr",
            )
            .outside_evidence
            .as_deref(),
            Some("Hors des données probantes"),
            "bug: the warning follows the reader's locale, as the chip does"
        );

        let blank = MessagingStringsRegistry::new();
        blank.update(KEY_EVIDENCE_OUTSIDE, "en", String::new(), String::new());
        let unworded = day_card(
            &day(serde_json::json!({ "findings": [finding] })),
            &fueling,
            &blank,
            "en",
        );
        let unworded = serde_json::to_value(&unworded).unwrap();
        assert!(
            unworded.get("outside_evidence").is_none(),
            "bug: a missing warning string never reaches the wire as an empty label"
        );
        assert_eq!(
            unworded["outside"], true,
            "bug: a warning the server has no words for still reaches a client that has them"
        );

        for clean in [
            serde_json::json!({ "findings": [] }),
            serde_json::Value::Null,
        ] {
            let card = day_card(&day(clean), &fueling, &strings, "en");
            assert_eq!(
                card.outside_evidence, None,
                "bug: a day with no finding is not marked"
            );
            let wire = serde_json::to_value(&card).unwrap();
            assert!(
                wire.get("outside_evidence").is_none() && wire.get("outside").is_none(),
                "bug: an unmarked day carries no key: {wire}"
            );
        }
    }

    /// The card names where a family day's evidence sits from the badge the
    /// save stored, in the reader's language, with the citation it opens on or,
    /// when the badge cites none, why by its status, and what a relabel now
    /// counts it as; an unqualified trial and an unmeasured day carry neither
    /// key (carnet#1495, step 3).
    #[test]
    fn a_day_carries_its_evidence_chip_and_relabel_line_from_what_the_save_stored(
    ) -> Result<(), &'static str> {
        let day = |envelope: serde_json::Value| -> PlannedDay {
            serde_json::from_value(serde_json::json!({
                "date": "2026-07-14", "sport": "run", "workout": "30/15",
                "template_slug": "vo2max_short", "envelope": envelope
            }))
            .expect("a stored day")
        };
        let fueling = FuelingDisclosure::from_gate(None);
        let strings = MessagingStringsRegistry::new();

        let transposed = day_card(
            &day(serde_json::json!({
                "findings": [], "relabelled_purpose": "threshold",
                "badge": { "status": "transposed", "basis": "Rønnestad 2015 [RCT]" }
            })),
            &fueling,
            &strings,
            "en",
        );
        assert_eq!(
            transposed.evidence,
            Some(DayEvidence {
                kind: EvidenceChipKind::Transposed,
                label: "Transposed from another sport".to_owned(),
                basis: Some("Rønnestad 2015 [RCT]".to_owned()),
                note: None,
                note_kind: None,
            }),
            "bug: a transposed preset that cites its study opens on the citation, with no note"
        );
        assert_eq!(
            transposed.counts_as.as_deref(),
            Some("Counts as threshold work"),
            "bug: a relabelled day names what it now counts as"
        );
        assert_eq!(
            serde_json::to_value(&transposed).unwrap()["counts_as_purpose"],
            "threshold",
            "bug: the relabel also travels as the purpose a client words in its own language"
        );

        let french = day_card(
            &day(serde_json::json!({ "findings": [], "badge": { "status": "convention" } })),
            &fueling,
            &strings,
            "fr",
        );
        assert_eq!(
            french.evidence.as_ref().map(|chip| chip.label.as_str()),
            Some("Convention d'entraînement"),
            "bug: the chip reads in the reader's language"
        );
        let chip = french
            .evidence
            .ok_or("bug: a convention day carries its chip")?;
        assert_eq!(
            chip.basis, None,
            "bug: a session no study covers cites none"
        );
        assert_eq!(
            chip.note.as_deref(),
            Some("Aucun essai ne couvre cette séance ; elle suit la pratique courante de l'entraînement."),
            "bug: it opens on why no study covers it, in the reader's language"
        );
        assert_eq!(
            serde_json::to_value(&chip).unwrap(),
            serde_json::json!({
                "kind": "convention",
                "label": "Convention d'entraînement",
                "note": "Aucun essai ne couvre cette séance ; elle suit la pratique courante de l'entraînement.",
                "note_kind": "no_basis_convention",
            }),
            "bug: the chip and its note travel by name, so a client reads them in its own language"
        );

        let uncited = day_card(
            &day(serde_json::json!({ "findings": [], "badge": { "status": "transposed" } })),
            &fueling,
            &strings,
            "en",
        );
        assert_eq!(
            uncited.evidence.and_then(|chip| chip.note).as_deref(),
            Some("Its trials ran in another sport; it reaches this one through the transposition rules."),
            "bug: a transposed chip with no citation opens on where its trial ran, never on 'no trial covers this'"
        );

        for quiet in [
            serde_json::json!({ "findings": [], "badge": { "status": "trialled" } }),
            serde_json::Value::Null,
        ] {
            let wire =
                serde_json::to_value(day_card(&day(quiet), &fueling, &strings, "en")).unwrap();
            assert!(
                wire.get("evidence").is_none()
                    && wire.get("counts_as").is_none()
                    && wire.get("counts_as_purpose").is_none(),
                "bug: an unqualified trial or an unmeasured day carries no chip and no relabel: {wire}"
            );
        }
        Ok(())
    }

    /// What the coach's family editor opens on (carnet#1515): the preset, the
    /// six values the day states — never its pacing or anchor — and the
    /// sport as a preset reads it, an mtb day as a ride; a rest day and a
    /// day that states none carry none of them.
    #[test]
    fn a_family_day_carries_its_preset_values_and_sport_for_the_editor() {
        let fueling = FuelingDisclosure::from_gate(None);
        let strings = MessagingStringsRegistry::new();
        let wire = |day: serde_json::Value| {
            let day: PlannedDay = serde_json::from_value(day).expect("a stored day");
            serde_json::to_value(day_card(&day, &fueling, &strings, "en")).unwrap()
        };

        let family = wire(serde_json::json!({
            "date": "2026-07-14", "sport": "mtb", "workout": "30/15",
            "template_slug": "vo2max_short", "template_preset": "30_15",
            "template_params": { "sets": 4, "reps": 11, "pacing": "fixed" }
        }));
        assert_eq!(
            family["template_preset"], "30_15",
            "bug: the card carries the preset the day instantiates: {family}"
        );
        assert_eq!(
            family["template_params"],
            serde_json::json!({ "sets": 4, "reps": 11 }),
            "bug: the card shows the six values only, pacing left to the agent"
        );
        assert_eq!(
            family["family_sport"], "ride",
            "bug: an mtb day reads as a ride"
        );

        let swim = wire(serde_json::json!({
            "date": "2026-07-15", "sport": "natation", "workout": "easy", "intensity": "Z2",
            "template_params": { "pacing": "fixed" }
        }));
        assert_eq!(
            swim["family_sport"], "swim",
            "bug: a natation day reads as a swim: {swim}"
        );
        assert!(
            swim.get("template_params").is_none() && swim.get("template_preset").is_none(),
            "bug: a day stating none of the six carries no values: {swim}"
        );

        for quiet in [
            serde_json::json!({ "date": "2026-07-16", "sport": "rest", "workout": "Rest" }),
            serde_json::json!({ "date": "2026-07-17", "sport": "strength", "workout": "gym" }),
        ] {
            let quiet = wire(quiet);
            assert!(
                quiet.get("family_sport").is_none(),
                "bug: a day no preset is written for names no family sport: {quiet}"
            );
        }
    }

    /// A day the coach edited in their draft carries the session the edit
    /// replaced; a day with no snapshot carries no `edited` key (carnet#1714).
    #[test]
    fn only_a_day_with_a_coach_snapshot_is_marked_edited() {
        let fueling = FuelingDisclosure::from_gate(None);
        let strings = MessagingStringsRegistry::new();
        let wire = |value: serde_json::Value| -> serde_json::Value {
            let day: PlannedDay = serde_json::from_value(value).expect("a stored day");
            serde_json::to_value(day_card(&day, &fueling, &strings, "en")).expect("a card")
        };

        let edited = wire(serde_json::json!({
            "date": "2026-07-14", "sport": "run", "workout": "Easy 50 min", "intensity": "Z2",
            "coach_original": {
                "date": "2026-07-14", "sport": "run", "workout": "Easy 45 min", "intensity": "Z2"
            }
        }));
        assert_eq!(
            edited["edited"],
            serde_json::json!({ "workout": "Easy 45 min", "rest": false }),
            "bug: an edited day names the session the edit replaced: {edited}"
        );
        assert_eq!(
            edited["workout"], "Easy 50 min",
            "bug: the card shows the day as edited, not its snapshot: {edited}"
        );

        let over_rest = wire(serde_json::json!({
            "date": "2026-07-15", "sport": "run", "workout": "Strides", "intensity": "Z2",
            "coach_original": { "date": "2026-07-15", "sport": "rest", "workout": "Rest" }
        }));
        assert_eq!(
            over_rest["edited"]["rest"], true,
            "bug: an edit over a rest day says it replaced a rest day: {over_rest}"
        );

        let untouched = wire(serde_json::json!({
            "date": "2026-07-16", "sport": "run", "workout": "Easy 45 min", "intensity": "Z2"
        }));
        assert!(
            untouched.get("edited").is_none(),
            "bug: a day with no snapshot is not marked edited: {untouched}"
        );
    }

    /// A day added by the coach carries `added`; every day carries its
    /// position, the first entry on its date by itself (carnet#1716).
    #[test]
    fn a_day_carries_its_position_and_only_an_added_session_is_marked_added() {
        let fueling = FuelingDisclosure::from_gate(None);
        let strings = MessagingStringsRegistry::new();
        let wire = |value: serde_json::Value| -> serde_json::Value {
            let day: PlannedDay = serde_json::from_value(value).expect("a stored day");
            serde_json::to_value(day_card(&day, &fueling, &strings, "en")).expect("a card")
        };

        let added = wire(serde_json::json!({
            "date": "2026-07-14", "sport": "strength", "workout": "Core 20 min",
            "coach_added": true
        }));
        assert_eq!(
            added["added"], true,
            "bug: an added session is marked added: {added}"
        );
        assert_eq!(
            added["position"], 0,
            "bug: the position is always on the wire, the first entry by itself: {added}"
        );

        let agents = wire(serde_json::json!({
            "date": "2026-07-14", "sport": "ride", "workout": "Endurance 2 h"
        }));
        assert!(
            agents.get("added").is_none(),
            "bug: a session the coach did not add carries no added key: {agents}"
        );
    }

    /// A day the coach marked as their call says so on its card; a day they
    /// did not carries no key (carnet#1814).
    #[test]
    fn only_a_day_the_coach_called_carries_coach_call() {
        let fueling = FuelingDisclosure::from_gate(None);
        let strings = MessagingStringsRegistry::new();
        let wire = |value: serde_json::Value| -> serde_json::Value {
            let day: PlannedDay = serde_json::from_value(value).expect("a stored day");
            serde_json::to_value(day_card(&day, &fueling, &strings, "en")).expect("a card")
        };

        let called = wire(serde_json::json!({
            "date": "2026-07-14", "sport": "ride", "workout": "Sweet spot 3 × 15",
            "coach_call": { "by": "8f6f2a4e-1c1b-4d38-9a51-0d6e8b8f4c11", "at": "2026-07-10T09:00:00Z" }
        }));
        assert_eq!(
            called["coach_call"], true,
            "bug: a day the coach called says so on its card: {called}"
        );
        assert!(
            !called.to_string().contains("8f6f2a4e"),
            "bug: the card says the day is the coach's call, never who by: {called}"
        );

        let uncalled = wire(serde_json::json!({
            "date": "2026-07-14", "sport": "ride", "workout": "Endurance 2 h"
        }));
        assert!(
            uncalled.get("coach_call").is_none(),
            "bug: a day the coach did not call carries no coach_call key: {uncalled}"
        );
    }
}
