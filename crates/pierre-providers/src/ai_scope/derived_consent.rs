// ABOUTME: The AI consents content derived from an athlete's data owes: recorded as it is served, applied when it is read back
// ABOUTME: A withdrawn key keeps every row naming it out of a model's prompt; Garmin never feeds a fact or summary (carnet#1157, #1158)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The consents a derived row owes (carnet#1157, #1158).
//!
//! Whatever a derivation serves of a provider whose AI use the athlete
//! consents to — an item of WHOOP's or Garmin's, a stored row naming them —
//! adds that consent's key to its [`Provenance`](super::Provenance). The row
//! it writes names those keys ([`derived_consent_sources`]). Read back for a
//! model, a row naming a key the athlete has withdrawn, or never gave, is
//! refused ([`admit_consent_sources`]); so is a memory fact or a compaction
//! summary naming a key kept out of the agent's memory
//! ([`KEPT_OUT_OF_AI_MEMORY`]). The athlete's own reads cross no gate, so
//! they always see their own rows. The parent module re-exports the public
//! surface.

use std::iter::once;

use pierre_core::ai_policy::Withheld;
use pierre_core::constants::oauth_providers::{
    consent_key, is_consent_class, kept_out_of_ai_memory, NoticeKind,
};
use pierre_core::transport::TransportPolicy;

#[cfg(doc)]
use pierre_core::constants::oauth_providers::KEPT_OUT_OF_AI_MEMORY;

use super::{
    admit_derived, current_provenance, exposure, flag, mark_members_only_served,
    names_members_only, record_withheld, serving_group_chat, withheld_providers, DISPLAY,
    FOR_MODEL, PROVENANCE, UNFILTERED,
};

/// Record on the running derivation the AI-consent key covering an item
/// served from `provider` (upstream `source`), if one covers either.
///
/// Not under [`unfiltered`](super::unfiltered), for the reason marking is
/// not: a cache write serves no one.
pub(super) fn note_served_consent(provider: &str, source: Option<&str>) {
    for name in once(provider).chain(source) {
        if let Some(key) = consent_key(&name.to_ascii_lowercase(), NoticeKind::AiConsent) {
            note_consent_key(key);
        }
    }
}

/// Record `key` on the running derivation, unless it is a consent class
/// (a sensitivity tier is no provider whose data a row was drawn from).
pub(super) fn note_consent_key(key: &str) {
    if is_consent_class(key) || flag(&UNFILTERED) {
        return;
    }
    let _ = PROVENANCE.try_with(|provenance| provenance.note_consent_source(key));
}

/// The AI-consent keys of everything the derivation running here served so
/// far, sorted: what a row it writes names. Empty outside any derivation.
#[must_use]
pub fn derived_consent_sources() -> Vec<String> {
    current_provenance().map_or_else(Vec::new, |provenance| provenance.consent_sources())
}

/// Whether a stored row naming `sources` may be read here, recording them on
/// the running derivation when it is.
///
/// A model's read (or a human coach's, which the athlete's AI consents
/// govern too) refuses a row naming a key the athlete has withdrawn or never
/// given. With `kept_out_applies` — a memory fact or a compaction summary —
/// a model's read also refuses one naming a key kept out of the agent's
/// memory, consent or not. A refusal is tallied like a withheld item. Every
/// other read admits the row, so the athlete always sees their own rows. An
/// admitted row's keys taint whatever is derived from it.
///
/// Whatever reads it, a row naming a key kept to Dravr's members
/// ([`names_members_only`]) is refused while the call feeds a messaging group
/// chat (carnet#1170), and taints the derivation reading it everywhere else.
#[must_use]
pub fn admit_consent_sources(sources: &[String], kept_out_applies: bool) -> bool {
    if sources.is_empty() {
        return true;
    }
    if names_members_only(sources) {
        if serving_group_chat() && !flag(&UNFILTERED) {
            record_withheld(Withheld {
                off_group_chat: 1,
                ..Withheld::default()
            });
            return false;
        }
        mark_members_only_served();
    }
    if let Some(refused) = refused_key(sources, kept_out_applies) {
        let mut withheld = Withheld {
            dropped: 1,
            ..Withheld::default()
        };
        withheld.sources.insert(refused);
        record_withheld(withheld);
        return false;
    }
    for key in sources {
        note_consent_key(key);
    }
    true
}

/// The first of `sources` a read made now must refuse, if any.
fn refused_key(sources: &[String], kept_out_applies: bool) -> Option<String> {
    if !exposure().is_some_and(|gate| gate.to_model) {
        return None;
    }
    let withheld = withheld_providers();
    let model_read = flag(&FOR_MODEL) && !flag(&DISPLAY);
    sources
        .iter()
        .map(|key| key.to_ascii_lowercase())
        .find(|key| {
            withheld.contains(key) || (kept_out_applies && model_read && kept_out_of_ai_memory(key))
        })
}

/// Keep only the stored rows whose consent sources [`admit_consent_sources`]
/// admits here.
pub fn retain_consented<T>(
    items: &mut Vec<T>,
    sources: impl Fn(&T) -> &[String],
    kept_out_applies: bool,
) {
    items.retain(|item| admit_consent_sources(sources(item), kept_out_applies));
}

/// Whether a memory fact stamped `policy` and naming `sources` may be read
/// here: its consents first ([`admit_consent_sources`], kept-out keys
/// included), then its transport stamp ([`admit_derived`]).
///
/// The gate a dossier hands its fact reads.
#[must_use]
pub fn admit_memory(policy: TransportPolicy, sources: &[String]) -> bool {
    admit_consent_sources(sources, true) && admit_derived(policy)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::ai_scope::{for_display, for_model, tracking, with_ai_consent, Provenance};

    fn keys(keys: &[&str]) -> Vec<String> {
        keys.iter().map(|key| (*key).to_owned()).collect()
    }

    fn consented_to_all() -> BTreeSet<String> {
        BTreeSet::new()
    }

    // carnet#1157: a row naming a withdrawn key never reaches a model, while
    // the athlete's own read of it is untouched.
    #[tokio::test]
    async fn a_withdrawn_key_keeps_its_rows_from_a_model_only() {
        let garmin = keys(&["garmin"]);
        let withdrawn = BTreeSet::from(["garmin".to_owned()]);
        let model_reads = for_model(with_ai_consent(withdrawn.clone(), async {
            admit_consent_sources(&garmin, false)
        }))
        .await;
        assert!(!model_reads, "a model never reads a withdrawn key's row");
        let athlete_reads =
            with_ai_consent(withdrawn, async { admit_consent_sources(&garmin, false) }).await;
        assert!(athlete_reads, "the athlete's own read is never gated");
    }

    // carnet#1157: the rule is the consent's, whichever provider it covers.
    #[tokio::test]
    async fn a_withdrawn_whoop_key_is_refused_like_garmin() {
        let withdrawn = BTreeSet::from(["whoop".to_owned()]);
        let (whoop, strava_only) = for_model(with_ai_consent(withdrawn, async {
            (
                admit_consent_sources(&keys(&["whoop"]), false),
                admit_consent_sources(&[], false),
            )
        }))
        .await;
        assert!(
            !whoop,
            "a WHOOP-stamped row stays out once WHOOP is withdrawn"
        );
        assert!(strava_only, "a row naming no consent is always read");
    }

    // carnet#1158: a fact or summary naming Garmin stays out of a model's
    // prompt even while the athlete consents; history rows do not.
    #[tokio::test]
    async fn a_kept_out_key_keeps_memory_from_a_model_even_with_consent() {
        let garmin = keys(&["garmin"]);
        let (memory, history) = for_model(with_ai_consent(consented_to_all(), async {
            (
                admit_consent_sources(&garmin, true),
                admit_consent_sources(&garmin, false),
            )
        }))
        .await;
        assert!(!memory, "no model reads a Garmin-derived fact or summary");
        assert!(history, "a consented Garmin history row is replayed");
        let displayed = for_model(for_display(with_ai_consent(consented_to_all(), async {
            admit_consent_sources(&garmin, true)
        })))
        .await;
        assert!(displayed, "output the athlete sees is not a model's read");
    }

    // An admitted row's keys taint what is derived from it, so a reply
    // drawn from a Garmin history row names Garmin in turn.
    #[tokio::test]
    async fn an_admitted_row_taints_the_derivation_reading_it() {
        let turn = Provenance::new();
        let admitted = tracking(
            turn.clone(),
            for_model(with_ai_consent(consented_to_all(), async {
                admit_consent_sources(&keys(&["garmin"]), false)
            })),
        )
        .await;
        assert!(admitted);
        assert_eq!(turn.consent_sources(), keys(&["garmin"]));
    }

    // carnet#1170: a row derived from Garmin never feeds a group-chat post,
    // and taints a derivation everywhere else; a row naming WHOOP alone is
    // read in a group chat like any other.
    #[cfg(any(feature = "provider-garmin", feature = "provider-sciotte"))]
    #[tokio::test]
    async fn a_garmin_derived_row_stays_out_of_a_group_chat_only() {
        use crate::ai_scope::for_group_chat;

        let garmin = keys(&["garmin"]);
        let in_group = for_group_chat(
            true,
            for_model(with_ai_consent(consented_to_all(), async {
                (
                    admit_consent_sources(&garmin, false),
                    admit_consent_sources(&keys(&["whoop"]), false),
                )
            })),
        )
        .await;
        assert_eq!(
            in_group,
            (false, true),
            "bug: only the Garmin row is refused"
        );

        let turn = Provenance::new();
        let in_dm = tracking(
            turn.clone(),
            for_model(with_ai_consent(consented_to_all(), async {
                admit_consent_sources(&garmin, false)
            })),
        )
        .await;
        assert!(in_dm, "the member's own direct message reads it");
        assert!(
            turn.members_only(),
            "bug: what is derived from it may not be posted into a group chat"
        );
    }

    #[tokio::test]
    async fn a_served_garmin_item_is_recorded_under_the_garmin_key() {
        let turn = Provenance::new();
        tracking(turn.clone(), async {
            note_served_consent("garmin", None);
            note_served_consent("sciotte", Some("sciotte_garmin"));
            note_served_consent("strava", None);
            note_served_consent("intervals", Some("strava"));
        })
        .await;
        assert_eq!(
            turn.consent_sources(),
            keys(&["garmin"]),
            "the API and the sign-in capture are both Garmin; Strava owes no consent"
        );
        assert!(
            derived_consent_sources().is_empty(),
            "no derivation runs here"
        );
    }
}
