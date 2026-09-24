// ABOUTME: Verifies the weekly-digest scheduler's pure decisions: the tier flag, the local slot and its week,
// ABOUTME: the group's zone, the chat it posts into, and the language that chat reads the digest in.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use chrono::{DateTime, TimeZone, Utc};
use chrono_tz::Tz;
use pierre_core::models::groups::{CoachingGroup, GroupDigestMode, GroupRespondMode};
use pierre_core::models::messaging::ChannelType;
use pierre_groups::strategies::tier::tier_enables_digest;
use pierre_routes_groups::group_digest_scheduler::{digest_room, plurality_locale, DigestRoom};
use pierre_routes_groups::group_digest_slot::{due_week, group_zone};
use uuid::Uuid;

const TORONTO: Tz = Tz::America__Toronto;
const PARIS: Tz = Tz::Europe__Paris;
const AUCKLAND: Tz = Tz::Pacific__Auckland;

fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
}

/// A group that has existed for years, so every week's slot is owed.
fn long_ago() -> DateTime<Utc> {
    utc(2020, 1, 1, 0, 0)
}

/// A wall-clock time in `zone`, as the UTC instant a tick would see.
fn local(zone: Tz, y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
    zone.with_ymd_and_hms(y, mo, d, h, mi, 0)
        .single()
        .unwrap()
        .with_timezone(&Utc)
}

#[test]
fn weekly_digest_eligibility_follows_the_tier_flag() {
    // Professional and Enterprise tiers enable weekly_digest; Starter does not.
    assert!(tier_enables_digest("professional"));
    assert!(tier_enables_digest("pro"));
    assert!(tier_enables_digest("enterprise"));
    assert!(!tier_enables_digest("starter"));
    // Unknown plans fall back to the Starter tier (no digest).
    assert!(!tier_enables_digest("free"));
}

/// Monday 2026-09-28 in Montreal: the slot opens at 08:00 and shuts at 20:00.
#[test]
fn the_slot_opens_at_eight_on_monday_and_shuts_at_twenty() {
    let zone = Some(TORONTO);
    assert_eq!(
        due_week(local(TORONTO, 2026, 9, 28, 7, 59), zone, long_ago()),
        None
    );
    assert_eq!(
        due_week(local(TORONTO, 2026, 9, 28, 8, 0), zone, long_ago()).as_deref(),
        Some("2026-W40")
    );
    assert_eq!(
        due_week(local(TORONTO, 2026, 9, 28, 19, 59), zone, long_ago()).as_deref(),
        Some("2026-W40")
    );
    assert_eq!(
        due_week(local(TORONTO, 2026, 9, 28, 20, 0), zone, long_ago()),
        None
    );
    // The 01:39 send that motivated the slot is refused outright.
    assert_eq!(
        due_week(local(TORONTO, 2026, 9, 29, 1, 39), zone, long_ago()),
        None
    );
}

/// A Monday no instance was up for is caught up later that week in daytime,
/// and every day of the week answers that same week.
#[test]
fn a_missed_monday_is_caught_up_the_same_week() {
    let zone = Some(TORONTO);
    assert_eq!(
        due_week(local(TORONTO, 2026, 9, 29, 10, 0), zone, long_ago()).as_deref(),
        Some("2026-W40"),
        "Tuesday 10:00 catches up the week"
    );
    assert_eq!(
        due_week(local(TORONTO, 2026, 10, 4, 10, 0), zone, long_ago()).as_deref(),
        Some("2026-W40"),
        "Sunday belongs to the week of the Monday before it"
    );
    assert_eq!(
        due_week(local(TORONTO, 2026, 10, 5, 8, 0), zone, long_ago()).as_deref(),
        Some("2026-W41"),
        "the next Monday opens the next week"
    );
}

/// The slot follows the zone's clock across a DST change, not a fixed offset.
#[test]
fn the_slot_follows_daylight_saving_time() {
    let zone = Some(TORONTO);
    // Autumn: EDT (UTC-4) the Monday before, EST (UTC-5) the Monday after the
    // 2026-11-01 change.
    assert_eq!(
        due_week(utc(2026, 10, 26, 12, 0), zone, long_ago()).as_deref(),
        Some("2026-W44")
    );
    assert_eq!(
        due_week(utc(2026, 11, 2, 12, 59), zone, long_ago()),
        None,
        "07:59 EST"
    );
    assert_eq!(
        due_week(utc(2026, 11, 2, 13, 0), zone, long_ago()).as_deref(),
        Some("2026-W45"),
        "08:00 EST"
    );
    // Spring: EST the Monday before, EDT the Monday after the 2026-03-08 change.
    assert_eq!(
        due_week(utc(2026, 3, 2, 12, 0), zone, long_ago()),
        None,
        "07:00 EST"
    );
    assert_eq!(
        due_week(utc(2026, 3, 9, 11, 59), zone, long_ago()),
        None,
        "07:59 EDT"
    );
    assert_eq!(
        due_week(utc(2026, 3, 9, 12, 0), zone, long_ago()).as_deref(),
        Some("2026-W11"),
        "08:00 EDT"
    );
}

/// One instant is a different hour, and can be a different week, in two
/// groups' zones.
#[test]
fn each_zone_reads_its_own_clock_and_calendar() {
    // Monday 08:00 in Paris is 02:00 in Montreal.
    let paris_morning = local(PARIS, 2026, 9, 28, 8, 0);
    assert_eq!(
        due_week(paris_morning, Some(PARIS), long_ago()).as_deref(),
        Some("2026-W40")
    );
    assert_eq!(due_week(paris_morning, Some(TORONTO), long_ago()), None);

    // Monday 08:00 in Auckland is still Sunday afternoon in Montreal: both are
    // in the slot, each in its own week.
    let auckland_monday = local(AUCKLAND, 2026, 10, 5, 8, 0);
    assert_eq!(auckland_monday, utc(2026, 10, 4, 19, 0));
    assert_eq!(
        due_week(auckland_monday, Some(AUCKLAND), long_ago()).as_deref(),
        Some("2026-W41")
    );
    assert_eq!(
        due_week(auckland_monday, Some(TORONTO), long_ago()).as_deref(),
        Some("2026-W40")
    );
}

/// With no zone on file the slot is read on UTC and opens at 12:00, which is
/// 08:00 in Montreal and 14:00 in Paris.
#[test]
fn with_no_zone_the_slot_opens_at_noon_utc() {
    assert_eq!(due_week(utc(2026, 9, 28, 11, 59), None, long_ago()), None);
    assert_eq!(
        due_week(utc(2026, 9, 28, 12, 0), None, long_ago()).as_deref(),
        Some("2026-W40")
    );
    assert_eq!(
        due_week(utc(2026, 9, 28, 19, 59), None, long_ago()).as_deref(),
        Some("2026-W40")
    );
    assert_eq!(due_week(utc(2026, 9, 28, 20, 0), None, long_ago()), None);
    // A Tuesday morning before noon UTC is still outside the no-zone window.
    assert_eq!(due_week(utc(2026, 9, 29, 9, 0), None, long_ago()), None);
}

#[test]
fn the_group_zone_is_the_owners_then_the_coachs_then_the_members() {
    let members = [
        Some("Europe/Paris"),
        Some("Europe/Paris"),
        Some("America/Toronto"),
    ];
    assert_eq!(
        group_zone(Some("America/Toronto"), Some("Europe/Paris"), members),
        Some(TORONTO),
        "the owner's zone wins"
    );
    assert_eq!(
        group_zone(Some("Mars/Olympus"), Some("Europe/Paris"), members),
        Some(PARIS),
        "an owner zone that does not parse is passed over for the coach's"
    );
    assert_eq!(
        group_zone(None, None, members),
        Some(PARIS),
        "else the zone most members share"
    );
    assert_eq!(
        group_zone(None, None, [Some("America/Toronto"), Some("Europe/Paris")]),
        Some(TORONTO),
        "the earliest-joined among equals"
    );
    assert_eq!(
        group_zone(None, Some("nowhere"), [None, Some(""), Some("UTC+2")]),
        None,
        "no usable zone anywhere: the no-zone rule"
    );
}

#[test]
fn the_room_reads_the_language_most_members_read() {
    assert_eq!(plurality_locale(["en", "fr", "en"], "fr"), "en");
    assert_eq!(
        plurality_locale(["en", "fr"], "fr"),
        "fr",
        "a tie goes to the owner"
    );
    assert_eq!(
        plurality_locale(["es", "en", "de"], "de"),
        "de",
        "a three-way tie goes to the owner too"
    );
    assert_eq!(
        plurality_locale(["en", "es", "es", "en", "pt"], "pt"),
        "en",
        "a tie the owner is not in goes to the first voted"
    );
    assert_eq!(plurality_locale([], "fr"), "fr");
}

/// A group whose digest goes to its chat, bound as given.
fn group(channel_type: Option<&str>, chat_id: Option<&str>) -> CoachingGroup {
    group_in_mode(GroupDigestMode::Chat, channel_type, chat_id)
}

fn group_in_mode(
    digest_mode: GroupDigestMode,
    channel_type: Option<&str>,
    chat_id: Option<&str>,
) -> CoachingGroup {
    CoachingGroup {
        id: Uuid::new_v4(),
        tenant_id: Uuid::new_v4().to_string(),
        name: "Les Rouleurs".to_owned(),
        description: None,
        agent_id: "agent".to_owned(),
        owner_id: Uuid::new_v4(),
        coach_user_id: None,
        peer_data_sharing: true,
        respond_mode: GroupRespondMode::default(),
        digest_mode,
        max_members: 20,
        is_active: true,
        channel_type: channel_type.map(str::to_owned),
        channel_chat_id: chat_id.map(str::to_owned),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

#[test]
fn only_a_chat_that_takes_a_proactive_message_is_posted_into() {
    let telegram = group(Some("telegram"), Some("-5284201188"));
    assert_eq!(
        digest_room(&telegram),
        Some(DigestRoom {
            channel: ChannelType::Telegram,
            chat_id: "-5284201188"
        })
    );
    let slack = group(Some("slack"), Some("C0123"));
    assert_eq!(
        digest_room(&slack).map(|r| r.channel),
        Some(ChannelType::Slack)
    );
    let discord = group(Some("discord"), Some("998877"));
    assert_eq!(
        digest_room(&discord).map(|r| r.channel),
        Some(ChannelType::Discord)
    );

    // Meta refuses a text the platform starts outside its 24-hour window.
    assert_eq!(digest_room(&group(Some("whatsapp"), Some("1203"))), None);
    assert_eq!(digest_room(&group(Some("messenger"), Some("1203"))), None);
    // A REST-created group, and a binding with no chat.
    assert_eq!(digest_room(&group(None, None)), None);
    assert_eq!(digest_room(&group(Some("telegram"), Some("  "))), None);
    assert_eq!(digest_room(&group(Some("carrier-pigeon"), Some("1"))), None);
}

/// Only a group whose digest goes to its chat is posted into: a Telegram
/// group that turned its digest off, or kept it for its managers, has no
/// room to post to however good its binding.
#[test]
fn only_the_chat_mode_posts_into_the_chat() {
    let bound = |mode| group_in_mode(mode, Some("telegram"), Some("-5284201188"));
    assert_eq!(
        digest_room(&bound(GroupDigestMode::Chat)).map(|r| r.chat_id),
        Some("-5284201188")
    );
    assert_eq!(digest_room(&bound(GroupDigestMode::Off)), None);
    assert_eq!(digest_room(&bound(GroupDigestMode::Managers)), None);
}

/// A group that never chose a mode, and a stored value the enum does not
/// know, both read as off: the digest is opt-in.
#[test]
fn the_digest_mode_defaults_to_off_and_parses_its_three_values() {
    assert_eq!(GroupDigestMode::default(), GroupDigestMode::Off);
    for mode in [
        GroupDigestMode::Off,
        GroupDigestMode::Chat,
        GroupDigestMode::Managers,
    ] {
        assert_eq!(GroupDigestMode::from_str_opt(mode.as_str()), Some(mode));
    }
    assert_eq!(GroupDigestMode::from_str_opt("weekly"), None);
    let legacy: CoachingGroup = serde_json::from_value(serde_json::json!({
        "id": Uuid::new_v4(),
        "tenant_id": "t",
        "name": "Before the mode",
        "description": null,
        "agent_id": "a",
        "owner_id": Uuid::new_v4(),
        "coach_user_id": null,
        "peer_data_sharing": true,
        "max_members": 20,
        "is_active": true,
        "channel_type": "telegram",
        "channel_chat_id": "-1",
        "created_at": Utc::now(),
        "updated_at": Utc::now(),
    }))
    .unwrap();
    assert_eq!(legacy.digest_mode, GroupDigestMode::Off);
    assert_eq!(digest_room(&legacy), None);
}

/// A group bound on Thursday owes nothing for the week it joined: its first
/// digest is the next Monday's, not a "weekly" recap minutes after the bot
/// arrived. A group created before Monday's slot owes that week, caught up
/// later in the week if the Monday tick never ran.
#[test]
fn a_week_is_owed_only_when_its_monday_slot_came_after_the_group() {
    let zone = Some(TORONTO);
    let thursday_bind = local(TORONTO, 2026, 10, 1, 10, 0);
    assert_eq!(
        due_week(local(TORONTO, 2026, 10, 1, 10, 15), zone, thursday_bind),
        None,
        "the week the group joined"
    );
    assert_eq!(
        due_week(local(TORONTO, 2026, 10, 5, 8, 0), zone, thursday_bind).as_deref(),
        Some("2026-W41"),
        "the next Monday"
    );

    let sunday_bind = local(TORONTO, 2026, 9, 27, 18, 0);
    assert_eq!(
        due_week(local(TORONTO, 2026, 9, 30, 11, 0), zone, sunday_bind).as_deref(),
        Some("2026-W40"),
        "created before Monday's slot: Wednesday catches it up"
    );

    let monday_after_slot = local(TORONTO, 2026, 9, 28, 9, 30);
    assert_eq!(
        due_week(local(TORONTO, 2026, 9, 28, 9, 45), zone, monday_after_slot),
        None,
        "created on Monday after 08:00"
    );
    assert_eq!(
        due_week(utc(2026, 10, 1, 13, 0), None, utc(2026, 9, 28, 12, 30)),
        None,
        "the no-zone slot is Monday 12:00 UTC"
    );
}
