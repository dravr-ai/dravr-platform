// ABOUTME: Response date goldens for the coach's attention reasons a source or a consent raises, and their dismissal
// ABOUTME: Pins the sharing-stopped instant, the provider-refused day, the reconnect and quiet fingerprints, and dismissed_at
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Response date goldens: the attention reasons of `/api/coach/roster` that
//! name an instant, and `POST /api/coach/attention/dismiss` (carnet#1620,
//! gate 3).
//!
//! A coach in Toronto holds three athletes:
//!
//! - **Eve** stopped sharing at a chosen instant: the consent reason carries
//!   it three times, as an evidence `at` (`Z`), as the evidence's `at`
//!   parameter and as the fingerprint (both `+00:00`);
//! - **Finn**, in `Pacific/Kiritimati`, and **Gus**, west of the date line,
//!   each rode once, forty days before today, and have a Strava connection that refuses
//!   access since a chosen instant: the reconnect reason names the day the
//!   refusal began on the athlete's calendar and is fingerprinted by the
//!   last ride; the nothing-received reason names that day too and is
//!   fingerprinted `source:` and the ride.
//!
//! Every instant is chosen, so every row is a literal: a six-digit fraction
//! where the field holds one, and whole seconds where it does not. The ride
//! alone is dated from today, because the roster reads sessions through a
//! rolling window; its time of day is chosen, so its days in both zones are
//! computed from it. The
//! dismissal is stamped with the wall clock cut to microseconds, so it is
//! compared by shape and held between two readings of the clock; the roster
//! then answers the same instant.

#![cfg(feature = "provider-strava")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::{json, Value};

use crate::common;
use crate::helpers::coach_roster_fixtures::{
    coach, coached_group, get, group_agent, join_at, person, post, ride_started, router, row,
    seed_activities, Person,
};
use crate::helpers::date_golden::{
    clock_shape, dates_in, day, instant, lies_between, on_the_clock, restamp, shifted, table,
    Instant, Today, KIRITIMATI, WEST_OF_THE_DATE_LINE,
};

const ROSTER: &str = "/api/coach/roster?limit=100";
const STOP_SHARING: &str = "UPDATE coaching_group_members SET coach_sharing_consent_at = $1 \
                            WHERE CAST(user_id AS TEXT) = $2 AND CAST(tenant_id AS TEXT) = $3";
const REFUSED_SINCE: &str = "UPDATE provider_connections SET status_changed_at = $1 \
                             WHERE CAST(user_id AS TEXT) = $2 AND CAST(tenant_id AS TEXT) = $3";

/// Eve stopped sharing at this instant.
const STOPPED: &str = "2026-11-12T23:59:59.250125";
/// How many days before today Finn and Gus each rode once: well inside the
/// roster's 180-day session window, well past its 5-day nothing-received
/// threshold.
const RODE_DAYS_AGO: i64 = 40;
/// The time of day, UTC, of that ride.
const RODE_AT: &str = "02:59:03.500250";
/// Strava has refused Finn's and Gus's tokens since this instant.
const REFUSED: &str = "2026-11-10T11:30:00";

struct Squad {
    res: Arc<ServerContext>,
    /// The ride's instant, as `2026-10-08T02:59:03.500250`.
    rode: String,
    coach: Person,
    eve: Person,
    finn: Person,
    gus: Person,
}

async fn squad() -> Squad {
    let res = common::create_test_server_resources().await.unwrap();
    let coach = coach(&res, "cole@attention-golden.test", "Cole Coach").await;
    res.common
        .repos
        .users
        .set_timezone(coach.id, "America/Toronto")
        .await
        .unwrap();
    let agent = group_agent(&res, &coach).await;
    let group = coached_group(&res, coach.tenant, coach.id, &agent, "Club").await;
    let mut people = Vec::new();
    for (name, zone) in [
        ("Eve", "America/Toronto"),
        ("Finn", KIRITIMATI),
        ("Gus", WEST_OF_THE_DATE_LINE),
    ] {
        let athlete = person(&res, &format!("{name}@attention-golden.test"), name).await;
        res.common
            .repos
            .users
            .set_timezone(athlete.id, zone)
            .await
            .unwrap();
        join_at(
            &res,
            group,
            athlete.id,
            athlete.tenant,
            true,
            instant("2026-08-02T10:00:00"),
        )
        .await;
        people.push(athlete);
    }
    let [eve, finn, gus]: [Person; 3] = people.try_into().ok().unwrap();
    let rode = format!("{}T{RODE_AT}", shifted(Today::read().utc, -RODE_DAYS_AGO));

    res.common
        .repos
        .groups
        .update_coach_sharing_consent(&group.to_string(), eve.id, false)
        .await
        .unwrap();
    restamp(
        &res.agent.database,
        STOP_SHARING,
        instant(STOPPED),
        &eve.id.to_string(),
        Some(eve.tenant),
    )
    .await;

    for athlete in [&finn, &gus] {
        seed_activities(
            &res,
            athlete,
            "strava",
            &[ride_started("old-ride", instant(&rode), 3_600)],
        )
        .await;
        res.common
            .repos
            .provider_connections
            .mark_needs_reauth(
                athlete.id,
                athlete.tenant,
                "strava",
                Some("invalid_grant"),
                instant("2099-01-01T00:00:00"),
            )
            .await
            .unwrap();
        restamp(
            &res.agent.database,
            REFUSED_SINCE,
            instant(REFUSED),
            &athlete.id.to_string(),
            Some(athlete.tenant),
        )
        .await;
    }
    Squad {
        res,
        rode,
        coach,
        eve,
        finn,
        gus,
    }
}

impl Squad {
    async fn roster(&self) -> Value {
        let (status, page) = get(&router(&self.res), ROSTER, &self.coach.auth).await;
        assert_eq!(status, 200, "{page}");
        page
    }

    fn attention<'a>(page: &'a Value, athlete: &Person) -> &'a Value {
        &row(page, athlete.id).unwrap_or_else(|| panic!("on the roster: {page}"))["attention"]
    }
}

/// The rows a source refusing since [`REFUSED`] raises for an athlete whose
/// one ride is `ridden_at`: `rode_on` and `refused` are those two instants'
/// days on the athlete's calendar.
fn source_rows(ridden_at: &str, rode_on: &str, refused: &str) -> Vec<(String, String)> {
    let ride_z = format!("{ridden_at}Z");
    let ride_offset = format!("{ridden_at}+00:00");
    let refused_z = format!("{REFUSED}Z");
    table(&[
        // nothing_received: the last session, then the dead source.
        ("/0/evidence/0/params/date", rode_on),
        ("/0/evidence/0/at", ride_z.as_str()),
        ("/0/evidence/1/params/date", refused),
        ("/0/evidence/1/at", refused_z.as_str()),
        ("/0/fingerprint#0", ride_offset.as_str()),
        // provider_reconnect: since when, then the last session.
        ("/1/evidence/0/params/date", refused),
        ("/1/evidence/0/at", refused_z.as_str()),
        ("/1/evidence/1/params/date", rode_on),
        ("/1/evidence/1/at", ride_z.as_str()),
        ("/1/fingerprint", ride_offset.as_str()),
    ])
}

#[tokio::test]
async fn the_reasons_a_stop_or_a_dead_source_raise_carry_their_instants_and_days() {
    let squad = squad().await;
    let page = squad.roster().await;

    let eve = Squad::attention(&page, &squad.eve);
    let stopped_z = format!("{STOPPED}Z");
    let stopped_offset = format!("{STOPPED}+00:00");
    assert_eq!(
        dates_in(eve),
        table(&[
            ("/0/evidence/0/at", stopped_z.as_str()),
            ("/0/evidence/0/params/at", stopped_offset.as_str()),
            ("/0/fingerprint", stopped_offset.as_str()),
        ]),
        "{eve}"
    );
    assert_eq!(
        dates_in(&json!({
            "sharing_stopped_at": row(&page, squad.eve.id).unwrap()["sharing_stopped_at"]
        })),
        table(&[("/sharing_stopped_at", stopped_z.as_str())]),
        "Eve's roster row names when she stopped sharing, with Z: {page}"
    );

    // 02:59 UTC is the same day in Kiritimati (16:59) and the day before
    // west of the line (14:59); 11:30 UTC on 2026-11-10 is the 11th and the
    // 9th.
    let ride_day = day(&squad.rode[..10]);
    for (athlete, rode, refused) in [
        (&squad.finn, ride_day.to_string(), "2026-11-11"),
        (&squad.gus, shifted(ride_day, -1), "2026-11-09"),
    ] {
        let reasons = Squad::attention(&page, athlete);
        let kinds: Vec<&str> = reasons
            .as_array()
            .unwrap()
            .iter()
            .map(|reason| reason["kind"].as_str().unwrap())
            .collect();
        assert_eq!(
            kinds,
            ["nothing_received", "provider_reconnect"],
            "{reasons}"
        );
        assert_eq!(
            reasons[0]["fingerprint"],
            format!("source:{}+00:00", squad.rode),
            "{reasons}"
        );
        assert_eq!(
            dates_in(reasons),
            source_rows(&squad.rode, &rode, refused),
            "{reasons}"
        );
    }
}

#[tokio::test]
async fn a_dismissal_answers_its_instant_in_utc_with_z_and_the_roster_keeps_it() {
    on_the_clock(|| async {
        let squad = squad().await;
        let page = squad.roster().await;
        let reconnect = Squad::attention(&page, &squad.finn)[1].clone();
        assert_eq!(reconnect["dismissed_at"], Value::Null, "{reconnect}");

        let before = Instant::now();
        let (status, body) = post(
            &router(&squad.res),
            "/api/coach/attention/dismiss",
            &squad.coach.auth,
            &json!({
                "athlete_user_id": squad.finn.id,
                "reason_id": reconnect["id"],
                "fingerprint": reconnect["fingerprint"],
            }),
        )
        .await;
        let after = Instant::now();
        assert_eq!(status, 200, "{body}");

        // Cut to the microsecond PostgreSQL keeps, with `Z`.
        let dismissed_at = body["dismissed_at"].as_str().unwrap();
        assert_eq!(
            clock_shape(dismissed_at),
            "dddd-dd-ddTdd:dd:dd.fZ",
            "{body}"
        );
        assert!(
            lies_between(dismissed_at, before, after),
            "stamped while the request ran: {body}"
        );
        assert_eq!(
            dates_in(&body),
            table(&[
                ("/dismissed_at", dismissed_at),
                ("/fingerprint", format!("{}+00:00", squad.rode).as_str()),
            ]),
            "{body}"
        );

        let page = squad.roster().await;
        let handled = &Squad::attention(&page, &squad.finn)[1];
        assert_eq!(handled["dismissed_at"], dismissed_at, "{handled}");
    })
    .await;
}
