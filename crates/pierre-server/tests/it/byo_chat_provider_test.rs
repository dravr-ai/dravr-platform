// ABOUTME: Verifies a tenant's stored BYO LLM key resolves to a per-tenant chat
// ABOUTME: provider for the chain it is handed, and that a tenant without one falls back to the singleton.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use crate::common::{create_test_server_resources, create_test_user_with_plan};
use pierre_auth::tenant::llm_manager::{LlmProvider, StoreLlmCredentialsRequest, TenantLlmManager};
use pierre_llm::config::LlmChain;
use pierre_services::tenant_chat_provider::{
    resolve_tenant_chat_provider, TenantChatProviderCache,
};
use secrecy::SecretString;

/// The model the stored Gemini key names, so the BYO provider built from it
/// carries no model the environment chose.
const BYO_MODEL: &str = "gemini-flash-latest";

#[tokio::test]
async fn byo_key_resolves_provider_and_absence_falls_back() {
    // A deployment whose chain is led by Gemini (an API provider), so a stored
    // Gemini key is eligible and the provider can be constructed.
    let chain = LlmChain::parse("gemini");

    let resources = create_test_server_resources().await.unwrap();
    let llm_creds = resources.common.repos.llm_credentials.as_ref();
    let security = resources.common.repos.security.as_ref();
    let cache = TenantChatProviderCache::new();

    // Tenant WITH a stored Gemini key → resolves to a per-tenant provider.
    let (user_id, _u, tenant_id) =
        create_test_user_with_plan(&resources.agent.database, "byo@test.com", "professional")
            .await
            .unwrap();
    TenantLlmManager::store_credentials(
        None, // tenant-level default
        tenant_id,
        StoreLlmCredentialsRequest {
            provider: LlmProvider::Gemini,
            api_key: SecretString::from("test-gemini-key"),
            base_url: None,
            default_model: Some(BYO_MODEL.to_owned()),
        },
        user_id, // created_by
        llm_creds,
        security,
    )
    .await
    .unwrap();

    let resolved =
        resolve_tenant_chat_provider(&cache, &chain, llm_creds, security, tenant_id, user_id)
            .await
            .expect("tenant with a stored Gemini key must resolve to a BYO provider");
    assert_eq!(resolved.name(), "gemini");
    assert_eq!(resolved.default_model(), BYO_MODEL);

    // Tenant WITHOUT a stored key → None (use the global singleton).
    let (other_user, _ou, other_tenant) =
        create_test_user_with_plan(&resources.agent.database, "nobyo@test.com", "professional")
            .await
            .unwrap();
    let fallback = resolve_tenant_chat_provider(
        &cache,
        &chain,
        llm_creds,
        security,
        other_tenant,
        other_user,
    )
    .await;
    assert!(
        fallback.is_none(),
        "tenant without a stored key must fall back to the singleton (None)"
    );
}

/// The resolver asks the chain it is handed which provider a tenant key must
/// be for — never the process environment. The same tenant, holding the same
/// stored Gemini key, gets its own provider under a Gemini-led chain and the
/// singleton under a chain led by a Copilot runtime, which takes no tenant key.
#[tokio::test]
async fn the_chain_handed_in_decides_which_stored_key_applies() {
    let resources = create_test_server_resources().await.unwrap();
    let llm_creds = resources.common.repos.llm_credentials.as_ref();
    let security = resources.common.repos.security.as_ref();

    let (user_id, _u, tenant_id) = create_test_user_with_plan(
        &resources.agent.database,
        "byo-chain@test.com",
        "professional",
    )
    .await
    .unwrap();
    TenantLlmManager::store_credentials(
        None,
        tenant_id,
        StoreLlmCredentialsRequest {
            provider: LlmProvider::Gemini,
            api_key: SecretString::from("test-gemini-key"),
            base_url: None,
            default_model: Some(BYO_MODEL.to_owned()),
        },
        user_id,
        llm_creds,
        security,
    )
    .await
    .unwrap();

    // A fresh cache per chain: the cache keys on (tenant, user) alone, and a
    // process only ever runs one chain.
    let copilot_led = LlmChain::parse("copilot_sdk=claude-haiku-4.5,gemini");
    let under_copilot = resolve_tenant_chat_provider(
        &TenantChatProviderCache::new(),
        &copilot_led,
        llm_creds,
        security,
        tenant_id,
        user_id,
    )
    .await;
    assert!(
        under_copilot.is_none(),
        "a copilot_sdk head takes no tenant key, so the stored Gemini key must not apply"
    );

    let gemini_led = LlmChain::parse("gemini=gemini-2.5-pro,copilot_sdk");
    let under_gemini = resolve_tenant_chat_provider(
        &TenantChatProviderCache::new(),
        &gemini_led,
        llm_creds,
        security,
        tenant_id,
        user_id,
    )
    .await
    .expect("a gemini head makes the stored Gemini key the tenant's provider");
    assert_eq!(under_gemini.name(), "gemini");
    assert_eq!(
        under_gemini.default_model(),
        BYO_MODEL,
        "the tenant's provider runs the model its key names, not the chain head's"
    );
}
