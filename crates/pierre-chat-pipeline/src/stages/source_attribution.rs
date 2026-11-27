// ABOUTME: The attribution line a coach reply owes when its turn handed the model Garmin device-sourced data
// ABOUTME: Owed only for the data of whom the reply is about; appended once by the platform, never by the model (carnet#521, #1464)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Garmin attribution on AI insights.
//!
//! Garmin's API brand guidelines (combined or derived data): any output an AI
//! model derives in part from Garmin device-sourced data carries a Garmin
//! attribution, such as "Insights derived in part from Garmin device-sourced
//! data". intervals.icu's API terms bind Dravr to those guidelines for the
//! Garmin data it relays.
//!
//! Whether a turn drew on such data is a fact the platform holds, so whether
//! the line is owed is decided here. It reads the turn's provenance
//! (carnet#769): every item the provider-terms filters serve notes its
//! attribution there, on the turn's own task and on every executor the turn
//! hands its accumulator to, the Copilot loopback included ([`pierre_providers::ai_scope::Provenance`]).
//! When Garmin is among them, dravr-colleur's
//! [`attribute_reply`](dravr_colleur::attribute_reply) appends the localized
//! line ([`pierre_services::reply_attribution`]) to the reply the athlete
//! receives and the conversation keeps.
//!
//! Each attribution is noted on the person whose data owed it, and the reply
//! owes only those of the people whose data reached the model
//! ([`owed_attributions`], carnet#1464): a coach's room message about one
//! athlete carries that athlete's roster card alone, so it is not credited to
//! Garmin because another member of the group, or the coach, records on one.
//!
//! The line is the platform's alone. A copy the model writes, having read it
//! at the end of an earlier reply, is dropped from the reply and from the
//! history the model reads, in every locale and whatever apostrophe it
//! carries ([`dravr_colleur::AttributionLines`]), so the line appears exactly
//! when provenance owes it, and once.

use std::collections::BTreeSet;

use pierre_providers::ai_scope;
use pierre_tool_runtime::coach_seat::TurnSeat;
use pierre_tool_runtime::tool_loop_io::ToolLoopResult;
use uuid::Uuid;

/// The attributions this turn's reply owes: those of what the turn served
/// about the people the reply is about, as its provenance records them; none
/// outside a tracked turn.
///
/// `sender` is the turn's sender, `seat` whose data the turn is about, and
/// `cards_carried` the members whose roster cards the prompt carried.
#[must_use]
pub(crate) fn owed_attributions(
    seat: TurnSeat,
    sender: Option<Uuid>,
    cards_carried: &BTreeSet<Uuid>,
) -> BTreeSet<&'static str> {
    ai_scope::current_provenance().map_or_default(|provenance| {
        let members_read = provenance.members_read();
        provenance.attributions_owed(|owner| {
            reply_is_about(owner, seat, sender, |member| {
                cards_carried.contains(&member) || members_read.contains(&member)
            })
        })
    })
}

/// Whether a reply on `seat` is derived from data `owner`'s reads served.
///
/// - Data whose owner no read named is owed: what it was about is not known,
///   and a Garmin attribution is never dropped where it may be owed.
/// - The sender's own data, only where the sender is the turn's subject: on a
///   coach's seat the coach's training is not what the reply is about.
/// - A member's data, when it was `before_the_model`: their roster card was in
///   the prompt, or the turn's tools read them. A coach's room message about
///   one athlete carries that athlete's card alone, so another member's
///   fetched card, which never reached the model, owes nothing.
fn reply_is_about(
    owner: Option<Uuid>,
    seat: TurnSeat,
    sender: Option<Uuid>,
    before_the_model: impl Fn(Uuid) -> bool,
) -> bool {
    match owner {
        None => true,
        Some(person) if Some(person) == sender => seat.sender_is_subject(),
        Some(person) => before_the_model(person),
    }
}

/// Whether the reply is the model's answer, which the turn's reads informed:
/// not the platform's own reconnect or blocked-for-safety text, which no insight
/// is derived into.
pub(crate) fn model_answered(result: &ToolLoopResult) -> bool {
    result.pending_provider_auth_required.is_none() && result.guardian_denied.is_none()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use pierre_core::models::CarriedRead;
    use pierre_providers::ai_scope::{self, Provenance};
    use pierre_tool_runtime::coach_seat::TurnSeat;
    use uuid::Uuid;

    use super::owed_attributions;

    /// A coach's turn in which the coach and `other` read Garmin data, and
    /// `subject` read `subject_attribution`. `read_subject` says whether a
    /// tool read the subject, and `other_card` whether `other`'s roster card
    /// went into the prompt beside the subject's.
    async fn coach_turn_owes(
        seat: TurnSeat,
        subject_attribution: Option<&'static str>,
        read_subject: bool,
        other_card: bool,
    ) -> BTreeSet<&'static str> {
        let (coach, subject, other) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let mut cards = BTreeSet::new();
        if seat == TurnSeat::Coach {
            cards.insert(subject);
            if other_card {
                cards.insert(other);
            }
        }
        ai_scope::tracking(Provenance::new(), async {
            ai_scope::reading_about(coach, async {
                ai_scope::note_attribution(Some("Garmin"));
            })
            .await;
            ai_scope::reading_about(other, async {
                ai_scope::note_attribution(Some("Garmin"));
            })
            .await;
            ai_scope::reading_about(subject, async {
                if read_subject {
                    ai_scope::note_member_read(CarriedRead::peer(subject, None));
                }
                ai_scope::note_attribution(subject_attribution);
            })
            .await;
            owed_attributions(seat, Some(coach), &cards)
        })
        .await
    }

    #[tokio::test]
    async fn a_coachs_message_about_one_athlete_owes_only_what_their_data_owes() {
        for read_subject in [false, true] {
            assert!(
                coach_turn_owes(TurnSeat::Coach, None, read_subject, false)
                    .await
                    .is_empty(),
                "read_subject {read_subject}: another member's fetched card that never reached \
                 the prompt, and the coach's own data, owe nothing"
            );
            assert_eq!(
                coach_turn_owes(TurnSeat::Coach, Some("Garmin"), read_subject, false).await,
                BTreeSet::from(["Garmin"]),
                "read_subject {read_subject}: the named athlete's own Garmin data is owed"
            );
        }
        assert!(
            coach_turn_owes(TurnSeat::AthleteThread, None, true, false)
                .await
                .is_empty(),
            "a thread about an athlete who records on no Garmin owes nothing"
        );
        assert_eq!(
            coach_turn_owes(TurnSeat::AthleteThread, Some("Garmin"), true, false).await,
            BTreeSet::from(["Garmin"]),
            "a thread about a Garmin athlete owes the line"
        );
    }

    #[tokio::test]
    async fn a_coachs_message_about_the_room_owes_every_carried_card_but_never_the_coachs_data() {
        assert_eq!(
            coach_turn_owes(TurnSeat::Coach, None, false, true).await,
            BTreeSet::from(["Garmin"]),
            "every card reached the prompt, so another member's Garmin card is owed"
        );
        let coach = Uuid::new_v4();
        let owed = ai_scope::tracking(Provenance::new(), async {
            ai_scope::reading_about(coach, async {
                ai_scope::note_attribution(Some("Garmin"));
            })
            .await;
            owed_attributions(TurnSeat::Coach, Some(coach), &BTreeSet::from([coach]))
        })
        .await;
        assert!(owed.is_empty(), "the coach's own data is never the group's");
    }

    #[tokio::test]
    async fn a_member_a_tool_read_is_owed_without_a_card() {
        let (coach, member) = (Uuid::new_v4(), Uuid::new_v4());
        let owed = ai_scope::tracking(Provenance::new(), async {
            ai_scope::reading_about(member, async {
                ai_scope::note_member_read(CarriedRead::peer(member, None));
                ai_scope::note_attribution(Some("Garmin"));
            })
            .await;
            owed_attributions(TurnSeat::Coach, Some(coach), &BTreeSet::new())
        })
        .await;
        assert_eq!(
            owed,
            BTreeSet::from(["Garmin"]),
            "a member the turn's tools read reached the model"
        );
    }

    #[tokio::test]
    async fn data_whose_owner_is_unknown_and_the_senders_own_on_their_seat_are_owed() {
        let sender = Uuid::new_v4();
        for seat in [TurnSeat::Subject, TurnSeat::Coach, TurnSeat::AthleteThread] {
            let owed = ai_scope::tracking(Provenance::new(), async {
                ai_scope::note_member_read(CarriedRead::peer(Uuid::new_v4(), None));
                ai_scope::note_attribution(Some("Garmin"));
                owed_attributions(seat, Some(sender), &BTreeSet::new())
            })
            .await;
            assert_eq!(owed, BTreeSet::from(["Garmin"]), "{seat:?}: unknown owner");
        }
        let own = ai_scope::tracking(Provenance::new(), async {
            ai_scope::reading_about(sender, async {
                ai_scope::note_attribution(Some("Garmin"));
            })
            .await;
            owed_attributions(TurnSeat::Subject, Some(sender), &BTreeSet::new())
        })
        .await;
        assert_eq!(own, BTreeSet::from(["Garmin"]), "the subject's own data");
        assert!(
            owed_attributions(TurnSeat::Subject, Some(sender), &BTreeSet::new()).is_empty(),
            "outside a tracked turn nothing is owed"
        );
    }
}
