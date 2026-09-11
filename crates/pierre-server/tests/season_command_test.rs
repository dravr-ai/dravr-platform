// ABOUTME: /season handler — activates the walk, quotes /calibrate's availability back, records its window
// ABOUTME: And the bounded expiry: a /calibrate re-run no longer sweeps up a season calendar captured after it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Content-asserting on real rows: the conversation state the walk opened,
//! the opener persisted as the agent's message, the profile window, and the
//! `valid_until` stamps a re-run leaves on the facts of the *other* walk.

use anyhow::Result;
use chrono::Utc;
use pierre_commands::calibration::CalibrateHandler;
use pierre_commands::season::SeasonHandler;
use pierre_commands::{CommandHandler, ConversationRotation, PlatformCommandContext};
use pierre_core::models::{GuidedFlow, GuidedWindow, OnboardingState, Pillar, TenantId};
use pierre_database::repositories::UpsertUserFactParams;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};
use pierre_runtime_context::AgentsCtx;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use uuid::Uuid;

mod common;

async fn setup() -> Result<(Arc<ServerContext>, Uuid, TenantId, String)> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    let email = format!("season_{}@example.com", Uuid::new_v4());
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
            "season",
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
    conversation_id: &str,
    raw_text: &str,
) -> PlatformCommandContext {
    PlatformCommandContext {
        user_id,
        tenant_id,
        channel_type: "telegram".to_owned(),
        args: vec![],
        raw_text: raw_text.to_owned(),
        ctx: Arc::<ServerContext>::clone(resources) as Arc<dyn pierre_runtime_context::CommandCtx>,
        locale: "en".to_owned(),
        is_direct_message: true,
        ambient_group_fallback: true,
        conversation_id: Some(conversation_id.to_owned()),
        conversation_tenant_id: tenant_id,
        sender_id: None,
        rotation: ConversationRotation::default(),
        tool_runtime: Arc::<ServerContext>::clone(resources),
    }
}

/// An onboarding-sourced fact in the training pillar, as the guided-turn
/// extraction writes one.
async fn land(
    resources: &Arc<ServerContext>,
    tenant: TenantId,
    user: &str,
    kind: FactKind,
    code: PredicateCode,
    object: &str,
) -> Result<String> {
    let fact = resources
        .common
        .repos
        .memory
        .upsert_user_fact(&UpsertUserFactParams {
            tenant_id: tenant,
            user_id: user,
            agent_id: None,
            scope: MemoryScope::User,
            kind,
            pillar: Some(Pillar::TrainingAndMovement),
            predicate_code: code,
            object,
            confidence: 0.9,
            source: FactSource::Onboarding,
            valid_until: None,
            source_msg_id: None,
        })
        .await?;
    Ok(fact.id)
}

/// Whether the fact is still current: an expiry a re-run wrote is in the past.
async fn is_live(resources: &Arc<ServerContext>, tenant: TenantId, user: &str, id: &str) -> bool {
    resources
        .common
        .repos
        .memory
        .list_user_facts_by_source(tenant, user, FactSource::Onboarding, 100)
        .await
        .expect("facts readable")
        .iter()
        .any(|f| f.id == id && f.valid_until.is_none_or(|until| until > Utc::now()))
}

#[tokio::test]
async fn the_walk_opens_on_the_conversation_and_records_its_window() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;

    let response = SeasonHandler
        .execute(&ctx(
            &resources,
            user_id,
            tenant,
            &conversation_id,
            "/season",
        ))
        .await?;
    assert!(
        response.text.contains("six short questions"),
        "the opener announces the walk: {}",
        response.text
    );
    assert!(
        !response
            .text
            .contains("You already told me about your time"),
        "nothing is on file to quote back yet: {}",
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
        .expect("the walk should be active");
    assert_eq!(state.flow, GuidedFlow::Season);
    assert_eq!(
        state.subject_user_id.as_deref(),
        Some(user_id.to_string().as_str()),
        "the walk binds to the athlete who typed the command"
    );

    // The opener is the agent's message, so the first answer has a question
    // attached and is not message #1.
    let history = resources
        .common
        .repos
        .chat
        .get_messages(&conversation_id, &user_id.to_string(), tenant)
        .await?;
    assert!(
        history
            .iter()
            .any(|m| m.role == "assistant" && m.content == response.text),
        "the opener must be persisted as an assistant message"
    );

    let profile = resources
        .common
        .repos
        .profiles
        .get_profile(user_id)
        .await?
        .expect("a first walk writes the profile");
    let window =
        GuidedWindow::last(Some(&profile), GuidedFlow::Season).expect("the window is recorded");
    assert_eq!(window.completed_at, None, "a running walk is open-ended");
    assert!(
        GuidedWindow::last(Some(&profile), GuidedFlow::Calibration).is_none(),
        "the season walk does not touch calibration's window"
    );
    Ok(())
}

#[tokio::test]
async fn the_opener_quotes_calibrations_availability_back_for_correction() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    let user = user_id.to_string();
    land(
        &resources,
        tenant,
        &user,
        FactKind::Schedule,
        PredicateCode::CanTrainOn,
        "Tuesdays and Thursdays before work, long ride Sunday",
    )
    .await?;

    let response = SeasonHandler
        .execute(&ctx(
            &resources,
            user_id,
            tenant,
            &conversation_id,
            "/season",
        ))
        .await?;
    assert!(
        response
            .text
            .contains("You can train Tuesdays and Thursdays before work, long ride Sunday"),
        "the availability /calibrate recorded is quoted in the athlete's locale: {}",
        response.text
    );
    assert!(
        response.text.contains("Say so if that has changed"),
        "the quote invites correction rather than re-asking: {}",
        response.text
    );
    Ok(())
}

#[tokio::test]
async fn a_calibration_re_run_leaves_a_season_captured_after_it_alone() -> Result<()> {
    // The hazard: both walks land onboarding facts in the training pillar,
    // and a re-run supersedes by window. Before the far bound, a /calibrate
    // re-run expired everything after its previous start — including the
    // race calendar a /season captured a month later.
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    let user = user_id.to_string();
    let repos = &resources.common.repos;

    // A calibration that ran and completed.
    let calibration_started = Utc::now().to_rfc3339();
    sleep(Duration::from_millis(20)).await;
    let calibration_answer = land(
        &resources,
        tenant,
        &user,
        FactKind::Preference,
        PredicateCode::Prefer,
        "more hard days",
    )
    .await?;
    sleep(Duration::from_millis(20)).await;
    let calibration_done = Utc::now().to_rfc3339();
    let profile = GuidedWindow::record_start(None, GuidedFlow::Calibration, &calibration_started);
    let profile =
        GuidedWindow::record_completion(Some(profile), GuidedFlow::Calibration, &calibration_done);
    repos.profiles.upsert_profile(user_id, profile).await?;

    // Then a season walk, whose calendar lands after the calibration closed.
    sleep(Duration::from_millis(20)).await;
    let calendar = land(
        &resources,
        tenant,
        &user,
        FactKind::Goal,
        PredicateCode::TrainingFor,
        "Ironman Mont-Tremblant, August",
    )
    .await?;
    assert!(is_live(&resources, tenant, &user, &calendar).await);

    // The athlete recalibrates.
    CalibrateHandler
        .execute(&ctx(
            &resources,
            user_id,
            tenant,
            &conversation_id,
            "/calibrate",
        ))
        .await?;

    assert!(
        !is_live(&resources, tenant, &user, &calibration_answer).await,
        "the previous calibration's own answer is superseded"
    );
    assert!(
        is_live(&resources, tenant, &user, &calendar).await,
        "the season's calendar, captured after that calibration completed, survives it"
    );

    // And the other direction: a season re-run supersedes its own previous
    // window and nothing else.
    let season_started = Utc::now().to_rfc3339();
    sleep(Duration::from_millis(20)).await;
    let old_best = land(
        &resources,
        tenant,
        &user,
        FactKind::Physiology,
        PredicateCode::HaveBaseline,
        "3:42 marathon, 2024",
    )
    .await?;
    sleep(Duration::from_millis(20)).await;
    let season_done = Utc::now().to_rfc3339();
    let profile = repos.profiles.get_profile(user_id).await?;
    let profile = GuidedWindow::record_start(profile, GuidedFlow::Season, &season_started);
    let profile = GuidedWindow::record_completion(Some(profile), GuidedFlow::Season, &season_done);
    repos.profiles.upsert_profile(user_id, profile).await?;
    sleep(Duration::from_millis(20)).await;
    let later_calibration = land(
        &resources,
        tenant,
        &user,
        FactKind::Injury,
        PredicateCode::Have,
        "left achilles, flares on hills",
    )
    .await?;

    SeasonHandler
        .execute(&ctx(
            &resources,
            user_id,
            tenant,
            &conversation_id,
            "/season",
        ))
        .await?;
    assert!(
        !is_live(&resources, tenant, &user, &old_best).await,
        "the previous season walk's own answer is superseded"
    );
    assert!(
        is_live(&resources, tenant, &user, &later_calibration).await,
        "an injury answer landed after the season walk completed survives its re-run"
    );
    Ok(())
}
