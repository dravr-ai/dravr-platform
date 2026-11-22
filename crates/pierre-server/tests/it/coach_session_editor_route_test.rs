// ABOUTME: GET families, POST session-preview, PUT draft/days/{date} under /api/chat/conversations/{id}/subject-plan — the coach's family editor over HTTP
// ABOUTME: Pins the catalogue's per-sport ranges, a warn-only preview that writes nothing, the draft-day write and every refusal by its reason

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! carnet#1515. Each day of the plan card in a coach's thread about one
//! athlete opens an editor that builds the day from a workout family. The
//! clients parse these bodies strictly (`packages/shared-types/src/
//! session-editor.ts`), so their keys are asserted, and every refusal by the
//! `details.reason` a client branches on.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use chrono::{Datelike, Days, NaiveDate, Utc};
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
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
use pierre_tool_runtime::implementations::training_plans::SaveTrainingPlanTool;
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

fn monday(weeks: u64) -> NaiveDate {
    let today = Utc::now().date_naive();
    let this = today - Days::new(u64::from(today.weekday().num_days_from_monday()));
    this + Days::new(7 * weeks)
}

fn ymd(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn week(start: NaiveDate, workout: &str) -> Value {
    let days: Vec<Value> = (0..7)
        .map(|offset| {
            json!({
                "date": ymd(start + Days::new(offset)),
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
    let now = Utc::now();
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
        let now = Utc::now();
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

/// The day `date` of `card`'s weeks.
fn day_of<'a>(card: &'a Value, date: &str) -> &'a Value {
    card["weeks"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|week| week["days"].as_array().into_iter().flatten())
        .find(|day| day["date"] == date)
        .unwrap_or_else(|| panic!("no day {date} in {card}"))
}

fn short_intervals(params: &Value) -> Value {
    json!({ "family": FAMILY, "preset": "30_15", "sport": "ride", "params": params })
}

#[tokio::test]
async fn only_the_coachs_own_thread_about_an_athlete_opens_the_editor() {
    let fx = Fixture::new(true).await;
    let date = ymd(monday(1) + Days::new(2));
    let build = short_intervals(&json!({}));
    let preview = json!({ "date": date, "family": FAMILY, "sport": "ride" });

    // Another user — the group's owner here — gets nothing of it.
    for (status, body) in [
        fx.families(&fx.thread, &fx.owner.auth, "en").await,
        fx.preview(&fx.thread, preview.clone(), &fx.owner.auth)
            .await,
        fx.put_day(&fx.thread, &date, build.clone(), &fx.owner.auth)
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
    for (status, body) in [
        fx.families(&plain, &fx.coach.auth, "en").await,
        fx.preview(&plain, preview.clone(), &fx.coach.auth).await,
        fx.put_day(&plain, &date, build.clone(), &fx.coach.auth)
            .await,
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

    // An API key the coach holds writes no day of the athlete's draft.
    let key = common::issue_test_api_key(&fx.res.agent.database, fx.coach.id, "coach script")
        .await
        .unwrap();
    let (status, body) = fx.put_day(&fx.thread, &date, build, &key).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "bug: an outside client is refused the draft-day write: {body}"
    );
    assert_eq!(
        reason(&body),
        "external_transport",
        "bug: an outside client is refused the write with the plan tools' reason"
    );
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
    let date = ymd(monday(1) + Days::new(2));

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
    let date = ymd(monday(1) + Days::new(2));

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

    let date = ymd(monday(1) + Days::new(3));
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
        let other = ymd(monday(1) + Days::new(offset));
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
