// ABOUTME: The line a human coach's plan publish leaves in the athlete's group room, naming the days it changed
// ABOUTME: Written from the published plan alone, in the agent's voice; also posted to the group's bound chat when it has one

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # A coach's publish, said in the room (carnet#1249)
//!
//! A coach drafts a coached athlete's plan privately — in their thread about
//! the athlete, which the room never reads — and publishes it in one step.
//! The publish tells the athlete through a notice
//! ([`tell_plan_published`](crate::coach_plan_notice::tell_plan_published));
//! this tells the room: one agent-voiced entry in the group's transcript
//! naming the days that changed, or saying the plan is new when the athlete
//! had none, and the same text in the group's bound Telegram, Slack or
//! Discord chat.
//!
//! The text is built from the plan the publish wrote and the one it replaced,
//! never from anything the coach and the agent said in the thread: that
//! thread is private by construction, and this is the only path from it
//! toward the room.
//!
//! It is posted only while the room is the coach's and the athlete's alone:
//! the group is active, the publisher is its human coach, and the athlete is
//! its one athlete ([`solo_athlete`](crate::coached_athlete::solo_athlete)).
//! The line names one athlete's plan, so a room with any other member — a
//! team, a Telegram group of twelve — hears nothing of it, in its transcript
//! or its chat. Attributed to the athlete, so the room's consent rule shows
//! it to exactly the readers of the athlete's own words — the athlete, and
//! the coach while the athlete shares with them. Best-effort, like the
//! notice: the publish has landed, and a post that cannot be made is logged.
//!
//! The coach's name is whatever the coach typed, set in a line the agent
//! says: it is held to one bounded line, and the room's copy escapes it as
//! Markdown, so a name shaped like a link is drawn as the text it is.

use dravr_tronc::dates::CivilDate;
use photograveur::Locale;
use pierre_contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_PLAN_PUBLISHED_ROOM_POST, KEY_PLAN_PUBLISHED_ROOM_POST_MORE,
    KEY_PLAN_PUBLISHED_ROOM_POST_NEW,
};
use pierre_core::civil_time::weekday_short;
use pierre_core::models::groups::{CoachingGroup, NewGroupTranscriptEntry, TranscriptSpeaker};
use pierre_core::transport::TransportPolicy;
use pierre_core::untrusted::{self, ROSTER_NAME_MAX_CHARS};
use pierre_database::RepositoryRegistry;
use pierre_memory::training_plans::PlanWeek;
use tracing::{debug, info, warn};
use uuid::Uuid;

use crate::athlete_display_name::fetch_user_display_name;
use crate::coached_athlete;
use crate::group_chat_poster::{proactive_chat, GroupChatPoster};
use crate::locale::resolve_user_locale;
use crate::plan_change_history;

/// How many changed days a post names before it counts the rest.
const LISTED_DAYS: usize = 4;

/// What a publish changed for the athlete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanChange {
    /// The athlete had no live season before it: the whole plan is new.
    New,
    /// The days whose session the publish changed — written, rewritten or
    /// taken away — in date order. Never empty.
    Days(Vec<CivilDate>),
}

/// What publishing `published` over `previous` changed, or `None` when it
/// changed no day.
///
/// `previous` is the athlete's live weeks before the publish, `None` when
/// they had no live season. Only the weeks the publish wrote are compared,
/// each with the live week of the same `week_start`: weeks the coach did not
/// draft keep the athlete's version, so nothing changed there. A date counts
/// as changed when its entries differ
/// ([`plan_change_history::dates_changed`]): a session
/// rewritten, added, taken away — one of two on the date included
/// (carnet#1716) — or moved within the date. The coach's call on a session
/// (carnet#1814) is not compared: marking a day the coach's choice changes
/// nothing the athlete does, so it is no plan change to announce. A date
/// that does not parse cannot be named, and is left out (logged).
#[must_use]
pub fn plan_change(previous: Option<&[PlanWeek]>, published: &[PlanWeek]) -> Option<PlanChange> {
    let Some(previous) = previous else {
        return Some(PlanChange::New);
    };
    let days = plan_change_history::dates_changed(previous, published);
    (!days.is_empty()).then_some(PlanChange::Days(days))
}

/// One day as the room post names it: the weekday, then the short date —
/// photograveur's, the one conversation titles and chart axes write — in
/// `locale`.
fn day_label(date: CivilDate, locale: &str) -> String {
    format!(
        "{} {}",
        weekday_short(date.weekday(), locale),
        Locale::from_tag(locale).format_date(&date.to_string())
    )
}

/// The post's text in `locale`, naming `coach`; `None` when the catalogue
/// holds no text for a key it needs, so no empty line reaches the room.
///
/// The changed days are joined with commas only, the count of the unlisted
/// ones last: a locale's "and" would read "Tue, Thu and +2 more".
#[must_use]
fn room_post_text(
    registry: &MessagingStringsRegistry,
    locale: &str,
    coach: &str,
    change: &PlanChange,
) -> Option<String> {
    let text = match change {
        PlanChange::New => registry.render(KEY_PLAN_PUBLISHED_ROOM_POST_NEW, locale, &[coach]),
        PlanChange::Days(days) => {
            let mut items: Vec<String> = days
                .iter()
                .take(LISTED_DAYS)
                .map(|day| day_label(*day, locale))
                .collect();
            let unlisted = days.len().saturating_sub(LISTED_DAYS);
            if unlisted > 0 {
                let more = registry.render(
                    KEY_PLAN_PUBLISHED_ROOM_POST_MORE,
                    locale,
                    &[&unlisted.to_string()],
                );
                if more.trim().is_empty() {
                    return None;
                }
                items.push(more);
            }
            let mut text = registry.render(
                KEY_PLAN_PUBLISHED_ROOM_POST,
                locale,
                &[coach, &items.join(", ")],
            );
            // A list ending on an abbreviated month ("oct.") before the
            // sentence's own full stop would read "oct..".
            if text.ends_with("..") {
                text.pop();
            }
            text
        }
    };
    (!text.trim().is_empty()).then_some(text)
}

/// The post's two renderings of one line.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PostText {
    /// The room transcript's line, which a Markdown client draws: the coach's
    /// name escaped as Markdown text.
    room: String,
    /// The bound chat's line: the coach's name as plain text, which the group
    /// chat poster escapes for its channel along with the rest.
    chat: String,
}

/// The post in `locale`, naming the coach whose display name is
/// `coach_name`; `None` when the catalogue holds no text for it.
///
/// The name is held to one bounded line, as a roster names a person, so it
/// can neither forge a second line nor run on. Only the room's line escapes
/// it as Markdown: the poster escapes the chat's line for its channel, and
/// escaping it twice would show the backslashes.
fn post_texts(
    registry: &MessagingStringsRegistry,
    locale: &str,
    coach_name: &str,
    change: &PlanChange,
) -> Option<PostText> {
    let coach = untrusted::display_line(coach_name, ROSTER_NAME_MAX_CHARS);
    let room = room_post_text(
        registry,
        locale,
        &untrusted::markdown_literal(&coach),
        change,
    )?;
    let chat = room_post_text(registry, locale, &coach, change)?;
    Some(PostText { room, chat })
}

/// A publish to say in the athlete's room.
#[derive(Debug)]
pub struct PlanRoomPost<'change> {
    /// The group the coach coaches the athlete in.
    pub group_id: Uuid,
    /// The coach who published.
    pub coach: Uuid,
    /// The athlete whose plan it is.
    pub athlete: Uuid,
    /// What the publish changed.
    pub change: &'change PlanChange,
    /// The strictest stamp of the plans the change was read from: the post
    /// names their days, so it may go only where they may.
    pub transport_policy: TransportPolicy,
}

/// Where a room post landed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RoomPostOutcome {
    /// The entry is in the group's room transcript.
    pub in_room: bool,
    /// At least one part reached the group's bound chat.
    pub in_chat: bool,
}

/// Leave `post` in its group's room, and in the group's bound chat through
/// `poster` when it has one the platform may post into.
///
/// Nothing is posted — and that is logged — when the group is no longer
/// active or no longer coached by `post.coach`, when it holds any member
/// besides the coach and the athlete or no longer holds the athlete, when the
/// catalogue holds no text for the post, or when a read fails. The chat gets
/// the post only once the room holds it.
pub async fn post_plan_published(
    repos: &RepositoryRegistry,
    registry: &MessagingStringsRegistry,
    poster: Option<&dyn GroupChatPoster>,
    post: &PlanRoomPost<'_>,
) -> RoomPostOutcome {
    let Some(group) = room_still_theirs(repos, post).await else {
        return RoomPostOutcome::default();
    };
    let Some(text) = post_text(repos, registry, post).await else {
        return RoomPostOutcome::default();
    };
    if !write_room_entry(repos, &group, post, &text.room).await {
        return RoomPostOutcome::default();
    }
    let in_chat = match poster {
        Some(poster) => post_in_chat(poster, &group, &text.chat).await,
        None => false,
    };
    RoomPostOutcome {
        in_room: true,
        in_chat,
    }
}

/// The post's text, in the athlete's language and naming the coach as a
/// roster names them; `None` (logged) when the catalogue holds none.
async fn post_text(
    repos: &RepositoryRegistry,
    registry: &MessagingStringsRegistry,
    post: &PlanRoomPost<'_>,
) -> Option<PostText> {
    let locale = resolve_user_locale(repos.users.as_ref(), post.athlete).await;
    let coach = fetch_user_display_name(repos, post.coach).await;
    let text = post_texts(registry, &locale, &coach, post.change);
    if text.is_none() {
        warn!(group_id = %post.group_id, locale = %locale, "plan room post: the catalogue holds no text for it; not posted");
    }
    text
}

/// Write `text` into `group`'s room transcript as the agent's line about the
/// athlete; whether it was written (a failure is logged).
async fn write_room_entry(
    repos: &RepositoryRegistry,
    group: &CoachingGroup,
    post: &PlanRoomPost<'_>,
    text: &str,
) -> bool {
    let entry = NewGroupTranscriptEntry {
        group_id: &group.id.to_string(),
        tenant_id: &group.tenant_id,
        author_user_id: post.athlete,
        speaker: TranscriptSpeaker::Coach,
        content: text,
        source_conversation_id: None,
        source_message_id: None,
        transport_policy: post.transport_policy,
    };
    if let Err(e) = repos.groups.append_transcript_entry(&entry).await {
        warn!(group_id = %post.group_id, error = %e, "plan room post: the room entry could not be written; not posted");
        return false;
    }
    info!(
        group_id = %post.group_id,
        coach_user_id = %post.coach,
        athlete_user_id = %post.athlete,
        new_plan = matches!(post.change, PlanChange::New),
        "plan room post: a coach's publish was said in the athlete's room"
    );
    true
}

/// The group, when the room is still the coach's and the athlete's alone:
/// active, coached by `post.coach`, and holding the athlete as its one
/// athlete ([`solo_athlete`](coached_athlete::solo_athlete)).
async fn room_still_theirs(
    repos: &RepositoryRegistry,
    post: &PlanRoomPost<'_>,
) -> Option<CoachingGroup> {
    let group = coached_group(repos, post).await?;
    match coached_athlete::solo_athlete(repos, &group).await {
        Ok(Some(athlete)) if athlete == post.athlete => Some(group),
        Ok(_) => {
            debug!(
                group_id = %post.group_id,
                coach_user_id = %post.coach,
                athlete_user_id = %post.athlete,
                "plan room post: the room is not the coach's and the athlete's alone; not posted"
            );
            None
        }
        Err(e) => {
            warn!(group_id = %post.group_id, error = %e, "plan room post: the group's members are unreadable; not posted");
            None
        }
    }
}

/// The group, when it is active and `post.coach` is still its human coach.
async fn coached_group(
    repos: &RepositoryRegistry,
    post: &PlanRoomPost<'_>,
) -> Option<CoachingGroup> {
    let coached = repos
        .groups
        .list_groups_coached_by(post.coach)
        .await
        .inspect_err(|e| warn!(group_id = %post.group_id, error = %e, "plan room post: the coach's groups are unreadable; not posted"))
        .ok()?;
    let group = coached
        .into_iter()
        .find(|group| group.id == post.group_id && group.is_active);
    if group.is_none() {
        info!(group_id = %post.group_id, "plan room post: the publisher no longer coaches the group; not posted");
    }
    group
}

/// Post `text` into `group`'s bound chat; whether any of it arrived.
async fn post_in_chat(poster: &dyn GroupChatPoster, group: &CoachingGroup, text: &str) -> bool {
    let Some(chat) = proactive_chat(group) else {
        return false;
    };
    let channel = chat.channel;
    let parts = poster
        .post(chat.tenant_id, channel, chat.chat_id, text)
        .await;
    if parts > 0 {
        info!(group_id = %group.id, channel = %channel, parts, "plan room post: posted into the group's chat");
    }
    parts > 0
}

#[cfg(test)]
mod tests {
    use dravr_tronc::dates::Timestamp;
    use pierre_memory::training_plans::{parse_plan_date, CoachCall, PlannedDay, WeekStatus};

    use super::*;

    fn day(date: &str, workout: &str) -> PlannedDay {
        PlannedDay {
            date: date.to_owned(),
            sport: "run".to_owned(),
            workout: workout.to_owned(),
            duration_min: Some(45),
            intensity: "Z2".to_owned(),
            steps: Vec::new(),
            fueling: None,
            template_slug: None,
            template_params: None,
            template_source: None,
            template_preset: None,
            envelope: None,
            coach_original: None,
            coach_added: false,
            coach_call: None,
        }
    }

    fn week(week_start: &str, days: Vec<PlannedDay>) -> PlanWeek {
        PlanWeek {
            id: format!("w-{week_start}"),
            tenant_id: "tenant".to_owned(),
            user_id: "athlete".to_owned(),
            plan_id: "plan".to_owned(),
            week_start: week_start.to_owned(),
            focus: "base".to_owned(),
            phase_index: None,
            days,
            status: WeekStatus::Active,
            supersedes_id: None,
            adjustment_reason: String::new(),
            author_agent_id: None,
            created_at: Timestamp::now(),
            updated_at: Timestamp::now(),
        }
    }

    fn date(raw: &str) -> CivilDate {
        parse_plan_date(raw).unwrap()
    }

    /// Removing one of two sessions on a date, or reordering them, names
    /// the date; the same two sessions in the same order do not
    /// (carnet#1716).
    #[test]
    fn a_session_taken_from_a_date_with_two_names_the_date() {
        let brick = vec![day("2026-10-27", "bike"), day("2026-10-27", "run")];
        let previous = [week("2026-10-26", brick.clone())];
        for (published, named) in [
            (vec![day("2026-10-27", "bike")], true),
            (
                vec![day("2026-10-27", "run"), day("2026-10-27", "bike")],
                true,
            ),
            (brick, false),
        ] {
            let change = plan_change(Some(&previous), &[week("2026-10-26", published.clone())]);
            assert_eq!(
                change,
                named.then(|| PlanChange::Days(vec![date("2026-10-27")])),
                "bug: a date's entries are compared whole: {published:?}"
            );
        }
    }

    #[test]
    fn a_plan_the_athlete_never_had_is_new() {
        let published = [week("2026-10-26", vec![day("2026-10-27", "easy")])];
        assert_eq!(
            plan_change(None, &published),
            Some(PlanChange::New),
            "with no live season before the publish, the whole plan is new"
        );
    }

    /// A rewritten session, a new day and a day taken away are each named
    /// once, in date order; an identical day and an undrafted week are not.
    #[test]
    fn only_the_days_the_publish_changed_are_named() {
        let previous = [
            week(
                "2026-10-26",
                vec![
                    day("2026-10-27", "easy"),
                    day("2026-10-29", "tempo"),
                    day("2026-10-31", "long"),
                ],
            ),
            week("2026-11-02", vec![day("2026-11-03", "easy")]),
        ];
        let published = [week(
            "2026-10-26",
            vec![
                day("2026-10-27", "easy"),
                day("2026-10-29", "intervals"),
                day("2026-10-30", "strides"),
            ],
        )];
        assert_eq!(
            plan_change(Some(&previous), &published),
            Some(PlanChange::Days(vec![
                date("2026-10-29"),
                date("2026-10-30"),
                date("2026-10-31"),
            ])),
            "the rewritten, added and removed days, nothing else"
        );
    }

    /// A day the coach marked as their call, and one whose call they took
    /// off, are the same sessions: the publish names neither (carnet#1814).
    /// A rewritten session beside them is still named.
    #[test]
    fn a_coachs_call_on_a_day_is_no_plan_change() {
        let call = Some(CoachCall {
            by: Uuid::from_u128(7),
            at: Timestamp::now(),
        });
        let previous = [week(
            "2026-10-26",
            vec![
                day("2026-10-27", "easy"),
                PlannedDay {
                    coach_call: call,
                    ..day("2026-10-29", "tempo")
                },
                day("2026-10-31", "long"),
            ],
        )];
        let published = [week(
            "2026-10-26",
            vec![
                PlannedDay {
                    coach_call: call,
                    ..day("2026-10-27", "easy")
                },
                day("2026-10-29", "tempo"),
                PlannedDay {
                    coach_call: call,
                    ..day("2026-10-31", "long run")
                },
            ],
        )];
        assert_eq!(
            plan_change(Some(&previous), &published),
            Some(PlanChange::Days(vec![date("2026-10-31")])),
            "bug: a call made or taken off is announced as a plan change, or hides a rewritten session"
        );
    }

    #[test]
    fn a_publish_that_changed_no_day_says_nothing() {
        let weeks = [week("2026-10-26", vec![day("2026-10-27", "easy")])];
        assert_eq!(
            plan_change(Some(&weeks), &weeks),
            None,
            "an identical week changes no day"
        );
    }

    fn registry() -> MessagingStringsRegistry {
        let registry = MessagingStringsRegistry::new();
        for (key, text) in [
            (
                KEY_PLAN_PUBLISHED_ROOM_POST,
                "{0} published an update to your plan: {1}.",
            ),
            (
                KEY_PLAN_PUBLISHED_ROOM_POST_NEW,
                "{0} published your new plan.",
            ),
            (KEY_PLAN_PUBLISHED_ROOM_POST_MORE, "+{0} more"),
        ] {
            // The same text under both locales: the test pins the days and
            // the joining, never the catalogue's wording.
            for locale in ["en", "fr"] {
                registry.update(key, locale, text.to_owned(), "sha".to_owned());
            }
        }
        registry
    }

    /// Up to four days are named, joined by commas only, and the rest
    /// counted last.
    #[test]
    fn the_post_names_four_days_and_counts_the_rest() {
        let days: Vec<CivilDate> = [
            "2026-10-27",
            "2026-10-29",
            "2026-10-30",
            "2026-10-31",
            "2026-11-01",
            "2026-11-03",
        ]
        .into_iter()
        .map(date)
        .collect();
        assert_eq!(
            room_post_text(&registry(), "en", "Coach Marie", &PlanChange::Days(days)).as_deref(),
            Some(
                "Coach Marie published an update to your plan: Tue Oct 27, Thu Oct 29, \
                 Fri Oct 30, Sat Oct 31, +2 more."
            ),
            "four named days, then the count"
        );
        let two = vec![date("2026-10-27"), date("2026-10-29")];
        assert_eq!(
            room_post_text(&registry(), "en", "Coach Marie", &PlanChange::Days(two)).as_deref(),
            Some("Coach Marie published an update to your plan: Tue Oct 27, Thu Oct 29."),
            "no count when every day is named"
        );
        assert_eq!(
            room_post_text(&registry(), "en", "Coach Marie", &PlanChange::New).as_deref(),
            Some("Coach Marie published your new plan."),
            "a new plan names no day"
        );
        // The days are French, and the abbreviated month does not double the
        // sentence's full stop.
        assert_eq!(
            room_post_text(
                &registry(),
                "fr",
                "Marie",
                &PlanChange::Days(vec![date("2026-10-29")])
            )
            .as_deref(),
            Some("Marie published an update to your plan: jeu 29 oct."),
            "one full stop after an abbreviated month"
        );
    }

    #[test]
    fn a_catalogue_without_the_text_posts_nothing() {
        // The embedded catalogue carries the text, so it is blanked here, in
        // both the reader's locale and the default one it falls back to.
        let registry = MessagingStringsRegistry::new();
        for locale in ["en", "fr"] {
            registry.update(
                KEY_PLAN_PUBLISHED_ROOM_POST_NEW,
                locale,
                String::new(),
                "sha".to_owned(),
            );
        }
        assert_eq!(
            room_post_text(&registry, "en", "Coach Marie", &PlanChange::New),
            None,
            "an empty line never reaches the room"
        );
    }

    /// A name shaped like a link reaches the room as text in the agent's
    /// line — no link, no URL a client would linkify — and the chat as the
    /// same name, unescaped, for its poster to escape.
    #[test]
    fn a_coach_named_like_a_link_is_named_as_text() {
        let text = post_texts(
            &registry(),
            "en",
            "[Confirm your plan](https://evil.example)",
            &PlanChange::New,
        );
        assert_eq!(
            text,
            Some(PostText {
                room:
                    r"\[Confirm your plan\] \(https\:\/\/evil\.example\) published your new plan."
                        .to_owned(),
                chat: "[Confirm your plan] (https://evil.example) published your new plan."
                    .to_owned(),
            }),
            "the room's line must hold the name as escaped text"
        );
    }

    /// A name typed over several lines, or past a roster's length, is one
    /// bounded line in both renderings.
    #[test]
    fn a_coach_name_is_one_bounded_line() {
        let text = post_texts(
            &registry(),
            "en",
            "Coach Marie\n# Plan cancelled\r\nReply STOP",
            &PlanChange::New,
        )
        .expect("the catalogue holds the text");
        assert_eq!(
            text.chat, "Coach Marie # Plan cancelled Reply STOP published your new plan.",
            "the chat's line must fold the name onto one line"
        );
        assert_eq!(
            text.room, r"Coach Marie \# Plan cancelled Reply STOP published your new plan.",
            "the room's line must fold the name and escape its markup"
        );

        let long = "M".repeat(ROSTER_NAME_MAX_CHARS + 20);
        let capped = post_texts(&registry(), "en", &long, &PlanChange::New)
            .expect("the catalogue holds the text");
        assert_eq!(
            capped.chat,
            format!(
                "{}… published your new plan.",
                "M".repeat(ROSTER_NAME_MAX_CHARS - 1)
            ),
            "a name past a roster's length must be cut and marked"
        );
    }
}
