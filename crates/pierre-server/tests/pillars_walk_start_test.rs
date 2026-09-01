// ABOUTME: /pillars handler — DM and room activation, arg screening, persisted opener message
// ABOUTME: Content-asserting: real DB rows for onboarding_state + the assistant opener in history
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `/pillars` opens the guided profile walk. Three things must hold, and each is
//! load-bearing for a defect the 2026-07-24 incident exposed:
//!
//! 1. **A room walk is the athlete's own, and covers room-safe topics only.**
//!    Typing `/pillars` in the room is the consent; the state binds the caller
//!    as subject with a room audience, and the DM-only pillars can be neither
//!    walked nor re-screened from there.
//! 2. **The opener is persisted as an assistant message.** Without it the coach
//!    receives the athlete's North Star answer with no question attached, and —
//!    because that answer is then message #1 — the first-turn coach startup
//!    prefetch fires and injects an activity dump plus the coach's own "build a
//!    block" query as if the athlete had asked for it.
//! 3. **A state write that matched no row is reported, not swallowed.**

use anyhow::Result;
use pierre_commands::onboarding::PillarsHandler;
use pierre_commands::{CommandHandler, PlatformCommandContext};
use pierre_core::models::{
    CoverageTarget, OnboardingState, Pillar, TenantId, TopicSlug as OnboardingTopicSlug,
};
use pierre_database::repositories::UpsertUserFactParams;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::{FactKind, FactSource, MemoryScope};
use pierre_runtime_context::CoachesCtx;
use std::sync::Arc;
use uuid::Uuid;

mod common;

async fn setup() -> Result<(Arc<ServerContext>, Uuid, TenantId, String)> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    let email = format!("pillars_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(resources.database(), &email).await?;
    let tenants = resources.common.repos.tenants.get_all().await?;
    let tenant = tenants
        .iter()
        .find(|t| t.owner_user_id == user_id)
        .map(|t| t.id)
        .expect("user should own a tenant");

    let conversation = resources
        .common
        .repos
        .chat
        .create_conversation(
            &user_id.to_string(),
            tenant,
            "pillars walk",
            "gemini-2.0-flash",
            None,
            None,
        )
        .await?;
    Ok((resources, user_id, tenant, conversation.id))
}

fn ctx(
    resources: &Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    conversation_id: Option<String>,
    is_direct_message: bool,
) -> PlatformCommandContext {
    PlatformCommandContext {
        user_id,
        tenant_id,
        channel_type: "telegram".to_owned(),
        args: vec![],
        raw_text: "/pillars".to_owned(),
        ctx: Arc::<ServerContext>::clone(resources) as Arc<dyn pierre_runtime_context::CommandCtx>,
        locale: "en".to_owned(),
        is_direct_message,
        ambient_group_fallback: true,
        conversation_id,
        conversation_tenant_id: tenant_id,
        sender_id: None,
        tool_runtime: Arc::<ServerContext>::clone(resources),
    }
}

#[tokio::test]
async fn pillars_in_a_group_activates_a_room_walk_bound_to_the_caller() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;

    let response = PillarsHandler
        .execute(&ctx(
            &resources,
            user_id,
            tenant,
            Some(conversation_id.clone()),
            false,
        ))
        .await?;

    // The room opener carries the visibility contract and still opens on the
    // North Star; a walk that answered with the DM opener would never have
    // told the athlete the room reads along.
    assert!(
        response.text.contains("visible to everyone"),
        "the room opener must state the exchange is room-visible, got: {}",
        response.text
    );
    assert!(
        response.text.contains("North Star"),
        "the room opener must still ask the North Star question, got: {}",
        response.text
    );

    let conv = resources
        .common
        .repos
        .chat
        .get_conversation(&conversation_id, &user_id.to_string(), tenant)
        .await?
        .expect("conversation");
    let state = OnboardingState::from_column(conv.onboarding_state.as_deref())
        .expect("room walk should be active");
    assert_eq!(
        state.subject_user_id.as_deref(),
        Some(user_id.to_string().as_str()),
        "the walk must bind the caller as its subject"
    );
    assert_eq!(
        conv.onboarding_state
            .as_deref()
            .map(|raw| raw.contains("\"audience\":\"room\"")),
        Some(true),
        "the stored state must record the room audience — a default would walk private"
    );

    let history = resources
        .common
        .repos
        .chat
        .get_messages(&conversation_id, &user_id.to_string(), tenant)
        .await?;
    assert_eq!(history.len(), 1, "the room opener must be persisted");
    assert_eq!(history[0].content, response.text);
    Ok(())
}

#[tokio::test]
async fn pillars_mental_in_a_group_is_refused_before_any_expiry() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;

    // Seed a Mental Resilience onboarding fact; the refusal must leave it
    // standing — a re-screen that expired it would strand the athlete with a
    // stale pillar no room walk can ever re-ask.
    resources
        .common
        .repos
        .memory
        .upsert_user_fact(&UpsertUserFactParams {
            tenant_id: tenant,
            user_id: &user_id.to_string(),
            coach_id: None,
            scope: MemoryScope::User,
            kind: FactKind::Preference,
            pillar: Some(Pillar::MentalResilience),
            subject: "you",
            predicate: "manage stress with",
            object: "evening walks",
            confidence: 0.9,
            source: FactSource::Onboarding,
            valid_until: None,
            source_msg_id: None,
            embedding: None,
        })
        .await?;

    for arg in ["mental", "substances", "full", "mental_resilience"] {
        let mut room_ctx = ctx(
            &resources,
            user_id,
            tenant,
            Some(conversation_id.clone()),
            false,
        );
        room_ctx.args = vec![(*arg).to_owned()];
        let response = PillarsHandler.execute(&room_ctx).await?;
        assert!(
            response.text.contains("covered in private"),
            "/pillars {arg} in a room must refuse toward a DM, got: {}",
            response.text
        );
    }

    // Nothing was expired and no walk opened.
    let conv = resources
        .common
        .repos
        .chat
        .get_conversation(&conversation_id, &user_id.to_string(), tenant)
        .await?
        .expect("conversation");
    assert!(
        conv.onboarding_state.is_none(),
        "a refused re-screen must not activate a walk"
    );
    let dossier = resources
        .common
        .repos
        .dossier
        .compose_dossier(tenant, user_id)
        .await?;
    assert!(
        dossier
            .pillars
            .get(&Pillar::MentalResilience)
            .is_some_and(|facts| facts.iter().any(|f| !f.stale)),
        "the seeded mental fact must survive the refusal un-expired"
    );
    Ok(())
}

#[tokio::test]
async fn pillars_in_a_dm_activates_the_walk_and_persists_the_opener() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;

    let response = PillarsHandler
        .execute(&ctx(
            &resources,
            user_id,
            tenant,
            Some(conversation_id.clone()),
            true,
        ))
        .await?;
    assert!(
        response.text.contains("North Star"),
        "the opener must ask the North Star question, got: {}",
        response.text
    );

    // The walk is active, with an empty probe history.
    let conv = resources
        .common
        .repos
        .chat
        .get_conversation(&conversation_id, &user_id.to_string(), tenant)
        .await?
        .expect("conversation");
    let state = OnboardingState::from_column(conv.onboarding_state.as_deref())
        .expect("walk should be active");
    assert!(state.active);
    assert!(
        state.probed.is_empty(),
        "no probe has been delivered by a turn yet"
    );

    // The opener is durable history, under the same tenant the pipeline reads.
    let history = resources
        .common
        .repos
        .chat
        .get_messages(&conversation_id, &user_id.to_string(), tenant)
        .await?;
    assert_eq!(
        history.len(),
        1,
        "exactly the opener should be in history, got {history:?}"
    );
    assert_eq!(history[0].role, "assistant");
    assert_eq!(
        history[0].content, response.text,
        "the persisted opener must be the text the athlete saw"
    );

    // The athlete's answer is therefore message #2, which is what keeps the
    // first-turn startup prefetch (`history_len == 1`) from firing on it.
    Ok(())
}

#[tokio::test]
async fn pillars_reports_a_state_write_that_matched_no_conversation() -> Result<()> {
    let (resources, user_id, tenant, _conversation_id) = setup().await?;

    // A conversation id that exists for nobody: the tenant-scoped UPDATE matches
    // zero rows. Returning the opener anyway would open a walk no turn runs in.
    let response = PillarsHandler
        .execute(&ctx(
            &resources,
            user_id,
            tenant,
            Some(Uuid::new_v4().to_string()),
            true,
        ))
        .await?;
    assert!(
        !response.text.contains("North Star"),
        "a failed activation must not answer with the opener, got: {}",
        response.text
    );
    Ok(())
}

/// A non-empty probe history survives the real `chat_conversations` column.
///
/// The walk records each delivered probe onto `onboarding_state` at the end of a
/// turn, so the whole optimistic-advance mechanism rests on this column holding
/// a growing list across turns. Written against the real repository (not just
/// the JSON shape) because the column is `TEXT` on one backend and the walk
/// reads it back through `get_conversation`.
#[tokio::test]
async fn probe_history_survives_the_onboarding_state_column() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    let chat = &resources.common.repos.chat;

    // Open the walk, then record two probes the way the pipeline does.
    PillarsHandler
        .execute(&ctx(
            &resources,
            user_id,
            tenant,
            Some(conversation_id.clone()),
            true,
        ))
        .await?;

    let mut state = OnboardingState::from_column(
        chat.get_conversation(&conversation_id, &user_id.to_string(), tenant)
            .await?
            .expect("conversation")
            .onboarding_state
            .as_deref(),
    )
    .expect("walk active");
    state = state
        .with_delivered_probe(CoverageTarget::NorthStar.slug())
        .with_delivered_probe(CoverageTarget::Pillar(Pillar::TrainingAndMovement).slug());
    assert!(
        chat.set_conversation_onboarding_state(&conversation_id, Some(&state.to_column()?), tenant)
            .await?
    );

    let reloaded = OnboardingState::from_column(
        chat.get_conversation(&conversation_id, &user_id.to_string(), tenant)
            .await?
            .expect("conversation")
            .onboarding_state
            .as_deref(),
    )
    .expect("walk still active");
    assert!(reloaded.active);
    let slugs: Vec<&str> = reloaded
        .probed
        .iter()
        .map(OnboardingTopicSlug::as_str)
        .collect();
    assert_eq!(
        slugs,
        vec!["north_star", "training_and_movement"],
        "the ask history must round-trip in order"
    );
    Ok(())
}
