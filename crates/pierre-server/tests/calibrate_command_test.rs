// ABOUTME: /calibrate handler — a profile read that fails must abort instead of overwriting the profile
// ABOUTME: Content-asserting on real rows: the stored profile blob, the conversation state, the history
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! `/calibrate` reads the athlete's profile, merges this interview's start time
//! into it, and writes the whole document back — `upsert_profile` is
//! `ON CONFLICT(user_id) DO UPDATE SET profile_data = $2`, a full replace.
//!
//! So "the profile could not be read" and "this athlete has no profile" must
//! stay distinct. Collapsing them makes the merge start from an empty document
//! and the write replace the athlete's nutrition and equipment blocks — which
//! `compose_dossier` reads out of this same blob — with a bare `{"calibration":
//! …}`, while the previous interview's answers go unsuperseded.
//!
//! A genuinely new athlete (`Ok(None)`) still opens an interview.

use anyhow::Result;
use chrono::Utc;
use pierre_commands::calibration::CalibrateHandler;
use pierre_commands::{CommandHandler, PlatformCommandContext};
use pierre_core::models::{GuidedFlow, OnboardingState, TenantId};
use pierre_database::backends::factory::Database;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_runtime_context::CoachesCtx;
use sqlx::{Pool, Sqlite};
use std::sync::Arc;
use uuid::Uuid;

mod common;

/// A stored profile the repository cannot hand back.
///
/// `get_profile` deserializes this column and propagates the parse failure,
/// which is the one read fault a SQLite-backed test can inject without also
/// breaking the write it has to prove never happens. The handler cannot tell
/// one repository error from another, so this stands in for the transient
/// pool-acquire class too.
///
/// It carries the athlete's nutrition and equipment blocks, so what the test
/// asserts is their survival rather than the bare existence of a row.
const UNREADABLE_PROFILE: &str =
    r#"{"nutrition":{"carbs_per_hour":60},"equipment":{"bike":"Tarmac"}"#;

async fn setup() -> Result<(Arc<ServerContext>, Uuid, TenantId, String)> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    let email = format!("calibrate_{}@example.com", Uuid::new_v4());
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
            "calibration",
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
) -> PlatformCommandContext {
    PlatformCommandContext {
        user_id,
        tenant_id,
        channel_type: "telegram".to_owned(),
        args: vec![],
        raw_text: "/calibrate".to_owned(),
        ctx: Arc::<ServerContext>::clone(resources) as Arc<dyn pierre_runtime_context::CommandCtx>,
        locale: "en".to_owned(),
        is_direct_message: true,
        conversation_id,
        conversation_tenant_id: tenant_id,
        sender_id: None,
        tool_runtime: Arc::<ServerContext>::clone(resources),
    }
}

/// The `user_profiles` column, straight out of the table.
///
/// Read raw because the point of the failing case is that the repository
/// cannot parse it — a readback through `get_profile` would fail the same way
/// and prove nothing about what is stored.
fn sqlite_pool(resources: &Arc<ServerContext>) -> &Pool<Sqlite> {
    match resources.database().as_ref() {
        Database::SQLite(sqlite) => sqlite.pool(),
        #[cfg(feature = "postgresql")]
        Database::PostgreSQL(_) => panic!("this test runs against the SQLite backend"),
    }
}

async fn store_raw_profile(pool: &Pool<Sqlite>, user_id: Uuid, profile_data: &str) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO user_profiles (user_id, profile_data, created_at, updated_at)
         VALUES (?, ?, ?, ?)",
    )
    .bind(user_id.to_string())
    .bind(profile_data)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(())
}

async fn read_raw_profile(pool: &Pool<Sqlite>, user_id: Uuid) -> Result<Option<String>> {
    let stored: Option<String> =
        sqlx::query_scalar("SELECT profile_data FROM user_profiles WHERE user_id = ?")
            .bind(user_id.to_string())
            .fetch_optional(pool)
            .await?;
    Ok(stored)
}

#[tokio::test]
async fn a_failed_profile_read_reports_failure_and_leaves_the_profile_untouched() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    let pool = sqlite_pool(&resources);
    store_raw_profile(pool, user_id, UNREADABLE_PROFILE).await?;

    let outcome = CalibrateHandler
        .execute(&ctx(
            &resources,
            user_id,
            tenant,
            Some(conversation_id.clone()),
        ))
        .await;
    // Checked after the stored document below, so the write the read failure
    // used to license is the assertion that fires, rather than being masked by
    // the reply the athlete got.
    let reported_failure = outcome.is_err();

    // The whole document is still the athlete's.
    let stored = read_raw_profile(pool, user_id)
        .await?
        .expect("the profile row must still exist");
    assert_eq!(
        stored, UNREADABLE_PROFILE,
        "a failed read must not rewrite the athlete's profile, got: {stored}"
    );
    assert!(
        stored.contains("carbs_per_hour") && stored.contains("Tarmac"),
        "the nutrition and equipment blocks must survive, got: {stored}"
    );
    assert!(
        reported_failure,
        "a profile read failure must be reported to the caller, not answered with an opener"
    );

    // And nothing else about the turn happened either.
    let conv = resources
        .common
        .repos
        .chat
        .get_conversation(&conversation_id, &user_id.to_string(), tenant)
        .await?
        .expect("conversation");
    assert!(
        conv.onboarding_state.is_none(),
        "an aborted /calibrate must not open an interview"
    );
    let history = resources
        .common
        .repos
        .chat
        .get_messages(&conversation_id, &user_id.to_string(), tenant)
        .await?;
    assert!(
        history.is_empty(),
        "an aborted /calibrate must not persist an opener, got {history:?}"
    );
    Ok(())
}

#[tokio::test]
async fn an_athlete_with_no_profile_still_starts_the_interview() -> Result<()> {
    let (resources, user_id, tenant, conversation_id) = setup().await?;
    let pool = sqlite_pool(&resources);
    assert!(
        read_raw_profile(pool, user_id).await?.is_none(),
        "this athlete has never had a profile row; the read returns Ok(None)"
    );

    let response = CalibrateHandler
        .execute(&ctx(
            &resources,
            user_id,
            tenant,
            Some(conversation_id.clone()),
        ))
        .await?;
    assert!(
        response.text.contains("six short questions"),
        "the opener must announce the interview, got: {}",
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
        .expect("the interview should be active");
    assert!(state.active);
    assert_eq!(state.flow, GuidedFlow::Calibration);

    // The first calibration records its window for the next re-run to supersede.
    let profile = resources
        .common
        .repos
        .profiles
        .get_profile(user_id)
        .await?
        .expect("a first calibration writes the profile");
    assert!(
        profile["calibration"]["last_started_at"].as_str().is_some(),
        "the interview start must be recorded, got: {profile}"
    );
    Ok(())
}
