// ABOUTME: A group's coach reads and sets a coached athlete's thresholds from Group info, through the one physiology writer
// ABOUTME: Pins the coach gate and the athlete's consent on every call, the coach origin, the read on record, and the athlete told

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![cfg(feature = "client-notifications")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#1561. `GET /api/chat/athletes/{id}/physiology?group_id=` and
//! `PUT …/physiology/{sport}/{anchor}?group_id=` let a group's human coach
//! read a coached athlete's per-sport anchors and set one. Every call runs
//! the coaching gates — coach access and the attachment, a live member, the
//! athlete's consent to share with the coach, one home tenant — and goes on
//! the athlete's record as a `physiology_view` read. A value the coach sets
//! is stored with origin `coach`, at the confidence the coach states, and
//! tells the athlete with `coach_physiology_set`.

use crate::common;
use crate::helpers;

use std::sync::Arc;
use std::time::Duration as StdDuration;

use axum::http::StatusCode;
use chrono::Utc;
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::athlete_home::athlete_home_routes;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_notifications::models::{Notification, NotificationCategory};
use pierre_notifications::TenantId as NoticeTenantId;
use pierre_test_support::delegation::{create_group_agent, grant_coach_access};
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::{json, Value};
use tokio::time::sleep;
use uuid::Uuid;

const COACH_NAME: &str = "Casey Coach";
const ATHLETE_NAME: &str = "Mira Athlete";
const NOTICE: &str = "coach_physiology_set";

#[derive(Debug, Clone)]
struct Person {
    id: Uuid,
    tenant: TenantId,
    auth: String,
}

/// A group owned by `owner` and coached by `coach`, with `athlete` a
/// consenting member and `outsider` a member of nothing.
struct Fixture {
    res: Arc<ServerContext>,
    owner: Person,
    coach: Person,
    athlete: Person,
    outsider: Person,
    group: CoachingGroup,
}

async fn person(res: &Arc<ServerContext>, label: &str, name: &str) -> Person {
    let email = format!("{label}-{}@coach-physiology.test", Uuid::new_v4());
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

async fn coached_group(
    repos: &RepositoryRegistry,
    owner: &Person,
    coach: &Person,
) -> CoachingGroup {
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
                name: "Threshold Squad".to_owned(),
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
    group
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
    async fn new() -> Self {
        let res = create_test_server_resources().await.unwrap();
        let owner = person(&res, "owner", "Olive Owner").await;
        let coach = person(&res, "coach", COACH_NAME).await;
        let athlete = person(&res, "athlete", ATHLETE_NAME).await;
        let outsider = person(&res, "outsider", "Otto Outsider").await;
        let group = coached_group(&res.common.repos, &owner, &coach).await;
        join(&res.common.repos, &group, &athlete).await;
        Self {
            res,
            owner,
            coach,
            athlete,
            outsider,
            group,
        }
    }

    fn repos(&self) -> &RepositoryRegistry {
        &self.res.common.repos
    }

    /// `GET` the coach's view of `athlete`'s thresholds as `auth`.
    async fn view(&self, athlete: Uuid, auth: &str) -> (StatusCode, Value) {
        let resp = AxumTestRequest::get(&format!(
            "/api/chat/athletes/{athlete}/physiology?group_id={}&locale=en",
            self.group.id
        ))
        .header("authorization", auth)
        .send(ChatRoutes::routes(Arc::clone(&self.res)))
        .await;
        (resp.status_code(), resp.json())
    }

    /// `PUT` one of `athlete`'s thresholds as `auth`.
    async fn set(
        &self,
        athlete: Uuid,
        auth: &str,
        path: &str,
        edit: &Value,
    ) -> (StatusCode, Value) {
        let resp = AxumTestRequest::put(&format!(
            "/api/chat/athletes/{athlete}/physiology/{path}?group_id={}&locale=en",
            self.group.id
        ))
        .header("authorization", auth)
        .json(edit)
        .send(ChatRoutes::routes(Arc::clone(&self.res)))
        .await;
        (resp.status_code(), resp.json())
    }

    /// The athlete's own read of their thresholds.
    async fn athletes_own(&self) -> Value {
        let resp = AxumTestRequest::get("/api/me/physiology?locale=en")
            .header("authorization", &self.athlete.auth)
            .send(athlete_home_routes().with_state(Arc::clone(&self.res)))
            .await;
        assert_eq!(
            resp.status_code(),
            StatusCode::OK,
            "the athlete reads theirs"
        );
        resp.json()
    }

    /// The athlete's own `PUT` of one of their thresholds.
    async fn athlete_sets(&self, path: &str, edit: &Value) {
        let resp = AxumTestRequest::put(&format!("/api/me/physiology/{path}?locale=en"))
            .header("authorization", &self.athlete.auth)
            .json(edit)
            .send(athlete_home_routes().with_state(Arc::clone(&self.res)))
            .await;
        assert_eq!(
            resp.status_code(),
            StatusCode::OK,
            "fixture: the athlete sets {path}"
        );
    }

    /// `who`'s threshold notices in `tenant`, once `expected` have landed or
    /// the dispatches had their time.
    async fn notices(&self, who: &Person, tenant: TenantId, expected: usize) -> Vec<Notification> {
        let service = self
            .res
            .common
            .notification_service
            .as_ref()
            .expect("the server boots with its notification service");
        let mut rows = Vec::new();
        for _ in 0..40 {
            let (all, _, _) = service
                .list_notifications(who.id, NoticeTenantId(tenant.as_uuid()), 50, 0, None, false)
                .await
                .unwrap();
            rows = all
                .into_iter()
                .filter(|row| row.notification_type == NOTICE)
                .collect();
            if rows.len() >= expected {
                break;
            }
            sleep(StdDuration::from_millis(50)).await;
        }
        rows
    }

    /// The labels of the athlete's recorded coach reads in the group.
    async fn reads(&self) -> Vec<String> {
        self.repos()
            .coach_data_reads
            .list_coach_reads(self.athlete.tenant, self.athlete.id, self.group.id, 50)
            .await
            .unwrap()
            .into_iter()
            .map(|read| read.tool)
            .collect()
    }
}

fn family<'body>(body: &'body Value, sport: &str) -> &'body Value {
    body["sports"]
        .as_array()
        .and_then(|sports| sports.iter().find(|entry| entry["sport"] == sport))
        .unwrap_or_else(|| panic!("no {sport} family in {body:#}"))
}

fn anchor<'body>(family: &'body Value, name: &str) -> &'body Value {
    family["anchors"]
        .as_array()
        .and_then(|anchors| anchors.iter().find(|entry| entry["anchor"] == name))
        .unwrap_or_else(|| panic!("no {name} anchor in {family:#}"))
}

fn reason(body: &Value) -> &str {
    body["details"]["reason"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a refusal naming its reason, got {body}"))
}

#[tokio::test]
async fn the_coach_reads_the_athletes_thresholds_worded_for_a_coach_and_on_the_record() {
    let fx = Fixture::new().await;
    fx.athlete_sets(
        "ride/ftp",
        &json!({ "value": 285, "kind": "measured", "confidence": "high" }),
    )
    .await;

    let (status, body) = fx.view(fx.athlete.id, &fx.coach.auth).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the coach reads the thresholds: {body}"
    );
    let ftp = anchor(family(&body, "ride"), "ftp");
    assert_eq!(
        ftp["value_text"], "285 W",
        "bug: the coach reads the value the athlete stored: {ftp}"
    );
    assert_eq!(
        ftp["source_text"], "the athlete's own value",
        "bug: a coach reads the athlete's own value as the athlete's: {ftp}"
    );
    assert_eq!(
        fx.reads().await,
        ["physiology_view"],
        "bug: the coach's read is not on the athlete's record"
    );
}

#[tokio::test]
async fn a_coach_with_access_and_consent_sets_a_threshold_stored_as_a_coachs_and_the_athlete_is_told(
) {
    let fx = Fixture::new().await;

    let (status, body) = fx
        .set(
            fx.athlete.id,
            &fx.coach.auth,
            "run/threshold_speed",
            &json!({ "value": 250, "pace_per": "km", "kind": "measured", "confidence": "high", "method": "30-min TT" }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "the coach sets it: {body}");
    let threshold = anchor(family(&body, "run"), "threshold_speed");
    assert_eq!(
        threshold["origin"], "coach",
        "bug: a value the coach sets is stored as a coach's: {threshold}"
    );
    assert_eq!(
        threshold["confidence"], "high",
        "bug: the confidence is the one the coach stated: {threshold}"
    );
    assert_eq!(
        threshold["value_text"], "4:10/km",
        "bug: a pace the coach typed per km does not read back as typed: {threshold}"
    );
    assert_eq!(
        threshold["source_text"], "set by a coach",
        "bug: the coach reads a coach's value as a coach's: {threshold}"
    );

    let own = fx.athletes_own().await;
    let threshold = anchor(family(&own, "run"), "threshold_speed");
    assert_eq!(
        threshold["source_text"],
        "set by your coach", // humanCoach.anchorSource.forAthlete
        "bug: the athlete reads a value their coach set as their coach's: {threshold}"
    );

    let notices = fx.notices(&fx.athlete, fx.athlete.tenant, 1).await;
    assert_eq!(
        notices.len(),
        1,
        "bug: the athlete is told once: {notices:?}"
    );
    let notice = &notices[0];
    // In the athlete's own language (a test account reads French), the anchor
    // named in it from its stored code.
    assert_eq!(
        notice.category,
        NotificationCategory::Coach,
        "bug: a coach's act reaches the athlete in the coach category"
    );
    assert_eq!(
        notice.tenant_id,
        NoticeTenantId(fx.athlete.tenant.as_uuid()),
        "bug: the notice lands outside the athlete's own tenant"
    );
    assert_eq!(
        notice.title,
        format!("{COACH_NAME} a mis à jour tes seuils"),
        "bug: the title does not name the coach in the athlete's language"
    );
    assert_eq!(
        notice.body,
        format!(
            "{COACH_NAME} a modifié une de tes valeurs : allure seuil. Tu la retrouves dans Réglages › Physiologie."
        ),
        "bug: the body does not name the coach and the anchor in the athlete's language"
    );
    let data = notice
        .data
        .as_ref()
        .expect("the notice carries its parameters");
    assert_eq!(
        data["params"],
        json!({ "coach_name": COACH_NAME, "anchor": "threshold_speed" }),
        "bug: the notice does not carry the coach and the anchor's stored code: {data}"
    );
    assert!(
        fx.notices(&fx.coach, fx.coach.tenant, 0).await.is_empty(),
        "bug: the coach is told of their own save"
    );
    assert_eq!(
        fx.reads().await,
        ["physiology_view"],
        "bug: the coach's write, which reads the anchors back, is not on the record"
    );
}

#[tokio::test]
async fn the_athletes_own_edit_tells_no_one() {
    let fx = Fixture::new().await;
    fx.athlete_sets("ride/ftp", &json!({ "value": 285, "kind": "measured" }))
        .await;
    assert!(
        fx.notices(&fx.athlete, fx.athlete.tenant, 0)
            .await
            .is_empty(),
        "bug: the athlete is told of their own edit"
    );
}

#[tokio::test]
async fn a_coach_whose_access_is_off_sets_nothing() {
    let fx = Fixture::new().await;
    fx.repos()
        .users
        .set_manages_roster(fx.coach.id, false)
        .await
        .unwrap();

    let (status, body) = fx
        .set(
            fx.athlete.id,
            &fx.coach.auth,
            "ride/ftp",
            &json!({ "value": 290, "kind": "measured" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "bug: a coach without coach access sets a threshold: {body}"
    );
    assert_eq!(
        reason(&body),
        "no_coach_access",
        "bug: the write is refused for another reason: {body}"
    );
    let (status, body) = fx.view(fx.athlete.id, &fx.coach.auth).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "bug: a coach without coach access reads the thresholds: {body}"
    );
    assert_eq!(
        reason(&body),
        "no_coach_access",
        "bug: the read is refused for another reason: {body}"
    );

    assert_eq!(
        fx.athletes_own().await["sports"],
        json!([]),
        "bug: a coach without coach access set a threshold"
    );
    assert!(
        fx.notices(&fx.athlete, fx.athlete.tenant, 0)
            .await
            .is_empty(),
        "bug: a refused coach told the athlete"
    );
    assert!(fx.reads().await.is_empty(), "a refused coach read nothing");
}

#[tokio::test]
async fn a_coach_without_the_athletes_consent_sets_nothing() {
    let fx = Fixture::new().await;
    assert!(
        fx.repos()
            .groups
            .update_coach_sharing_consent(&fx.group.id.to_string(), fx.athlete.id, false)
            .await
            .unwrap(),
        "fixture: the athlete's consent is revoked"
    );

    let (status, body) = fx
        .set(
            fx.athlete.id,
            &fx.coach.auth,
            "ride/ftp",
            &json!({ "value": 290, "kind": "measured" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "bug: a coach without the athlete's consent sets a threshold: {body}"
    );
    assert_eq!(
        reason(&body),
        "coach_sharing_revoked",
        "bug: the write is refused for another reason: {body}"
    );
    let (status, body) = fx.view(fx.athlete.id, &fx.coach.auth).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "bug: a coach without the athlete's consent reads the thresholds: {body}"
    );
    assert_eq!(
        reason(&body),
        "coach_sharing_revoked",
        "bug: the read is refused for another reason: {body}"
    );

    assert_eq!(
        fx.athletes_own().await["sports"],
        json!([]),
        "bug: a coach without the athlete's consent set a threshold"
    );
    assert!(
        fx.notices(&fx.athlete, fx.athlete.tenant, 0)
            .await
            .is_empty(),
        "bug: a refused coach told the athlete"
    );
}

#[tokio::test]
async fn neither_the_owner_nor_the_coach_of_a_non_member_sets_a_threshold() {
    let fx = Fixture::new().await;

    let (status, body) = fx
        .set(
            fx.athlete.id,
            &fx.owner.auth,
            "ride/ftp",
            &json!({ "value": 290, "kind": "measured" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the owner is not the coach: {body}"
    );
    assert_eq!(
        reason(&body),
        "not_group_coach",
        "bug: the owner is refused for another reason: {body}"
    );

    let (status, body) = fx
        .set(
            fx.outsider.id,
            &fx.coach.auth,
            "ride/ftp",
            &json!({ "value": 290, "kind": "measured" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a non-member is not found: {body}"
    );
    assert_eq!(
        reason(&body),
        "not_a_member",
        "bug: a non-member is refused for another reason: {body}"
    );
}

#[tokio::test]
async fn a_coachs_value_that_contradicts_a_stored_one_is_refused_and_tells_no_one() {
    let fx = Fixture::new().await;
    fx.athlete_sets(
        "ride/critical_power",
        &json!({ "value": 300, "kind": "measured", "confidence": "high" }),
    )
    .await;

    let (status, body) = fx
        .set(
            fx.athlete.id,
            &fx.coach.auth,
            "ride/map",
            &json!({ "value": 290, "kind": "measured" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "bug: a coach's MAP below critical power is saved: {body}"
    );
    assert!(
        body["message"]
            .as_str()
            .is_some_and(|m| m.contains("ride map (290) must be above its critical_power (300)")),
        "the coach's refusal is the writer's own: {body}"
    );
    assert!(
        fx.notices(&fx.athlete, fx.athlete.tenant, 0)
            .await
            .is_empty(),
        "bug: a refused save told the athlete"
    );
}

/// While the athlete's tenant has `set_physiology` switched off, the coach
/// sets nothing: refused after the coaching gates, as the plan read refuses
/// with `get_training_plan` off, with nothing recorded and no one told. The
/// coach still reads the anchors.
#[tokio::test]
async fn a_coach_sets_nothing_while_the_athletes_tenant_switched_set_physiology_off() {
    let fx = Fixture::new().await;
    fx.athlete_sets(
        "ride/ftp",
        &json!({ "value": 285, "kind": "measured", "confidence": "high" }),
    )
    .await;
    fx.res
        .tool_selection()
        .set_tool_override(
            fx.athlete.tenant,
            "set_physiology",
            false,
            fx.athlete.id,
            None,
        )
        .await
        .unwrap();

    let (status, body) = fx
        .set(
            fx.athlete.id,
            &fx.coach.auth,
            "ride/ftp",
            &json!({ "value": 300, "kind": "measured" }),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "bug: a coach's edit went through with set_physiology switched off: {body}"
    );
    assert_eq!(
        reason(&body),
        "physiology_disabled",
        "bug: the refusal does not name its reason: {body}"
    );
    assert_eq!(
        body["message"], "Editing thresholds is turned off in this athlete's workspace",
        "bug: the coach's refusal is not worded for the coach: {body}"
    );
    assert!(
        fx.reads().await.is_empty(),
        "bug: a refused edit went on the athlete's record as a read"
    );
    assert!(
        fx.notices(&fx.athlete, fx.athlete.tenant, 0)
            .await
            .is_empty(),
        "bug: a refused edit told the athlete"
    );

    let (status, body) = fx.view(fx.athlete.id, &fx.coach.auth).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "bug: the write tool's switch hid the anchors from the coach: {body}"
    );
    assert_eq!(
        anchor(family(&body, "ride"), "ftp")["value"],
        285.0,
        "bug: the refused edit replaced the stored FTP: {body}"
    );
}
