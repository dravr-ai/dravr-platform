// ABOUTME: Resolves a per-tenant ChatProvider from stored BYO LLM credentials so
// ABOUTME: production chat traffic uses a tenant's own key, with a short TTL cache.

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Per-tenant chat-provider resolution.
//!
//! The process holds a single env-configured [`ChatProvider`] singleton. Tenants
//! can also store their own API key for the system's active provider type
//! ("bring your own key"). This module resolves, for a `(tenant, user)`, whether
//! such a stored credential exists and builds a [`ChatProvider`] from it; the
//! chat dispatch layer swaps it into the turn context so the tenant's key drives
//! their traffic. A short TTL cache keeps the common (no-BYO) path off the
//! database on every turn while letting a settings change take effect promptly.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use uuid::Uuid;

use pierre_auth::tenant::llm_manager::{CredentialSource, LlmProvider, TenantLlmManager};
use pierre_core::models::TenantId;
use pierre_database::database::repositories::{LlmCredentialRepository, SecurityRepository};
use pierre_llm::config::LlmChain;
use pierre_llm::ChatProvider;

use crate::chat_provider_factory::{builds_from_credentials, chat_provider_from_credentials};

/// How long a resolved (tenant, user) provider entry stays valid before the
/// next turn re-resolves it. Bounds the staleness of a credential change.
const CACHE_TTL: Duration = Duration::from_mins(1);

struct CachedProvider {
    /// `None` means "this tenant/user has no BYO credential — use the global
    /// singleton". `Some` is the tenant's own provider.
    provider: Option<Arc<ChatProvider>>,
    cached_at: Instant,
}

/// In-memory cache of resolved per-(tenant, user) chat providers.
#[derive(Clone, Default)]
pub struct TenantChatProviderCache {
    inner: Arc<RwLock<HashMap<(TenantId, Uuid), CachedProvider>>>,
}

/// Result of a cache lookup: a fresh entry (carrying the resolved provider, or
/// `None` for "no BYO — use the singleton"), or a miss requiring re-resolution.
enum Lookup {
    Fresh(Option<Arc<ChatProvider>>),
    Miss,
}

impl TenantChatProviderCache {
    /// Create an empty cache.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn get_fresh(&self, key: (TenantId, Uuid)) -> Lookup {
        let Ok(guard) = self.inner.read() else {
            return Lookup::Miss;
        };
        match guard.get(&key) {
            Some(entry) if entry.cached_at.elapsed() < CACHE_TTL => {
                Lookup::Fresh(entry.provider.clone())
            }
            _ => Lookup::Miss,
        }
    }

    fn store(&self, key: (TenantId, Uuid), provider: Option<Arc<ChatProvider>>) {
        if let Ok(mut guard) = self.inner.write() {
            guard.insert(
                key,
                CachedProvider {
                    provider,
                    cached_at: Instant::now(),
                },
            );
        }
    }
}

/// Resolve the chat provider a `(tenant, user)` should use, or `None` to fall
/// back to the global env singleton.
///
/// BYO only applies when the system's active provider type — the head of
/// `llm_chain`, the chain the server read at boot — has stored user-specific
/// or tenant-level credentials; environment/admin-override sources resolve to
/// `None` (the singleton already reflects them).
pub async fn resolve_tenant_chat_provider(
    cache: &TenantChatProviderCache,
    llm_chain: &LlmChain,
    llm_creds: &dyn LlmCredentialRepository,
    security: &dyn SecurityRepository,
    tenant_id: TenantId,
    user_id: Uuid,
) -> Option<Arc<ChatProvider>> {
    let key = (tenant_id, user_id);
    if let Lookup::Fresh(cached) = cache.get_fresh(key) {
        return cached;
    }

    let resolved = resolve_uncached(llm_chain, llm_creds, security, tenant_id, user_id).await;
    cache.store(key, resolved.clone());
    resolved
}

async fn resolve_uncached(
    llm_chain: &LlmChain,
    llm_creds: &dyn LlmCredentialRepository,
    security: &dyn SecurityRepository,
    tenant_id: TenantId,
    user_id: Uuid,
) -> Option<Arc<ChatProvider>> {
    // BYO targets the provider type at the head of the system's chain
    // (PIERRE_LLM_CHAIN). CLI/subprocess runners and the Copilot runtimes don't
    // map to a tenant LlmProvider and so have no BYO. An `anthropic` head does
    // map, but no provider is built from a tenant's Anthropic key — that head
    // runs on the platform's own pooled organizations — so a stored one is
    // never looked up.
    let provider = LlmProvider::parse_str(&llm_chain.head().provider.to_string())
        .filter(|provider| builds_from_credentials(*provider))?;

    let creds =
        TenantLlmManager::get_credentials(Some(user_id), tenant_id, provider, llm_creds, security)
            .await
            .ok()?;

    // Only genuine stored credentials warrant a per-tenant provider; the env /
    // admin-override fallbacks are already served by the global singleton.
    if !matches!(
        creds.source,
        CredentialSource::UserSpecific | CredentialSource::TenantDefault
    ) {
        return None;
    }

    chat_provider_from_credentials(creds).ok().map(Arc::new)
}
