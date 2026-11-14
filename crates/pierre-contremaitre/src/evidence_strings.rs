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

/// The key of the one chip `badge` earns, or `None` for a session trialled in
/// this sport without a qualifier.
///
/// Weaker evidence wins the slot: convention, then a transposed trial, then
/// the qualifiers of a trial in this sport, both together in one chip.
#[must_use]
fn evidence_chip_key(badge: &EvidenceBadge) -> Option<&'static str> {
    match badge.status {
        EvidenceStatus::Convention => Some(KEY_EVIDENCE_CONVENTION),
        EvidenceStatus::Transposed => Some(KEY_EVIDENCE_TRANSPOSED),
        EvidenceStatus::Trialled => match (badge.acute_only, badge.uncontrolled_only) {
            (true, true) => Some(KEY_EVIDENCE_ACUTE_UNCONTROLLED),
            (true, false) => Some(KEY_EVIDENCE_ACUTE_ONLY),
            (false, true) => Some(KEY_EVIDENCE_UNCONTROLLED_ONLY),
            (false, false) => None,
        },
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
    if badge.basis.is_some() {
        return None;
    }
    let key = match badge.status {
        EvidenceStatus::Convention => KEY_EVIDENCE_NO_BASIS_CONVENTION,
        EvidenceStatus::Transposed => KEY_EVIDENCE_NO_BASIS_TRANSPOSED,
        EvidenceStatus::Trialled => return None,
    };
    let note = strings.get(key, locale);
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

#[cfg(test)]
mod tests {
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
