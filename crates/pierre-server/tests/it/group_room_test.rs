// ABOUTME: POST /api/chat/groups/{group_id}/room — the caller's conversation in a group's room, found before one is made
// ABOUTME: Pins that asking twice returns one room, that a member's existing room comes back, and that an outsider is refused

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coach Home's "Write in the group" opens the group's room for a human
//! coach who may never have posted in it. A second tap, or a second button
//! for the same group, must land in the same room instead of making another.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use chrono::Utc;
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
};
use pierre_core::models::TenantId;
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::{GroupRoomClaim, NewConversation};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_test_support::delegation::create_group_agent;
use serde_json::Value;
use uuid::Uuid;

struct Person {
    id: Uuid,
    tenant: TenantId,
    auth: String,
}

async fn person(res: &Arc<ServerContext>, label: &str) -> Person {
    let email = format!("{label}-{}@group-room.test", Uuid::new_v4());
    let (id, user, tenant) =
        create_test_user_with_plan(&res.agent.database, &email, "professional")
            .await
            .unwrap();
    res.common
        .repos
        .email_verification
        .mark_verified(id)
        .await
        .unwrap();
    let token = generate_test_token(res, &user).await;
    Person {
        id,
        tenant,
        auth: format!("Bearer {token}"),
    }
}

/// A group `owner` owns, `coach` coaches and `member` belongs to.
struct Fixture {
    res: Arc<ServerContext>,
    coach: Person,
    member: Person,
    outsider: Person,
    group: CoachingGroup,
}

impl Fixture {
    async fn new() -> Self {
        let res = create_test_server_resources().await.unwrap();
        let owner = person(&res, "owner").await;
        let coach = person(&res, "coach").await;
        let member = person(&res, "member").await;
        let outsider = person(&res, "outsider").await;
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
                    name: "Room Squad".to_owned(),
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
                    created_at: now,
                    updated_at: now,
                },
            )
            .await
            .unwrap();
        assert!(
            repos
                .groups
                .set_group_coach_user(&group.id.to_string(), Some(coach.id), owner.tenant)
                .await
                .unwrap(),
            "the fixture seats its coach on the group"
        );
        repos
            .groups
            .add_member(&GroupMember {
                id: Uuid::new_v4(),
                group_id: group.id,
                user_id: member.id,
                tenant_id: member.tenant.to_string(),
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
        Self {
            res,
            coach,
            member,
            outsider,
            group,
        }
    }

    /// `POST /api/chat/groups/{group}/room` as `who`.
    async fn open_as(&self, who: &Person) -> (StatusCode, Value) {
        let resp = AxumTestRequest::post(&format!("/api/chat/groups/{}/room", self.group.id))
            .header("authorization", &who.auth)
            .send(ChatRoutes::routes(Arc::clone(&self.res)))
            .await;
        (resp.status_code(), resp.json())
    }

    /// How many conversations `who` holds in the group's room.
    async fn rooms_held(&self, who: &Person) -> usize {
        self.res
            .common
            .repos
            .chat
            .list_conversations(
                &who.id.to_string(),
                who.tenant,
                100,
                0,
                TransportPolicy::AnyTransport,
            )
            .await
            .unwrap()
            .items
            .iter()
            .filter(|c| c.group_id.as_deref() == Some(self.group.id.to_string().as_str()))
            .count()
    }
}

#[tokio::test]
async fn a_coach_with_no_room_gets_one_and_the_same_one_again() {
    let fx = Fixture::new().await;
    assert_eq!(
        fx.rooms_held(&fx.coach).await,
        0,
        "the coach starts with no room"
    );

    let (status, first) = fx.open_as(&fx.coach).await;
    assert_eq!(status, StatusCode::CREATED, "{first}");
    assert_eq!(
        first["group_id"],
        fx.group.id.to_string(),
        "the room is the group's"
    );

    let (status, again) = fx.open_as(&fx.coach).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(
        again["id"], first["id"],
        "asking again returns the same room"
    );
    assert_eq!(fx.rooms_held(&fx.coach).await, 1, "two asks hold one room");
}

#[tokio::test]
async fn a_member_gets_the_room_they_already_hold() {
    let fx = Fixture::new().await;
    let held = fx
        .res
        .common
        .repos
        .chat
        .create_conversation(
            &fx.member.id.to_string(),
            fx.member.tenant,
            "Room Squad",
            "gemini-2.0-flash",
            None,
            Some(&fx.group.id.to_string()),
        )
        .await
        .unwrap();

    let (status, body) = fx.open_as(&fx.member).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["id"], held.id, "the member's own room comes back");
    assert_eq!(fx.rooms_held(&fx.member).await, 1, "no second room is made");
}

#[tokio::test]
async fn an_outsider_is_refused_and_given_no_room() {
    let fx = Fixture::new().await;
    let (status, body) = fx.open_as(&fx.outsider).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(
        fx.rooms_held(&fx.outsider).await,
        0,
        "a refused open writes nothing"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_devices_opening_at_once_land_in_one_room() {
    let fx = Arc::new(Fixture::new().await);
    let (first, second) = tokio::join!(
        {
            let fx = Arc::clone(&fx);
            tokio::spawn(async move { fx.open_as(&fx.coach).await })
        },
        {
            let fx = Arc::clone(&fx);
            tokio::spawn(async move { fx.open_as(&fx.coach).await })
        },
    );
    let ((first_status, first), (second_status, second)) = (first.unwrap(), second.unwrap());

    assert_eq!(first["id"], second["id"], "{first} / {second}");
    let mut statuses = [first_status, second_status];
    statuses.sort_unstable_by_key(StatusCode::as_u16);
    assert_eq!(
        statuses,
        [StatusCode::OK, StatusCode::CREATED],
        "one open creates the room, the other finds it"
    );
    assert_eq!(
        fx.rooms_held(&fx.coach).await,
        1,
        "racing opens hold one room"
    );
}

#[tokio::test]
async fn the_cap_refuses_only_a_room_not_yet_held() {
    let fx = Fixture::new().await;
    let chat = &fx.res.common.repos.chat;
    let coach = fx.coach.id.to_string();
    let group = fx.group.id.to_string();
    let room = NewConversation {
        user_id: &coach,
        tenant_id: fx.coach.tenant,
        title: "Room Squad",
        model: "gemini-2.0-flash",
        agent_id: None,
        group_id: Some(&group),
    };
    chat.create_conversation(
        &coach,
        fx.coach.tenant,
        "Elsewhere",
        "gemini-2.0-flash",
        None,
        None,
    )
    .await
    .unwrap();

    match chat.open_group_conversation(&room, Some(1)).await.unwrap() {
        GroupRoomClaim::AtCap { current } => {
            assert_eq!(current, 1, "the cap names the slots the coach holds");
        }
        other => panic!("opened a room past the cap: {other:?}"),
    }
    assert_eq!(
        fx.rooms_held(&fx.coach).await,
        0,
        "a refused open writes nothing"
    );

    let opened = match chat.open_group_conversation(&room, Some(2)).await.unwrap() {
        GroupRoomClaim::Opened(conversation) => conversation,
        other => panic!("a free slot did not open the room: {other:?}"),
    };
    assert_eq!(
        opened.group_id.as_deref(),
        Some(group.as_str()),
        "the opened room is the group's"
    );

    // Holding the room, the coach gets it back even at the cap.
    match chat.open_group_conversation(&room, Some(2)).await.unwrap() {
        GroupRoomClaim::Held(conversation) => {
            assert_eq!(conversation.id, opened.id, "the held room comes back");
        }
        other => panic!("a held room was not returned at the cap: {other:?}"),
    }
}

#[tokio::test]
async fn opening_a_room_for_no_group_is_refused() {
    let fx = Fixture::new().await;
    let coach = fx.coach.id.to_string();
    let err = fx
        .res
        .common
        .repos
        .chat
        .open_group_conversation(
            &NewConversation {
                user_id: &coach,
                tenant_id: fx.coach.tenant,
                title: "No group",
                model: "gemini-2.0-flash",
                agent_id: None,
                group_id: None,
            },
            None,
        )
        .await
        .expect_err("a room is opened for a group");
    assert!(
        err.to_string()
            .contains("a group room is opened for a group"),
        "the refusal names the missing group: {err}"
    );
}
