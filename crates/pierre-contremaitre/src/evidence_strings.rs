// ABOUTME: Catalogue keys of where a workout-family day's evidence sits, as the plan card and /plan print it
// ABOUTME: One chip by precedence (convention, transposed, then the trial qualifiers), a relabel line, the warning
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Keys of the plan card's evidence chip (carnet#1495, step 3).
//!
//! Every saved workout-family day stores the evidence badge of its preset's
//! realization for the day's sport. The platform resolves these strings into
//! the plan card and writes them on `/plan`'s day lines, so the card, the
//! calendar and the slash command read one source in the reader's language.
//!
//! A day trialled without a qualifier carries no chip: the card stays quiet
//! where the evidence is strongest. Any other day carries one chip at most, the
//! weakest evidence winning the slot (founder decisions of 2026-11-12).

use pierre_core::models::periodization::{EvidenceBadge, EvidenceStatus, WorkoutPurpose};
use serde::Serialize;

use crate::messaging_strings::{envelope_purpose_key, MessagingStringsRegistry};

/// Key: the chip of a session that follows coaching convention, no trial.
/// No format placeholders.
pub const KEY_EVIDENCE_CONVENTION: &str = "messaging.evidence.convention";
/// Key: the chip of a session whose trials ran in another sport. No format
/// placeholders.
pub const KEY_EVIDENCE_TRANSPOSED: &str = "messaging.evidence.transposed";
/// Key: the chip of a session whose every study in this sport ran a single
/// session. No format placeholders.
pub const KEY_EVIDENCE_ACUTE_ONLY: &str = "messaging.evidence.acute_only";
/// Key: the chip of a session whose every study in this sport had no
/// comparator. No format placeholders.
pub const KEY_EVIDENCE_UNCONTROLLED_ONLY: &str = "messaging.evidence.uncontrolled_only";
/// Key: the chip of a session whose studies were both single-session and
/// uncontrolled. No format placeholders.
pub const KEY_EVIDENCE_ACUTE_UNCONTROLLED: &str = "messaging.evidence.acute_uncontrolled";
/// Key: a relabelled day's line. `{0}` = the purpose label it now counts as
/// (`messaging.envelope.purpose.*`).
pub const KEY_EVIDENCE_COUNTS_AS: &str = "messaging.evidence.counts_as";
/// Key: the warning on a day saved outside its family's evidence. No format
/// placeholders.
pub const KEY_EVIDENCE_OUTSIDE: &str = "messaging.evidence.outside";
/// Key: a `/plan` day's session with its notes. `{0}` = the session, `{1}` =
/// its warning, chip and relabel.
pub const KEY_PLAN_SESSION_NOTES: &str = "commands.plan.session_notes";
/// Key: what a convention chip opens on, when its badge cites no study. No
/// format placeholders.
pub const KEY_EVIDENCE_NO_BASIS_CONVENTION: &str = "messaging.evidence.no_basis_convention";
/// Key: what a transposed chip opens on, when its badge cites no study. No
/// format placeholders.
pub const KEY_EVIDENCE_NO_BASIS_TRANSPOSED: &str = "messaging.evidence.no_basis_transposed";

/// The one chip a badge earns, by name: what a client words as
/// `messaging.evidence.<kind>` in its reader's language, whatever language the
/// turn that saved the card spoke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceChipKind {
    /// The session follows coaching convention; no trial covers it.
    Convention,
    /// Its trials ran in another sport.
    Transposed,
    /// Every study in this sport ran a single session.
    AcuteOnly,
    /// Every study in this sport had no comparator.
    UncontrolledOnly,
    /// Its studies were both single-session and uncontrolled.
    AcuteUncontrolled,
}

impl EvidenceChipKind {
    /// The chip's catalogue key.
    #[must_use]
    const fn key(self) -> &'static str {
        match self {
            Self::Convention => KEY_EVIDENCE_CONVENTION,
            Self::Transposed => KEY_EVIDENCE_TRANSPOSED,
            Self::AcuteOnly => KEY_EVIDENCE_ACUTE_ONLY,
            Self::UncontrolledOnly => KEY_EVIDENCE_UNCONTROLLED_ONLY,
            Self::AcuteUncontrolled => KEY_EVIDENCE_ACUTE_UNCONTROLLED,
        }
    }
}

/// The one chip `badge` earns, or `None` for a session trialled in this sport
/// without a qualifier.
///
/// Weaker evidence wins the slot: convention, then a transposed trial, then
/// the qualifiers of a trial in this sport, both together in one chip.
#[must_use]
pub const fn evidence_chip_kind(badge: &EvidenceBadge) -> Option<EvidenceChipKind> {
    match badge.status {
        EvidenceStatus::Convention => Some(EvidenceChipKind::Convention),
        EvidenceStatus::Transposed => Some(EvidenceChipKind::Transposed),
        EvidenceStatus::Trialled => match (badge.acute_only, badge.uncontrolled_only) {
            (true, true) => Some(EvidenceChipKind::AcuteUncontrolled),
            (true, false) => Some(EvidenceChipKind::AcuteOnly),
            (false, true) => Some(EvidenceChipKind::UncontrolledOnly),
            (false, false) => None,
        },
    }
}

/// The key of the one chip `badge` earns; see [`evidence_chip_kind`].
fn evidence_chip_key(badge: &EvidenceBadge) -> Option<&'static str> {
    evidence_chip_kind(badge).map(EvidenceChipKind::key)
}

/// Why a badge cites no study, by name: what a client words as
/// `messaging.evidence.<kind>` in its reader's language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceNoteKind {
    /// No trial covers the session; it follows coaching convention.
    NoBasisConvention,
    /// Its trials ran in another sport and it arrives here by transposition.
    NoBasisTransposed,
}

impl EvidenceNoteKind {
    /// The note's catalogue key.
    #[must_use]
    const fn key(self) -> &'static str {
        match self {
            Self::NoBasisConvention => KEY_EVIDENCE_NO_BASIS_CONVENTION,
            Self::NoBasisTransposed => KEY_EVIDENCE_NO_BASIS_TRANSPOSED,
        }
    }
}

/// Why `badge` cites no study, by its status: what its chip opens on in place
/// of a citation. `None` when the badge cites one, and for a trialled badge,
/// which always cites its trial.
#[must_use]
pub const fn evidence_note_kind(badge: &EvidenceBadge) -> Option<EvidenceNoteKind> {
    if badge.basis.is_some() {
        return None;
    }
    match badge.status {
        EvidenceStatus::Convention => Some(EvidenceNoteKind::NoBasisConvention),
        EvidenceStatus::Transposed => Some(EvidenceNoteKind::NoBasisTransposed),
        EvidenceStatus::Trialled => None,
    }
}

/// The chip `badge` earns, in `locale`'s words; `None` for an unqualified
/// trial, and for a chip whose string is missing.
#[must_use]
pub fn evidence_chip_label(
    strings: &MessagingStringsRegistry,
    locale: &str,
    badge: &EvidenceBadge,
) -> Option<String> {
    let label = strings.get(evidence_chip_key(badge)?, locale);
    (!label.is_empty()).then_some(label)
}

/// Why `badge` cites no study, by its status, in `locale`'s words: what its
/// chip opens on in place of a citation.
///
/// `None` when the badge cites one (the citation is what it opens on), for a
/// trialled badge, which always cites its trial, and when the string is
/// missing.
///
/// A day keeps the badge its save stored, so a transposed badge cites the
/// other sport's study (carnet#1551) only if its catalogue entry did when the
/// day was saved; one that cites none opens on the transposed note.
#[must_use]
pub fn evidence_note(
    strings: &MessagingStringsRegistry,
    locale: &str,
    badge: &EvidenceBadge,
) -> Option<String> {
    let note = strings.get(evidence_note_kind(badge)?.key(), locale);
    (!note.is_empty()).then_some(note)
}

/// "Counts as <purpose>" for a session an edit relabelled to `purpose`, in
/// `locale`'s words; `None` when the label or the line is missing.
#[must_use]
pub fn counts_as_line(
    strings: &MessagingStringsRegistry,
    locale: &str,
    purpose: WorkoutPurpose,
) -> Option<String> {
    let label = strings.get(&envelope_purpose_key(purpose.as_str()), locale);
    if label.is_empty() {
        return None;
    }
    let line = strings.render(KEY_EVIDENCE_COUNTS_AS, locale, &[&label]);
    (!line.trim().is_empty()).then_some(line)
}

/// The warning on a day saved outside its family's evidence, in `locale`'s
/// words; `None` when the string is missing, as for a chip.
///
/// The plan card carries it and `/plan` notes it, so both print the one
/// label (carnet#1552).
#[must_use]
pub fn outside_evidence_label(strings: &MessagingStringsRegistry, locale: &str) -> Option<String> {
    let label = strings.get(KEY_EVIDENCE_OUTSIDE, locale);
    (!label.is_empty()).then_some(label)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    fn badge(status: EvidenceStatus, acute_only: bool, uncontrolled_only: bool) -> EvidenceBadge {
        EvidenceBadge {
            status,
            acute_only,
            uncontrolled_only,
            basis: None,
            caveat: None,
        }
    }

    /// The key a client builds from a kind's wire name.
    fn client_key(kind: impl Serialize) -> String {
        let wire = serde_json::to_value(kind).unwrap();
        format!(
            "messaging.evidence.{}",
            wire.as_str().expect("a kind travels as a string")
        )
    }

    #[test]
    fn a_chip_and_its_note_travel_by_the_name_of_their_key() {
        // Every badge a day can store: whatever chip or note it earns, a client
        // words it as `messaging.evidence.<wire name>`, so that is its key.
        let mut reached = BTreeSet::new();
        for &status in EvidenceStatus::ALL {
            for (acute_only, uncontrolled_only) in
                [(false, false), (true, false), (false, true), (true, true)]
            {
                for basis in [None, Some("Rønnestad 2015 [RCT]".to_owned())] {
                    let stored = EvidenceBadge {
                        basis,
                        ..badge(status, acute_only, uncontrolled_only)
                    };
                    if let Some(kind) = evidence_chip_kind(&stored) {
                        assert_eq!(
                            client_key(kind),
                            kind.key(),
                            "bug: a client cannot word the {kind:?} chip from its name"
                        );
                        reached.insert(kind.key());
                    }
                    if let Some(note) = evidence_note_kind(&stored) {
                        assert_eq!(
                            client_key(note),
                            note.key(),
                            "bug: a client cannot word the {note:?} note from its name"
                        );
                        reached.insert(note.key());
                    }
                }
            }
        }
        assert_eq!(
            reached,
            BTreeSet::from([
                KEY_EVIDENCE_CONVENTION,
                KEY_EVIDENCE_TRANSPOSED,
                KEY_EVIDENCE_ACUTE_ONLY,
                KEY_EVIDENCE_UNCONTROLLED_ONLY,
                KEY_EVIDENCE_ACUTE_UNCONTROLLED,
                KEY_EVIDENCE_NO_BASIS_CONVENTION,
                KEY_EVIDENCE_NO_BASIS_TRANSPOSED,
            ]),
            "bug: some chip or note no stored badge reaches by name"
        );
        // Wire values the clients parse (`PLAN_EVIDENCE_KINDS`,
        // `PLAN_EVIDENCE_NOTE_KINDS` in @pierre/shared-types).
        assert_eq!(
            serde_json::to_value(EvidenceChipKind::AcuteUncontrolled).unwrap(),
            "acute_uncontrolled",
            "bug: the clients read the chip by this name"
        );
        assert_eq!(
            serde_json::to_value(EvidenceNoteKind::NoBasisConvention).unwrap(),
            "no_basis_convention",
            "bug: the clients read the note by this name"
        );
        let cited = EvidenceBadge {
            basis: Some("Rønnestad 2015 [RCT]".to_owned()),
            ..badge(EvidenceStatus::Transposed, false, false)
        };
        assert_eq!(
            evidence_note_kind(&cited),
            None,
            "bug: a cited badge opens on its citation"
        );
    }

    #[test]
    fn an_unqualified_trial_earns_no_chip() {
        assert_eq!(
            evidence_chip_key(&badge(EvidenceStatus::Trialled, false, false)),
            None,
            "bug: the card stays quiet where the evidence is strongest"
        );
    }

    #[test]
    fn the_weakest_evidence_wins_the_one_chip() {
        assert_eq!(
            evidence_chip_key(&badge(EvidenceStatus::Convention, true, true)),
            Some(KEY_EVIDENCE_CONVENTION),
            "bug: convention outranks any qualifier"
        );
        assert_eq!(
            evidence_chip_key(&badge(EvidenceStatus::Transposed, true, false)),
            Some(KEY_EVIDENCE_TRANSPOSED),
            "bug: a transposed trial outranks a qualifier"
        );
        assert_eq!(
            evidence_chip_key(&badge(EvidenceStatus::Trialled, true, false)),
            Some(KEY_EVIDENCE_ACUTE_ONLY),
            "bug: a single-session trial says so"
        );
        assert_eq!(
            evidence_chip_key(&badge(EvidenceStatus::Trialled, false, true)),
            Some(KEY_EVIDENCE_UNCONTROLLED_ONLY),
            "bug: an uncontrolled trial says so"
        );
        assert_eq!(
            evidence_chip_key(&badge(EvidenceStatus::Trialled, true, true)),
            Some(KEY_EVIDENCE_ACUTE_UNCONTROLLED),
            "bug: both qualifiers share one chip, never two"
        );
    }

    #[test]
    fn the_chip_and_the_relabel_read_in_the_readers_language() {
        let strings = MessagingStringsRegistry::new();
        assert_eq!(
            evidence_chip_label(
                &strings,
                "en",
                &badge(EvidenceStatus::Transposed, false, false)
            )
            .as_deref(),
            Some("Transposed from another sport"),
            "bug: the chip reads the catalogue's English"
        );
        assert_eq!(
            evidence_chip_label(
                &strings,
                "fr",
                &badge(EvidenceStatus::Convention, false, false)
            )
            .as_deref(),
            Some("Convention d'entraînement"),
            "bug: the chip follows the reader's locale"
        );
        assert_eq!(
            evidence_chip_label(
                &strings,
                "en",
                &badge(EvidenceStatus::Trialled, false, false)
            ),
            None,
            "bug: an unqualified trial prints no chip"
        );
        assert_eq!(
            counts_as_line(&strings, "en", WorkoutPurpose::Threshold).as_deref(),
            Some("Counts as threshold work"),
            "bug: the relabel line names the purpose in the reader's words"
        );
    }

    #[test]
    fn the_outside_warning_reads_in_the_readers_language_or_not_at_all() {
        let strings = MessagingStringsRegistry::new();
        assert_eq!(
            outside_evidence_label(&strings, "en").as_deref(),
            Some("Outside the evidence"),
            "bug: the warning reads the catalogue's English"
        );
        assert_eq!(
            outside_evidence_label(&strings, "fr").as_deref(),
            Some("Hors des données probantes"),
            "bug: the warning follows the reader's locale"
        );

        let blank = MessagingStringsRegistry::new();
        for locale in ["en", "fr"] {
            blank.update(KEY_EVIDENCE_OUTSIDE, locale, String::new(), String::new());
        }
        assert_eq!(
            outside_evidence_label(&blank, "fr"),
            None,
            "bug: a missing warning string prints nothing, never an empty label"
        );
    }

    #[test]
    fn a_chip_with_no_citation_opens_on_why_none_by_status() {
        let strings = MessagingStringsRegistry::new();
        assert_eq!(
            evidence_note(&strings, "en", &badge(EvidenceStatus::Transposed, false, false)).as_deref(),
            Some("Its trials ran in another sport; it reaches this one through the transposition rules."),
            "bug: a transposed session's sheet says its trial ran elsewhere, never that none exists"
        );
        assert_eq!(
            evidence_note(
                &strings,
                "en",
                &badge(EvidenceStatus::Convention, false, false)
            )
            .as_deref(),
            Some("No trial covers this session; it follows coaching practice."),
            "bug: a convention session's sheet says no trial covers it"
        );
        let cited = EvidenceBadge {
            basis: Some("Seiler 2007 [CT]".to_owned()),
            ..badge(EvidenceStatus::Transposed, false, false)
        };
        assert_eq!(
            evidence_note(&strings, "en", &cited),
            None,
            "bug: a badge that cites its study opens on the citation, not on a note"
        );
        assert_eq!(
            evidence_note(
                &strings,
                "en",
                &badge(EvidenceStatus::Trialled, true, false)
            ),
            None,
            "bug: a trialled badge cites its trial; it gets no note"
        );
    }
}
