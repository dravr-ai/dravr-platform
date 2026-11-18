// ABOUTME: An admin's support session makes no grant through a slash command: /group invite, /group join, /group consent … yes
// ABOUTME: Each is refused on the web chat route under impersonation and served on the athlete's own session; reads still run

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! The invite and consent routes refuse an admin's support session
//! (carnet#1634). The same grants are one chat message away: `/group invite`
//! mints a code, `/group join <code>` joins a group (which grants its coach
//! sharing), `/group consent … yes` turns a sharing consent on. Each test
//! types the command twice on the same athlete: under an impersonation token
//! it is refused before the turn starts and nothing is written; on the
//! athlete's own session it is served.

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::http::StatusCode;
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use helpers::axum_test::{AxumTestRequest, AxumTestResponse};
use helpers::impersonation::{assert_refusal_audited, assert_support_refused, impersonation_of};
use pierre_core::models::agents::CreateAgentRequest;
use pierre_core::models::groups::{CreateGroupRequest, GroupInviteKind};
use pierre_core::models::{TenantId, COMMAND_FINISH_REASON};
use pierre_groups::strategies::tier::ProfessionalTierStrategy;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::{ChatRoutes, ConversationResponse, TurnResponse};
use serde_json::json;
use uuid::Uuid;

/// An athlete on a tenant of their own, with their session and an admin's
/// support session on them.
struct Athlete {
    id: Uuid,
    tenant_id: TenantId,
    session: String,
    support: String,
}

async fn athlete(resources: &Arc<ServerContext>, email: &str) -> Athlete {
    let (id, user, tenant_id) =
        create_test_user_with_plan(&resources.agent.database, email, "professional")
            .await
            .unwrap();
    let session = format!("Bearer {}", generate_test_token(resources, &user).await);
    let support = impersonation_of(resources, &session).await;
    Athlete {
        id,
        tenant_id,
        session,
        support,
    }
}

/// A group `owner` owns, with one open member invite; returns the group id
/// and the code.
async fn group_with_invite(resources: &Arc<ServerContext>, owner: &Athlete) -> (Uuid, String) {
    let agent: CreateAgentRequest = serde_json::from_value(json!({
        "title": "Group Coach",
        "description": null,
        "system_prompt": "You coach the group.",
    }))
    .unwrap();
    let agent = resources
        .common
        .repos
        .agents
        .create(owner.id, owner.tenant_id, &agent)
        .await
        .unwrap();
    let request = CreateGroupRequest {
        name: "Support Crew".to_owned(),
        description: None,
        agent_id: agent.id.to_string(),
        max_members: None,
        coach_user_id: None,
    };
    let group = resources
        .group_service()
        .create_group(
            &request,
            owner.id,
            owner.tenant_id,
            &ProfessionalTierStrategy,
        )
        .await
        .unwrap();
    let invite = resources
        .group_service()
        .create_invite(
            group.id,
            owner.id,
            owner.tenant_id,
            None,
            None,
            GroupInviteKind::Member,
        )
        .await
        .unwrap();
    (group.id, invite.code)
}

/// A conversation of `athlete`'s, bound to `group_id` when one is given.
async fn conversation(
    resources: &Arc<ServerContext>,
    router: &axum::Router,
    athlete: &Athlete,
    group_id: Option<Uuid>,
) -> String {
    let resp = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &athlete.session)
        .json(&json!({ "title": "Support thread" }))
        .send(router.clone())
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::CREATED,
        "the athlete's own session opens the thread: {}",
        resp.body_text()
    );
    let id = resp.json::<ConversationResponse>().id;
    if let Some(group_id) = group_id {
        resources
            .common
            .repos
            .chat
            .set_conversation_group_id(&id, Some(&group_id.to_string()), athlete.tenant_id)
            .await
            .unwrap();
    }
    id
}

async fn type_in(
    router: &axum::Router,
    bearer: &str,
    conversation_id: &str,
    text: &str,
) -> AxumTestResponse {
    AxumTestRequest::post(&format!(
        "/api/chat/conversations/{conversation_id}/messages"
    ))
    .header("authorization", bearer)
    .json(&json!({ "content": text }))
    .send(router.clone())
    .await
}

fn assert_refused(response: &AxumTestResponse, text: &str) {
    assert_support_refused(
        response.status(),
        &response.body_text(),
        &format!("run `{text}`"),
    );
}

fn assert_command_served(response: AxumTestResponse, text: &str) {
    assert_eq!(
        response.status_code(),
        StatusCode::OK,
        "`{text}`: {}",
        response.body_text()
    );
    let turn: TurnResponse = response.json();
    assert_eq!(
        turn.assistant.finish_reason.as_deref(),
        Some(COMMAND_FINISH_REASON),
        "`{text}` is answered as a command"
    );
}

#[tokio::test]
async fn a_support_session_mints_no_invite_from_the_chat() {
    let resources = create_test_server_resources().await.unwrap();
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let owner = athlete(&resources, "imp-chat-invite@example.com").await;
    let (group_id, _) = group_with_invite(&resources, &owner).await;
    let thread = conversation(&resources, &router, &owner, Some(group_id)).await;
    let invites = || async {
        resources
            .group_service()
            .list_invites(&group_id.to_string())
            .await
            .unwrap()
            .len()
    };
    assert_eq!(invites().await, 1, "the seeded invite");

    for text in ["/group invite", "/group invite coach"] {
        assert_refused(&type_in(&router, &owner.support, &thread, text).await, text);
    }
    assert_eq!(invites().await, 1, "no invite was minted");
    assert_refusal_audited(&resources, owner.id, "run the group-invite command").await;

    let text = "/group invite";
    assert_command_served(type_in(&router, &owner.session, &thread, text).await, text);
    assert_eq!(invites().await, 2, "the owner's own session mints one");
}

#[tokio::test]
async fn a_support_session_joins_no_group_from_the_chat() {
    let resources = create_test_server_resources().await.unwrap();
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let owner = athlete(&resources, "imp-chat-join-owner@example.com").await;
    let (group_id, code) = group_with_invite(&resources, &owner).await;
    let member = athlete(&resources, "imp-chat-join-member@example.com").await;
    let thread = conversation(&resources, &router, &member, None).await;
    let joined = || async {
        resources
            .common
            .repos
            .groups
            .get_member(&group_id.to_string(), member.id)
            .await
            .unwrap()
            .is_some()
    };

    let text = format!("/group join {code}");
    assert_refused(
        &type_in(&router, &member.support, &thread, &text).await,
        &text,
    );
    assert!(!joined().await, "the support session joined no group");
    assert_refusal_audited(&resources, member.id, "run the group-join command").await;

    assert_command_served(
        type_in(&router, &member.session, &thread, &text).await,
        &text,
    );
    assert!(joined().await, "the athlete's own session joins");
}

#[tokio::test]
async fn a_support_session_grants_no_sharing_from_the_chat_but_may_withdraw() {
    let resources = create_test_server_resources().await.unwrap();
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let owner = athlete(&resources, "imp-chat-consent-owner@example.com").await;
    let (group_id, code) = group_with_invite(&resources, &owner).await;
    let member = athlete(&resources, "imp-chat-consent-member@example.com").await;
    let solo = conversation(&resources, &router, &member, None).await;
    let join = format!("/group join {code}");
    assert_command_served(type_in(&router, &member.session, &solo, &join).await, &join);
    let thread = conversation(&resources, &router, &member, Some(group_id)).await;
    let consents = || async {
        let row = resources
            .common
            .repos
            .groups
            .get_member(&group_id.to_string(), member.id)
            .await
            .unwrap()
            .expect("a live member");
        (row.peer_sharing_consent, row.coach_sharing_consent)
    };

    // Withdrawing narrows who reads the athlete: still served.
    let text = "/group consent coach no";
    assert_command_served(type_in(&router, &member.support, &thread, text).await, text);
    assert_eq!(
        consents().await,
        (false, false),
        "the withdrawal is served and stored"
    );

    for text in ["/group consent yes", "/group consent coach yes"] {
        assert_refused(
            &type_in(&router, &member.support, &thread, text).await,
            text,
        );
    }
    assert_eq!(consents().await, (false, false), "nothing was granted");
    assert_refusal_audited(&resources, member.id, "run the group-consent command").await;

    for text in ["/group consent yes", "/group consent coach yes"] {
        assert_command_served(type_in(&router, &member.session, &thread, text).await, text);
    }
    assert_eq!(
        consents().await,
        (true, true),
        "the member's own session grants"
    );
}

/// `/privacy on` gives the analytics consent only the athlete gives; a
/// support session may turn it off, never on (carnet#1634).
#[tokio::test]
async fn a_support_session_turns_privacy_off_never_on() {
    let resources = create_test_server_resources().await.unwrap();
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let a = athlete(&resources, "imp-chat-privacy@example.com").await;
    let thread = conversation(&resources, &router, &a, None).await;
    let consent = || async {
        resources
            .common
            .repos
            .users
            .get_global(a.id)
            .await
            .unwrap()
            .unwrap()
            .analytics_consent
    };

    let off = "/privacy off";
    assert_command_served(type_in(&router, &a.support, &thread, off).await, off);
    let on = "/privacy on";
    assert_refused(&type_in(&router, &a.support, &thread, on).await, on);
    assert!(!consent().await, "no consent was given");
    assert_refusal_audited(&resources, a.id, "run the privacy-on command").await;

    assert_command_served(type_in(&router, &a.session, &thread, on).await, on);
    assert!(consent().await, "the athlete's own session gives it");
}

#[tokio::test]
async fn a_support_session_still_runs_a_command_that_grants_nothing() {
    let resources = create_test_server_resources().await.unwrap();
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let a = athlete(&resources, "imp-chat-read@example.com").await;
    let thread = conversation(&resources, &router, &a, None).await;

    for text in ["/help", "/status"] {
        assert_command_served(type_in(&router, &a.support, &thread, text).await, text);
    }
}

/// `/deleteaccount <email>` deletes the account, a decision only its owner
/// makes; an admin deletes through the admin console. The bare warning
/// deletes nothing and is still read (carnet#1634).
#[tokio::test]
async fn a_support_session_deletes_no_account_from_the_chat() {
    let resources = create_test_server_resources().await.unwrap();
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let email = "imp-chat-delete@example.com";
    let a = athlete(&resources, email).await;
    let thread = conversation(&resources, &router, &a, None).await;
    let exists = || async {
        resources
            .common
            .repos
            .users
            .get_global(a.id)
            .await
            .unwrap()
            .is_some()
    };

    let warning = "/deleteaccount";
    assert_command_served(
        type_in(&router, &a.support, &thread, warning).await,
        warning,
    );
    let delete = format!("/deleteaccount {email}");
    assert_refused(
        &type_in(&router, &a.support, &thread, &delete).await,
        &delete,
    );
    assert!(exists().await, "the support session deleted nothing");
    assert_refusal_audited(&resources, a.id, "run the delete-account command").await;

    assert_command_served(
        type_in(&router, &a.session, &thread, &delete).await,
        &delete,
    );
    assert!(!exists().await, "the athlete's own session deletes it");
}

/// A tapped suggestion is resolved into the words it stands for inside the
/// pipeline, after the chat route read the postback. An agent's Example Input
/// that is a granting command is refused there all the same (carnet#1634).
#[tokio::test]
async fn a_support_session_grants_nothing_through_a_tapped_suggestion() {
    let resources = create_test_server_resources().await.unwrap();
    let router = ChatRoutes::routes(Arc::clone(&resources));
    let a = athlete(&resources, "imp-chat-tap@example.com").await;
    let agent: CreateAgentRequest = serde_json::from_value(json!({
        "title": "Privacy Coach",
        "description": null,
        "system_prompt": "You coach.",
        "sample_prompts": ["/privacy on"],
    }))
    .unwrap();
    let agent = resources
        .common
        .repos
        .agents
        .create(a.id, a.tenant_id, &agent)
        .await
        .unwrap();
    let resp = AxumTestRequest::post("/api/chat/conversations")
        .header("authorization", &a.session)
        .json(&json!({ "title": "Tap thread", "agent_id": agent.id.to_string() }))
        .send(router.clone())
        .await;
    assert_eq!(
        resp.status_code(),
        StatusCode::CREATED,
        "{}",
        resp.body_text()
    );
    let thread = resp.json::<ConversationResponse>().id;
    let consent = || async {
        resources
            .common
            .repos
            .users
            .get_global(a.id)
            .await
            .unwrap()
            .unwrap()
            .analytics_consent
    };

    // The first Example Input, tapped in the first slot.
    let tap = "ex:0:0";
    assert_refused(&type_in(&router, &a.support, &thread, tap).await, tap);
    assert!(!consent().await, "the tap gave no consent");
    assert_refusal_audited(&resources, a.id, "run the privacy-on command").await;

    assert_command_served(type_in(&router, &a.session, &thread, tap).await, tap);
    assert!(consent().await, "the athlete's own tap gives it");
}
