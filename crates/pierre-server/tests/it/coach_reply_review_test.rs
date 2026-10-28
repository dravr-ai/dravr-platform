// ABOUTME: A coach reads, flags and corrects the agent's replies in the groups they coach, and never anyone else's
// ABOUTME: Pins tenant isolation, the coach-consent rule, verdict chips, the athlete-visible correction and the weekly count
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#847. Group replies go straight to the athletes, so the coach's
//! control is after the fact: `GET /api/coach/replies` lists every reply the
//! agent sent into a group the caller coaches, a flag stays with the coach,
//! and a correction joins the group thread linked to the reply it corrects.
//!
//! The world: coach Casey coaches Alex (shares with her) and Ben (revoked
//! coach sharing) in tenant A; coach Dana coaches Eve in the same tenant; coach
//! Finn coaches Gus in tenant B. Casey must see Alex's replies and nobody
//! else's — not Ben's, not her own, not Dana's athlete's, not tenant B's —
//! and must be refused, as not found, on every reply outside that set.

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::Router;
use chrono::Utc;
use common::{create_test_server_resources, create_test_user_with_plan, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::groups::{
    CoachConsents, CoachingGroup, GroupDigestMode, GroupMember, GroupRespondMode, GroupRole,
    NewGroupTranscriptEntry, TranscriptSpeaker,
};
use pierre_core::models::{
    AddMessageParams, AgentCategory, ConversationSubject, CreateAgentRequest, TenantId, User,
};
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::{InsertClaimVerdictParams, NewSubjectConversation};
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_mcp_server::routes::chat::ChatRoutes;
use pierre_mcp_server::routes::coach_replies::CoachReplyRoutes;
use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};
use pierre_test_support::account::delete_account;
use serde_json::{json, Value};
use uuid::Uuid;

/// The agent's reply to Alex that carries the invented figure.
const ALEX_REPLY: &str = "Your 2:41 marathon in May shows you are ready for a sub-2:40.";
/// A later reply to Alex, with no claim judged.
const ALEX_LATER_REPLY: &str = "Keep Thursday easy.";
/// The newest reply to Alex.
const ALEX_NEWEST_REPLY: &str = "Good job on the tempo run.";
/// The flag's note — the coach's alone, never in anything the athlete reads.
const FLAG_NOTE: &str = "Invented race, Alex ran a half in May";
/// The correction, which the athlete reads in the thread.
const CORRECTION: &str = "Alex, your May race was a half marathon, not a marathon.";

/// One signed-in user.
struct Person {
    id: Uuid,
    tenant: TenantId,
    auth: String,
}

/// Every reply the world holds, by whom it answered.
struct Replies {
    alex_first: Uuid,
    alex_later: Uuid,
    alex_newest: Uuid,
    ben: Uuid,
    casey_own: Uuid,
    eve: Uuid,
    gus: Uuid,
}

struct World {
    res: Arc<ServerContext>,
    router: Router,
    casey: Person,
    dana: Person,
    finn: Person,
    alex: Person,
    casey_group: Uuid,
    replies: Replies,
    /// The chat message Alex's first reply was fanned out from.
    alex_first_message: String,
}

async fn person(res: &Arc<ServerContext>, email: &str, name: &str) -> (Person, User) {
    let (id, user, tenant) = create_test_user_with_plan(&res.agent.database, email, "professional")
        .await
        .unwrap();
    res.common
        .repos
        .users
        .update_display_name(id, name)
        .await
        .unwrap();
    let auth = format!("Bearer {}", generate_test_token(res, &user).await);
    (Person { id, tenant, auth }, user)
}

/// Make `user` a member of `tenant` and sign them in there, so two coaches
/// share one tenant.
async fn join_tenant(res: &Arc<ServerContext>, person: &mut Person, user: &User, tenant: TenantId) {
    res.common
        .repos
        .users
        .update_tenant_id(person.id, tenant)
        .await
        .unwrap();
    let token = res
        .auth
        .auth_manager
        .generate_token_with_tenant(user, &res.auth.jwks_manager, Some(tenant.to_string()))
        .unwrap();
    person.auth = format!("Bearer {token}");
    person.tenant = tenant;
}

/// The agent every group in the world answers with.
async fn group_agent(res: &Arc<ServerContext>, owner: &Person) -> String {
    res.common
        .repos
        .agents
        .create(
            owner.id,
            owner.tenant,
            &CreateAgentRequest {
                title: "Group Agent".to_owned(),
                description: None,
                system_prompt: "You are helpful".to_owned(),
                category: AgentCategory::Custom,
                tags: vec![],
                sample_prompts: vec![],
                startup_query: None,
                data_requirements: None,
                purpose: None,
                when_to_use: None,
                instructions: None,
                example_inputs: None,
                example_outputs: None,
                success_criteria: None,
                max_tool_iterations: None,
            },
        )
        .await
        .unwrap()
        .id
        .to_string()
}

/// A group in `tenant`, coached by `coach`, which also owns it.
async fn coached_group(
    res: &Arc<ServerContext>,
    tenant: TenantId,
    coach: Uuid,
    agent_id: &str,
    name: &str,
) -> Uuid {
    let now = Utc::now();
    let group = res
        .common
        .repos
        .groups
        .create_group(
            tenant,
            &CoachingGroup {
                id: Uuid::new_v4(),
                tenant_id: tenant.to_string(),
                name: name.to_owned(),
                description: None,
                agent_id: agent_id.to_owned(),
                owner_id: coach,
                coach_user_id: Some(coach),
                peer_data_sharing: false,
                respond_mode: GroupRespondMode::default(),
                digest_mode: GroupDigestMode::Off,
                max_members: 10,
                is_active: true,
                channel_type: None,
                channel_chat_id: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap()
        .id;
    join_group(res, group, coach, tenant, GroupRole::Owner, true).await;
    group
}

async fn join_group(
    res: &Arc<ServerContext>,
    group: Uuid,
    user: Uuid,
    tenant: TenantId,
    role: GroupRole,
    shares_with_coach: bool,
) {
    let now = Utc::now();
    res.common
        .repos
        .groups
        .add_member(&GroupMember {
            id: Uuid::new_v4(),
            group_id: group,
            user_id: user,
            tenant_id: tenant.to_string(),
            role,
            peer_sharing_consent: false,
            coach_sharing_consent: shares_with_coach,
            coach_consents: CoachConsents::default(),
            consent_given_at: now,
            joined_at: now,
            left_at: None,
            display_name: None,
        })
        .await
        .unwrap();
}

/// One agent reply to `athlete` in `group`: the assistant row of the
/// athlete's group conversation and its room entry, fanned out from it as turn
/// persistence does. Returns the entry id and the message id.
async fn agent_reply(
    res: &Arc<ServerContext>,
    group: Uuid,
    athlete: &Person,
    content: &str,
) -> (Uuid, String) {
    let repos = &res.common.repos;
    let user = athlete.id.to_string();
    let conversation = repos
        .chat
        .create_conversation(
            &user,
            athlete.tenant,
            "Group",
            "model",
            None,
            Some(&group.to_string()),
        )
        .await
        .unwrap();
    let message = repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id: athlete.tenant,
            conversation_id: &conversation.id,
            user_id: &user,
            role: "assistant",
            content,
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
            transport_policy: TransportPolicy::AnyTransport,
            channel_type: None,
        })
        .await
        .unwrap();
    repos
        .groups
        .append_transcript_entry(&NewGroupTranscriptEntry {
            group_id: &group.to_string(),
            tenant_id: &athlete.tenant.to_string(),
            author_user_id: athlete.id,
            speaker: TranscriptSpeaker::Coach,
            content,
            source_conversation_id: Some(&conversation.id),
            source_message_id: Some(&message.id),
            transport_policy: TransportPolicy::AnyTransport,
        })
        .await
        .unwrap();
    // The newest entry the athlete is the author of is the one just written.
    let entries = repos
        .groups
        .list_transcript_visible_to(&group.to_string(), athlete.id, 1)
        .await
        .unwrap();
    assert_eq!(entries[0].content, content);
    (entries[0].id, message.id)
}

async fn verdict(
    res: &Arc<ServerContext>,
    athlete: &Person,
    message_id: &str,
    claim: &str,
    status: ClaimStatus,
) {
    res.common
        .repos
        .claim_verdicts
        .insert_claim_verdict(&InsertClaimVerdictParams {
            tenant_id: athlete.tenant,
            user_id: &athlete.id.to_string(),
            agent_id: None,
            conversation_id: None,
            message_id: Some(message_id),
            claim_text: claim,
            category: ClaimCategory::AthleteData,
            status,
            evidence_strength: EvidenceStrength::None,
            confidence: 0.0,
            layer_fired: VerdictLayer::AthleteData,
            explanation: None,
            evidence_refs: None,
            transport_policy: TransportPolicy::AnyTransport,
        })
        .await
        .unwrap();
}

/// A group `coach` runs in their own tenant, answered by `agent`, with each
/// of `athletes` joined as a member sharing with the coach or not.
async fn squad(
    res: &Arc<ServerContext>,
    (coach, agent, name): (&Person, &str, &str),
    athletes: &[(&Person, bool)],
) -> Uuid {
    let group = coached_group(res, coach.tenant, coach.id, agent, name).await;
    for (athlete, shares_with_coach) in athletes {
        join_group(
            res,
            group,
            athlete.id,
            athlete.tenant,
            GroupRole::Member,
            *shares_with_coach,
        )
        .await;
    }
    group
}

/// Alex's first reply and the three claims the checker judged in it: the
/// invented marathon (unmatched), a claim naming no figure, a matched figure.
async fn reply_with_claims(res: &Arc<ServerContext>, group: Uuid, alex: &Person) -> (Uuid, String) {
    let (entry, message) = agent_reply(res, group, alex, ALEX_REPLY).await;
    for (claim, status) in [
        ("your 2:41 marathon in May", ClaimStatus::Unverifiable),
        ("you ran with the club", ClaimStatus::Unverifiable),
        ("your 21 km long run", ClaimStatus::Supported),
    ] {
        verdict(res, alex, &message, claim, status).await;
    }
    (entry, message)
}

async fn world() -> World {
    let res = create_test_server_resources().await.unwrap();
    let (casey, _) = person(&res, "casey@review.test", "Casey Coach").await;
    let (mut dana, dana_user) = person(&res, "dana@review.test", "Dana Coach").await;
    let (finn, _) = person(&res, "finn@review.test", "Finn Coach").await;
    let (alex, _) = person(&res, "alex@review.test", "Alex Athlete").await;
    let (ben, _) = person(&res, "ben@review.test", "Ben Athlete").await;
    let (eve, _) = person(&res, "eve@review.test", "Eve Athlete").await;
    let (gus, _) = person(&res, "gus@review.test", "Gus Athlete").await;
    // Dana coaches in Casey's tenant: the same tenant, another coach.
    join_tenant(&res, &mut dana, &dana_user, casey.tenant).await;
    let agent = group_agent(&res, &casey).await;

    let casey_group = squad(
        &res,
        (&casey, &agent, "Casey's squad"),
        &[(&alex, true), (&ben, false)],
    )
    .await;
    let dana_group = squad(&res, (&dana, &agent, "Dana's squad"), &[(&eve, true)]).await;
    let finn_group = squad(&res, (&finn, &agent, "Finn's squad"), &[(&gus, true)]).await;

    let (alex_first, alex_first_message) = reply_with_claims(&res, casey_group, &alex).await;
    let (ben_reply, _) = agent_reply(&res, casey_group, &ben, "Ben, rest today.").await;
    let (casey_own, _) = agent_reply(&res, casey_group, &casey, "Casey, noted.").await;
    let (eve_reply, _) = agent_reply(&res, dana_group, &eve, "Eve, great swim.").await;
    let (gus_reply, _) = agent_reply(&res, finn_group, &gus, "Gus, ride easy.").await;
    let (alex_later, _) = agent_reply(&res, casey_group, &alex, ALEX_LATER_REPLY).await;
    let (alex_newest, _) = agent_reply(&res, casey_group, &alex, ALEX_NEWEST_REPLY).await;

    let router = Router::new()
        .merge(CoachReplyRoutes::routes(Arc::clone(&res)))
        .merge(ChatRoutes::routes(Arc::clone(&res)));
    World {
        res,
        router,
        casey,
        dana,
        finn,
        alex,
        casey_group,
        alex_first_message,
        replies: Replies {
            alex_first,
            alex_later,
            alex_newest,
            ben: ben_reply,
            casey_own,
            eve: eve_reply,
            gus: gus_reply,
        },
    }
}

async fn get(router: &Router, uri: &str, auth: &str) -> (u16, Value) {
    let response = AxumTestRequest::get(uri)
        .header("authorization", auth)
        .send(router.clone())
        .await;
    let status = response.status();
    let body = response.body_text();
    (
        status,
        serde_json::from_str(&body).unwrap_or(Value::String(body)),
    )
}

async fn post(router: &Router, uri: &str, auth: &str, body: &Value) -> (u16, Value) {
    let response = AxumTestRequest::post(uri)
        .header("authorization", auth)
        .json(body)
        .send(router.clone())
        .await;
    let status = response.status();
    let body = response.body_text();
    (
        status,
        serde_json::from_str(&body).unwrap_or(Value::String(body)),
    )
}

fn reply_ids(page: &Value) -> Vec<String> {
    page["replies"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["id"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test]
async fn a_coach_reads_only_the_replies_to_the_athletes_who_share_with_them() {
    let w = world().await;
    let (status, page) = get(&w.router, "/api/coach/replies", &w.casey.auth).await;
    assert_eq!(status, 200, "{page}");

    // Newest first, Alex's three replies and nothing else: not Ben's (he
    // revoked coach sharing), not Casey's own, not Dana's athlete's in the same
    // tenant, not tenant B's.
    assert_eq!(
        reply_ids(&page),
        vec![
            w.replies.alex_newest.to_string(),
            w.replies.alex_later.to_string(),
            w.replies.alex_first.to_string(),
        ]
    );
    let body = page.to_string();
    for hidden in [
        "Ben, rest today.",
        "Casey, noted.",
        "Eve, great swim.",
        "Gus, ride easy.",
    ] {
        assert!(!body.contains(hidden), "{hidden} leaked into Casey's page");
    }
    assert!(page["next_before"].is_null());

    // The filter offers Casey's one group and her athletes, never herself;
    // Ben is listed as not sharing.
    assert_eq!(page["groups"].as_array().unwrap().len(), 1);
    assert_eq!(page["groups"][0]["name"], "Casey's squad");
    let athletes = page["athletes"].as_array().unwrap();
    let names: Vec<&str> = athletes
        .iter()
        .map(|a| a["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["Alex Athlete", "Ben Athlete"]);
    assert_eq!(athletes[0]["shares_with_coach"], true);
    assert_eq!(athletes[1]["shares_with_coach"], false);

    // The reply carrying claims: its chips, and the one unmatched figure.
    let first = &page["replies"][2];
    assert_eq!(first["content"], ALEX_REPLY);
    assert_eq!(first["athlete_name"], "Alex Athlete");
    assert_eq!(first["group_name"], "Casey's squad");
    assert_eq!(first["verdicts"].as_array().unwrap().len(), 3);
    assert_eq!(first["unmatched_figures"], 1);
    assert_eq!(page["replies"][0]["unmatched_figures"], 0);
    let got = page["replies"][0]["verdicts"]
        .as_array()
        .unwrap()
        .as_slice();
    assert!(got.is_empty(), "expected empty, got {got:?}");

    // Dana, in the same tenant, reads only Eve's reply; Finn only Gus's.
    let (_, dana_page) = get(&w.router, "/api/coach/replies", &w.dana.auth).await;
    assert_eq!(reply_ids(&dana_page), vec![w.replies.eve.to_string()]);
    let (_, finn_page) = get(&w.router, "/api/coach/replies", &w.finn.auth).await;
    assert_eq!(reply_ids(&finn_page), vec![w.replies.gus.to_string()]);

    // An athlete coaches nothing: an empty page, no replies at all.
    let (status, alex_page) = get(&w.router, "/api/coach/replies", &w.alex.auth).await;
    assert_eq!(status, 200);
    let got = alex_page["replies"].as_array().unwrap();
    assert!(got.is_empty(), "expected empty, got {got:?}");
    let got = alex_page["groups"].as_array().unwrap();
    assert!(got.is_empty(), "expected empty, got {got:?}");
}

#[tokio::test]
async fn the_page_narrows_by_athlete_and_pages_back() {
    let w = world().await;
    let alex = w.alex.id;
    let (_, page) = get(
        &w.router,
        &format!("/api/coach/replies?athlete_id={alex}&limit=2"),
        &w.casey.auth,
    )
    .await;
    assert_eq!(
        reply_ids(&page),
        vec![
            w.replies.alex_newest.to_string(),
            w.replies.alex_later.to_string()
        ]
    );
    let next = page["next_before"].as_str().unwrap().to_owned();
    assert_eq!(next, w.replies.alex_later.to_string());
    let (_, older) = get(
        &w.router,
        &format!("/api/coach/replies?athlete_id={alex}&limit=2&before={next}"),
        &w.casey.auth,
    )
    .await;
    assert_eq!(reply_ids(&older), vec![w.replies.alex_first.to_string()]);
    assert!(older["next_before"].is_null());

    // Another coach's athlete as the filter is an empty page, not their replies.
    let (_, none) = get(
        &w.router,
        &format!("/api/coach/replies?athlete_id={}", Uuid::new_v4()),
        &w.casey.auth,
    )
    .await;
    let got = none["replies"].as_array().unwrap();
    assert!(got.is_empty(), "expected empty, got {got:?}");

    // A cursor naming a reply outside Casey's groups is no position in her
    // review: tenant B's reply, written after Alex's first, pages nothing.
    let (status, foreign_cursor) = get(
        &w.router,
        &format!("/api/coach/replies?before={}", w.replies.gus),
        &w.casey.auth,
    )
    .await;
    assert_eq!(status, 200);
    let got = foreign_cursor["replies"].as_array().unwrap();
    assert!(got.is_empty(), "expected empty, got {got:?}");

    let (status, _) = get(
        &w.router,
        "/api/coach/replies?athlete_id=nope",
        &w.casey.auth,
    )
    .await;
    assert_eq!(status, 400);
}

#[tokio::test]
async fn a_coach_cannot_flag_or_correct_a_reply_outside_their_groups() {
    let w = world().await;
    let flag = json!({ "reason": "wrong", "note": "no" });
    let correction = json!({ "content": "no" });
    let foreign = [
        // Dana's athlete, same tenant.
        w.replies.eve,
        // Tenant B.
        w.replies.gus,
        // Ben does not share with Casey.
        w.replies.ben,
        // Casey's own conversation is not an athlete's.
        w.replies.casey_own,
    ];
    for reply in foreign {
        let (status, _) = post(
            &w.router,
            &format!("/api/coach/replies/{reply}/flags"),
            &w.casey.auth,
            &flag,
        )
        .await;
        assert_eq!(status, 404, "Casey flagged {reply}");
        let (status, _) = post(
            &w.router,
            &format!("/api/coach/replies/{reply}/corrections"),
            &w.casey.auth,
            &correction,
        )
        .await;
        assert_eq!(status, 404, "Casey corrected {reply}");
    }
    // Finn, in tenant B, cannot touch Casey's athlete's reply.
    let (status, _) = post(
        &w.router,
        &format!("/api/coach/replies/{}/flags", w.replies.alex_first),
        &w.finn.auth,
        &flag,
    )
    .await;
    assert_eq!(status, 404);
    // Nor can the athlete flag the reply sent to them.
    let (status, _) = post(
        &w.router,
        &format!("/api/coach/replies/{}/flags", w.replies.alex_first),
        &w.alex.auth,
        &flag,
    )
    .await;
    assert_eq!(status, 404);

    // Nothing was written: no correction reached Eve's room.
    let (_, weekly) = get(
        &w.router,
        "/api/coach/replies/weekly?weeks=1",
        &w.casey.auth,
    )
    .await;
    assert_eq!(weekly["weeks"][0]["flags"], 0);
    assert_eq!(weekly["weeks"][0]["corrections"], 0);
}

/// Only the group's attached coach writes a correction, so the client may sign
/// every correction in a room with that coach's name (carnet#1191): another
/// owner or admin of the same group, in the same tenant, who shares with the
/// coach, is refused like a stranger, and nothing reaches the room.
#[tokio::test]
async fn only_the_attached_coach_corrects_a_reply_not_another_group_admin() {
    let w = world().await;
    // Dana, a coach in Casey's tenant, joins Casey's group as an admin.
    join_group(
        &w.res,
        w.casey_group,
        w.dana.id,
        w.casey.tenant,
        GroupRole::Admin,
        true,
    )
    .await;

    let reply = w.replies.alex_first;
    let (status, body) = post(
        &w.router,
        &format!("/api/coach/replies/{reply}/corrections"),
        &w.dana.auth,
        &json!({ "content": CORRECTION }),
    )
    .await;
    assert_eq!(
        status, 404,
        "bug: a group admin who is not the reply's coach must not correct it: {body}"
    );
    let (status, body) = post(
        &w.router,
        &format!("/api/coach/replies/{reply}/flags"),
        &w.dana.auth,
        &json!({ "reason": "wrong", "note": "no" }),
    )
    .await;
    assert_eq!(
        status, 404,
        "bug: a group admin who is not the reply's coach must not flag it: {body}"
    );

    // The room Alex reads holds no correction.
    let (status, room) = get(
        &w.router,
        &format!("/api/chat/groups/{}/transcript", w.casey_group),
        &w.alex.auth,
    )
    .await;
    assert_eq!(
        status, 200,
        "bug: the athlete must read their own group transcript: {room}"
    );
    let corrections = room["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["speaker"] == "correction")
        .count();
    assert_eq!(
        corrections, 0,
        "bug: a refused correction must never reach the athlete's room: {room}"
    );
}

#[tokio::test]
async fn a_flag_stays_with_the_coach_and_a_correction_reaches_the_athletes_thread() {
    let w = world().await;
    let reply = w.replies.alex_first;

    let (status, flagged) = post(
        &w.router,
        &format!("/api/coach/replies/{reply}/flags"),
        &w.casey.auth,
        &json!({ "reason": "off_tone", "note": FLAG_NOTE }),
    )
    .await;
    assert_eq!(status, 201, "{flagged}");
    assert_eq!(flagged["flag"]["reason"], "off_tone");
    assert_eq!(flagged["flag"]["note"], FLAG_NOTE);
    assert_eq!(flagged["flag"]["reply_entry_id"], reply.to_string());

    let (status, corrected) = post(
        &w.router,
        &format!("/api/coach/replies/{reply}/corrections"),
        &w.casey.auth,
        &json!({ "content": format!("  {CORRECTION}  ") }),
    )
    .await;
    assert_eq!(status, 201, "{corrected}");
    assert_eq!(corrected["correction"]["content"], CORRECTION);
    // The group is bound to no messaging chat.
    assert_eq!(corrected["posted_to_chat"], false);

    // Casey's page carries both on the reply.
    let (_, page) = get(&w.router, "/api/coach/replies", &w.casey.auth).await;
    let row = page["replies"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == reply.to_string())
        .unwrap();
    assert_eq!(row["flags"].as_array().unwrap().len(), 1);
    assert_eq!(row["flags"][0]["note"], FLAG_NOTE);
    assert_eq!(row["corrections"][0]["content"], CORRECTION);
    // The other replies carry none.
    let got = page["replies"][0]["flags"].as_array().unwrap();
    assert!(got.is_empty(), "expected empty, got {got:?}");

    // Alex reads the room: the correction is there, linked to the reply it
    // corrects, and the flag is nowhere.
    let (status, room) = get(
        &w.router,
        &format!("/api/chat/groups/{}/transcript", w.casey_group),
        &w.alex.auth,
    )
    .await;
    assert_eq!(status, 200, "{room}");
    let correction = room["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["speaker"] == "correction")
        .expect("the correction is in the room");
    assert_eq!(correction["content"], CORRECTION);
    assert_eq!(correction["own"], true);
    assert_eq!(correction["corrects"]["entry_id"], reply.to_string());
    assert_eq!(correction["corrects"]["message_id"], w.alex_first_message);
    assert_eq!(correction["corrects"]["excerpt"], ALEX_REPLY);
    let room_text = room.to_string();
    assert!(
        !room_text.contains(FLAG_NOTE),
        "the flag reached the athlete"
    );
    assert!(
        !room_text.contains("off_tone"),
        "the flag reached the athlete"
    );
    // Every other entry carries no correction link.
    assert!(room["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["speaker"] != "correction")
        .all(|e| e["corrects"].is_null()));

    // The week counts one flag and one correction.
    let (_, weekly) = get(
        &w.router,
        "/api/coach/replies/weekly?weeks=2",
        &w.casey.auth,
    )
    .await;
    let weeks = weekly["weeks"].as_array().unwrap();
    assert_eq!(weeks.len(), 2);
    assert_eq!(weeks[1]["flags"], 1);
    assert_eq!(weeks[1]["corrections"], 1);
    assert_eq!(weeks[0]["flags"], 0);
    // Another coach's count is untouched.
    let (_, dana_weekly) = get(&w.router, "/api/coach/replies/weekly?weeks=1", &w.dana.auth).await;
    assert_eq!(dana_weekly["weeks"][0]["flags"], 0);
    assert_eq!(dana_weekly["weeks"][0]["corrections"], 0);

    // The athlete's own coach page shows no flag: she coaches nothing.
    let (_, alex_page) = get(&w.router, "/api/coach/replies", &w.alex.auth).await;
    assert!(!alex_page.to_string().contains(FLAG_NOTE));
}

/// carnet#1168 D27: deleting an athlete's account reaches what their coach
/// holds about them — the coach's thread about them with every message and
/// tool result in it, the coach's flags and corrections of replies to them,
/// and their room transcript — and the deletion counts each. The coach's own
/// entries stay.
#[tokio::test]
async fn deleting_the_athletes_account_reaches_what_their_coach_holds_about_them() {
    let w = world().await;
    let repos = &w.res.common.repos;
    let reply = w.replies.alex_first;
    for (path, body) in [
        ("flags", json!({ "reason": "off_tone", "note": FLAG_NOTE })),
        ("corrections", json!({ "content": CORRECTION })),
    ] {
        let (status, answer) = post(
            &w.router,
            &format!("/api/coach/replies/{reply}/{path}"),
            &w.casey.auth,
            &body,
        )
        .await;
        assert_eq!(status, 201, "{answer}");
    }

    // Casey's thread about Alex, holding a tool result read from Alex's data.
    let casey = w.casey.id.to_string();
    let thread = repos
        .chat
        .open_subject_conversation(&NewSubjectConversation {
            user_id: &casey,
            tenant_id: w.casey.tenant,
            title: "Alex Athlete",
            model: "model",
            agent_id: &group_agent(&w.res, &w.casey).await,
            subject: ConversationSubject {
                athlete_user_id: w.alex.id,
                group_id: w.casey_group,
            },
        })
        .await
        .unwrap()
        .conversation;
    repos
        .chat
        .add_message(&AddMessageParams {
            tenant_id: w.casey.tenant,
            conversation_id: &thread.id,
            user_id: &casey,
            role: "tool_result",
            content: "Alex: 21 km long run on Sunday, average HR 148",
            token_count: None,
            finish_reason: None,
            prompt_tokens: None,
            model: None,
            content_blocks: None,
            transport_policy: TransportPolicy::AnyTransport,
            channel_type: None,
        })
        .await
        .unwrap();

    let alex_replies = [
        w.replies.alex_first.to_string(),
        w.replies.alex_later.to_string(),
        w.replies.alex_newest.to_string(),
    ];
    let (_, before) = get(&w.router, "/api/coach/replies", &w.casey.auth).await;
    assert!(
        alex_replies
            .iter()
            .all(|id| reply_ids(&before).contains(id)),
        "premise: Casey reads Alex's replies: {before}"
    );

    let deletion = delete_account(repos, w.alex.id, None).await.unwrap();

    // Alex's three group conversations and Casey's thread about her.
    assert_eq!(deletion.removed_from("chat_conversations"), 4);
    assert!(repos
        .chat
        .get_conversation(&thread.id, &casey, w.casey.tenant)
        .await
        .unwrap()
        .is_none());
    // The flag and the correction.
    assert_eq!(deletion.removed_from("coach_reply_reviews"), 2);
    let (_, weekly) = get(
        &w.router,
        "/api/coach/replies/weekly?weeks=1",
        &w.casey.auth,
    )
    .await;
    assert_eq!(weekly["weeks"][0]["flags"], 0, "{weekly}");
    assert_eq!(weekly["weeks"][0]["corrections"], 0, "{weekly}");
    // The three replies to Alex; the correction of one may go with the reply
    // it corrects, by cascade, which no row count sees.
    assert!(deletion.removed_from("group_transcript_entries") >= 3);
    let (_, after) = get(&w.router, "/api/coach/replies", &w.casey.auth).await;
    assert!(
        alex_replies
            .iter()
            .all(|id| !reply_ids(&after).contains(id)),
        "{after}"
    );
    let room = repos
        .groups
        .list_transcript_visible_to(&w.casey_group.to_string(), w.casey.id, 50)
        .await
        .unwrap();
    assert!(
        room.iter().any(|entry| entry.id == w.replies.casey_own),
        "the coach's own entry stays"
    );
    assert!(room
        .iter()
        .all(|entry| entry.content != ALEX_REPLY && entry.content != CORRECTION));
}

#[tokio::test]
async fn a_flag_or_correction_out_of_bounds_is_refused() {
    let w = world().await;
    let reply = w.replies.alex_first;
    let (status, _) = post(
        &w.router,
        &format!("/api/coach/replies/{reply}/flags"),
        &w.casey.auth,
        &json!({ "reason": "rude" }),
    )
    .await;
    assert!(status == 400 || status == 422, "unknown reason: {status}");
    let (status, _) = post(
        &w.router,
        &format!("/api/coach/replies/{reply}/flags"),
        &w.casey.auth,
        &json!({ "reason": "unsafe", "note": "x".repeat(1001) }),
    )
    .await;
    assert_eq!(status, 400);
    let (status, _) = post(
        &w.router,
        &format!("/api/coach/replies/{reply}/corrections"),
        &w.casey.auth,
        &json!({ "content": "   " }),
    )
    .await;
    assert_eq!(status, 400);
    let (status, _) = post(
        &w.router,
        &format!("/api/coach/replies/{reply}/corrections"),
        &w.casey.auth,
        &json!({ "content": "y".repeat(2001) }),
    )
    .await;
    assert_eq!(status, 400);
    // A flag with no note is a flag.
    let (status, flagged) = post(
        &w.router,
        &format!("/api/coach/replies/{reply}/flags"),
        &w.casey.auth,
        &json!({ "reason": "unsafe" }),
    )
    .await;
    assert_eq!(status, 201);
    assert!(flagged["flag"]["note"].is_null());
}
