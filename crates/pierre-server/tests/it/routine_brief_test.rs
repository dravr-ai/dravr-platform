// ABOUTME: The coach morning brief end to end: who it lists, who it names as left out, an empty morning, acting on it
// ABOUTME: Drives the routine engine's wakes over a real roster, and the brief's routes in To handle (carnet#1791)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Routines P2, the coach morning brief (carnet#1791, ADR-036).
//!
//! The engine gives every coach a brief and fires it on the coach's clock;
//! these tests deliver its wakes at their own time over a roster built the
//! way To handle reads it, and pin the issue's done-when list: only athletes
//! the brief may read are listed and the rest are named, an empty morning
//! posts nothing and says it ran, and Handled or an open within a day counts
//! once as acted on.
#![cfg(feature = "client-messaging")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use std::sync::Arc;

use axum::Router;
use dravr_tronc::dates::{Elapsed, Timestamp};
use pierre_contremaitre::messaging_strings::{
    KEY_PLATFORM_POST_MORNING_BRIEF_LINKS, KEY_PLATFORM_POST_MORNING_BRIEF_MISSED_OFF,
    KEY_PLATFORM_POST_MORNING_BRIEF_MISSED_UNREADABLE, KEY_PLATFORM_POST_MORNING_BRIEF_NOT_SHARING,
    KEY_PLATFORM_POST_MORNING_BRIEF_PROMPT, KEY_PLATFORM_POST_MORNING_BRIEF_UNREADABLE,
};
use pierre_core::models::{LeftOut, RoutineKind, RunOutcome, SUPPORTED_LOCALES};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::coach_attention::CoachAttentionRoutes;
use pierre_mcp_server::routes::coach_brief::CoachBriefRoutes;
use pierre_mcp_server::routes::coach_roster::CoachRosterRoutes;
use pierre_mcp_server::services::platform_post::PostAccess;
use pierre_mcp_server::services::routine_engine::WakeOutcome;
use pierre_mcp_server::services::routine_waker::Wake;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::check_in_post_test::{asked_for, fixture, Fixture};
use crate::helpers::axum_test::AxumTestRequest;
use crate::helpers::coach_roster_fixtures::{
    coach, coached_group, get, group_agent, join_at, person, plan_with_days, post, row,
    seed_activities, seed_ride, Person,
};
use crate::recording_post::RecordingPost;

/// The brief's words, registered whatever the pinned catalogue carries.
const BRIEF_STRINGS: [(&str, &str); 6] = [
    (
        KEY_PLATFORM_POST_MORNING_BRIEF_PROMPT,
        "Morning brief from these lines:\n{0}",
    ),
    (
        KEY_PLATFORM_POST_MORNING_BRIEF_NOT_SHARING,
        "{0} not included: sharing off",
    ),
    (
        KEY_PLATFORM_POST_MORNING_BRIEF_UNREADABLE,
        "{0} not included: unreadable",
    ),
    (KEY_PLATFORM_POST_MORNING_BRIEF_LINKS, "Open in To handle:"),
    (
        KEY_PLATFORM_POST_MORNING_BRIEF_MISSED_UNREADABLE,
        "Missed sessions not checked for {0}: calendar unreadable",
    ),
    (
        KEY_PLATFORM_POST_MORNING_BRIEF_MISSED_OFF,
        "Missed sessions not checked for {0}: switched off",
    ),
];

/// A coach with one group, on the engine's fixture.
struct Desk {
    fx: Fixture,
    coach: Person,
    group: Uuid,
}

async fn desk(tag: &str) -> Desk {
    let fx = fixture(RecordingPost::posting(true)).await;
    for locale in SUPPORTED_LOCALES {
        for (key, text) in BRIEF_STRINGS {
            fx.resources.mcp.messaging_strings_registry.update(
                key,
                locale,
                text.to_owned(),
                String::new(),
            );
        }
    }
    let res = &fx.resources;
    let coach = coach(res, &format!("casey@{tag}.test"), "Casey Coach").await;
    let agent = group_agent(res, &coach).await;
    let group = coached_group(res, coach.tenant, coach.id, &agent, "Squad").await;
    Desk { fx, coach, group }
}

/// The outcomes of the routine wakes among `delivered`.
fn routine_outcomes(delivered: Vec<(Wake, WakeOutcome)>) -> Vec<WakeOutcome> {
    delivered
        .into_iter()
        .filter(|(wake, _)| matches!(wake, Wake::Routine { .. }))
        .map(|(_, outcome)| outcome)
        .collect()
}

impl Desk {
    fn res(&self) -> &Arc<ServerContext> {
        &self.fx.resources
    }

    /// An athlete who joined a month ago, sharing or not.
    async fn athlete(&self, name: &str, shares: bool) -> Person {
        let email = format!("{}@{}.test", name.to_lowercase(), self.coach.id.simple());
        let athlete = person(self.res(), &email, name).await;
        join_at(
            self.res(),
            self.group,
            athlete.id,
            athlete.tenant,
            shares,
            Timestamp::now() - Elapsed::days(30),
        )
        .await;
        athlete
    }

    /// A sharer whose last ride came in nine days ago: To handle lists them
    /// as nothing received.
    async fn quiet(&self, name: &str) -> Person {
        let athlete = self.athlete(name, true).await;
        seed_ride(self.res(), &athlete, 9).await;
        athlete
    }

    /// An athlete who shared, then stopped: To handle lists the stop today;
    /// the brief only counts them.
    async fn stopped_sharing(&self, name: &str) -> Person {
        let athlete = self.athlete(name, true).await;
        self.res()
            .common
            .repos
            .groups
            .update_coach_sharing_consent(&self.group.to_string(), athlete.id, false)
            .await
            .unwrap();
        athlete
    }

    /// Reconcile now, then deliver the brief's next slot alone: the one still
    /// open now, else the next to come. Returns what the brief's delivery did.
    ///
    /// A fixed window counted the wrong number of mornings at some hours: a
    /// slot still inside its window is current, so an hour after 07:00 a
    /// one-day window held today's late slot and tomorrow's. Delivering up
    /// to the slot the engine scheduled holds exactly one, whatever the hour.
    async fn next_morning(&self) -> Vec<WakeOutcome> {
        let now = Timestamp::now();
        let due_now = routine_outcomes(self.fx.run(now, now).await);
        if !due_now.is_empty() {
            return due_now;
        }
        let fire = self
            .res()
            .common
            .repos
            .routines
            .builtin_routine(self.coach.tenant, self.coach.id, RoutineKind::MorningBrief)
            .await
            .unwrap()
            .expect("bug: the engine gave the coach no brief")
            .next_fire
            .expect("bug: the brief has no next firing");
        routine_outcomes(self.fx.run_to(fire).await)
    }

    fn router(&self) -> Router {
        CoachRosterRoutes::routes(Arc::clone(self.res()))
            .merge(CoachAttentionRoutes::routes(Arc::clone(self.res())))
            .merge(CoachBriefRoutes::routes(Arc::clone(self.res())))
    }

    async fn last_outcome(&self) -> Option<(RunOutcome, u32)> {
        let repos = &self.res().common.repos;
        let brief = repos
            .routines
            .builtin_routine(self.coach.tenant, self.coach.id, RoutineKind::MorningBrief)
            .await
            .unwrap()
            .expect("bug: the engine gave the coach no brief");
        repos
            .routines
            .last_run(self.coach.tenant, brief.id)
            .await
            .unwrap()
            .map(|run| (run.outcome, run.items))
    }

    async fn last_left_out(&self) -> LeftOut {
        let repos = &self.res().common.repos;
        let brief = repos
            .routines
            .builtin_routine(self.coach.tenant, self.coach.id, RoutineKind::MorningBrief)
            .await
            .unwrap()
            .expect("bug: the engine gave the coach no brief");
        repos
            .routines
            .last_run(self.coach.tenant, brief.id)
            .await
            .unwrap()
            .expect("bug: the brief recorded no run")
            .left_out
    }

    /// Whether a brief still has an un-acted item for `athlete`: marking it
    /// here consumes it, so call this last.
    async fn still_unacted(&self, athlete: Uuid) -> bool {
        !self
            .res()
            .common
            .repos
            .routines
            .mark_listed_acted(
                self.coach.tenant,
                self.coach.id,
                athlete,
                Timestamp::now() - Elapsed::days(2),
                "probe",
                Timestamp::now(),
            )
            .await
            .unwrap()
            .is_empty()
    }
}

async fn put(router: &Router, uri: &str, auth: &str, body: &Value) -> (u16, Value) {
    let response = AxumTestRequest::put(uri)
        .header("authorization", auth)
        .json(body)
        .send(router.clone())
        .await;
    let text = response.body_text();
    (
        response.status(),
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

#[tokio::test]
async fn the_brief_lists_who_needs_the_coach_and_names_who_it_could_not_read() {
    let d = desk("lists").await;
    let ben = d.quiet("Ben").await;
    // Stopped sharing, and joined without ever sharing: the brief lists
    // neither, sends neither to the model, and counts both.
    d.stopped_sharing("Sam").await;
    d.athlete("Hugo", false).await;

    let outcomes = d.next_morning().await;
    assert!(
        outcomes.iter().any(|o| matches!(o, WakeOutcome::Posted(_))),
        "bug: the brief did not post: {outcomes:?}"
    );
    let asked = asked_for(&d.fx, d.coach.id);
    assert_eq!(
        asked.len(),
        1,
        "bug: one brief a morning, one turn: {}",
        asked.len()
    );
    let brief = &asked[0];
    assert_eq!(
        brief.access,
        PostAccess::SuggestOnly,
        "bug: a routine's turn may only suggest"
    );
    assert!(
        brief.prompt.contains("Ben") && brief.prompt.contains("nothing_received"),
        "bug: the finding never reached the prompt: {}",
        brief.prompt
    );
    assert!(
        !brief.prompt.contains("Hugo") && !brief.prompt.contains("Sam"),
        "bug: an athlete who does not share reached the model: {}",
        brief.prompt
    );
    let appendix = brief
        .appendix
        .as_deref()
        .expect("bug: the brief carried no appendix");
    assert!(
        appendix.contains("2 not included: sharing off"),
        "bug: the athlete the brief could not read was dropped silently: {appendix}"
    );
    if let Some(url) = d.res().common.config.frontend_url.as_deref() {
        assert!(
            appendix.contains(&format!(
                "{}/#home/athlete/{}",
                url.trim_end_matches('/'),
                ben.id
            )),
            "bug: the listed athlete carries no link into To handle: {appendix}"
        );
    }
    assert_eq!(
        d.last_outcome().await,
        Some((RunOutcome::Posted, 1)),
        "bug: the run did not record the post and its one item"
    );
    assert_eq!(
        d.last_left_out().await.not_sharing,
        2,
        "bug: the run did not count who the brief left out"
    );
}

#[tokio::test]
async fn a_morning_with_nobody_to_list_posts_nothing_and_says_it_ran() {
    let d = desk("empty").await;
    d.athlete("Hugo", false).await;

    let outcomes = d.next_morning().await;
    assert_eq!(
        outcomes,
        [WakeOutcome::Empty],
        "bug: an empty morning did something else"
    );
    assert!(
        asked_for(&d.fx, d.coach.id).is_empty(),
        "bug: a turn was spent on a brief with nothing in it"
    );
    assert_eq!(
        d.last_outcome().await,
        Some((RunOutcome::Empty, 0)),
        "bug: the empty morning's run was not recorded as empty"
    );
    let (status, body) = get(&d.router(), "/api/coach/brief", &d.coach.auth).await;
    assert_eq!(status, 200, "{body:#}");
    assert_eq!(
        body["last_run"]["outcome"], "empty",
        "bug: To handle cannot say the brief ran with nothing to report: {body:#}"
    );
    assert_eq!(
        body["last_run"]["not_sharing"], 1,
        "bug: an empty morning hid the athlete it could not read: {body:#}"
    );
    // Both counts are on the wire: a client refuses a run missing either.
    assert_eq!(
        body["last_run"]["unreadable"], 0,
        "bug: the run's unreadable count is missing or wrong: {body:#}"
    );
}

#[tokio::test]
async fn handled_within_a_day_counts_once_as_acted_on() {
    let d = desk("handled").await;
    let ben = d.quiet("Ben").await;
    d.next_morning().await;

    let (status, page) = get(&d.router(), "/api/coach/roster?limit=100", &d.coach.auth).await;
    assert_eq!(status, 200, "{page:#}");
    let reason = row(&page, ben.id).unwrap()["attention"][0].clone();
    let (status, body) = post(
        &d.router(),
        "/api/coach/attention/dismiss",
        &d.coach.auth,
        &json!({
            "athlete_user_id": ben.id,
            "reason_id": reason["id"],
            "fingerprint": reason["fingerprint"],
        }),
    )
    .await;
    assert_eq!(status, 200, "{body:#}");
    assert!(
        !d.still_unacted(ben.id).await,
        "bug: marking the listed athlete Handled did not count as acting on the brief"
    );
}

#[tokio::test]
async fn opening_a_listed_athlete_counts_and_an_unlisted_one_does_not() {
    let d = desk("opened").await;
    let ben = d.quiet("Ben").await;
    let cleo = d.quiet("Cleo").await;
    let hugo = d.athlete("Hugo", false).await;
    d.next_morning().await;
    assert_eq!(
        d.last_outcome().await,
        Some((RunOutcome::Posted, 2)),
        "bug: the brief did not list Ben and Cleo"
    );

    for athlete in [hugo.id, ben.id] {
        let (status, body) = post(
            &d.router(),
            "/api/coach/brief/opened",
            &d.coach.auth,
            &json!({ "athlete_user_id": athlete }),
        )
        .await;
        assert_eq!(status, 204, "{body:#}");
    }
    assert!(
        !d.still_unacted(ben.id).await,
        "bug: opening the listed athlete did not count"
    );
    assert!(
        d.still_unacted(cleo.id).await,
        "bug: opening an athlete the brief never listed, or another one it did, counted for Cleo"
    );
}

#[tokio::test]
async fn the_coach_pauses_moves_and_trims_the_brief_from_to_handle() {
    let d = desk("controls").await;
    let ben = d.athlete("Ben", true).await;
    let router = d.router();

    let (status, body) = get(&router, "/api/coach/brief", &d.coach.auth).await;
    assert_eq!(status, 200, "{body:#}");
    assert_eq!(
        body["state"], "active",
        "bug: the brief is on for every coach: {body:#}"
    );
    assert_eq!(body["hour"], 7, "{body:#}");

    let (status, body) = put(
        &router,
        "/api/coach/brief",
        &d.coach.auth,
        &json!({ "hour": 4 }),
    )
    .await;
    assert_eq!(
        status, 400,
        "bug: an hour inside the quiet night was taken: {body:#}"
    );

    let change = json!({
        "paused": true,
        "hour": 9,
        "reasons_off": ["plan_ended", "plan_ended", "consent_revoked"],
        "muted": [ben.id],
    });
    let (status, body) = put(&router, "/api/coach/brief", &d.coach.auth, &change).await;
    assert_eq!(status, 200, "{body:#}");
    assert_eq!(body["state"], "paused", "{body:#}");
    assert_eq!(body["hour"], 9, "{body:#}");
    assert_eq!(
        body["reasons_off"],
        json!(["plan_ended"]),
        "bug: a repeat, or the stop the brief never lists, was kept: {body:#}"
    );
    assert_eq!(body["muted"], json!([ben.id]), "{body:#}");
    assert!(
        body["next_run"].is_null(),
        "bug: a paused brief has a next run: {body:#}"
    );

    let (status, body) = put(
        &router,
        "/api/coach/brief",
        &d.coach.auth,
        &json!({ "reasons_off": ["not_a_reason"] }),
    )
    .await;
    assert!(
        (400..500).contains(&status),
        "bug: an unknown reason was taken: {status} {body:#}"
    );

    let (status, body) = get(&router, "/api/coach/brief", &ben.auth).await;
    assert_eq!(
        status, 403,
        "bug: an athlete reached a coach's brief: {body:#}"
    );
}

/// Whether `athlete`'s row on the coach's To handle carries the missed
/// sessions reason.
fn flags_missed(page: &Value, athlete: Uuid) -> bool {
    row(page, athlete)
        .and_then(|r| r["attention"].as_array())
        .is_some_and(|reasons| {
            reasons
                .iter()
                .any(|reason| reason["kind"] == "planned_sessions_missed")
        })
}

#[tokio::test]
async fn the_athletes_switch_takes_them_out_of_the_check_and_the_brief_says_so() {
    let d = desk("missed-off").await;
    d.quiet("Ben").await;
    // Mia's plan rode six and four days ago; her Strava, connected, holds
    // neither ride. (With no source at all nothing could come in, and the
    // check is not run.)
    let mia = d.athlete("Mia", true).await;
    seed_activities(d.res(), &mia, "strava", &[]).await;
    let today = Timestamp::now().utc_date();
    let (six, four) = (today - Elapsed::days(6), today - Elapsed::days(4));
    plan_with_days(
        d.res(),
        (mia.id, mia.tenant),
        six,
        &[(six, "ride"), (four, "ride")],
    )
    .await;

    let (status, page) = get(&d.router(), "/api/coach/roster", &d.coach.auth).await;
    assert_eq!(status, 200, "{page:#}");
    assert!(
        flags_missed(&page, mia.id),
        "bug: two missed planned days raised nothing: {page:#}"
    );

    let switched = d
        .res()
        .common
        .repos
        .groups
        .set_coach_missed_session_check(&d.group.to_string(), mia.id, false, Timestamp::now())
        .await
        .unwrap()
        .expect("Mia is a live member");
    assert!(
        !switched.coach_consents.missed_session_check,
        "bug: the switch did not turn the check off"
    );
    assert!(
        switched.coach_consents.missed_session_check_at.is_some(),
        "bug: turning the check off was not stamped"
    );

    let (status, page) = get(&d.router(), "/api/coach/roster", &d.coach.auth).await;
    assert_eq!(status, 200, "{page:#}");
    assert!(
        !flags_missed(&page, mia.id),
        "bug: the athlete switched the check off and the coach still sees it: {page:#}"
    );

    let outcomes = d.next_morning().await;
    assert!(
        outcomes.iter().any(|o| matches!(o, WakeOutcome::Posted(_))),
        "bug: the brief did not post: {outcomes:?}"
    );
    let asked = asked_for(&d.fx, d.coach.id);
    let appendix = asked[0]
        .appendix
        .as_deref()
        .expect("bug: the brief carried no appendix");
    assert!(
        appendix.contains("Missed sessions not checked for 1: switched off"),
        "bug: the footer does not say whose missed sessions went unchecked: {appendix}"
    );
    assert!(
        !asked[0].prompt.contains("planned_sessions_missed"),
        "bug: a switched-off check reached the model: {}",
        asked[0].prompt
    );
    let left_out = d.last_left_out().await;
    assert_eq!(
        (left_out.missed_off, left_out.missed_unreadable),
        (1, 0),
        "bug: the run did not record the switched-off athlete"
    );
    let (status, body) = get(&d.router(), "/api/coach/brief", &d.coach.auth).await;
    assert_eq!(status, 200, "{body:#}");
    assert_eq!(body["last_run"]["missed_off"], 1, "{body:#}");
    assert_eq!(body["last_run"]["missed_unreadable"], 0, "{body:#}");
}

/// The push half of delivery: compiled only where notifications are.
#[cfg(feature = "client-notifications")]
mod push {
    use std::time::Duration;

    use pierre_mcp_server::services::platform_post::PostSurface;
    use pierre_notifications::TenantId as NoticeTenantId;
    use tokio::time;

    use super::*;
    use crate::recording_post::ANSWERED_BY;

    /// The notification type an agent's message raises: a persisted value,
    /// stored on every notification row.
    const AGENT_MESSAGE: &str = "coach_message";

    /// A brief that lands in the app's thread, with nobody in it to see it
    /// arrive, pushes the coach once, naming the agent that wrote it.
    #[tokio::test]
    async fn a_brief_that_lands_in_the_app_pushes_the_coach() {
        let d = desk("push").await;
        d.quiet("Ben").await;

        let outcomes = d.next_morning().await;
        assert!(
            outcomes
                .iter()
                .any(|o| matches!(o, WakeOutcome::Posted(PostSurface::InApp))),
            "bug: the brief did not land in the app: {outcomes:?}"
        );
        let service = d
            .res()
            .common
            .notification_service
            .as_ref()
            .expect("bug: the test server booted without its notification service");
        let mut pushed = Vec::new();
        for _ in 0..40 {
            let (rows, _, _) = service
                .list_notifications(
                    d.coach.id,
                    NoticeTenantId(d.coach.tenant.as_uuid()),
                    10,
                    0,
                    None,
                    false,
                )
                .await
                .unwrap();
            pushed = rows
                .into_iter()
                .filter(|row| row.notification_type == AGENT_MESSAGE)
                .collect();
            if !pushed.is_empty() {
                break;
            }
            time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(
            pushed.len(),
            1,
            "bug: the brief that landed in the app did not push the coach once: {pushed:?}"
        );
        assert!(
            pushed[0].body.contains(ANSWERED_BY),
            "bug: the push does not name the agent that wrote the brief: {}",
            pushed[0].body
        );
    }
}
