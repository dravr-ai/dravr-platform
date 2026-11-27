// ABOUTME: Which reply an athlete disputes and what they supplied, read from the assembled turn
// ABOUTME: Feeds dravr-colleur's figure count, the structural half of the DisputedClaims trigger

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The turn's half of capability recovery's `DisputedClaims` trigger.
//!
//! dravr-colleur counts the figures a reply asserted ([`asserts_facts`]) and,
//! for a turn an interview owns, only those the athlete never supplied
//! ([`asserts_unsupplied_facts`]). What the reply is and what the athlete
//! supplied are facts of the assembled turn, read here: the reply is the last
//! assistant message, and the athlete supplied their own messages before it
//! (never the compaction summary, which a model wrote from assistant turns)
//! and the dossier facts fenced into the system prompt ([`dossier_facts`]).

use dravr_colleur::{asserts_facts, asserts_unsupplied_facts, dossier_facts};
use pierre_llm::{ChatMessage, MessageRole};
use pierre_services::conversation_compaction::REPLAYED_SUMMARY_PREFIX;

/// The last assistant message and its index in the assembled turn — the reply
/// the athlete is responding to.
fn previous_reply(llm_messages: &[ChatMessage]) -> Option<(usize, &ChatMessage)> {
    llm_messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, m)| matches!(m.role, MessageRole::Assistant))
}

/// Whether the reply the athlete is answering asserted concrete facts about
/// their own training.
#[must_use]
pub(super) fn previous_reply_asserted_athlete_facts(llm_messages: &[ChatMessage]) -> bool {
    previous_reply(llm_messages).is_some_and(|(_, reply)| asserts_facts(&reply.content))
}

/// [`previous_reply_asserted_athlete_facts`] for a turn an interview owns: a
/// figure the athlete supplied is not a claim.
///
/// Supplied are the dossier facts of every system message and the athlete's
/// messages before the reply. The compaction summary rides in the User role
/// but is written by a model from earlier assistant turns, so an ungrounded
/// figure the agent asserted must not come back through it as the athlete's.
#[must_use]
pub(super) fn previous_reply_asserted_unsupplied_facts(llm_messages: &[ChatMessage]) -> bool {
    let Some((at, reply)) = previous_reply(llm_messages) else {
        return false;
    };
    let supplied = llm_messages
        .iter()
        .enumerate()
        .flat_map(|(index, message)| -> Vec<&str> {
            match message.role {
                MessageRole::System => dossier_facts(&message.content).collect(),
                MessageRole::User
                    if index < at && !message.content.starts_with(REPLAYED_SUMMARY_PREFIX) =>
                {
                    vec![message.content.as_str()]
                }
                _ => Vec::new(),
            }
        });
    asserts_unsupplied_facts(&reply.content, supplied)
}

#[cfg(test)]
mod tests {
    //! Which messages the count reads. How text becomes figures is colleur's,
    //! and tested there; these pin the selection, and the OKF renderer against
    //! colleur's dossier parser.

    use dravr_colleur::USER_FACT_OPEN;
    use dravr_tronc::dates::Timestamp;
    use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
    use pierre_core::models::{Dossier, DossierFact};
    use pierre_services::memory_facts::SentenceRenderer;
    use pierre_services::okf::render_okf_bundle_default;
    use uuid::Uuid;

    use super::*;

    /// The agent's actual reply from the turn Raph corrected.
    const RECONSTRUCTION: &str = "Ça donne: mardi ta grosse sortie (161 km/2391m), \
    mercredi ce matin le Date ride (16 km/414m, plus léger), dimanche Roooadie \
    (52 km/485m), vendredi Passion rando (26 km/895m).";

    /// It reads the most recent assistant turn, which is the one the athlete is
    /// answering — not an older one further up the window.
    #[test]
    fn it_reads_the_reply_the_athlete_is_answering() {
        let messages = vec![
            ChatMessage::assistant(RECONSTRUCTION),
            ChatMessage::user("road 2 aus etait hier, mardi"),
            ChatMessage::assistant("Bonne idée, on se reparle demain."),
        ];

        assert!(
            !previous_reply_asserted_athlete_facts(&messages),
            "the last assistant turn asserted nothing; an older one must not arm \
             the trigger for it"
        );
    }

    /// An empty turn cannot assert anything, and must not panic.
    #[test]
    fn no_assistant_turn_yet_is_not_an_assertion() {
        assert!(
            !previous_reply_asserted_athlete_facts(&[]),
            "bug: an empty turn has no reply to assert anything"
        );
        assert!(
            !previous_reply_asserted_athlete_facts(&[
                ChatMessage::system("coach prompt"),
                ChatMessage::user("salut"),
            ]),
            "bug: a turn with no assistant message has no reply to assert anything"
        );
    }

    /// Figures the athlete never gave still count mid-interview, and a number the
    /// athlete only mentions AFTER the reply does not excuse it.
    #[test]
    fn an_interview_question_inventing_volume_still_asserts_facts() {
        let messages = vec![
            ChatMessage::user("Je fais surtout du trail, 25 km le dimanche."),
            ChatMessage::assistant(
                "Le mois dernier tu as couru 161 km avec 2391 m de dénivelé en 6,2h, pour des \
                 sorties de 25 km. Quelle est ta course A ?",
            ),
            ChatMessage::user("C'est faux, pas 161 km ni 2391 m."),
        ];
        assert!(
            previous_reply_asserted_unsupplied_facts(&messages),
            "bug: figures the athlete gave only after the reply must not excuse it"
        );
        assert!(
            !previous_reply_asserted_unsupplied_facts(&[]),
            "bug: an empty turn has no reply to assert anything"
        );
    }

    /// An earlier assistant turn does not vouch for a figure: an ungrounded claim
    /// repeated is still ungrounded.
    #[test]
    fn an_earlier_unsupported_claim_does_not_launder_itself() {
        let messages = vec![
            ChatMessage::system("coach prompt"),
            ChatMessage::user("Je fais du trail."),
            ChatMessage::assistant("Le mois dernier : 161 km, 2391 m, 6,2 h."),
            ChatMessage::user("Et donc ?"),
            ChatMessage::assistant("Comme dit : 161 km, 2391 m de dénivelé en 6,2 h."),
        ];
        assert!(
            previous_reply_asserted_unsupplied_facts(&messages),
            "bug: an earlier assistant turn must not supply the figures it repeats"
        );
    }

    /// The compaction summary rides in the User role but a model wrote it from
    /// earlier assistant turns, so the figures in it are not the athlete's.
    #[test]
    fn the_compaction_summary_supplies_nothing() {
        let messages = vec![
            ChatMessage::user(format!(
                "{REPLAYED_SUMMARY_PREFIX}The coach noted 161 km, 2391 m and 6,2 h last month."
            )),
            ChatMessage::user("Je fais du trail."),
            ChatMessage::assistant("Le mois dernier : 161 km, 2391 m, 6,2 h."),
        ];
        assert!(
            previous_reply_asserted_unsupplied_facts(&messages),
            "bug: the compaction summary must not supply figures as the athlete's"
        );
    }

    /// The athlete's dossier as the OKF renderer writes it into the system
    /// prompt, with `instructions` spliced in between the bundle header and the
    /// first fenced fact.
    fn rendered_dossier_prompt(goal: &str, instructions: &str) -> String {
        let mut dossier = Dossier::empty(Uuid::nil(), Uuid::nil());
        dossier.north_star.push(DossierFact {
            kind: "goal".to_owned(),
            predicate_code: "training_for".to_owned(),
            object: goal.to_owned(),
            confidence: 0.9,
            source: "onboarding".to_owned(),
            updated_at: Timestamp::now(),
            valid_until: None,
            stale: false,
        });
        let strings = MessagingStringsRegistry::new();
        let bundle = render_okf_bundle_default(&dossier, SentenceRenderer::new(&strings, "en"))
            .expect("a dossier holding a fact renders a bundle");
        let fact_open = format!("{USER_FACT_OPEN} ");
        assert!(
            bundle.contains(&fact_open),
            "the bundle fences its fact: {bundle}"
        );
        format!(
            "agent persona{}",
            bundle.replacen(&fact_open, &format!("{instructions}\n{fact_open}"), 1)
        )
    }

    /// The bundle header names the `<user_fact>` tag in prose; only the fenced
    /// facts after it are the athlete's, never the text around them.
    #[test]
    fn only_well_formed_dossier_facts_supply_figures() {
        let prompt = rendered_dossier_prompt(
            "Ultra 150 km on 2027-06-05",
            "Call at most 30 tools, 9 per turn, for 7 h.",
        );
        assert!(
            prompt.contains("`<user_fact>` tags"),
            "the bundle header names the tag in prose before the facts: {prompt}"
        );
        let invented = vec![
            ChatMessage::system(prompt.clone()),
            ChatMessage::user("Salut"),
            ChatMessage::assistant("Tu fais 30 km, 9 sorties, 7 h par semaine."),
        ];
        assert!(
            previous_reply_asserted_unsupplied_facts(&invented),
            "bug: numbers in the prompt's prose outside a fenced fact are not the athlete's"
        );

        let restated = vec![
            ChatMessage::system(prompt),
            ChatMessage::user("Salut"),
            ChatMessage::assistant("Ton ultra de 150 km le 5 juin 2027, et 150 km encore."),
        ];
        assert!(
            !previous_reply_asserted_unsupplied_facts(&restated),
            "bug: a figure from a fenced dossier fact was supplied by the athlete"
        );
    }
}
