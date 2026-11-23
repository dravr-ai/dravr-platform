// ABOUTME: GET families, POST session-preview, the draft/days/{date} writes and each session's under /api/chat/conversations/{id}/subject-plan
// ABOUTME: Pins the catalogue's ranges, a warn-only preview, the family and hand-written writes, add, remove, restore, and every refusal by its reason

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1515. Each day of the plan card in a coach's thread about one
//! athlete opens an editor that builds the day from a workout family. The
//! clients parse these bodies strictly (`packages/shared-types/src/
//! session-editor.ts`), so their keys are asserted, and every refusal by the
//! `details.reason` a client branches on.
//!
//! carnet#1714. The editor also opens on the day's own session: the coach
//! edits it by hand or makes it a rest day, the card marks the day `edited`
//! with the session the first edit replaced, and restore puts that back.
//!
//! carnet#1716. A date holds several sessions, each addressed by its
//! position and named as the client last saw it (`expect`): the coach adds
//! one, which restore removes, removes one of several, and a write whose
//! session moved is a 409 `session_moved` that writes nothing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use dravr_tronc::dates::{CivilDate, Elapsed, Timestamp};
use dravr_tronc::mcp::tool::{McpTool, ToolContext};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_test_support::delegation::{create_group_agent, grant_coach_access};
use pierre_tool_runtime::context::CONVERSATION_ID;
use pierre_tool_runtime::implementations::training_plans::{
    GetTrainingPlanTool, SaveTrainingPlanTool,
};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use uuid::Uuid;

const SUBJECT_NAME: &str = "Nadia Subject";
const ATHLETE_WORKOUT: &str = "athlete run";
const COACH_WORKOUT: &str = "coach-drafted tempo";
const FAMILY: &str = "vo2max_short";

#[derive(Debug, Clone)]
struct Person {
    id: Uuid,
    tenant: TenantId,
    auth: String,
}

/// A group owned by `owner` and coached by `coach`, the subject a consenting
/// member, and the coach's thread about them.
struct Fixture {
    res: Arc<ServerContext>,
    owner: Person,
    coach: Person,
    subject: Person,
    thread: String,
}

async fn person(res: &Arc<ServerContext>, label: &str, name: &str) -> Person {
    let email = format!("{label}-{}@session-editor.test", Uuid::new_v4());
    let (id, user, tenant) =
        create_test_user_with_plan(&res.agent.database, &email, "professional")
            .await
            .unwrap();
    let repos = &res.common.repos;
    repos.users.update_display_name(id, name).await.unwrap();
    repos.email_verification.mark_verified(id).await.unwrap();
    let token = generate_test_token(res, &user).await;
    Person {
        id,
        tenant,
        auth: format!("Bearer {token}"),
    }
}

fn monday(weeks: i64) -> CivilDate {
    let today = Timestamp::now().utc_date();
    let this = today - Elapsed::days(i64::from(today.weekday().num_days_from_monday()));
    this + Elapsed::days(7 * weeks)
}

fn ymd(date: CivilDate) -> String {
    date.to_string()
}

fn week(start: CivilDate, workout: &str) -> Value {
    let days: Vec<Value> = (0..7)
        .map(|offset| {
            json!({
                "date": ymd(start + Elapsed::days(offset)),
                "sport": "run",
                "workout": workout,
                "duration_min": 40,
                "intensity": "Z2",
            })
        })
        .collect();
    json!({ "week_start": ymd(start), "focus": workout, "days": days })
}

async fn call<T: McpTool<dyn ToolRuntime>>(
    res: &Arc<ServerContext>,
    who: &Person,
    tool: &T,
    conversation: Option<&str>,
    args: Value,
) -> Value {
    let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(res);
    let ctx = ToolContext::new()
        .with_user(who.id.to_string())
        .with_tenant(who.tenant.to_string())
        .with_auth_method("jwt_bearer");
    CONVERSATION_ID
        .scope(conversation.map(ToOwned::to_owned), async {
            tool.execute(&runtime, &ctx, args)
                .await
                .structured_content
                .expect("tool result carries structured content")
        })
        .await
}

async fn join(repos: &RepositoryRegistry, group: &CoachingGroup, who: &Person) {
    let now = Timestamp::now();
    repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id: group.id,
            user_id: who.id,
            tenant_id: who.tenant.to_string(),
            role: GroupRole::Member,
            peer_sharing_consent: false,
            coach_sharing_consent: true,
            coach_sharing_consent_at: None,
            coach_consents: CoachConsents::default(),
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();
}

impl Fixture {
    /// The group, the coach's thread about the subject, and — with
    /// `season` — the subject's own season of this week and the next.
    async fn new(season: bool) -> Self {
        let res = create_test_server_resources().await.unwrap();
        let owner = person(&res, "owner", "Olive Owner").await;
        let coach = person(&res, "coach", "Casey Coach").await;
        let subject = person(&res, "subject", SUBJECT_NAME).await;
        let repos = &res.common.repos;
        let agent_id = create_group_agent(repos, owner.id, owner.tenant)
            .await
            .unwrap();
        let now = Timestamp::now();
        let group = repos
            .groups
            .create_group(
                owner.tenant,
                &CoachingGroup {
                    id: Uuid::new_v4(),
                    tenant_id: owner.tenant.to_string(),
                    name: "Interval Squad".to_owned(),
                    description: None,
                    agent_id,
                    owner_id: owner.id,
                    coach_user_id: None,
                    peer_data_sharing: false,
                    respond_mode: GroupRespondMode::default(),
                    digest_mode: GroupDigestMode::Off,
                    max_members: 20,
                    is_active: true,
                    channel_type: None,
                    channel_chat_id: None,
                    chat_connection: None,
                    created_at: now,
                    updated_at: now,
                },
            )
            .await
            .unwrap();
        grant_coach_access(repos, coach.id).await.unwrap();
        assert!(
            repos
                .groups
                .set_group_coach_user(&group.id.to_string(), Some(coach.id), owner.tenant)
                .await
                .unwrap(),
            "fixture: the coach is attached to the group"
        );
        join(repos, &group, &subject).await;
        let resp = AxumTestRequest::post(&format!("/api/chat/athletes/{}/thread", subject.id))
            .header("authorization", &coach.auth)
            .json(&json!({ "group_id": group.id }))
            .send(ChatRoutes::routes(Arc::clone(&res)))
            .await;
        assert_eq!(
            resp.status_code(),
            StatusCode::OK,
            "fixture: the coach opens a thread bound to the athlete"
        );
        let thread = resp.json::<Value>()["id"].as_str().unwrap().to_owned();
        let fx = Self {
            res,
            owner,
            coach,
            subject,
            thread,
        };
        if season {
            let saved = call(
                &fx.res,
                &fx.subject,
                &SaveTrainingPlanTool,
                None,
                json!({
                    "outline": {
                        "goal_race": { "name": "Athlete Race", "date": "2027-05-16", "discipline": "trail", "priority": "A" },
                        "strategy": "the athlete's season",
                    },
                    "weeks": [week(monday(0), ATHLETE_WORKOUT), week(monday(1), ATHLETE_WORKOUT)],
                }),
            )
            .await;
            assert!(
                saved.get("error").is_none(),
                "fixture: the athlete's season saves: {saved}"
            );
        }
        fx
    }

    fn repos(&self) -> &RepositoryRegistry {
        &self.res.common.repos
    }

    async fn request(&self, request: AxumTestRequest, auth: &str) -> (StatusCode, Value) {
        let resp = request
            .header("authorization", auth)
            .send(ChatRoutes::routes(Arc::clone(&self.res)))
            .await;
        let status = resp.status_code();
        (status, resp.json())
    }

    fn base(conversation: &str) -> String {
        format!("/api/chat/conversations/{conversation}/subject-plan")
    }

    async fn families(&self, conversation: &str, auth: &str, locale: &str) -> (StatusCode, Value) {
        self.request(
            AxumTestRequest::get(&format!(
                "{}/families?locale={locale}",
                Self::base(conversation)
            )),
            auth,
        )
        .await
    }

    async fn preview(&self, conversation: &str, body: Value, auth: &str) -> (StatusCode, Value) {
        self.request(
            AxumTestRequest::post(&format!(
                "{}/session-preview?locale=en",
                Self::base(conversation)
            ))
            .json(&body),
            auth,
        )
        .await
    }

    async fn put_day(
        &self,
        conversation: &str,
        date: &str,
        body: Value,
        auth: &str,
    ) -> (StatusCode, Value) {
        self.request(
            AxumTestRequest::put(&format!(
                "{}/draft/days/{date}?locale=en",
                Self::base(conversation)
            ))
            .json(&body),
            auth,
        )
        .await
    }

    /// `PUT …/draft/days/{date}/{n}`: a family build over session n.
    async fn put_session(
        &self,
        conversation: &str,
        (date, position): (&str, u32),
        body: Value,
        auth: &str,
    ) -> (StatusCode, Value) {
        self.request(
            AxumTestRequest::put(&format!(
                "{}/draft/days/{date}/{position}?locale=en",
                Self::base(conversation)
            ))
            .json(&body),
            auth,
        )
        .await
    }

    /// `PUT …/draft/days/{date}/{n}/session`: session n written by hand.
    async fn put_hand_session(
        &self,
        conversation: &str,
        (date, position): (&str, u32),
        day: Value,
        expect: &Value,
        auth: &str,
    ) -> (StatusCode, Value) {
        self.request(
            AxumTestRequest::put(&format!(
                "{}/draft/days/{date}/{position}/session?locale=en",
                Self::base(conversation)
            ))
            .json(&json!({ "day": day, "expect": expect })),
            auth,
        )
        .await
    }

    /// `POST …/draft/days/{date}/{n}/{action}`: session n restored or
    /// removed.
    async fn on_session(
        &self,
        conversation: &str,
        (date, position, action): (&str, u32, &str),
        expect: &Value,
        auth: &str,
    ) -> (StatusCode, Value) {
        self.request(
            AxumTestRequest::post(&format!(
                "{}/draft/days/{date}/{position}/{action}?locale=en",
                Self::base(conversation)
            ))
            .json(&json!({ "expect": expect })),
            auth,
        )
        .await
    }

    async fn restore(
        &self,
        conversation: &str,
        (date, position): (&str, u32),
        expect: &Value,
        auth: &str,
    ) -> (StatusCode, Value) {
        self.on_session(conversation, (date, position, "restore"), expect, auth)
            .await
    }

    async fn remove(
        &self,
        conversation: &str,
        (date, position): (&str, u32),
        expect: &Value,
        auth: &str,
    ) -> (StatusCode, Value) {
        self.on_session(conversation, (date, position, "remove"), expect, auth)
            .await
    }

    /// `POST …/draft/days/{date}/sessions`: a session added to the date.
    async fn add_session(
        &self,
        conversation: &str,
        date: &str,
        body: Value,
        auth: &str,
    ) -> (StatusCode, Value) {
        self.request(
            AxumTestRequest::post(&format!(
                "{}/draft/days/{date}/sessions?locale=en",
                Self::base(conversation)
            ))
            .json(&body),
            auth,
        )
        .await
    }

    async fn get_plan(&self) -> Value {
        let (status, body) = self
            .request(
                AxumTestRequest::get(&format!("{}?locale=en", Self::base(&self.thread))),
                &self.coach.auth,
            )
            .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "bug: the coach reads the subject plan of their own thread: {body}"
        );
        body
    }

    /// Whether the coach holds a draft of the subject's plan.
    async fn has_draft(&self) -> bool {
        self.get_plan().await["draft_state"].is_object()
    }
}

/// The `details.reason` of a refused route call.
fn reason(body: &Value) -> &str {
    body["details"]["reason"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a refusal naming its reason, got {body}"))
}

/// Every entry on `date` of `card`'s weeks, in order.
fn entries_of<'a>(card: &'a Value, date: &str) -> Vec<&'a Value> {
    card["weeks"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|week| week["days"].as_array().into_iter().flatten())
        .filter(|day| day["date"] == date)
        .collect()
}

/// The first entry on `date` of `card`'s weeks.
fn day_of<'a>(card: &'a Value, date: &str) -> &'a Value {
    entries_of(card, date)
        .first()
        .copied()
        .unwrap_or_else(|| panic!("no day {date} in {card}"))
}

/// A session as the client last saw it: the card's exact `sport` and
/// `workout`, what every session write carries as `expect`.
fn seen(day: &Value) -> Value {
    json!({ "sport": day["sport"], "workout": day["workout"] })
}

/// The athlete's own session on any date of their season, as the card shows
/// it before the coach edits anything.
fn athletes_session() -> Value {
    json!({ "sport": "run", "workout": ATHLETE_WORKOUT })
}

fn short_intervals(params: &Value) -> Value {
    json!({ "family": FAMILY, "preset": "30_15", "sport": "ride", "params": params })
}

/// A run the coach writes by hand: 10 min warm-up, 6 × 20 s strides, 38 min
/// steady — 50 minutes, and no `duration_min`: the server takes the steps'.
fn easy_fifty() -> Value {
    json!({
        "rest": false, "sport": "run", "workout": "Easy 50 min", "intensity": "Z2",
        "steps": [
            { "label": "Warm-up", "duration_seconds": 600, "target_zone": "Z1" },
            { "label": "Strides", "duration_seconds": 20, "target_zone": "Z5", "repeat": 6 },
            { "label": "Steady", "duration_seconds": 2280, "target_zone": "Z2" }
        ]
    })
}

#[tokio::test]
async fn only_the_coachs_own_thread_about_an_athlete_opens_the_editor() {
    let fx = Fixture::new(true).await;
    let date = ymd(monday(1) + Elapsed::days(2));
    let build = short_intervals(&json!({}));
    let preview = json!({ "date": date, "family": FAMILY, "sport": "ride" });
    let expect = athletes_session();
    let session_build = json!({ "build": build, "expect": expect });
    let added = json!({ "day": easy_fifty() });
    let at = (date.as_str(), 0);

    // Another user — the group's owner here — gets nothing of it.
    let owner = fx.owner.auth.as_str();
    for (status, body) in [
        fx.families(&fx.thread, owner, "en").await,
        fx.preview(&fx.thread, preview.clone(), owner).await,
        fx.put_day(&fx.thread, &date, build.clone(), owner).await,
        fx.put_session(&fx.thread, at, session_build.clone(), owner)
            .await,
        fx.put_hand_session(&fx.thread, at, easy_fifty(), &expect, owner)
            .await,
        fx.restore(&fx.thread, at, &expect, owner).await,
        fx.remove(&fx.thread, at, &expect, owner).await,
        fx.add_session(&fx.thread, &date, added.clone(), owner)
            .await,
    ] {
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "bug: another user's call on the coach's thread is a 404: {body}"
        );
    }

    // The coach's own thread about no athlete.
    let plain = fx
        .repos()
        .chat
        .create_conversation(
            &fx.coach.id.to_string(),
            fx.coach.tenant,
            "plain",
            "gemini-2.0-flash",
            None,
            None,
        )
        .await
        .unwrap()
        .id;
    let coach = fx.coach.auth.as_str();
    for (status, body) in [
        fx.families(&plain, coach, "en").await,
        fx.preview(&plain, preview.clone(), coach).await,
        fx.put_day(&plain, &date, build.clone(), coach).await,
        fx.put_session(&plain, at, session_build.clone(), coach)
            .await,
        fx.put_hand_session(&plain, at, easy_fifty(), &expect, coach)
            .await,
        fx.restore(&plain, at, &expect, coach).await,
        fx.remove(&plain, at, &expect, coach).await,
        fx.add_session(&plain, &date, added.clone(), coach).await,
    ] {
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "bug: the editor's routes on a thread about no athlete are a 404: {body}"
        );
        assert_eq!(
            reason(&body),
            "no_subject",
            "bug: a thread about no athlete opens no editor"
        );
    }

    // An API key the coach holds writes no day of the athlete's draft, by
    // any of the writes.
    let key = common::issue_test_api_key(&fx.res.agent.database, fx.coach.id, "coach script")
        .await
        .unwrap();
    for (write, (status, body)) in [
        (
            "family day",
            fx.put_day(&fx.thread, &date, build, &key).await,
        ),
        (
            "family session",
            fx.put_session(&fx.thread, at, session_build, &key).await,
        ),
        (
            "hand session",
            fx.put_hand_session(&fx.thread, at, easy_fifty(), &expect, &key)
                .await,
        ),
        ("restore", fx.restore(&fx.thread, at, &expect, &key).await),
        ("remove", fx.remove(&fx.thread, at, &expect, &key).await),
        ("add", fx.add_session(&fx.thread, &date, added, &key).await),
    ] {
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "bug: an outside client is refused the {write} write: {body}"
        );
        assert_eq!(
            reason(&body),
            "external_transport",
            "bug: an outside client is refused the {write} write with the plan tools' reason"
        );
    }
    assert!(!fx.has_draft().await, "bug: a refused write opened a draft");
}

#[tokio::test]
async fn the_families_list_each_preset_with_its_ranges_per_sport() {
    let fx = Fixture::new(true).await;

    let (status, body) = fx.families(&fx.thread, &fx.coach.auth, "en").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the coach reads the families of their own thread: {body}"
    );
    let families = body["families"].as_array().unwrap();
    assert!(
        families
            .iter()
            .all(|family| family["presets"].as_array().is_some_and(|p| !p.is_empty())),
        "bug: a template with no presets is listed as a family: {body}"
    );
    let family = families
        .iter()
        .find(|family| family["slug"] == FAMILY)
        .unwrap_or_else(|| panic!("{FAMILY} is listed: {body}"));
    assert_eq!(
        family["purpose"], "vo2max_short",
        "bug: the family names the kernel's purpose word"
    );
    assert_eq!(
        family["purpose_label"], "short-interval VO2max work",
        "bug: the purpose reads in the reader's words"
    );
    assert_eq!(
        family["default_preset"], "30_15",
        "bug: the default preset is the family's first"
    );
    assert_eq!(
        family["sports"],
        json!(["ride", "run"]),
        "bug: the family lists every sport a preset has a form for"
    );
    let preset = family["presets"]
        .as_array()
        .unwrap()
        .iter()
        .find(|preset| preset["key"] == "30_15")
        .unwrap();
    assert_eq!(
        preset["name"], "30/15 (Rønnestad)",
        "bug: the preset keeps the catalogue's name"
    );
    assert_eq!(
        preset["sports"],
        json!(["ride", "run"]),
        "bug: the preset lists the sports it has a form for"
    );
    // The preset's ranges in each sport, as the kernel builds and measures
    // the session there: the run form asks fewer sets than the ride's.
    assert_eq!(
        preset["params"]["ride"]["sets"],
        json!({ "min": 2, "max": 4, "default": 3 }),
        "bug: the ride form's sets are the preset's range in that sport: {preset}"
    );
    assert_eq!(
        preset["params"]["run"]["sets"],
        json!({ "min": 2, "max": 3, "default": 2 }),
        "bug: the run form's sets are the preset's range in that sport, fewer than the ride's: {preset}"
    );
    assert_eq!(
        preset["params"]["ride"]["work_seconds"],
        json!({ "min": 30, "max": 30, "default": 30 }),
        "bug: the work bout is the preset's fixed 30 s: {preset}"
    );

    // The purpose reads in the locale the client asked for.
    let (status, body) = fx.families(&fx.thread, &fx.coach.auth, "fr").await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the coach reads the families in the locale asked for: {body}"
    );
    let french = body["families"]
        .as_array()
        .unwrap()
        .iter()
        .find(|family| family["slug"] == FAMILY)
        .unwrap();
    assert_eq!(
        french["purpose_label"], "une séance de VO2max en intervalles courts",
        "bug: the purpose reads in the locale the client asked for"
    );
}

#[tokio::test]
async fn a_value_outside_the_range_previews_with_a_finding_and_writes_nothing() {
    let fx = Fixture::new(true).await;
    let date = ymd(monday(1) + Elapsed::days(2));

    let mut body = short_intervals(&json!({ "reps": 20 }));
    body["date"] = json!(date);
    let (status, preview) = fx.preview(&fx.thread, body, &fx.coach.auth).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: a value outside the range previews, never refused (D3): {preview}"
    );
    assert_eq!(
        preview["family"], FAMILY,
        "bug: the preview names the family it built"
    );
    assert_eq!(
        preview["preset"], "30_15",
        "bug: the preview names the preset it resolved to"
    );
    assert_eq!(
        preview["sport"], "ride",
        "bug: the preview names the sport it built for"
    );
    assert!(
        preview["steps"]
            .as_array()
            .is_some_and(|steps| steps.iter().any(|step| step["repeat"] == 20)),
        "bug: the value is built as written (D3): {preview}"
    );
    assert!(
        preview["duration_min"].as_u64().is_some_and(|m| m > 0),
        "bug: the preview carries the generated session's length: {preview}"
    );
    assert_eq!(
        preview["derived"]["reps"], 20,
        "bug: the kernel derives the reps as built: {preview}"
    );
    assert_eq!(
        preview["derived"]["aerobic_max"], "MAP",
        "bug: a ride's aerobic maximum is MAP: {preview}"
    );
    let findings = preview["findings"].as_array().unwrap();
    assert!(
        findings
            .iter()
            .any(|finding| finding["code"] == "param_out_of_range"
                && finding["param"] == "reps"
                && finding["text"]
                    == "20 reps per set is outside this format's tested range (limit 13)."),
        "bug: the finding names the value the editor marks and reads as the envelope notice words it: {preview}"
    );
    assert!(
        findings
            .iter()
            .filter(|finding| finding["code"] != "param_out_of_range")
            .all(|finding| finding.get("param").is_none()),
        "bug: only a value outside its range names a field to mark: {preview}"
    );
    assert_eq!(
        preview["outside_evidence"], "Outside the evidence",
        "bug: the preview carries the card's warning beside a finding"
    );

    // A value inside the range previews clean.
    let mut inside = short_intervals(&json!({ "reps": 12 }));
    inside["date"] = json!(date);
    let (status, clean) = fx.preview(&fx.thread, inside, &fx.coach.auth).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: a value inside the range previews: {clean}"
    );
    assert!(
        clean["findings"]
            .as_array()
            .is_some_and(|findings| !findings
                .iter()
                .any(|finding| finding["code"] == "param_out_of_range")),
        "bug: a value inside the range earns no out-of-range finding: {clean}"
    );

    // Nothing was written.
    assert!(!fx.has_draft().await, "bug: a preview opened a draft");

    // What the catalogue cannot build is refused by its reason.
    for (build, expected) in [
        (
            json!({ "date": date, "family": "no_such_family", "sport": "ride" }),
            "unknown_family",
        ),
        (
            json!({ "date": date, "family": FAMILY, "preset": "99_99", "sport": "ride" }),
            "unknown_preset",
        ),
        (
            json!({ "date": date, "family": FAMILY, "preset": "30_15", "sport": "swim" }),
            "sport_not_offered",
        ),
        (
            json!({ "date": "next tuesday", "family": FAMILY, "sport": "ride" }),
            "invalid_date",
        ),
        (
            json!({ "date": date, "family": FAMILY, "sport": "ride", "params": { "sets": 51 } }),
            "params_out_of_bounds",
        ),
    ] {
        let (status, body) = fx.preview(&fx.thread, build.clone(), &fx.coach.auth).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "bug: a build the catalogue cannot make is a 400: {build}: {body}"
        );
        assert_eq!(
            reason(&body),
            expected,
            "bug: the refusal names the reason a client branches on: {build}: {body}"
        );
    }
}

#[tokio::test]
async fn saving_a_day_opens_a_draft_with_it_generated_and_leaves_the_live_plan() {
    let fx = Fixture::new(true).await;
    let date = ymd(monday(1) + Elapsed::days(2));

    let (status, saved) = fx
        .put_day(
            &fx.thread,
            &date,
            short_intervals(&json!({ "sets": 4 })),
            &fx.coach.auth,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the coach saves a family day into their draft: {saved}"
    );
    assert_eq!(
        saved["athlete"],
        json!({ "user_id": fx.subject.id, "display_name": SUBJECT_NAME }),
        "bug: the answer names the thread's athlete"
    );
    assert!(
        saved["draft_state"].is_object(),
        "bug: the save opens a draft from the live plan: {saved}"
    );
    let day = day_of(&saved["draft"], &date);
    assert_eq!(
        day["template_slug"], FAMILY,
        "bug: the saved day names its family: {day}"
    );
    assert_eq!(
        day["template_preset"], "30_15",
        "bug: the saved day names its preset: {day}"
    );
    assert_eq!(
        day["template_params"],
        json!({ "sets": 4 }),
        "bug: the saved day keeps the values the coach stated: {day}"
    );
    assert_eq!(
        day["family_sport"], "ride",
        "bug: the saved day reads as a ride: {day}"
    );
    assert_eq!(
        day["template_source"], "catalogue",
        "bug: the saved family comes from the catalogue: {day}"
    );
    assert_eq!(
        day["workout"], "VO2max — short intervals — 30/15 (Rønnestad)",
        "bug: the session is named as the catalogue names the preset: {day}"
    );
    let steps = day["steps"].as_array().expect("the day is generated");
    assert!(
        steps
            .iter()
            .filter(|step| step["repeat"].as_u64().is_some_and(|r| r > 1))
            .filter_map(|step| step["repeat_group"].as_u64())
            .max()
            == Some(4),
        "bug: the stated sets are built: {day}"
    );
    assert!(
        day["duration_min"].as_u64().is_some_and(|m| m > 0),
        "bug: the saved day carries the generated session's length: {day}"
    );

    // The athlete's live plan is untouched.
    assert_eq!(
        day_of(&saved["live"], &date)["workout"],
        ATHLETE_WORKOUT,
        "bug: the save changed the athlete's live plan"
    );
    let season = fx
        .repos()
        .training_plans
        .get_active_plan(&fx.subject.tenant.to_string(), &fx.subject.id.to_string())
        .await
        .unwrap()
        .expect("the athlete's season");
    assert_eq!(
        season.strategy, "the athlete's season",
        "bug: the save re-laid the athlete's season"
    );

    // The answer is the subject plan as the read gives it.
    assert_eq!(
        fx.get_plan().await,
        saved,
        "bug: the write's answer is not the subject plan as the read gives it"
    );

    // The day the editor previewed is the day the card shows.
    let mut build = short_intervals(&json!({ "sets": 4 }));
    build["date"] = json!(date);
    let (_, preview) = fx.preview(&fx.thread, build, &fx.coach.auth).await;
    assert_eq!(
        preview["steps"], day["steps"],
        "bug: the preview's steps are the saved day's: {preview}"
    );
    assert_eq!(
        preview["duration_min"], day["duration_min"],
        "bug: the preview's length is the saved day's: {preview}"
    );
    assert_eq!(
        preview.get("evidence"),
        day.get("evidence"),
        "bug: the preview's chip is not the card's: {preview}"
    );
}

#[tokio::test]
async fn a_day_saved_into_an_open_draft_keeps_the_weeks_other_days() {
    let fx = Fixture::new(true).await;
    let drafted = call(
        &fx.res,
        &fx.coach,
        &SaveTrainingPlanTool,
        Some(&fx.thread),
        json!({ "weeks": [week(monday(1), COACH_WORKOUT)] }),
    )
    .await;
    assert_eq!(
        drafted["draft"], true,
        "fixture: the coach's save from the thread drafts the athlete's plan: {drafted}"
    );
    let before = fx.get_plan().await;
    let draft_id = before["draft_state"]["draft_id"].clone();

    let date = ymd(monday(1) + Elapsed::days(3));
    let (status, saved) = fx
        .put_day(
            &fx.thread,
            &date,
            json!({ "family": FAMILY, "sport": "run" }),
            &fx.coach.auth,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the coach saves a family day into their open draft: {saved}"
    );
    assert_eq!(
        saved["draft_state"]["draft_id"], draft_id,
        "bug: the day went into another draft: {saved}"
    );
    let day = day_of(&saved["draft"], &date);
    assert_eq!(
        day["template_preset"], "30_15",
        "bug: a build naming no preset takes the family's default: {day}"
    );
    assert_eq!(
        day["family_sport"], "run",
        "bug: the saved day reads as a run: {day}"
    );
    assert_eq!(
        day["sport"], "run",
        "bug: a run built over a run day keeps its own label: {day}"
    );
    for offset in [0, 1, 2, 4, 5, 6] {
        let other = ymd(monday(1) + Elapsed::days(offset));
        assert_eq!(
            day_of(&saved["draft"], &other)["workout"],
            COACH_WORKOUT,
            "bug: the save rewrote another day of the week: {saved}"
        );
    }
    assert_eq!(
        day_of(&saved["live"], &date)["workout"],
        ATHLETE_WORKOUT,
        "bug: the save changed the athlete's live plan"
    );

    // The run form's evidence is a single-session trial: the preview's chip
    // is the one the card shows on the saved day.
    let (status, preview) = fx
        .preview(
            &fx.thread,
            json!({ "date": date, "family": FAMILY, "sport": "run" }),
            &fx.coach.auth,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the coach previews the run build: {preview}"
    );
    assert!(
        day["evidence"]["label"].is_string(),
        "bug: the run form's single-session trial carries an evidence chip: {day}"
    );
    assert_eq!(
        preview["evidence"], day["evidence"],
        "bug: the preview's chip is the one the card shows on the saved day: {preview}"
    );
    assert_eq!(
        preview["steps"], day["steps"],
        "bug: the preview's steps are the saved day's: {preview}"
    );
}

#[tokio::test]
async fn a_date_no_week_holds_and_a_plan_that_does_not_exist_write_nothing() {
    let fx = Fixture::new(true).await;
    let far = ymd(monday(6));
    let (status, body) = fx
        .put_day(
            &fx.thread,
            &far,
            short_intervals(&json!({})),
            &fx.coach.auth,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "bug: a date no week of the plan holds is a 422: {body}"
    );
    assert_eq!(
        reason(&body),
        "no_plan_week",
        "bug: a date no week holds is refused by its reason"
    );
    assert!(!fx.has_draft().await, "bug: a refused write opened a draft");

    let (status, body) = fx
        .put_day(
            &fx.thread,
            "2026-13-40",
            short_intervals(&json!({})),
            &fx.coach.auth,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "bug: a date that is not one is a 400: {body}"
    );
    assert_eq!(
        reason(&body),
        "invalid_date",
        "bug: a date that is not one is refused by its reason"
    );

    let bare = Fixture::new(false).await;
    let (status, body) = bare
        .put_day(
            &bare.thread,
            &ymd(monday(1)),
            short_intervals(&json!({})),
            &bare.coach.auth,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "bug: a thread with no plan and no draft is a 404: {body}"
    );
    assert_eq!(
        reason(&body),
        "no_plan",
        "bug: a thread with no plan and no draft is refused by its reason"
    );
    assert!(
        !bare.has_draft().await,
        "bug: a refused write opened a draft"
    );
}

#[tokio::test]
async fn a_hand_day_saves_into_the_draft_marked_edited_and_restores() {
    let fx = Fixture::new(true).await;
    let date = ymd(monday(1) + Elapsed::days(2));

    let (status, saved) = fx
        .put_hand_session(
            &fx.thread,
            (&date, 0),
            easy_fifty(),
            &athletes_session(),
            &fx.coach.auth,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the coach saves a hand-written day into their draft: {saved}"
    );
    assert!(
        saved["draft_state"].is_object(),
        "bug: the save opens a draft from the live plan: {saved}"
    );
    let day = day_of(&saved["draft"], &date);
    assert_eq!(
        day["workout"], "Easy 50 min",
        "bug: the day takes the coach's name: {day}"
    );
    assert_eq!(
        day["sport"], "run",
        "bug: the day takes the coach's sport: {day}"
    );
    assert_eq!(
        day["intensity"], "Z2",
        "bug: the day takes the coach's intensity: {day}"
    );
    assert_eq!(
        day["duration_min"], 50,
        "bug: a day with steps and no stated length takes their total: {day}"
    );
    assert_eq!(
        day["steps"].as_array().map(Vec::len),
        Some(3),
        "bug: the day keeps the coach's steps: {day}"
    );
    assert_eq!(
        day["steps"][1],
        json!({ "label": "Strides", "duration_seconds": 20, "target_zone": "Z5", "repeat": 6 }),
        "bug: a step reads back as the card's step: {day}"
    );
    for key in [
        "template_slug",
        "template_source",
        "template_preset",
        "template_params",
    ] {
        assert!(
            day.get(key).is_none(),
            "bug: a hand-written day names no family ({key}): {day}"
        );
    }
    assert!(
        day.get("evidence").is_none() && day.get("outside_evidence").is_none(),
        "bug: no envelope check measures a hand-written day: {day}"
    );
    assert_eq!(
        day["edited"],
        json!({ "workout": ATHLETE_WORKOUT, "rest": false }),
        "bug: the card marks the day with the session the edit replaced: {day}"
    );
    for offset in [0, 1, 3, 4, 5, 6] {
        let other = day_of(&saved["draft"], &ymd(monday(1) + Elapsed::days(offset)));
        assert!(
            other.get("edited").is_none() && other["workout"] == ATHLETE_WORKOUT,
            "bug: the save edited another day of the week: {other}"
        );
    }
    let live = day_of(&saved["live"], &date);
    assert!(
        live["workout"] == ATHLETE_WORKOUT && live.get("edited").is_none(),
        "bug: the save changed the athlete's live plan: {live}"
    );
    assert_eq!(
        fx.get_plan().await,
        saved,
        "bug: the write's answer is not the subject plan as the read gives it"
    );

    // The thread's agent reads the edited day and never the snapshot.
    let read = call(
        &fx.res,
        &fx.coach,
        &GetTrainingPlanTool,
        Some(&fx.thread),
        json!({}),
    )
    .await;
    assert!(
        read["draft"]["weeks"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|week| week["days"].as_array().into_iter().flatten())
            .any(|day| day["date"] == date && day["workout"] == "Easy 50 min"),
        "bug: the agent reads the draft as the coach edited it: {read}"
    );
    assert!(
        !read.to_string().contains("coach_original"),
        "bug: the coach's snapshot reached the agent's read: {read}"
    );

    // A second edit, to a rest day, keeps what the first one replaced.
    let (status, rested) = fx
        .put_hand_session(
            &fx.thread,
            (&date, 0),
            json!({ "rest": true }),
            &seen(day),
            &fx.coach.auth,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the coach makes the day a rest day: {rested}"
    );
    let rest = day_of(&rested["draft"], &date);
    assert_eq!(rest["rest"], true, "bug: the day is a rest day: {rest}");
    assert_eq!(
        rest["sport"], "rest",
        "bug: a rest day is written as the agent writes one: {rest}"
    );
    assert!(
        rest.get("steps").is_none() && rest.get("duration_min").is_none(),
        "bug: a rest day carries no steps or length: {rest}"
    );
    assert_eq!(
        rest["edited"],
        json!({ "workout": ATHLETE_WORKOUT, "rest": false }),
        "bug: a second edit keeps the first edit's snapshot: {rest}"
    );

    // Restore puts the day back as it stood before the first edit.
    let (status, restored) = fx
        .restore(&fx.thread, (&date, 0), &seen(rest), &fx.coach.auth)
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the coach restores the day they edited: {restored}"
    );
    let back = day_of(&restored["draft"], &date);
    assert_eq!(
        back,
        day_of(&restored["live"], &date),
        "bug: the restored day is the day as it stood, and no longer marked edited: {back}"
    );
    assert_eq!(
        restored["draft_state"]["draft_id"], saved["draft_state"]["draft_id"],
        "bug: the restore wrote into another draft: {restored}"
    );

    // Nothing is left to restore.
    let (status, body) = fx
        .restore(&fx.thread, (&date, 0), &athletes_session(), &fx.coach.auth)
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "bug: a day with nothing to restore is a 422: {body}"
    );
    assert_eq!(
        reason(&body),
        "nothing_to_restore",
        "bug: a day with nothing to restore is refused by its reason"
    );
}

#[tokio::test]
async fn a_family_build_over_a_day_is_marked_edited_and_restores() {
    let fx = Fixture::new(true).await;
    let date = ymd(monday(1) + Elapsed::days(2));

    let (status, saved) = fx
        .put_day(
            &fx.thread,
            &date,
            short_intervals(&json!({})),
            &fx.coach.auth,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the family build saves: {saved}"
    );
    assert_eq!(
        day_of(&saved["draft"], &date)["edited"],
        json!({ "workout": ATHLETE_WORKOUT, "rest": false }),
        "bug: a family build marks the day with the session it replaced: {saved}"
    );

    // A hand edit over the family day keeps the snapshot the build took.
    let built = seen(day_of(&saved["draft"], &date));
    let (status, edited) = fx
        .put_hand_session(&fx.thread, (&date, 0), easy_fifty(), &built, &fx.coach.auth)
        .await;
    assert_eq!(status, StatusCode::OK, "bug: the hand edit saves: {edited}");
    assert_eq!(
        day_of(&edited["draft"], &date)["edited"]["workout"],
        ATHLETE_WORKOUT,
        "bug: a hand edit after a build keeps the build's snapshot: {edited}"
    );

    let (status, restored) = fx
        .restore(
            &fx.thread,
            (&date, 0),
            &seen(day_of(&edited["draft"], &date)),
            &fx.coach.auth,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "bug: the restore saves: {restored}");
    assert_eq!(
        day_of(&restored["draft"], &date),
        day_of(&restored["live"], &date),
        "bug: restoring a built day puts back the day the build replaced: {restored}"
    );
}

#[tokio::test]
async fn a_hand_day_past_the_agents_bounds_is_refused_and_writes_nothing() {
    let fx = Fixture::new(true).await;
    let date = ymd(monday(1) + Elapsed::days(2));
    let step = |target: &str, repeat: u32| json!({ "label": "Work", "duration_seconds": 60, "target_zone": target, "repeat": repeat });
    for body in [
        json!({ "rest": false, "sport": "run", "workout": " ", "intensity": "Z2" }),
        json!({ "rest": false, "sport": "run", "workout": "Reps", "steps": [step("Z4", 101)] }),
        json!({ "rest": false, "sport": "run", "workout": "Reps", "steps": [step("fast", 1)] }),
        json!({ "rest": false, "sport": "run", "workout": "Long", "duration_min": 1441 }),
        json!({ "rest": false, "sport": "run", "workout": "Reps", "duration_min": 30,
                "steps": [step("Z4", 1)] }),
    ] {
        let (status, refused) = fx
            .put_hand_session(
                &fx.thread,
                (&date, 0),
                body.clone(),
                &athletes_session(),
                &fx.coach.auth,
            )
            .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "bug: a day past the agent's day bounds is a 400: {body}: {refused}"
        );
        assert_eq!(
            reason(&refused),
            "invalid_session",
            "bug: a day past the agent's day bounds is refused by its reason: {body}: {refused}"
        );
    }
    assert!(!fx.has_draft().await, "bug: a refused write opened a draft");
}

#[tokio::test]
async fn restore_refuses_with_no_draft_no_week_and_no_edit() {
    let fx = Fixture::new(true).await;
    let date = ymd(monday(1) + Elapsed::days(3));

    // No draft: restore never opens one.
    let (status, body) = fx
        .restore(&fx.thread, (&date, 0), &athletes_session(), &fx.coach.auth)
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "bug: a restore with no draft is a 404: {body}"
    );
    assert_eq!(
        reason(&body),
        "no_plan",
        "bug: a restore with no draft is refused by its reason"
    );
    assert!(!fx.has_draft().await, "bug: a restore opened a draft");

    // A draft the agent wrote: its days carry no snapshot.
    let drafted = call(
        &fx.res,
        &fx.coach,
        &SaveTrainingPlanTool,
        Some(&fx.thread),
        json!({ "weeks": [week(monday(1), COACH_WORKOUT)] }),
    )
    .await;
    assert_eq!(
        drafted["draft"], true,
        "fixture: the coach's save from the thread drafts the athlete's plan: {drafted}"
    );
    let drafted_session = json!({ "sport": "run", "workout": COACH_WORKOUT });
    let (status, body) = fx
        .restore(&fx.thread, (&date, 0), &drafted_session, &fx.coach.auth)
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "bug: a day the coach never edited is a 422: {body}"
    );
    assert_eq!(
        reason(&body),
        "nothing_to_restore",
        "bug: a day the coach never edited is refused by its reason"
    );

    // A date no week holds.
    let (status, body) = fx
        .restore(
            &fx.thread,
            (&ymd(monday(6)), 0),
            &athletes_session(),
            &fx.coach.auth,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "bug: a date no week holds is a 422: {body}"
    );
    assert_eq!(
        reason(&body),
        "no_plan_week",
        "bug: a date no week holds is refused by its reason"
    );

    // An edit over the agent's day snapshots that day, and restore puts it
    // back.
    let (status, edited) = fx
        .put_hand_session(
            &fx.thread,
            (&date, 0),
            easy_fifty(),
            &drafted_session,
            &fx.coach.auth,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "fixture: the coach edits the day");
    let (status, restored) = fx
        .restore(
            &fx.thread,
            (&date, 0),
            &seen(day_of(&edited["draft"], &date)),
            &fx.coach.auth,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "bug: the restore saves: {restored}");
    let back = day_of(&restored["draft"], &date);
    assert!(
        back["workout"] == COACH_WORKOUT && back.get("edited").is_none(),
        "bug: restoring puts back the day the agent drafted: {back}"
    );
}

/// A hand session the coach adds to a date: strength beside the athlete's
/// run.
fn core_twenty() -> Value {
    json!({ "rest": false, "sport": "strength", "workout": "Core 20 min", "duration_min": 20 })
}

/// The `(workout, position, added)` of each entry on `date` of `card`.
fn placed_on(card: &Value, date: &str) -> Vec<(String, u64, bool)> {
    entries_of(card, date)
        .into_iter()
        .map(|day| {
            (
                day["workout"].as_str().unwrap_or_default().to_owned(),
                day["position"].as_u64().unwrap_or(u64::MAX),
                day["added"] == true,
            )
        })
        .collect()
}

#[tokio::test]
async fn a_coach_adds_sessions_to_a_date_and_restoring_an_added_one_removes_it() {
    let fx = Fixture::new(true).await;
    let date = ymd(monday(1) + Elapsed::days(2));
    let coach = fx.coach.auth.as_str();

    let (status, with_core) = fx
        .add_session(&fx.thread, &date, json!({ "day": core_twenty() }), coach)
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the coach adds a hand session to a date: {with_core}"
    );
    assert_eq!(
        placed_on(&with_core["draft"], &date),
        [
            (ATHLETE_WORKOUT.to_owned(), 0, false),
            ("Core 20 min".to_owned(), 1, true)
        ],
        "bug: the added session follows the date's entry, marked added"
    );
    assert!(
        entries_of(&with_core["draft"], &date)[1]
            .get("edited")
            .is_none(),
        "bug: an added session carries no snapshot"
    );
    assert_eq!(
        placed_on(&with_core["live"], &date),
        [(ATHLETE_WORKOUT.to_owned(), 0, false)],
        "bug: adding a session changed the athlete's live plan"
    );

    let (status, with_build) = fx
        .add_session(
            &fx.thread,
            &date,
            json!({ "build": short_intervals(&json!({})) }),
            coach,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the coach adds a family session to a date: {with_build}"
    );
    let built = entries_of(&with_build["draft"], &date)[2];
    assert!(
        built["template_slug"] == FAMILY && built["position"] == 2 && built["added"] == true,
        "bug: a family session is added third, marked added: {built}"
    );

    // The thread's agent reads the sessions, never the coach's marks.
    let read = call(
        &fx.res,
        &fx.coach,
        &GetTrainingPlanTool,
        Some(&fx.thread),
        json!({}),
    )
    .await;
    assert!(
        !read.to_string().contains("coach_added"),
        "bug: the added-session flag reached the agent's read: {read}"
    );

    // An edit of an added session keeps it added, with no snapshot.
    let core = seen(entries_of(&with_build["draft"], &date)[1]);
    let (status, edited) = fx
        .put_hand_session(
            &fx.thread,
            (&date, 1),
            json!({ "rest": false, "sport": "strength", "workout": "Core 30 min" }),
            &core,
            coach,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "bug: the hand edit saves: {edited}");
    let core_edited = entries_of(&edited["draft"], &date)[1];
    assert!(
        core_edited["workout"] == "Core 30 min"
            && core_edited["added"] == true
            && core_edited.get("edited").is_none(),
        "bug: an edited added session stays added: {core_edited}"
    );

    // Restoring it removes it; the date keeps its other entries.
    let (status, restored) = fx
        .restore(&fx.thread, (&date, 1), &seen(core_edited), coach)
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: restoring an added session saves: {restored}"
    );
    let left: Vec<String> = placed_on(&restored["draft"], &date)
        .into_iter()
        .map(|(workout, ..)| workout)
        .collect();
    assert_eq!(
        left,
        [
            ATHLETE_WORKOUT.to_owned(),
            built["workout"].as_str().unwrap().to_owned()
        ],
        "bug: restoring an added session removes it and leaves the others"
    );
}

#[tokio::test]
async fn a_coach_removes_one_of_several_sessions_and_never_the_last() {
    let fx = Fixture::new(true).await;
    let date = ymd(monday(1) + Elapsed::days(3));
    let coach = fx.coach.auth.as_str();
    let (status, _) = fx
        .add_session(&fx.thread, &date, json!({ "day": core_twenty() }), coach)
        .await;
    assert_eq!(status, StatusCode::OK, "fixture: the coach adds a session");

    let (status, removed) = fx
        .remove(&fx.thread, (&date, 0), &athletes_session(), coach)
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the coach removes one of two sessions: {removed}"
    );
    assert_eq!(
        placed_on(&removed["draft"], &date),
        [("Core 20 min".to_owned(), 0, true)],
        "bug: the other session stays, now the date's first"
    );

    let last = seen(entries_of(&removed["draft"], &date)[0]);
    let (status, body) = fx.remove(&fx.thread, (&date, 0), &last, coach).await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "bug: the date's last session is a 422: {body}"
    );
    assert_eq!(
        reason(&body),
        "last_session",
        "bug: the last session is refused by its reason"
    );
    assert_eq!(
        placed_on(&fx.get_plan().await["draft"], &date),
        [("Core 20 min".to_owned(), 0, true)],
        "bug: a refused removal wrote something"
    );
}

#[tokio::test]
async fn a_write_whose_session_moved_is_a_409_and_writes_nothing() {
    let fx = Fixture::new(true).await;
    let date = ymd(monday(1) + Elapsed::days(2));
    let coach = fx.coach.auth.as_str();
    let (status, _) = fx
        .add_session(&fx.thread, &date, json!({ "day": core_twenty() }), coach)
        .await;
    assert_eq!(status, StatusCode::OK, "fixture: the coach adds a session");
    let before = fx.get_plan().await;

    let stale = json!({ "sport": "run", "workout": "a session no longer there" });
    let build = json!({ "build": short_intervals(&json!({})), "expect": stale });
    for (write, (status, body)) in [
        (
            "family session",
            fx.put_session(&fx.thread, (&date, 0), build, coach).await,
        ),
        (
            "hand session",
            fx.put_hand_session(
                &fx.thread,
                (&date, 1),
                easy_fifty(),
                &athletes_session(),
                coach,
            )
            .await,
        ),
        (
            "restore",
            fx.restore(&fx.thread, (&date, 0), &stale, coach).await,
        ),
        (
            "remove",
            fx.remove(&fx.thread, (&date, 2), &athletes_session(), coach)
                .await,
        ),
    ] {
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "bug: a {write} whose session moved is a 409: {body}"
        );
        assert_eq!(
            reason(&body),
            "session_moved",
            "bug: a {write} whose session moved is refused by its reason"
        );
    }
    assert_eq!(
        fx.get_plan().await,
        before,
        "bug: a write whose session moved wrote something"
    );
}

#[tokio::test]
async fn a_date_holds_five_sessions_one_rest_and_takes_a_session_where_it_held_none() {
    let fx = Fixture::new(true).await;
    let coach = fx.coach.auth.as_str();
    let date = ymd(monday(1) + Elapsed::days(4));
    for n in 1..5 {
        let (status, body) = fx
            .add_session(
                &fx.thread,
                &date,
                json!({ "day": { "rest": false, "sport": "run", "workout": format!("Run {n}") } }),
                coach,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "bug: session {n} is added: {body}");
    }
    let (status, body) = fx
        .add_session(&fx.thread, &date, json!({ "day": core_twenty() }), coach)
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "bug: a sixth session on a date is a 422: {body}"
    );
    assert_eq!(
        reason(&body),
        "date_full",
        "bug: a full date is refused by its reason"
    );
    assert_eq!(
        entries_of(&fx.get_plan().await["draft"], &date).len(),
        5,
        "bug: a refused add wrote something"
    );

    let rest_date = ymd(monday(1) + Elapsed::days(5));
    let (status, rested) = fx
        .add_session(
            &fx.thread,
            &rest_date,
            json!({ "day": { "rest": true } }),
            coach,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: a rest entry beside the date's session is added: {rested}"
    );
    let (status, body) = fx
        .add_session(
            &fx.thread,
            &rest_date,
            json!({ "day": { "rest": true } }),
            coach,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "bug: a second rest entry on a date is a 400: {body}"
    );
    assert_eq!(
        reason(&body),
        "invalid_session",
        "bug: a second rest entry is refused by the plan's day bounds"
    );

    let (status, body) = fx
        .add_session(
            &fx.thread,
            &ymd(monday(6)),
            json!({ "day": core_twenty() }),
            coach,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "bug: a date no week holds is a 422: {body}"
    );
    assert_eq!(reason(&body), "no_plan_week", "bug: refused by its reason");

    // A draft week the agent wrote with one day: another date of it holds no
    // entry, and takes an added session.
    let sparse = json!({
        "week_start": ymd(monday(0)),
        "days": [{ "date": ymd(monday(0)), "sport": "run", "workout": COACH_WORKOUT }],
    });
    let drafted = call(
        &fx.res,
        &fx.coach,
        &SaveTrainingPlanTool,
        Some(&fx.thread),
        json!({ "weeks": [sparse] }),
    )
    .await;
    assert_eq!(
        drafted["draft"], true,
        "fixture: the agent drafts a week: {drafted}"
    );
    let empty = ymd(monday(0) + Elapsed::days(3));
    let (status, added) = fx
        .add_session(&fx.thread, &empty, json!({ "day": core_twenty() }), coach)
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: a date of a plan week that holds no entry takes a session: {added}"
    );
    assert_eq!(
        placed_on(&added["draft"], &empty),
        [("Core 20 min".to_owned(), 0, true)],
        "bug: the added session is the date's only entry"
    );
}

#[tokio::test]
async fn the_released_put_writes_the_dates_first_entry_and_a_build_lands_by_position() {
    let fx = Fixture::new(true).await;
    let date = ymd(monday(1) + Elapsed::days(2));
    let coach = fx.coach.auth.as_str();
    let (status, _) = fx
        .add_session(&fx.thread, &date, json!({ "day": core_twenty() }), coach)
        .await;
    assert_eq!(status, StatusCode::OK, "fixture: the coach adds a session");

    // The route released app builds call names no session: it builds over
    // the date's first entry and leaves the second.
    let (status, saved) = fx
        .put_day(&fx.thread, &date, short_intervals(&json!({})), coach)
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the released PUT still saves: {saved}"
    );
    let entries = entries_of(&saved["draft"], &date);
    assert!(
        entries.len() == 2
            && entries[0]["template_slug"] == FAMILY
            && entries[0]["edited"] == json!({ "workout": ATHLETE_WORKOUT, "rest": false })
            && entries[1]["workout"] == "Core 20 min",
        "bug: the released PUT builds over position 0 only: {entries:?}"
    );

    // A build by position lands on the session it names.
    let (status, built) = fx
        .put_session(
            &fx.thread,
            (&date, 1),
            json!({ "build": short_intervals(&json!({})), "expect": seen(entries[1]) }),
            coach,
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: a family build over session 1 saves: {built}"
    );
    let second = entries_of(&built["draft"], &date)[1];
    assert!(
        second["template_slug"] == FAMILY && second["added"] == true,
        "bug: the build lands on session 1, still an added session: {second}"
    );
    assert_eq!(
        entries_of(&built["draft"], &date)[0],
        entries[0],
        "bug: a build by position leaves the date's other session"
    );
}
