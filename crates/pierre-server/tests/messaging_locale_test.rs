// ABOUTME: End-to-end locale tests for messaging command handlers and the registry
// ABOUTME: Validates FR/EN/ES/DE/PT rendering + user.locale/channel_link override resolution
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Locale plumbing tests.
//!
//! Covers the chain from `MessagingStringsRegistry` lookups through the
//! messaging ingress resolver up to a single slash-command handler (`/status`)
//! so a regression anywhere in that path fails here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod common;

use std::sync::Arc;

use chrono::Utc;
use pierre_database::plugins::CreateChannelLinkParams;
use pierre_mcp_server::contremaitre::messaging_strings::{
    MessagingStringsRegistry, KEY_CAPABILITY_REFUSAL, KEY_COACH_ASSIGN_FORBIDDEN,
    KEY_GROUP_LIST_EMPTY, KEY_HELP_FOOTER, KEY_SCOPE_REFUSAL, KEY_STATUS_CHANNEL_LABEL,
    KEY_STATUS_HEADER, KEY_STATUS_PROVIDERS_NONE,
};
use pierre_mcp_server::mcp::resources::ServerResources;
use pierre_mcp_server::models::{Tenant, TenantId, User, UserStatus};
use pierre_mcp_server::services::commands::status::StatusHandler;
use pierre_mcp_server::services::commands::{CommandHandler, PlatformCommandContext};
use pierre_mcp_server::services::messaging_ingress::{
    detect_turn_locale, resolve_messaging_locale,
};
use tokio::task::spawn_blocking;
use uuid::Uuid;

/// Registry smoke test: every compiled-in locale resolves to a non-empty
/// string for the hot-path keys used by the user-visible surface. Guards
/// against an accidental drop when editing the `COMPILED_IN` table.
#[test]
fn registry_has_five_compiled_locales_for_hot_keys() {
    let reg = MessagingStringsRegistry::new();
    let hot_keys = [
        KEY_STATUS_HEADER,
        KEY_STATUS_PROVIDERS_NONE,
        KEY_STATUS_CHANNEL_LABEL,
        KEY_HELP_FOOTER,
        KEY_GROUP_LIST_EMPTY,
        KEY_COACH_ASSIGN_FORBIDDEN,
    ];
    for locale in ["fr", "en", "es", "de", "pt"] {
        for key in hot_keys {
            let value = reg.get(key, locale);
            assert!(
                !value.trim().is_empty(),
                "registry missing {key} for {locale}"
            );
        }
    }
}

/// Scope + capability refusal strings must exist in all 5 compiled-in
/// locales. Off-topic and missing-capability asks route through these keys
/// so the LLM emits a deterministic localized refusal instead of
/// translating on the fly.
#[test]
fn registry_exposes_refusal_strings_for_every_locale() {
    let reg = MessagingStringsRegistry::new();
    for locale in ["fr", "en", "es", "de", "pt"] {
        let scope = reg.get(KEY_SCOPE_REFUSAL, locale);
        let capability = reg.get(KEY_CAPABILITY_REFUSAL, locale);
        assert!(
            !scope.trim().is_empty(),
            "missing scope refusal for {locale}"
        );
        assert!(
            !capability.trim().is_empty(),
            "missing capability refusal for {locale}"
        );
    }
    // Canonical French + English anchors so drift from the agreed wording
    // fails loudly instead of silently translating per-build.
    assert!(reg
        .get(KEY_SCOPE_REFUSAL, "fr")
        .contains("assistant fitness"));
    assert!(reg
        .get(KEY_SCOPE_REFUSAL, "en")
        .contains("fitness assistant"));
}

/// `detect_turn_locale` picks the message-content language over the stored
/// fallback so status placeholders match the LLM's reply language even when
/// the user is writing in a language different from their saved preference.
#[test]
fn detect_turn_locale_matches_message_language_over_fallback() {
    // English sentence, user.locale = "fr" → detected "en".
    let l = detect_turn_locale(
        "Are my latest workouts making me on track for the race on May 29?",
        "fr",
    );
    assert_eq!(l, "en", "English message must override French fallback");

    // French sentence, user.locale = "en" → detected "fr".
    let l = detect_turn_locale(
        "Peux-tu analyser ma dernière sortie et me dire si je suis sur la bonne voie?",
        "en",
    );
    assert_eq!(l, "fr", "French message must override English fallback");

    // Short message (<12 chars) → fallback regardless of content.
    assert_eq!(detect_turn_locale("ok", "fr"), "fr");
    assert_eq!(detect_turn_locale("oui", "en"), "en");

    // Message in an unsupported locale → fallback.
    // Japanese text is clearly detected but not in our locale set.
    assert_eq!(
        detect_turn_locale(
            "これは日本語のテストメッセージです。トレーニングについて質問があります。",
            "fr"
        ),
        "fr"
    );
}

/// Requested locale matches → exact string. Requested unknown locale (`"xx"`)
/// falls back to the French default. Sanity check that EN is distinct so the
/// test is catching fallback behavior, not coincidental equality.
#[test]
fn registry_falls_back_to_default_locale_on_unknown() {
    let reg = MessagingStringsRegistry::new();
    let fr = reg.get(KEY_STATUS_HEADER, "fr");
    assert_eq!(reg.get(KEY_STATUS_HEADER, "xx"), fr);
    assert_ne!(reg.get(KEY_STATUS_HEADER, "en"), fr);
}

/// Create a minimal active user + tenant pair with both rows linked via the
/// `tenants.owner_user_id` column. Returns `(user_id, tenant_id)`.
async fn seed_user_and_tenant(resources: &ServerResources) -> (Uuid, TenantId) {
    let password_hash = spawn_blocking(|| bcrypt::hash("Pass123!", bcrypt::DEFAULT_COST).unwrap())
        .await
        .unwrap();

    let mut user = User::new(
        format!("locale-{}@test.com", Uuid::new_v4()),
        password_hash,
        Some("Locale Test User".to_owned()),
    );
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(Utc::now());

    let user_id = user.id;
    resources.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::new();
    let tenant = Tenant {
        id: tenant_id,
        name: "Locale Test Tenant".to_owned(),
        slug: format!("locale-{tenant_id}"),
        domain: None,
        plan: "professional".to_owned(),
        owner_user_id: user_id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    resources.repos.tenants.create(&tenant).await.unwrap();

    (user_id, tenant_id)
}

/// `/status` emits French by default (`users.locale = 'fr'` from migration).
#[tokio::test]
async fn status_handler_renders_french_by_default() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, tenant_id) = seed_user_and_tenant(&resources).await;

    let ctx = PlatformCommandContext {
        user_id,
        tenant_id,
        channel_type: "telegram".to_owned(),
        args: vec![],
        raw_text: "/status".to_owned(),
        resources: Arc::clone(&resources),
        locale: "fr".to_owned(),
        is_direct_message: false,
    };

    let response = StatusHandler.execute(&ctx).await.expect("status FR");
    assert!(
        response.text.starts_with("Ton statut Dravr"),
        "expected FR header, got: {}",
        response.text
    );
    assert!(response.text.contains("Fournisseurs : aucun connecté"));
    assert!(response.text.contains("Canal : telegram"));
}

/// `/status` switches to English when `ctx.locale == "en"`.
#[tokio::test]
async fn status_handler_switches_to_english_when_locale_set() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, tenant_id) = seed_user_and_tenant(&resources).await;

    let ctx = PlatformCommandContext {
        user_id,
        tenant_id,
        channel_type: "telegram".to_owned(),
        args: vec![],
        raw_text: "/status".to_owned(),
        resources: Arc::clone(&resources),
        locale: "en".to_owned(),
        is_direct_message: false,
    };

    let response = StatusHandler.execute(&ctx).await.expect("status EN");
    assert!(
        response.text.starts_with("Your Dravr status"),
        "expected EN header, got: {}",
        response.text
    );
    assert!(response.text.contains("Providers: none connected"));
    assert!(response.text.contains("Channel: telegram"));
}

/// `users.locale` round-trip: `update_locale` persists and `get_global` returns it.
#[tokio::test]
async fn user_repository_persists_locale_round_trip() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, _tenant_id) = seed_user_and_tenant(&resources).await;

    let u = resources
        .repos
        .users
        .get_global(user_id)
        .await
        .expect("get_global ok")
        .expect("user exists");
    assert_eq!(u.locale, "fr", "default locale should be 'fr'");

    resources
        .repos
        .users
        .update_locale(user_id, "en")
        .await
        .expect("update_locale ok");

    let u = resources
        .repos
        .users
        .get_global(user_id)
        .await
        .expect("get_global ok")
        .expect("user exists");
    assert_eq!(u.locale, "en");
}

/// Channel-link locale override wins over `users.locale`; clearing it falls
/// back to the user preference; missing-everything falls back to default.
#[tokio::test]
async fn resolve_locale_prefers_channel_link_override() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, tenant_id) = seed_user_and_tenant(&resources).await;

    resources
        .repos
        .users
        .update_locale(user_id, "es")
        .await
        .expect("user locale es");

    let link_id = Uuid::new_v4().to_string();
    resources
        .repos
        .messaging
        .create_channel_link(&CreateChannelLinkParams {
            id: &link_id,
            tenant_id,
            user_id: &user_id.to_string(),
            channel_type: "telegram",
            channel_user_id: "tg-42",
            display_name: Some("Jean"),
        })
        .await
        .expect("create_channel_link");

    resources
        .repos
        .messaging
        .set_channel_link_locale(tenant_id, &user_id.to_string(), "telegram", Some("de"))
        .await
        .expect("set_channel_link_locale");

    let resolved =
        resolve_messaging_locale(&resources, tenant_id, user_id, "telegram", "tg-42").await;
    assert_eq!(
        resolved, "de",
        "channel-link override must beat users.locale"
    );

    resources
        .repos
        .messaging
        .set_channel_link_locale(tenant_id, &user_id.to_string(), "telegram", None)
        .await
        .expect("clear channel_link_locale");

    let resolved =
        resolve_messaging_locale(&resources, tenant_id, user_id, "telegram", "tg-42").await;
    assert_eq!(
        resolved, "es",
        "cleared override must fall through to users.locale"
    );
}

/// When no channel link exists and the user still has the migration default,
/// the resolver returns `"fr"`.
#[tokio::test]
async fn resolve_locale_returns_default_when_nothing_set() {
    let resources = common::create_test_server_resources().await.unwrap();
    let (user_id, tenant_id) = seed_user_and_tenant(&resources).await;

    let resolved =
        resolve_messaging_locale(&resources, tenant_id, user_id, "telegram", "never-linked").await;
    assert_eq!(resolved, "fr");
}
