// ABOUTME: Response date goldens for the coach's roster: the REST read, its attention reasons, the roster tool, and the reads record
// ABOUTME: Pins each athlete's days on their own calendar in two zones a day apart, and the instants a reason carries
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Response date goldens: `/api/coach/roster` with its attention reasons,
//! `get_roster_overview`, the athlete's record of their coach's reads over
//! REST, and `/group reads` (carnet#1620, gate 3).
//!
//! A coach in Toronto holds four athletes who all live in one of two zones
//! whose offsets never change, `Pacific/Kiritimati` and `Etc/GMT+12`:
//!
//! - **Ada** has 120 daily runs at noon on her own calendar, the last six
//!   long and hard and ending on her yesterday, and a plan whose second
//!   phase opens on her today: a deep-fatigue streak;
//! - **Bo** joined two days ago, has one ride from twenty days ago and no
//!   plan: a new member with no plan, and nothing received;
//! - **Cy** rode yesterday and holds a plan that ended with its race: a plan
//!   that ended;
//! - **Dee** holds a plan whose first phase closes on her today.
//!
//! Ada's and Dee's phases turn on the athlete's own today, one at each edge:
//! a roster that reads the day in UTC puts Ada a phase back when UTC is a day
//! behind her, and Dee a phase on when it is a day ahead.
//!
//! Every date is hand-formatted from a day on the athlete's calendar, so each
//! is compared with the day computed from one reading of the clock; the
//! coach's own zone is a third one and never the answer.
//!
//! Stamped with the wall clock, and so compared by shape: the `at` of an
//! evidence line that is a reading of now.

#![cfg(feature = "provider-strava")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use axum::http::StatusCode;
use pierre_commands::group_reads::GroupReadsHandler;
use pierre_commands::{CommandHandler, ConversationRotation, PlatformCommandContext};
use pierre_core::config::profiles::FitnessLevel;
use pierre_core::models::periodization::PhaseKind;
use pierre_core::models::{
    ActivityBuilder, AnchorProvenance, SportType, ThresholdProvenance, ThresholdStatedOn,
    UserPhysiologicalProfile,
};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::{CoachReadClass, NewCoachDataRead};
use pierre_mcp_server::mcp::multitenant::ProviderToolRouter;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::training_plans::{GoalRace, RacePriority};
use pierre_tool_runtime::context::{CONVERSATION_ID, TURN_DIRECT_MESSAGE};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::common;
use crate::helpers::coach_roster_fixtures::{
    coach, coached_group, get as roster_get, group_agent, join_at, live_plan, person, phase,
    ride_started, router, seed_activities, Person,
};
use crate::helpers::date_golden::{
    dates_in, dates_in_stamped, day, days_between, epoch, get, instant, mcp_call, mcp_refusal,
    monday_of, noon_in, on_the_clock, restamp_seconds, send, shifted, table, tool_payload,
    weekday_name, with_values, Athlete, Today, KIRITIMATI, WEST_OF_THE_DATE_LINE,
};

const ZONES: [&str; 2] = [KIRITIMATI, WEST_OF_THE_DATE_LINE];
const STRAVA: &str = "strava";
/// The mornings of deep fatigue the roster counts for Ada, whose six hard
/// runs sit at noon on her own calendar and end on her yesterday.
///
/// The roster computes her form as of her own today, each session on the day
/// it started where she lives, as `get_training_history` does: six mornings,
/// from the day after her first hard run to her today, in either zone at any
/// hour. Read on UTC's day it would be five, six or seven (carnet#1640).
const DEEP_FATIGUE_MORNINGS: i64 = 6;

struct Roster {
    coach: Person,
    group: Uuid,
    ada: Person,
    bo: Person,
    cy: Person,
    dee: Person,
}

/// The day a start at 11:30 UTC, `back` days before today's UTC date, falls
/// on in `zone`: the next day in Kiritimati, the day before west of the line.
fn started_on(today: &Today, zone: &str, back: i64) -> String {
    shifted(
        today.utc,
        if zone == KIRITIMATI {
            1 - back
        } else {
            -1 - back
        },
    )
}

async fn thresholds(res: &Arc<ServerContext>, athlete: &Person) {
    res.common
        .repos
        .user_physiological_profile
        .upsert_user_physiological_profile(
            athlete.tenant,
            athlete.id,
            &UserPhysiologicalProfile {
                user_id: athlete.id,
                vo2_max: Some(52.0),
                resting_hr: Some(50),
                max_hr: Some(190),
                lactate_threshold_percentage: Some(0.85),
                threshold_hr: None,
                age: Some(34),
                weight: Some(72.0),
                fitness_level: FitnessLevel::Advanced,
                primary_sport: SportType::Run,
                training_experience_years: Some(10),
                ftp_watts: Some(280),
                threshold_pace_sec_per_km: Some(225.0),
                hr_zones: None,
                power_zones: None,
                critical_power_watts: None,
                w_prime_joules: None,
                critical_speed_mps: None,
                d_prime_meters: None,
                threshold_provenance: ThresholdProvenance::default(),
                threshold_stated_on: ThresholdStatedOn::default(),
                anchor_provenance: AnchorProvenance::default(),
                transport_policy: TransportPolicy::AnyTransport,
                sex: None,
            },
        )
        .await
        .unwrap();
}

async fn roster_in(res: &Arc<ServerContext>, zone: &str, today: &Today) -> Roster {
    let tag = Uuid::new_v4();
    let users = &res.common.repos.users;
    let coach = coach(res, &format!("coach-{tag}@example.com"), "Cole Coach").await;
    users
        .set_timezone(coach.id, "America/Toronto")
        .await
        .unwrap();
    let agent = group_agent(res, &coach).await;
    let group = coached_group(res, coach.tenant, coach.id, &agent, "Club").await;
    let mut people = Vec::new();
    for name in ["Ada", "Bo", "Cy", "Dee"] {
        let athlete = person(res, &format!("{name}-{tag}@example.com"), name).await;
        users.set_timezone(athlete.id, zone).await.unwrap();
        people.push(athlete);
    }
    let [ada, bo, cy, dee]: [Person; 4] = people.try_into().ok().unwrap();
    let here = today.in_zone(zone);
    let monday = monday_of(here);

    // Ada: 120 daily runs, the last six long and hard, and a build phase
    // that opens on her today, after a week of base.
    join_at(
        res,
        group,
        ada.id,
        coach.tenant,
        true,
        instant("2026-08-02T10:00:00.250125"),
    )
    .await;
    let runs: Vec<_> = (1..=120)
        .map(|back| {
            let (seconds, heart_rate) = if back <= 6 {
                (14_400, 178)
            } else {
                (1_800, 120)
            };
            ActivityBuilder::new(
                format!("run-{back}"),
                format!("run {back}"),
                SportType::Run,
                instant(&noon_in(day(&shifted(here, -back)), zone)),
                seconds,
                STRAVA,
            )
            .distance_meters(6_000.0)
            .average_heart_rate(heart_rate)
            .build()
        })
        .collect();
    seed_activities(res, &ada, STRAVA, &runs).await;
    thresholds(res, &ada).await;
    let race = GoalRace {
        name: "Harricana 65".to_owned(),
        date: shifted(monday, 61),
        discipline: "trail".to_owned(),
        priority: RacePriority::A,
    };
    let phases = [
        phase(PhaseKind::Base, day(&shifted(here, -7)), 1),
        phase(PhaseKind::Build, here, 3),
    ];
    live_plan(res, &ada, &race, &[], &phases).await;

    // Bo: joined two days ago at 11:30:00.250125 UTC, one ride twenty days ago.
    join_at(
        res,
        group,
        bo.id,
        coach.tenant,
        true,
        instant(&format!("{}T11:30:00.250125", shifted(today.utc, -2))),
    )
    .await;
    let old = ride_started(
        "old-ride",
        instant(&format!("{}T11:30:00.750375", shifted(today.utc, -20))),
        3_600,
    );
    seed_activities(res, &bo, STRAVA, &[old]).await;

    // Cy: rode yesterday; the plan's one phase and its race are behind.
    join_at(
        res,
        group,
        cy.id,
        coach.tenant,
        true,
        instant("2026-08-02T10:00:00.250125"),
    )
    .await;
    let ride = ride_started(
        "ride",
        instant(&format!("{}T11:30:00", shifted(today.utc, -1))),
        3_600,
    );
    seed_activities(res, &cy, STRAVA, &[ride]).await;
    let raced = GoalRace {
        name: "Past".to_owned(),
        date: shifted(here, -10),
        discipline: "trail".to_owned(),
        priority: RacePriority::A,
    };
    let behind = phase(PhaseKind::Build, day(&shifted(here, -35)), 3);
    live_plan(res, &cy, &raced, &[], &[behind]).await;
    restamp_seconds(
        &res.agent.database,
        "UPDATE training_plans SET updated_at = $1 WHERE user_id = $2 AND CAST(tenant_id AS TEXT) = $3",
        epoch("2026-08-28T02:59:03"),
        &cy.id.to_string(),
        Some(cy.tenant),
    )
    .await;

    // Dee: a week of base that closes on her today, then a build.
    join_at(
        res,
        group,
        dee.id,
        coach.tenant,
        true,
        instant("2026-08-02T10:00:00.250125"),
    )
    .await;
    let phases = [
        phase(PhaseKind::Base, day(&shifted(here, -6)), 1),
        phase(PhaseKind::Build, day(&shifted(here, 1)), 3),
    ];
    live_plan(res, &dee, &race, &[], &phases).await;
    let ride = ride_started(
        "dee-ride",
        instant(&format!("{}T11:30:00", shifted(today.utc, -1))),
        3_600,
    );
    seed_activities(res, &dee, STRAVA, &[ride]).await;

    Roster {
        coach,
        group,
        ada,
        bo,
        cy,
        dee,
    }
}

fn row<'a>(page: &'a Value, athlete: &Person) -> &'a Value {
    page["athletes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["user_id"] == athlete.id.to_string())
        .unwrap_or_else(|| panic!("{} is on the roster: {page}", athlete.id))
}

fn rows(expected: Vec<(String, String)>) -> Vec<(String, String)> {
    let mut expected = expected;
    expected.sort();
    expected
}

fn owned(expected: &[(&str, String)]) -> Vec<(String, String)> {
    expected
        .iter()
        .map(|(pointer, text)| ((*pointer).to_owned(), text.clone()))
        .collect()
}

#[tokio::test]
async fn the_roster_dates_each_athlete_on_their_own_calendar() {
    on_the_clock(|| async {
        let res = common::create_test_server_resources().await.unwrap();
        for zone in ZONES {
            let (today, roster, page) = loop {
                let today = Today::read();
                let roster = roster_in(&res, zone, &today).await;
                let (status, page) =
                    roster_get(&router(&res), "/api/coach/roster", &roster.coach.auth).await;
                assert_eq!(status, 200, "{page}");
                if today.holds() {
                    break (today, roster, page);
                }
            };
            let here = today.in_zone(zone);
            let monday = monday_of(here);
            // The days since a session are days of the athlete's calendar,
            // from the one it started on to her today: they turn at her
            // midnight, never at the hour it started (carnet#1640).
            let since_run = 1;
            let since_ride = days_between(day(&started_on(&today, zone, 20)), here);

            // Ada: the form is as of her today, her last session the day it
            // started on where she lives, and she is in the build that opens
            // today, the plan's second week. The season ends the day after its
            // race.
            let ada = row(&page, &roster.ada);
            let last_run = shifted(here, -1);
            // The six hard runs end on her yesterday, so the streak that
            // ends on her today has a known length at any hour.
            let mornings = DEEP_FATIGUE_MORNINGS;
            let mut expected = owned(&[
                ("/training/form/as_of", shifted(here, 0)),
                ("/training/last_session/date", last_run.clone()),
                (
                    "/training/last_activity_per_source/strava",
                    last_run.clone(),
                ),
                // Her yesterday: one day, in either zone at any hour.
                ("/training/days_since_last_activity", since_run.to_string()),
                ("/plan/goal_race/date", shifted(monday, 61)),
                ("/plan/ends", shifted(monday, 62)),
                ("/plan/phases/0/start", shifted(here, -7)),
                ("/plan/phases/0/end", shifted(here, 0)),
                ("/plan/phases/1/start", shifted(here, 0)),
                ("/plan/phases/1/end", shifted(here, 21)),
                ("/plan/phase", "\"build\"".to_owned()),
                ("/plan/week", "2".to_owned()),
                ("/season/race/date", shifted(monday, 61)),
                ("/season/ends", shifted(monday, 62)),
                // The streak is named by the day it began.
                ("/attention/0/fingerprint", shifted(here, 1 - mornings)),
            ]);
            let mut stamped = Vec::new();
            for morning in 0..mornings {
                expected.push((
                    format!("/attention/0/evidence/{morning}/params/date"),
                    shifted(here, -morning),
                ));
                expected.push((
                    format!("/attention/0/evidence/{morning}/at"),
                    "dddd-dd-ddTdd:dd:dd.fZ".to_owned(),
                ));
                stamped.push(format!("/attention/0/evidence/{morning}/at"));
            }
            expected.push((
                format!("/attention/0/evidence/{mornings}/params/date"),
                last_run,
            ));
            expected.push((
                format!("/attention/0/evidence/{mornings}/at"),
                format!("{}Z", noon_in(day(&shifted(here, -1)), zone)),
            ));
            let stamped: Vec<&str> = stamped.iter().map(String::as_str).collect();
            assert_eq!(
                with_values(
                    dates_in_stamped(ada, &stamped),
                    ada,
                    &[
                        "/training/days_since_last_activity",
                        "/plan/phase",
                        "/plan/week"
                    ]
                ),
                rows(expected),
                "{zone}: {ada}"
            );

            // Dee: still in the base week that closes today.
            let dee = &row(&page, &roster.dee)["plan"];
            assert_eq!(
                with_values(dates_in(dee), dee, &["/phase", "/week"]),
                rows(owned(&[
                    ("/goal_race/date", shifted(monday, 61)),
                    ("/ends", shifted(monday, 62)),
                    ("/phases/0/start", shifted(here, -6)),
                    ("/phases/0/end", shifted(here, 1)),
                    ("/phases/1/start", shifted(here, 1)),
                    ("/phases/1/end", shifted(here, 22)),
                    ("/phase", "\"base\"".to_owned()),
                    ("/week", "1".to_owned()),
                ])),
                "{zone}: {dee}"
            );

            // Bo: joined on the day 11:30:00.250125 UTC two days ago was for him,
            // and that instant, with an offset, names the reason. His one ride
            // names the other, after `quiet:`.
            let bo = row(&page, &roster.bo);
            let joined = started_on(&today, zone, 2);
            let joined_at = format!("{}T11:30:00.250125", shifted(today.utc, -2));
            let rode = started_on(&today, zone, 20);
            let rode_at = format!("{}T11:30:00.750375", shifted(today.utc, -20));
            assert_eq!(
                with_values(
                    dates_in_stamped(
                        bo,
                        &["/attention/0/evidence/1/at", "/attention/1/evidence/1/at"]
                    ),
                    bo,
                    &[
                        "/training/days_since_last_activity",
                        "/attention/1/params/days"
                    ]
                ),
                rows(owned(&[
                    ("/training/form/as_of", shifted(here, 0)),
                    ("/training/last_activity_per_source/strava", rode.clone()),
                    ("/training/days_since_last_activity", since_ride.to_string()),
                    ("/attention/0/params/date", joined.clone()),
                    ("/attention/0/evidence/0/params/date", joined),
                    ("/attention/0/evidence/0/at", format!("{joined_at}Z")),
                    (
                        "/attention/0/evidence/1/at",
                        "dddd-dd-ddTdd:dd:dd.fZ".to_owned()
                    ),
                    ("/attention/0/fingerprint", format!("{joined_at}+00:00")),
                    ("/attention/1/params/days", since_ride.to_string()),
                    ("/attention/1/evidence/0/params/date", rode),
                    ("/attention/1/evidence/0/at", format!("{rode_at}Z")),
                    (
                        "/attention/1/evidence/1/at",
                        "dddd-dd-ddTdd:dd:dd.fZ".to_owned()
                    ),
                    ("/attention/1/fingerprint#0", format!("{rode_at}+00:00")),
                ])),
                "{zone}: {bo}"
            );

            // Cy: the plan ended with its race, ten days before her today; the
            // reason is named by the plan and that day, and stamped with the
            // second the plan was last written.
            let cy = row(&page, &roster.cy);
            let ridden = started_on(&today, zone, 1);
            assert_eq!(
                with_values(dates_in(cy), cy, &["/plan/week"]),
                rows(owned(&[
                    ("/training/form/as_of", shifted(here, 0)),
                    ("/training/last_session/date", ridden.clone()),
                    ("/training/last_activity_per_source/strava", ridden),
                    ("/plan/goal_race/date", shifted(here, -10)),
                    ("/plan/ends", shifted(here, -9)),
                    ("/plan/phases/0/start", shifted(here, -35)),
                    ("/plan/phases/0/end", shifted(here, -14)),
                    ("/plan/week", "null".to_owned()),
                    ("/season/race/date", shifted(here, -10)),
                    ("/season/ends", shifted(here, -9)),
                    ("/attention/0/params/date", shifted(here, -10)),
                    ("/attention/0/evidence/0/params/date", shifted(here, -10)),
                    (
                        "/attention/0/evidence/0/at",
                        "2026-08-28T02:59:03Z".to_owned()
                    ),
                    ("/attention/0/fingerprint#0", shifted(here, -10)),
                ])),
                "{zone}: {cy}"
            );
        }
    })
    .await;
}

#[tokio::test]
async fn the_roster_tool_names_each_recent_session_by_its_day_and_weekday() {
    let res = common::create_test_server_resources().await.unwrap();
    for zone in ZONES {
        let (today, payload) = loop {
            let today = Today::read();
            let roster = roster_in(&res, zone, &today).await;
            let as_coach = Athlete {
                user_id: roster.coach.id,
                tenant: roster.coach.tenant,
                token: String::new(),
            };
            // The coach's own direct thread: the turn that offers the tool.
            let payload = TURN_DIRECT_MESSAGE
                .scope(
                    Some(true),
                    CONVERSATION_ID.scope(
                        None,
                        tool_payload(&res, &as_coach, "get_roster_overview", json!({})),
                    ),
                )
                .await;
            if today.holds() {
                break (today, payload);
            }
        };
        let athlete = |name: &str| {
            payload["athletes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["name"] == name)
                .unwrap_or_else(|| panic!("{name} is on the roster: {payload}"))
        };

        // Ada's three newest runs, each on the day it started where she
        // lives, with that day's name in full.
        let ada = athlete("Ada");
        let days = [1, 2, 3].map(|back| shifted(today.in_zone(zone), -back));
        let mut expected = owned(&[("/training/last_activity_per_source/strava", days[0].clone())]);
        let mut pointers = Vec::new();
        for (index, date) in days.iter().enumerate() {
            expected.push((
                format!("/training/recent_activities/{index}/date"),
                date.clone(),
            ));
            expected.push((
                format!("/training/recent_activities/{index}/weekday"),
                format!("\"{}\"", weekday_name(date)),
            ));
            pointers.push(format!("/training/recent_activities/{index}/weekday"));
        }
        // Her weeks are her calendar days too: this week is her today and the
        // six days before it, six 6 km runs; the week before, seven. Counted
        // as 24-hour spans back from now, the run seven days back would join
        // this week whenever it is before noon where she lives.
        for (pointer, value) in [
            ("/training/sessions_this_week", "6"),
            ("/training/km_this_week", "36.0"),
            ("/training/km_previous_week", "42.0"),
        ] {
            expected.push((pointer.to_owned(), value.to_owned()));
            pointers.push(pointer.to_owned());
        }
        let pinned: Vec<&str> = pointers.iter().map(String::as_str).collect();
        assert_eq!(
            with_values(dates_in(ada), ada, &pinned),
            rows(expected),
            "{zone}: {ada}"
        );

        // Bo's ride is too old to list; its day still names his source.
        let bo = athlete("Bo");
        assert_eq!(
            dates_in(bo),
            table(&[(
                "/training/last_activity_per_source/strava",
                started_on(&today, zone, 20).as_str()
            )]),
            "{zone}: {bo}"
        );
    }
}

/// The templates `/group reads` renders, as the catalogue ships them.
const READS_TEMPLATES: [(&str, &str); 4] = [
    (
        "commands.group.reads_header",
        "Your human coach's reads of your data in {0}, newest first:",
    ),
    ("commands.group.reads_line", "• {0} — {1}: {2} ({3})"),
    ("commands.group.reads_training", "training"),
    ("commands.group.reads_wellness", "wellness"),
];

#[tokio::test]
async fn a_coachs_reads_are_listed_to_the_athlete_at_the_minute_in_utc() {
    let res = common::create_test_server_resources().await.unwrap();
    let today = Today::read();
    let roster = roster_in(&res, KIRITIMATI, &today).await;
    // The record keeps twelve months, so the reads are dated from today:
    // three days ago at 02:59:03.500 UTC and four days ago at 23:59:59.
    let [newer, older] = [3, 4].map(|back| shifted(today.utc, -back));
    for (at, class) in [
        (format!("{newer}T02:59:03.500"), CoachReadClass::Training),
        (format!("{older}T23:59:59"), CoachReadClass::Wellness),
    ] {
        res.common
            .repos
            .coach_data_reads
            .record_coach_read(&NewCoachDataRead {
                tenant: roster.ada.tenant,
                athlete: roster.ada.id,
                coach: roster.coach.id,
                group: roster.group,
                tool: "get_activities",
                class,
                read_at: instant(&at),
            })
            .await
            .unwrap();
    }

    // Over REST the instant is whole, in UTC with an offset.
    let token = roster.ada.auth.trim_start_matches("Bearer ").to_owned();
    let (status, body) = send(
        ProviderToolRouter::build_http_app(&res),
        get(
            &format!("/api/groups/{}/members/me/coach-reads", roster.group),
            &token,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        dates_in(&body),
        table(&[
            (
                "/reads/0/read_at",
                format!("{newer}T02:59:03.500+00:00").as_str()
            ),
            (
                "/reads/1/read_at",
                format!("{older}T23:59:59+00:00").as_str()
            ),
        ]),
        "{body}"
    );

    // The command cuts it to the minute and says UTC, whatever the
    // athlete's zone: 02:59 UTC is 16:59 in Kiritimati.
    let registry = &res.mcp.messaging_strings_registry;
    for (key, template) in READS_TEMPLATES {
        registry.update(key, "en", template.to_owned(), String::new());
    }
    let listed = GroupReadsHandler
        .execute(&PlatformCommandContext {
            user_id: roster.ada.id,
            tenant_id: roster.ada.tenant,
            channel_type: "telegram".to_owned(),
            args: Vec::new(),
            raw_text: "/group reads".to_owned(),
            ctx: Arc::<ServerContext>::clone(&res),
            locale: "en".to_owned(),
            is_direct_message: true,
            ambient_group_fallback: true,
            conversation_id: None,
            conversation_tenant_id: roster.ada.tenant,
            sender_id: None,
            rotation: ConversationRotation::default(),
            tool_runtime: Arc::<ServerContext>::clone(&res),
        })
        .await
        .unwrap()
        .text;
    assert_eq!(
        listed,
        format!(
            "Your human coach's reads of your data in Club, newest first:\n\
             • {newer} 02:59 UTC — Cole Coach: training (get_activities)\n\
             • {older} 23:59 UTC — Cole Coach: wellness (get_activities)"
        ),
        "each read at its minute in UTC, never in the athlete's zone"
    );
}

/// The roster tool runs in the coach's own app and channels only: an MCP
/// client is refused it through `POST /mcp`, so no date of a roster reaches
/// that wire.
#[tokio::test]
async fn the_roster_tool_is_refused_over_mcp_and_answers_no_date_there() {
    let res = common::create_test_server_resources().await.unwrap();
    let today = Today::read();
    let roster = roster_in(&res, KIRITIMATI, &today).await;
    let as_coach = Athlete {
        user_id: roster.coach.id,
        tenant: roster.coach.tenant,
        token: String::new(),
    };

    let envelope = mcp_call(&res, &as_coach, "get_roster_overview", json!({})).await;

    assert_eq!(
        mcp_refusal(&envelope),
        "Error: 'get_roster_overview' is not available over this interface: it runs in Dravr's \
         own app or channels.",
        "the roster tool is refused on the MCP wire"
    );
    assert_eq!(dates_in(&envelope), table(&[]), "{envelope}");
}
