// ABOUTME: The public invite card, the coach's invite funnel and a group owner claiming their coachless group (carnet#1649, carnet#1650)
// ABOUTME: Drives GET /api/invites/{code}, GET /api/groups/{id}/invites and PUT /api/groups/{id}/coach over one database
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! What the invite card shows and never shows, who is counted as having
//! opened a link, what the coach's list says of how far each link got, and
//! how an owner with coach access becomes the coach of their coachless group:
//!
//! 1. the card answers without a session, names the sender by first name and
//!    last initial, and carries no id; every unknown code is the same 404;
//! 2. each opening of a known link is counted except by the people running
//!    the group, and a read signed out or with a broken session counts;
//! 3. the card is metered per address;
//! 4. the coach's list carries a funnel per invite, lapsed holds excluded,
//!    from one tenant-scoped query;
//! 5. the claim attaches the owner as coach through the grant's own attach,
//!    with the grant's room line, and refuses as 403 / 409 / 404.

use crate::common;
use crate::held_group_invite_test::{
    athlete, bearer, body_of, expire_invite, held, hold_link, join_held, rendered, Coach,
};
use crate::helpers;

use std::fmt::Display;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::Arc;

use axum::extract::connect_info::MockConnectInfo;
use axum::Router;
use chrono::{Duration, Utc};
use common::create_test_server_resources;
use helpers::axum_test::AxumTestRequest;
use pierre_config::constants::oauth_rate_limiting::INVITE_PREVIEW_ATTEMPTS;
use pierre_contremaitre::messaging_strings::KEY_COACH_ACCESS_ROOM_LINE_COACH;
use pierre_core::models::groups::GroupInviteKind;
use pierre_core::models::{GroupInviteFunnelRow, User, UserStatus};
use pierre_groups::SIGNUP_INVITE_MAX_DAYS;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_auth::AuthRoutes;
use pierre_routes_groups::GroupRoutes;
use serde_json::{json, Value};
use uuid::Uuid;

const SENDER: &str = "Jeremy Bonneau";

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// The auth routes behind client address `ip`.
fn routes_from(resources: &ServerContext, ip: Ipv4Addr) -> Router {
    AuthRoutes::routes(resources.auth_routes_context())
        .layer(MockConnectInfo(SocketAddr::new(IpAddr::V4(ip), 40_000)))
}

/// The auth routes behind a fresh client address per call, so the per-address
/// card window never decides a test that is not about it.
fn fresh_routes(resources: &ServerContext) -> Router {
    static NEXT: AtomicU16 = AtomicU16::new(1);
    let [hi, lo] = NEXT.fetch_add(1, Ordering::Relaxed).to_be_bytes();
    routes_from(resources, Ipv4Addr::new(198, 20, hi, lo))
}

/// `GET /api/invites/{code}`, signed out or with `authorization`.
async fn card(resources: &ServerContext, code: &str, authorization: Option<&str>) -> (u16, Value) {
    let mut request = AxumTestRequest::get(&format!("/api/invites/{code}"));
    if let Some(authorization) = authorization {
        request = request.header("authorization", authorization);
    }
    let response = request.send(fresh_routes(resources)).await;
    (response.status(), body_of(&response.body_text()))
}

fn group_routes(resources: &Arc<ServerContext>) -> Router {
    GroupRoutes::routes(Arc::clone(resources))
}

/// `GET /api/groups/{id}/invites` as `authorization`.
async fn list_invites(
    resources: &Arc<ServerContext>,
    group_id: Uuid,
    authorization: &str,
) -> (u16, Value) {
    let response = AxumTestRequest::get(&format!("/api/groups/{group_id}/invites"))
        .header("authorization", authorization)
        .send(group_routes(resources))
        .await;
    (response.status(), body_of(&response.body_text()))
}

/// `PUT /api/groups/{id}/coach` as `authorization`.
async fn claim(
    resources: &Arc<ServerContext>,
    group_id: impl Display,
    authorization: &str,
) -> (u16, Value) {
    let response = AxumTestRequest::put(&format!("/api/groups/{group_id}/coach"))
        .header("authorization", authorization)
        .send(group_routes(resources))
        .await;
    (response.status(), body_of(&response.body_text()))
}

/// A coach whose profile names them [`SENDER`].
async fn named_coach(resources: &Arc<ServerContext>, max_members: i32) -> Coach {
    let coach = Coach::new(resources, max_members).await;
    resources
        .common
        .repos
        .users
        .update_display_name(coach.id, SENDER)
        .await
        .unwrap();
    coach
}

/// Open count and last opening of every invite of the coach's group, read
/// through the repository the list reads.
async fn funnel_rows(resources: &ServerContext, coach: &Coach) -> Vec<GroupInviteFunnelRow> {
    resources
        .common
        .repos
        .signup_codes
        .group_invite_funnels(
            coach.group_id,
            &coach.tenant.to_string(),
            Utc::now() - Duration::days(SIGNUP_INVITE_MAX_DAYS),
        )
        .await
        .unwrap()
}

async fn open_count(resources: &ServerContext, coach: &Coach, invite_id: Uuid) -> i64 {
    funnel_rows(resources, coach)
        .await
        .into_iter()
        .find(|row| row.invite_id == invite_id)
        .expect("the invite has a funnel row")
        .open_count
}

// ---------------------------------------------------------------------------
// 1. The card
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_live_card_names_the_sender_and_the_group_and_carries_no_id() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = named_coach(&resources, 10).await;
    let invite = coach.invite(&resources, None).await;

    let (status, body) = card(&resources, &invite.code, None).await;
    assert_eq!(status, 200, "signed out: {body}");
    assert!(
        body["expires_at"].is_string(),
        "a link with an expiry says when: {body}"
    );
    let mut keys: Vec<&str> = body
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "coach_name",
            "expires_at",
            "group_name",
            "has_coach",
            "inviter_initials",
            "inviter_name",
            "kind",
            "status"
        ],
        "exactly the card's fields: {body}"
    );
    assert_eq!(body["status"], "live");
    assert_eq!(body["kind"], "member");
    assert_eq!(body["group_name"], "Held Invite Crew");
    assert_eq!(body["inviter_name"], "Jeremy B.");
    assert_eq!(body["inviter_initials"], "JB");
    assert_eq!(
        body["has_coach"], false,
        "a group with no coach says so, so the card claims no sharing: {body}"
    );
    assert!(body["coach_name"].is_null(), "{body}");

    let text = body.to_string();
    for secret in [
        invite.id.to_string(),
        coach.group_id.to_string(),
        coach.id.to_string(),
        coach.tenant.to_string(),
        invite.code.clone(),
    ] {
        assert!(!text.contains(&secret), "the card leaks {secret}: {text}");
    }

    // The code is read as the link carried it, in any case.
    let (status, lower) = card(&resources, &invite.code.to_lowercase(), None).await;
    assert_eq!(status, 200, "{lower}");
    assert_eq!(lower["group_name"], body["group_name"]);
}

#[tokio::test]
async fn a_coach_invite_card_says_it_is_the_coach_seat() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = named_coach(&resources, 10).await;
    let invite = coach
        .invite_of(&resources, GroupInviteKind::Coach, None)
        .await;
    let (status, body) = card(&resources, &invite.code, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["kind"], "coach");
    assert_eq!(body["status"], "live");
}

#[tokio::test]
async fn a_card_says_why_a_link_no_longer_works() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = named_coach(&resources, 10).await;
    let repos = &resources.common.repos;

    let expired = coach.invite(&resources, None).await;
    expire_invite(
        &resources,
        coach.tenant,
        expired.id,
        Utc::now() - Duration::days(1),
    )
    .await;
    let (status, body) = card(&resources, &expired.code, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["status"], "expired", "{body}");

    let single_use = coach.invite(&resources, Some(1)).await;
    let (joiner, _) = athlete(&resources, "uses-it-up").await;
    resources
        .group_service()
        .join_group(&single_use.code, joiner.id, coach.tenant)
        .await
        .unwrap();
    let (_, body) = card(&resources, &single_use.code, None).await;
    assert_eq!(body["status"], "used_up", "{body}");

    let withdrawn = coach.invite(&resources, None).await;
    assert!(repos
        .groups
        .deactivate_invite(&coach.group_id.to_string(), &withdrawn.id.to_string())
        .await
        .unwrap());
    let (status, body) = card(&resources, &withdrawn.code, None).await;
    assert_eq!(status, 200, "a withdrawn link still has a card: {body}");
    assert_eq!(body["status"], "inactive", "{body}");

    let archived = Coach::new(&resources, 10).await;
    let archived_invite = archived.invite(&resources, None).await;
    assert!(repos
        .groups
        .delete_group(&archived.group_id.to_string(), archived.tenant)
        .await
        .unwrap());
    let (_, body) = card(&resources, &archived_invite.code, None).await;
    assert_eq!(body["status"], "inactive", "an archived group: {body}");
}

#[tokio::test]
async fn every_unknown_code_is_the_same_404() {
    let resources = create_test_server_resources().await.unwrap();
    let (first_status, first) = card(&resources, "ZZZZ9999", None).await;
    let (second_status, second) = card(&resources, "QQQQ2222", None).await;
    let (blank_status, blank) = card(&resources, "%20", None).await;
    assert_eq!((first_status, second_status, blank_status), (404, 404, 404));
    let reading = |body: &Value| (body["code"].clone(), body["message"].clone());
    assert_eq!(
        reading(&first),
        reading(&second),
        "no unknown code reads differently"
    );
    assert_eq!(
        reading(&first),
        reading(&blank),
        "a blank code reads like any unknown one"
    );
    assert!(
        !first.to_string().contains("ZZZZ9999"),
        "the code is never echoed: {first}"
    );
}

#[tokio::test]
async fn a_sender_with_no_name_reads_as_none_never_by_email() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = Coach::new(&resources, 10).await;
    // The link comes from an account that never set a display name (every
    // fixture account has one, so this one is made bare).
    let mut sender = User::new(
        format!("nameless-sender-{}@held-invite.test", Uuid::new_v4()),
        "hash_not_verified_in_tests".to_owned(),
        None,
    );
    sender.user_status = UserStatus::Active;
    resources.common.repos.users.create(&sender).await.unwrap();
    let invite = resources
        .group_service()
        .create_invite(
            coach.group_id,
            sender.id,
            coach.tenant,
            Some(14),
            None,
            GroupInviteKind::Member,
        )
        .await
        .unwrap();

    let (status, body) = card(&resources, &invite.code, None).await;
    assert_eq!(status, 200, "{body}");
    assert!(body["inviter_name"].is_null(), "no stand-in name: {body}");
    assert!(body["inviter_initials"].is_null(), "{body}");
    assert_eq!(
        body["group_name"], "Held Invite Crew",
        "the group still names the invite: {body}"
    );
    let text = body.to_string();
    assert!(
        !text.contains(&sender.email) && !text.contains(sender.email.split('@').next().unwrap()),
        "the card never shows the email: {text}"
    );
}

#[tokio::test]
async fn the_card_names_the_groups_coach_who_reads_a_members_training() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = named_coach(&resources, 10).await;
    let invite = coach.invite(&resources, None).await;
    // Another account coaches the group: the sharing line names them, not the
    // inviter (ADR-002).
    let (other, _) = athlete(&resources, "group-coach").await;
    resources
        .common
        .repos
        .users
        .update_display_name(other.id, "Ana Lopez")
        .await
        .unwrap();
    assert!(resources
        .common
        .repos
        .groups
        .set_group_coach_user(&coach.group_id.to_string(), Some(other.id), coach.tenant)
        .await
        .unwrap());

    let (status, body) = card(&resources, &invite.code, None).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["has_coach"], true, "{body}");
    assert_eq!(body["coach_name"], "Ana L.", "{body}");
    assert_eq!(
        body["inviter_name"], "Jeremy B.",
        "the inviter is still who sent it: {body}"
    );
}

// ---------------------------------------------------------------------------
// 2. Who is counted as having opened a link
// ---------------------------------------------------------------------------

#[tokio::test]
async fn openings_are_counted_except_by_the_people_running_the_group() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = named_coach(&resources, 10).await;
    let invite = coach.invite(&resources, None).await;
    let staff = coach.staff_coach(&resources).await;

    assert_eq!(
        open_count(&resources, &coach, invite.id).await,
        0,
        "nothing opened yet"
    );

    // The creator (who is also the group's owner) opening it is not counted.
    assert_eq!(
        card(&resources, &invite.code, Some(&coach.auth)).await.0,
        200
    );
    assert_eq!(open_count(&resources, &coach, invite.id).await, 0);

    // Nor is the group's attached coach.
    assert!(resources
        .common
        .repos
        .groups
        .set_group_coach_user(&coach.group_id.to_string(), Some(staff.id), coach.tenant)
        .await
        .unwrap());
    let staff_auth = bearer(&resources, &staff).await;
    assert_eq!(
        card(&resources, &invite.code, Some(&staff_auth)).await.0,
        200
    );
    assert_eq!(
        open_count(&resources, &coach, invite.id).await,
        0,
        "the group's coach is not a reader"
    );

    // A stranger's session, a signed-out read and a broken session all count.
    let (stranger, _) = athlete(&resources, "stranger").await;
    let stranger_auth = bearer(&resources, &stranger).await;
    assert_eq!(
        card(&resources, &invite.code, Some(&stranger_auth)).await.0,
        200
    );
    assert_eq!(card(&resources, &invite.code, None).await.0, 200);
    let (status, body) = card(&resources, &invite.code, Some("Bearer not.a.session")).await;
    assert_eq!(status, 200, "a broken session is never a refusal: {body}");
    assert_eq!(open_count(&resources, &coach, invite.id).await, 3);

    let row = funnel_rows(&resources, &coach)
        .await
        .into_iter()
        .find(|row| row.invite_id == invite.id)
        .unwrap();
    assert!(row.last_opened_at.is_some(), "the last opening is stamped");

    // An unknown code counts nothing anywhere.
    card(&resources, "ZZZZ9999", None).await;
    assert_eq!(open_count(&resources, &coach, invite.id).await, 3);
}

#[tokio::test]
async fn an_opening_is_counted_only_under_the_invites_own_tenant() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = named_coach(&resources, 10).await;
    let other = Coach::new(&resources, 10).await;
    let invite = coach.invite(&resources, None).await;
    let signup_codes = &resources.common.repos.signup_codes;

    assert!(
        !signup_codes
            .record_group_invite_open(invite.id, &other.tenant.to_string(), Utc::now())
            .await
            .unwrap(),
        "another tenant's id counts nothing"
    );
    assert_eq!(open_count(&resources, &coach, invite.id).await, 0);
    assert!(signup_codes
        .record_group_invite_open(invite.id, &coach.tenant.to_string(), Utc::now())
        .await
        .unwrap());
    assert_eq!(open_count(&resources, &coach, invite.id).await, 1);
}

// ---------------------------------------------------------------------------
// 3. The meter
// ---------------------------------------------------------------------------

#[tokio::test]
async fn too_many_cards_from_one_address_are_refused() {
    let resources = create_test_server_resources().await.unwrap();
    let client = Ipv4Addr::new(198, 51, 100, 77);
    for attempt in 0..INVITE_PREVIEW_ATTEMPTS {
        let response = AxumTestRequest::get(&format!("/api/invites/GUESS{attempt:03}"))
            .send(routes_from(&resources, client))
            .await;
        assert_eq!(
            response.status(),
            404,
            "attempt {attempt} is an unknown code"
        );
    }
    let response = AxumTestRequest::get("/api/invites/GUESSMORE")
        .send(routes_from(&resources, client))
        .await;
    assert_eq!(response.status(), 429, "past the window, the address waits");

    // Another address is not held up by it.
    let (status, _) = card(&resources, "GUESSMORE", None).await;
    assert_eq!(status, 404);
}

// ---------------------------------------------------------------------------
// 4. The funnel on the coach's list
// ---------------------------------------------------------------------------

#[tokio::test]
async fn the_invite_list_carries_each_links_funnel() {
    let resources = create_test_server_resources().await.unwrap();
    // The owner's membership plus one athlete fills it.
    let coach = named_coach(&resources, 2).await;
    let busy = coach.invite(&resources, None).await;
    let quiet = coach.invite(&resources, None).await;

    let (status, body) = list_invites(&resources, coach.group_id, &coach.auth).await;
    assert_eq!(status, 200, "{body}");
    let quiet_row = body["invites"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == quiet.id.to_string())
        .expect("the quiet invite is listed");
    assert_eq!(
        quiet_row["funnel"],
        json!({
            "open_count": 0,
            "last_opened_at": null,
            "held_count": 0,
            "last_held_at": null,
            "full_count": 0,
        }),
        "a link nobody touched is all zeros"
    );
    assert_eq!(
        quiet_row["code"], quiet.code,
        "the invite's own fields are still there"
    );

    // Two outside openings, three holds, one lapsed, one refused by a full
    // group, and one account that joined.
    card(&resources, &busy.code, None).await;
    card(&resources, &busy.code, None).await;
    card(&resources, &busy.code, Some(&coach.auth)).await; // not counted

    let (joined, _) = athlete(&resources, "joined").await;
    let (refused, _) = athlete(&resources, "refused").await;
    let (waiting, _) = athlete(&resources, "waiting").await;
    let (stale, _) = athlete(&resources, "stale").await;
    for user in [&joined, &refused, &waiting] {
        assert_eq!(hold_link(&resources, user, &busy.code).await.0, 200);
    }
    // The first account to ask joins; the group is then full.
    let (_, body) = join_held(&resources, &joined).await;
    assert_eq!(body["outcome"], "joined", "{body}");
    let (_, body) = join_held(&resources, &refused).await;
    assert_eq!(body["reason"], "group_full", "{body}");
    resources
        .common
        .repos
        .signup_codes
        .hold_group_invite(
            stale.id,
            busy.id,
            Utc::now() - Duration::days(SIGNUP_INVITE_MAX_DAYS + 1),
        )
        .await
        .unwrap();
    assert!(held(&resources, stale.id).await.is_some());

    let (status, body) = list_invites(&resources, coach.group_id, &coach.auth).await;
    assert_eq!(status, 200, "{body}");
    let rows = body["invites"].as_array().unwrap();
    let busy_row = rows
        .iter()
        .find(|row| row["id"] == busy.id.to_string())
        .unwrap();
    let funnel = &busy_row["funnel"];
    assert_eq!(funnel["open_count"], 2, "{funnel}");
    assert!(funnel["last_opened_at"].is_string(), "{funnel}");
    assert_eq!(
        funnel["held_count"], 2,
        "refused and waiting hold it; the joiner released theirs and the stale hold lapsed: {funnel}"
    );
    assert_eq!(funnel["full_count"], 1, "{funnel}");
    assert!(funnel["last_held_at"].is_string(), "{funnel}");
    let quiet_row = rows
        .iter()
        .find(|row| row["id"] == quiet.id.to_string())
        .unwrap();
    assert_eq!(
        quiet_row["funnel"]["held_count"], 0,
        "another invite's holds are not counted here"
    );
}

#[tokio::test]
async fn another_tenants_coach_cannot_read_a_groups_funnel() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = named_coach(&resources, 10).await;
    let outsider = Coach::new(&resources, 10).await;
    let invite = coach.invite(&resources, None).await;
    card(&resources, &invite.code, None).await;

    let (status, body) = list_invites(&resources, coach.group_id, &outsider.auth).await;
    assert_eq!(status, 404, "another tenant's coach is no member: {body}");
    assert!(!body.to_string().contains(&invite.code), "{body}");

    // And the aggregate itself is tenant-scoped.
    let leaked = resources
        .common
        .repos
        .signup_codes
        .group_invite_funnels(
            coach.group_id,
            &outsider.tenant.to_string(),
            Utc::now() - Duration::days(SIGNUP_INVITE_MAX_DAYS),
        )
        .await
        .unwrap();
    assert!(leaked.is_empty(), "no row crosses tenants: {leaked:?}");
}

// ---------------------------------------------------------------------------
// 5. Becoming the coach of a coachless group
// ---------------------------------------------------------------------------

#[tokio::test]
async fn an_owner_with_coach_access_becomes_the_coach_of_their_coachless_group() {
    let resources = create_test_server_resources().await.unwrap();
    let coach = named_coach(&resources, 10).await;
    assert_eq!(
        coach.coach_user_id(&resources).await,
        None,
        "fixture: coachless"
    );
    // The owner's own copy of the group's chat, where the room line lands.
    let chat = resources
        .common
        .repos
        .chat
        .create_conversation(
            &coach.id.to_string(),
            coach.tenant,
            "Held Invite Crew",
            "gemini-2.0-flash",
            None,
            Some(&coach.group_id.to_string()),
        )
        .await
        .unwrap()
        .id;

    let (status, body) = claim(&resources, coach.group_id, &coach.auth).await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["group"]["id"], coach.group_id.to_string(), "{body}");
    assert_eq!(
        body["group"]["coach_user_id"],
        coach.id.to_string(),
        "the answer carries the group with its coach: {body}"
    );
    assert_eq!(coach.coach_user_id(&resources).await, Some(coach.id));

    let line = rendered(&resources, KEY_COACH_ACCESS_ROOM_LINE_COACH, &[SENDER]);
    assert!(!line.trim().is_empty(), "fixture: the room line renders");
    assert_eq!(
        coach.rows_reading(&resources, &chat, &line).await,
        1,
        "the grant's room line is in the coach's copy of the room"
    );

    // A second claim finds a coach in place.
    let (status, body) = claim(&resources, coach.group_id, &coach.auth).await;
    assert_eq!(status, 409, "{body}");
}

#[tokio::test]
async fn a_claim_is_refused_without_ownership_or_coach_access_or_over_a_coach() {
    let resources = create_test_server_resources().await.unwrap();

    // Owner without coach access: 403, and the group stays coachless.
    let plain_owner = Coach::new(&resources, 10).await;
    resources
        .common
        .repos
        .users
        .set_manages_roster(plain_owner.id, false)
        .await
        .unwrap();
    let (status, body) = claim(&resources, plain_owner.group_id, &plain_owner.auth).await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(plain_owner.coach_user_id(&resources).await, None);

    // A coach of the same tenant who does not own the group: 403.
    let owner = Coach::new(&resources, 10).await;
    let staff = owner.staff_coach(&resources).await;
    let staff_auth = bearer(&resources, &staff).await;
    let (status, body) = claim(&resources, owner.group_id, &staff_auth).await;
    assert_eq!(status, 403, "{body}");
    assert_eq!(owner.coach_user_id(&resources).await, None);

    // A group that already has a coach: 409, whoever the coach is.
    assert!(resources
        .common
        .repos
        .groups
        .set_group_coach_user(&owner.group_id.to_string(), Some(staff.id), owner.tenant)
        .await
        .unwrap());
    let (status, body) = claim(&resources, owner.group_id, &owner.auth).await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(owner.coach_user_id(&resources).await, Some(staff.id));
}

#[tokio::test]
async fn a_claim_never_reaches_an_unknown_other_tenant_or_archived_group() {
    let resources = create_test_server_resources().await.unwrap();
    let owner = Coach::new(&resources, 10).await;
    let other_tenant = Coach::new(&resources, 10).await;
    assert_ne!(owner.tenant, other_tenant.tenant, "fixture: two tenants");

    let (status, body) = claim(&resources, Uuid::new_v4(), &owner.auth).await;
    assert_eq!(status, 404, "an unknown group: {body}");

    let (status, body) = claim(&resources, owner.group_id, &other_tenant.auth).await;
    assert_eq!(status, 404, "another tenant's group: {body}");
    assert_eq!(
        owner.coach_user_id(&resources).await,
        None,
        "nothing attached"
    );

    assert!(resources
        .common
        .repos
        .groups
        .delete_group(&owner.group_id.to_string(), owner.tenant)
        .await
        .unwrap());
    let (status, body) = claim(&resources, owner.group_id, &owner.auth).await;
    assert_eq!(status, 404, "an archived group: {body}");
}
