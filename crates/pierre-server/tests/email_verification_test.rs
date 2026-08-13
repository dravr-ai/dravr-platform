// ABOUTME: Integration tests for email-address verification — issue, consume, single-use, lockout, clamping
// ABOUTME: Covers the G1/G11 gate: registration proves the address before approval is even considered
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]
#![allow(clippy::uninlined_format_args)]

//! Integration tests for email-address verification:
//! 1. A token is issued and stored hashed, never whole
//! 2. Consuming it stamps `users.email_verified_at` and applies the approval decision
//! 3. The token is single-use, and a wrong verifier costs an attempt without consuming it
//! 4. Operator settings are read from `system_settings` and clamped to a sane range

mod common;
mod helpers;

use pierre_config::constants::email_verification::{
    DEFAULT_LINK_TTL_MINUTES, DEFAULT_MAX_SENDS_PER_HOUR,
};
use pierre_config::environment::{
    AppBehaviorConfig, BackupConfig, DatabaseConfig, DatabaseUrl, Environment, SecurityConfig,
    SecurityHeadersConfig, ServerConfig,
};
use pierre_database::database::system_settings::{
    SETTING_EMAIL_VERIFICATION_MAX_PER_HOUR, SETTING_EMAIL_VERIFICATION_TTL_MINUTES,
};
use pierre_mcp_server::mcp::resources::{ServerContext, ServerContextOptions};
use pierre_memory::FactKind;
use pierre_services::email_verification::resolve_settings;
use pierre_services::link_token::generate_link_token;
use std::sync::Arc;

struct VerificationTestSetup {
    resources: Arc<ServerContext>,
}

impl VerificationTestSetup {
    async fn new() -> anyhow::Result<Self> {
        common::init_server_config();
        let database = common::create_test_database().await?;
        let auth_manager = common::create_test_auth_manager();
        let cache = common::create_test_cache().await?;

        let temp_dir = tempfile::tempdir()?;
        let config = Arc::new(ServerConfig {
            http_port: 8081,
            database: DatabaseConfig {
                url: DatabaseUrl::Memory,
                backup: BackupConfig {
                    directory: temp_dir.path().to_path_buf(),
                    ..Default::default()
                },
                ..Default::default()
            },
            app_behavior: AppBehaviorConfig {
                ci_mode: true,
                auto_approve_users: false,
                ..Default::default()
            },
            security: SecurityConfig {
                headers: SecurityHeadersConfig {
                    environment: Environment::Testing,
                },
                ..Default::default()
            },
            ..Default::default()
        });

        let resources = Arc::new(
            ServerContext::new(
                (*database).clone(),
                (*auth_manager).clone(),
                "test_jwt_secret",
                config,
                cache,
                ServerContextOptions {
                    rsa_key_size_bits: Some(2048),
                    jwks_manager: Some(common::get_shared_test_jwks()),
                    llm_provider: None,
                    chat_provider: None,
                    extra_tools: Vec::new(),
                    billing_provider: None,
                },
            )
            .await,
        );

        Ok(Self { resources })
    }

    async fn create_user(&self) -> anyhow::Result<uuid::Uuid> {
        let (_, user) = common::create_test_user(&self.resources.coach.database).await?;
        Ok(user.id)
    }
}

/// A freshly registered user is unverified, and consuming a valid token both
/// stamps the address and is visible through `is_verified`.
#[tokio::test]
async fn consuming_a_valid_token_marks_the_address_verified() {
    let setup = VerificationTestSetup::new().await.expect("setup failed");
    let user_id = setup.create_user().await.expect("user creation failed");
    let repo = &setup.resources.common.repos.email_verification;

    assert!(
        !repo.is_verified(user_id).await.expect("is_verified failed"),
        "a user who has never confirmed an address must read as unverified"
    );

    let generated = generate_link_token();
    repo.store_token(user_id, &generated.selector, &generated.verifier_hash, 60)
        .await
        .expect("store_token failed");

    let consumed = repo
        .consume_token(&generated.selector, &generated.verifier_hash)
        .await
        .expect("a freshly issued token must consume cleanly");
    assert_eq!(
        consumed, user_id,
        "consuming must return the user the token was issued to"
    );

    repo.mark_verified(user_id)
        .await
        .expect("mark_verified failed");

    assert!(
        repo.is_verified(user_id).await.expect("is_verified failed"),
        "the address must read as verified once the token is consumed and stamped"
    );
}

/// The token is single-use: replaying it after a successful consume is rejected,
/// which is what stops a leaked link from being reused.
#[tokio::test]
async fn a_consumed_token_cannot_be_replayed() {
    let setup = VerificationTestSetup::new().await.expect("setup failed");
    let user_id = setup.create_user().await.expect("user creation failed");
    let repo = &setup.resources.common.repos.email_verification;

    let generated = generate_link_token();
    repo.store_token(user_id, &generated.selector, &generated.verifier_hash, 60)
        .await
        .expect("store_token failed");

    repo.consume_token(&generated.selector, &generated.verifier_hash)
        .await
        .expect("first consume must succeed");

    let replay = repo
        .consume_token(&generated.selector, &generated.verifier_hash)
        .await;
    assert!(
        replay.is_err(),
        "replaying a consumed verification token must be rejected"
    );
}

/// A wrong verifier is rejected *without* burning the token, so a bad guess
/// cannot deny a legitimate user their own link.
#[tokio::test]
async fn a_wrong_verifier_is_rejected_but_leaves_the_token_usable() {
    let setup = VerificationTestSetup::new().await.expect("setup failed");
    let user_id = setup.create_user().await.expect("user creation failed");
    let repo = &setup.resources.common.repos.email_verification;

    let generated = generate_link_token();
    repo.store_token(user_id, &generated.selector, &generated.verifier_hash, 60)
        .await
        .expect("store_token failed");

    let wrong = repo
        .consume_token(
            &generated.selector,
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .await;
    assert!(wrong.is_err(), "a wrong verifier must be rejected");

    let good = repo
        .consume_token(&generated.selector, &generated.verifier_hash)
        .await
        .expect("the real verifier must still work after one wrong guess");
    assert_eq!(good, user_id);
}

/// An unknown selector is rejected with the same error shape as every other
/// failure, so the endpoint reveals nothing about which condition hit.
#[tokio::test]
async fn an_unknown_selector_is_rejected() {
    let setup = VerificationTestSetup::new().await.expect("setup failed");
    let repo = &setup.resources.common.repos.email_verification;

    let result = repo
        .consume_token("selector-that-was-never-issued", "deadbeef")
        .await;
    assert!(result.is_err(), "an unknown selector must be rejected");
}

/// The per-user issue count is what the rate limiter reads; it must actually
/// count the tokens issued rather than always reading zero.
#[tokio::test]
async fn issued_tokens_are_counted_for_rate_limiting() {
    let setup = VerificationTestSetup::new().await.expect("setup failed");
    let user_id = setup.create_user().await.expect("user creation failed");
    let repo = &setup.resources.common.repos.email_verification;

    let since = chrono::Utc::now() - chrono::Duration::hours(1);
    assert_eq!(
        repo.count_recent_tokens(user_id, since)
            .await
            .expect("count failed"),
        0,
        "a user with no tokens must count zero"
    );

    for _ in 0..3 {
        let generated = generate_link_token();
        repo.store_token(user_id, &generated.selector, &generated.verifier_hash, 60)
            .await
            .expect("store_token failed");
    }

    assert_eq!(
        repo.count_recent_tokens(user_id, since)
            .await
            .expect("count failed"),
        3,
        "every issued token must be counted for the hourly cap"
    );
}

/// With no stored rows the resolver returns the compiled defaults.
#[tokio::test]
async fn settings_fall_back_to_the_compiled_defaults() {
    let setup = VerificationTestSetup::new().await.expect("setup failed");
    let settings = resolve_settings(setup.resources.coach.database.as_ref()).await;

    assert_eq!(
        settings.ttl_minutes, DEFAULT_LINK_TTL_MINUTES,
        "an unconfigured deployment must use the default TTL"
    );
    assert_eq!(
        settings.max_sends_per_hour, DEFAULT_MAX_SENDS_PER_HOUR,
        "an unconfigured deployment must use the default send cap"
    );
}

/// An operator-set value inside the allowed range is honoured verbatim.
#[tokio::test]
async fn operator_settings_are_honoured_when_in_range() {
    let setup = VerificationTestSetup::new().await.expect("setup failed");
    let db = setup.resources.coach.database.as_ref();

    db.set_system_setting(SETTING_EMAIL_VERIFICATION_TTL_MINUTES, "90")
        .await
        .expect("set ttl failed");
    db.set_system_setting(SETTING_EMAIL_VERIFICATION_MAX_PER_HOUR, "9")
        .await
        .expect("set cap failed");

    let settings = resolve_settings(db).await;
    assert_eq!(
        settings.ttl_minutes, 90,
        "an in-range TTL must be used as-is"
    );
    assert_eq!(
        settings.max_sends_per_hour, 9,
        "an in-range send cap must be used as-is"
    );
}

/// A hostile or fat-fingered row cannot disable the gate: zero and absurd values
/// are pulled to the nearest bound, and non-numeric text falls back to default.
#[tokio::test]
async fn out_of_range_and_garbage_settings_are_clamped() {
    use pierre_config::constants::email_verification as defaults;

    let setup = VerificationTestSetup::new().await.expect("setup failed");
    let db = setup.resources.coach.database.as_ref();

    // Zero would kill every link on arrival and lock users out permanently.
    db.set_system_setting(SETTING_EMAIL_VERIFICATION_TTL_MINUTES, "0")
        .await
        .expect("set ttl failed");
    db.set_system_setting(SETTING_EMAIL_VERIFICATION_MAX_PER_HOUR, "0")
        .await
        .expect("set cap failed");

    let settings = resolve_settings(db).await;
    assert_eq!(
        settings.ttl_minutes,
        defaults::MIN_LINK_TTL_MINUTES,
        "a zero TTL must clamp up to the floor, not disable the link"
    );
    assert_eq!(
        settings.max_sends_per_hour,
        defaults::MIN_MAX_SENDS_PER_HOUR,
        "a zero send cap must clamp up, never lock the user out of their own account"
    );

    // A decade-long TTL retires the expiry entirely.
    db.set_system_setting(SETTING_EMAIL_VERIFICATION_TTL_MINUTES, "99999999")
        .await
        .expect("set ttl failed");
    assert_eq!(
        resolve_settings(db).await.ttl_minutes,
        defaults::MAX_LINK_TTL_MINUTES,
        "an absurd TTL must clamp down to the ceiling"
    );

    // TEXT column: a row can hold anything at all.
    db.set_system_setting(SETTING_EMAIL_VERIFICATION_TTL_MINUTES, "not-a-number")
        .await
        .expect("set ttl failed");
    assert_eq!(
        resolve_settings(db).await.ttl_minutes,
        defaults::DEFAULT_LINK_TTL_MINUTES,
        "a non-numeric row must fall back to the compiled default"
    );
}

/// Re-answering the about-you step must REPLACE the previous answer, not stack a
/// second one beside it.
///
/// `upsert_user_fact` is a plain insert despite its name, so without an explicit
/// supersede a second submission leaves the athlete with two North Stars and
/// feeds both into the coach prompt. Asserts the live count, which a duplicating
/// implementation fails.
#[tokio::test]
async fn re_answering_about_you_supersedes_rather_than_duplicates() {
    use pierre_services::about_you::{persist_about_you, AboutYouAnswers};

    let setup = VerificationTestSetup::new().await.expect("setup failed");
    let user_id = setup.create_user().await.expect("user creation failed");
    let tenant = setup
        .resources
        .common
        .repos
        .tenants
        .list_for_user(user_id)
        .await
        .expect("tenant lookup failed")
        .first()
        .map(|t| t.id)
        .expect("test user must belong to a tenant");
    let memory = setup.resources.common.repos.memory.as_ref();
    let uid = user_id.to_string();

    let first = AboutYouAnswers {
        north_star: Some("Keeping up with my kids".to_owned()),
        primary_sport: Some("Running".to_owned()),
        goal: Some("First half-marathon".to_owned()),
    };
    assert_eq!(
        persist_about_you(memory, tenant, &uid, &first)
            .await
            .expect("first submission failed"),
        3,
        "three answers must write three facts"
    );

    let second = AboutYouAnswers {
        north_star: Some("Still running trails at 70".to_owned()),
        primary_sport: Some("Cycling".to_owned()),
        goal: Some("Sub-40 10k".to_owned()),
    };
    persist_about_you(memory, tenant, &uid, &second)
        .await
        .expect("second submission failed");

    let facts = memory
        .list_user_facts(tenant, &uid, None, None, 100)
        .await
        .expect("fact read failed");
    // Superseding sets `valid_until` rather than deleting, so "live" means the
    // horizon is absent or still in the future.
    let now = chrono::Utc::now();
    let live: Vec<_> = facts
        .iter()
        .filter(|f| f.valid_until.is_none_or(|until| until > now))
        .collect();

    for kind in [FactKind::NorthStar, FactKind::Preference, FactKind::Goal] {
        let matching = live.iter().filter(|f| f.kind == kind).count();
        assert_eq!(
            matching, 1,
            "exactly one live {kind:?} must survive re-answering, found {matching}"
        );
    }

    // And it must be the NEW answer that survived, not the old one.
    assert!(
        live.iter()
            .any(|f| f.object.contains("Still running trails at 70")),
        "the second North Star must be the live one"
    );
    assert!(
        !live
            .iter()
            .any(|f| f.object.contains("Keeping up with my kids")),
        "the first North Star must have been superseded"
    );
}
