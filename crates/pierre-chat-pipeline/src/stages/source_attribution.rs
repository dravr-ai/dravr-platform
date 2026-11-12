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
//! Whether a turn drew on such data is a fact the platform holds, so the line
//! is the platform's. It reads the turn's provenance (carnet#769): every item
//! the provider-terms filters serve notes its attribution there, on the turn's
//! own task and on every executor the turn hands its accumulator to, the
//! Copilot loopback included ([`pierre_providers::ai_scope::Provenance`]).
//! When Garmin is among them, this stage appends the localized line to the
//! reply the athlete receives and the conversation keeps.
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
//! carries ([`without_echoes`]), so the line appears exactly when provenance
//! owes it, and once.

use std::collections::BTreeSet;
use std::{iter, mem};

use pierre_contremaitre::messaging_strings::KEY_REPLY_GARMIN_ATTRIBUTION;
use pierre_core::constants::oauth_providers::GARMIN_ATTRIBUTION;
use pierre_core::models::SUPPORTED_LOCALES;
use pierre_database::database::MessageRecord;
use pierre_providers::ai_scope;
use pierre_tool_runtime::coach_seat::TurnSeat;
use pierre_tool_runtime::tool_loop_io::ToolLoopResult;
use uuid::Uuid;

use super::reply_locale::resolve_banner_locale;
use crate::ChatPipelineContext;

/// Between the model's words and the attribution line.
const ATTRIBUTION_SEPARATOR: &str = "\n\n";

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
    ai_scope::current_provenance()
        .map(|provenance| {
            let members_read = provenance.members_read();
            provenance.attributions_owed(|owner| {
                reply_is_about(owner, seat, sender, |member| {
                    cards_carried.contains(&member) || members_read.contains(&member)
                })
            })
        })
        .unwrap_or_default()
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

/// `reply` with the Garmin attribution line appended when `owed` carries
/// Garmin and the model `answered`, in the reply's own language (else
/// `locale`), and without any copy of the line the model wrote itself.
/// Unchanged for an empty reply.
pub(crate) fn attribute_reply(
    ctx: &ChatPipelineContext,
    reply: String,
    owed: &BTreeSet<&'static str>,
    answered: bool,
    locale: &str,
) -> String {
    let reply = without_echoes(reply, &attribution_lines(ctx));
    if !answered || !owed.contains(GARMIN_ATTRIBUTION) || reply.trim().is_empty() {
        return reply;
    }
    let line = ctx.messaging_strings_registry.get(
        KEY_REPLY_GARMIN_ATTRIBUTION,
        &resolve_banner_locale(&reply, locale),
    );
    with_attribution(reply, &line)
}

/// Drop the attribution line from every assistant row of `history`, so the
/// model never reads it as something its replies say (carnet#1464).
pub(crate) fn strip_from_history(ctx: &ChatPipelineContext, history: &mut [MessageRecord]) {
    let lines = attribution_lines(ctx);
    for row in history.iter_mut().filter(|row| row.role == "assistant") {
        row.content = without_echoes(mem::take(&mut row.content), &lines);
    }
}

/// The attribution line in every supported locale.
fn attribution_lines(ctx: &ChatPipelineContext) -> Vec<String> {
    SUPPORTED_LOCALES
        .iter()
        .filter_map(|locale| {
            ctx.messaging_strings_registry
                .get_exact(KEY_REPLY_GARMIN_ATTRIBUTION, locale)
        })
        .filter(|line| !line.trim().is_empty())
        .collect()
}

/// `text` without any copy of one of `lines`: a line that restates one, or a
/// line that ends with one after its own words, compared on their
/// [`normalized`] forms. Unchanged when it carries none, or when it is nothing
/// but such copies, which leaves no reply to deliver without them.
fn without_echoes(text: String, lines: &[String]) -> String {
    let attributions: Vec<String> = lines.iter().map(|line| normalized(line)).collect();
    let mut stripped = false;
    let kept: Vec<&str> = text
        .lines()
        .filter_map(|line| {
            before_copy(line, &attributions).map_or(Some(line), |words| {
                stripped = true;
                (!words.is_empty()).then_some(words)
            })
        })
        .collect();
    if !stripped {
        return text;
    }
    let kept = kept.join("\n");
    let kept = kept.trim_end();
    if kept.trim().is_empty() {
        return text;
    }
    kept.to_owned()
}

/// The words of `line` before a copy of one of `attributions` that ends it,
/// trimmed (empty when the line is nothing but the copy); `None` when no copy
/// ends it. A copy starts the line or follows a space.
fn before_copy<'line>(line: &'line str, attributions: &[String]) -> Option<&'line str> {
    let starts = iter::once(0).chain(
        line.char_indices()
            .filter(|(_, c)| c.is_whitespace())
            .map(|(at, c)| at + c.len_utf8()),
    );
    starts
        .filter(|&start| start < line.len())
        .find(|&start| attributions.contains(&normalized(&line[start..])))
        .map(|start| line[..start].trim_end())
}

/// `line` as two renderings of the same sentence compare: apostrophes and
/// spaces folded to their plain forms, runs of spaces collapsed, the markdown
/// emphasis or quote marks around it and its final full stop dropped, and
/// letters lowercased.
fn normalized(line: &str) -> String {
    let folded: String = line
        .chars()
        .map(|c| match c {
            '\u{2019}' | '\u{2018}' | '\u{02BC}' | '\u{00B4}' | '`' => '\'',
            '\u{00A0}' | '\u{202F}' => ' ',
            other => other,
        })
        .collect();
    let collapsed = folded.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .trim_matches(|c: char| matches!(c, '*' | '_' | '>') || c.is_whitespace())
        .trim_end_matches('.')
        .to_lowercase()
}

/// `reply` followed by `line`, once.
fn with_attribution(reply: String, line: &str) -> String {
    let ends_with_line = reply
        .lines()
        .rev()
        .find(|last| !last.trim().is_empty())
        .is_some_and(|last| normalized(last) == normalized(line));
    if line.is_empty() || ends_with_line {
        return reply;
    }
    format!("{reply}{ATTRIBUTION_SEPARATOR}{line}")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use pierre_providers::ai_scope::{self, Provenance};
    use pierre_tool_runtime::coach_seat::TurnSeat;
    use uuid::Uuid;

    use super::{owed_attributions, with_attribution, without_echoes};

    const LINE: &str = "Insights derived in part from Garmin device-sourced data.";

    /// The French line as the registry carries it, with a typographic
    /// apostrophe.
    const LINE_FR: &str = "Analyse issue en partie de données provenant d\u{2019}appareils Garmin.";

    fn lines() -> Vec<String> {
        vec![LINE.to_owned(), LINE_FR.to_owned()]
    }

    #[test]
    fn the_line_follows_the_reply_once() {
        let once = with_attribution("Keep Monday easy.".to_owned(), LINE);
        assert_eq!(once, format!("Keep Monday easy.\n\n{LINE}"));
        assert_eq!(with_attribution(once.clone(), LINE), once, "never twice");
        assert_eq!(
            with_attribution("Keep Monday easy.".to_owned(), ""),
            "Keep Monday easy.",
            "a missing string adds nothing"
        );
    }

    // carnet#1464: the model copied the French line from an earlier reply with
    // a straight apostrophe, and the platform appended its own after it.
    #[test]
    fn a_copy_the_model_wrote_is_dropped_whatever_its_apostrophe() {
        let straight = LINE_FR.replace('\u{2019}', "'");
        for echo in [
            straight.clone(),
            LINE_FR.to_owned(),
            format!("*{straight}*"),
            format!("_{}_", LINE_FR.trim_end_matches('.')),
            straight.replace(' ', "\u{00A0}").to_uppercase(),
            LINE.to_owned(),
        ] {
            let reply = format!("Garde lundi facile.\n\n{echo}\n");
            assert_eq!(
                without_echoes(reply, &lines()),
                "Garde lundi facile.",
                "the copy `{echo}` is dropped"
            );
        }
        for echo in [
            straight.clone(),
            LINE_FR.to_owned(),
            format!("*{straight}*"),
        ] {
            assert_eq!(
                without_echoes(format!("Garde lundi facile. {echo}"), &lines()),
                "Garde lundi facile.",
                "a copy that ends the last paragraph, `{echo}`, is dropped too"
            );
        }
        let owed = with_attribution(
            without_echoes(format!("Garde lundi facile.\n\n{straight}"), &lines()),
            LINE_FR,
        );
        assert_eq!(
            owed,
            format!("Garde lundi facile.\n\n{LINE_FR}"),
            "an owed line appears exactly once, the platform's"
        );
    }

    #[test]
    fn a_reply_that_restates_no_line_is_left_as_written() {
        let reply = "Ta montre Garmin a enregistré 200 km.\n\nGarde lundi facile.\n".to_owned();
        assert_eq!(
            without_echoes(reply.clone(), &lines()),
            reply,
            "a reply that names Garmin without restating the line is the model's own words"
        );
        let quoted = format!("{LINE_FR} C'est la mention que Garmin demande.");
        assert_eq!(
            without_echoes(quoted.clone(), &lines()),
            quoted,
            "the line quoted mid-paragraph is the model's words, not a footer"
        );
        assert_eq!(
            without_echoes(LINE_FR.to_owned(), &lines()),
            LINE_FR,
            "a reply that is nothing but the line keeps it, rather than going out empty"
        );
    }

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
                    ai_scope::note_member_read(subject);
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
                ai_scope::note_member_read(member);
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
                ai_scope::note_member_read(Uuid::new_v4());
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
