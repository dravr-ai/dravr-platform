// ABOUTME: Verifies a tenant's stored BYO LLM key resolves to a per-tenant chat
// ABOUTME: provider, and that a tenant without one falls back to the singleton.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;

use std::env;

use common::{create_test_server_resources, create_test_user_with_plan};
use pierre_auth::tenant::llm_manager::{LlmProvider, StoreLlmCredentialsRequest, TenantLlmManager};
use pierre_services::tenant_chat_provider::{
    resolve_tenant_chat_provider, TenantChatProviderCache,
};
use serial_test::serial;

#[tokio::test]
#[serial]
async fn byo_key_resolves_provider_and_absence_falls_back() {
    // Simulate a deployment whose active provider is Gemini (an API provider),
    // so a stored Gemini key is eligible and the provider can be constructed.
    // The dev env default is a CLI runner with no Gemini model config.
    let saved: Vec<(&str, Option<String>)> = [
        ("PIERRE_LLM_PROVIDER", "gemini"),
        ("PIERRE_LLM_DEFAULT_MODEL", "gemini-flash-latest"),
        ("PIERRE_LLM_FALLBACK_MODEL", "gemini-flash-lite-latest"),
    ]
    .iter()
    .map(|(k, v)| {
        let prev = env::var(k).ok();
        env::set_var(k, v);
        (*k, prev)
    })
    .collect();

    let resources = create_test_server_resources().await.unwrap();
    let llm_creds = resources.common.repos.llm_credentials.as_ref();
    let security = resources.common.repos.security.as_ref();
    let cache = TenantChatProviderCache::new();

    // Tenant WITH a stored Gemini key → resolves to a per-tenant provider.
    let (user_id, _u, tenant_id) =
        create_test_user_with_plan(&resources.coach.database, "byo@test.com", "professional")
            .await
            .unwrap();
    TenantLlmManager::store_credentials(
        None, // tenant-level default
        tenant_id,
        StoreLlmCredentialsRequest {
            provider: LlmProvider::Gemini,
            api_key: "test-gemini-key".to_owned(),
            base_url: None,
            default_model: None,
        },
        user_id, // created_by
        llm_creds,
        security,
    )
    .await
    .unwrap();

    let resolved =
        resolve_tenant_chat_provider(&cache, llm_creds, security, tenant_id, user_id).await;
    assert!(
        resolved.is_some(),
        "tenant with a stored Gemini key must resolve to a BYO provider"
    );

    // Tenant WITHOUT a stored key → None (use the global singleton).
    let (other_user, _ou, other_tenant) =
        create_test_user_with_plan(&resources.coach.database, "nobyo@test.com", "professional")
            .await
            .unwrap();
    let fallback =
        resolve_tenant_chat_provider(&cache, llm_creds, security, other_tenant, other_user).await;
    assert!(
        fallback.is_none(),
        "tenant without a stored key must fall back to the singleton (None)"
    );

    for (key, prev) in saved {
        match prev {
            Some(v) => env::set_var(key, v),
            None => env::remove_var(key),
        }
    }
}
