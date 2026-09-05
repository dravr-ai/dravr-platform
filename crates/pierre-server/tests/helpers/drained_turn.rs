// ABOUTME: Fixtures for the tests that interrupt a live messaging turn — providers that hang, a WhatsApp athlete, the outbound ledger
// ABOUTME: Shared by the drain (resume) and watchdog suites so both drive the same webhook route through the same stored rows
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
#![allow(dead_code)]

//! What an interrupted turn is made of.
//!
//! The 2026-08-26 incident (registre#109, registre#126) is a turn parked on an
//! LLM call that never returns while the instance under it goes away. Every
//! test of that shape needs the same four things: a provider that hangs, an
//! Active athlete with a synthetic provider so the turn clears the status and
//! onboarding gates, a `WhatsApp` channel the webhook route accepts, and a way
//! to read what the athlete's channel received. They live here so the drain
//! suite and the watchdog suite — separate binaries, because the watchdog
//! ceiling is process-wide configuration — assert against one fixture.
//!
//! `WhatsApp` on purpose: it has no status placeholder, so a notice or a reply
//! is a plain outbound row in the session ledger, readable without a mock of
//! a channel API.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use futures_util::stream;
use hmac::{Hmac, Mac};
use pierre_core::errors::AppError;
use pierre_core::llm::{
    ChatRequest, ChatResponse, ChatStream, LlmCapabilities, LlmProvider, StreamChunk,
};
use pierre_core::models::{ConnectionType, Tenant, TenantId, User, UserStatus};
use pierre_database::backends::{
    CreateChannelLinkParams, MessagingRepository, UpsertChannelConfigParams,
};
use pierre_mcp_server::mcp::resources::ServerContext;
use serde_json::json;
use sha2::Sha256;
use tokio::task::spawn_blocking;
use tokio::time::{sleep, Instant};
use uuid::Uuid;

/// The `WhatsApp` business account id the fixture channel config carries.
pub const WA_BUSINESS_ID: &str = "wa_drain_test_business_id";
/// The `WhatsApp` phone number id the fixture channel config carries.
pub const WA_PHONE_NUMBER_ID: &str = "15550000003";

/// Far longer than any test's drain or watchdog budget: a turn parked on this
/// must be ended by the interruption, never by the provider returning.
const NEVER: Duration = Duration::from_mins(10);

/// A provider that never answers, standing in for the ACP session the
/// 2026-08-26 turn was parked on when its instance was drained.
pub struct HangingProvider;

#[async_trait]
impl LlmProvider for HangingProvider {
    fn name(&self) -> &'static str {
        "hanging_mock"
    }
    fn display_name(&self) -> &'static str {
        "Hanging Mock LLM (turn-lifecycle e2e)"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "mock-model"
    }
    fn available_models(&self) -> &[String] {
        &[]
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        sleep(NEVER).await;
        Err(AppError::internal("hanging provider must never answer"))
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        sleep(NEVER).await;
        let chunk = StreamChunk {
            delta: String::new(),
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        };
        Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// A provider that hangs every call until the test releases it, then answers.
///
/// The shape of a turn drained on one instance and resumed on another: the
/// run the webhook started parks on this (and is cancelled by the drain);
/// once the test has drained that instance it calls [`Self::release`], and
/// the run the next instance's sweeper starts gets the real answer. Releasing
/// is a test step rather than a call count because on a slow database the
/// drain can land before the first run ever reaches the provider, and a
/// count-based hang would then park the resumed run instead.
pub struct ParkedProvider {
    released: AtomicBool,
    calls: AtomicUsize,
    answer: String,
}

impl ParkedProvider {
    /// The reply every call made after [`Self::release`] returns.
    pub fn answering(answer: &str) -> Self {
        Self {
            released: AtomicBool::new(false),
            calls: AtomicUsize::new(0),
            answer: answer.to_owned(),
        }
    }

    /// Let every call from here on answer: the instance the turn was parked
    /// on is gone, and the next instance's provider is a working one.
    pub fn release(&self) {
        self.released.store(true, Ordering::SeqCst);
    }

    /// How many LLM calls the provider has received so far.
    pub fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }

    fn parked(&self) -> bool {
        self.calls.fetch_add(1, Ordering::SeqCst);
        !self.released.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl LlmProvider for ParkedProvider {
    fn name(&self) -> &'static str {
        "parked_mock"
    }
    fn display_name(&self) -> &'static str {
        "Parked Mock LLM (turn-resume e2e)"
    }
    fn capabilities(&self) -> LlmCapabilities {
        LlmCapabilities::SYSTEM_MESSAGES
    }
    fn default_model(&self) -> &'static str {
        "mock-model"
    }
    fn available_models(&self) -> &[String] {
        &[]
    }

    async fn complete(&self, _request: &ChatRequest) -> Result<ChatResponse, AppError> {
        if self.parked() {
            sleep(NEVER).await;
            return Err(AppError::internal(
                "a parked call must be cancelled by the drain",
            ));
        }
        Ok(ChatResponse {
            content: self.answer.clone(),
            model: "mock-model".to_owned(),
            usage: None,
            finish_reason: Some("stop".to_owned()),
            warnings: None,
            tool_calls: None,
        })
    }

    async fn complete_stream(&self, _request: &ChatRequest) -> Result<ChatStream, AppError> {
        if self.parked() {
            sleep(NEVER).await;
            return Err(AppError::internal(
                "a parked call must be cancelled by the drain",
            ));
        }
        let chunk = StreamChunk {
            delta: self.answer.clone(),
            is_final: true,
            finish_reason: Some("stop".to_owned()),
        };
        Ok(Box::pin(stream::iter(vec![Ok(chunk)])))
    }

    async fn health_check(&self) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// Meta's webhook signature for `body` under `secret`.
pub fn compute_whatsapp_sig(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(body);
    format!("sha256={}", hex::encode(mac.finalize().into_bytes()))
}

/// An Active user with a tenant and a synthetic provider, so the turn
/// clears the status and onboarding gates and reaches the pipeline.
pub async fn create_active_user(resources: &ServerContext, email: &str) -> (Uuid, TenantId) {
    let password_hash =
        spawn_blocking(|| bcrypt::hash("DrainPin123!", bcrypt::DEFAULT_COST).unwrap())
            .await
            .unwrap();

    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Drain User".to_owned()),
    );
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(Utc::now());

    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Drain Tenant {email}"),
        slug: format!("drain-tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    resources
        .common
        .repos
        .tenants
        .create(&tenant)
        .await
        .unwrap();

    resources
        .common
        .repos
        .provider_connections
        .register_connection(
            user_id,
            tenant_id,
            "synthetic",
            &ConnectionType::Synthetic,
            None,
        )
        .await
        .unwrap();

    (user_id, tenant_id)
}

/// Store the `WhatsApp` channel config the webhook route verifies against.
pub async fn setup_whatsapp_config(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    secret: &str,
) {
    let config_id = Uuid::new_v4().to_string();
    db.upsert_channel_config(&UpsertChannelConfigParams {
        id: &config_id,
        tenant_id,
        channel_type: "whatsapp",
        api_key: Some("wa_drain_test_token"),
        api_secret: None,
        webhook_secret: Some(secret),
        verify_token: None,
        account_id: Some(WA_BUSINESS_ID),
        phone_number: Some(WA_PHONE_NUMBER_ID),
        bot_token: None,
        is_active: true,
    })
    .await
    .unwrap();
}

/// Link `sender_id` on `WhatsApp` to the athlete, so the webhook resolves a
/// session and dispatches a turn instead of prompting to link.
pub async fn link_channel(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    user_id: Uuid,
    sender_id: &str,
) {
    let link_id = Uuid::new_v4().to_string();
    let user_id_str = user_id.to_string();
    db.create_channel_link(&CreateChannelLinkParams {
        id: &link_id,
        tenant_id,
        user_id: &user_id_str,
        channel_type: "whatsapp",
        channel_user_id: sender_id,
        display_name: Some("Drain Linked User"),
    })
    .await
    .unwrap();
}

/// One inbound `WhatsApp` text message in Meta's webhook shape.
pub fn whatsapp_text_payload(sender_id: &str, msg_id: &str, text: &str) -> serde_json::Value {
    json!({
        "object": "whatsapp_business_account",
        "entry": [{
            "id": WA_BUSINESS_ID,
            "changes": [{
                "value": {
                    "messaging_product": "whatsapp",
                    "metadata": {
                        "display_phone_number": "+15550000003",
                        "phone_number_id": WA_PHONE_NUMBER_ID
                    },
                    "messages": [{
                        "from": sender_id,
                        "id": msg_id,
                        "timestamp": "1234567890",
                        "type": "text",
                        "text": { "body": text }
                    }]
                },
                "field": "messages"
            }]
        }]
    })
}

/// Outbound bodies stored for the sender's session, oldest first. A reply
/// whose live send fails in tests is still persisted alongside its
/// retry-queue entry, so a notice or an answer is observable here either way.
pub async fn outbound_bodies(
    db: &dyn MessagingRepository,
    tenant_id: TenantId,
    sender_id: &str,
) -> Vec<String> {
    let Ok(Some(session)) = db
        .get_session_by_channel_identity(tenant_id, "whatsapp", sender_id, None)
        .await
    else {
        return Vec::new();
    };
    let Some(session_id) = session["id"].as_str() else {
        return Vec::new();
    };
    db.get_session_messages(session_id, tenant_id, 100, 0)
        .await
        .map(|rows| {
            rows.into_iter()
                .filter(|r| r["direction"].as_str() == Some("outbound"))
                .filter_map(|r| r["content_body"].as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Wait until the in-flight tracker sees a turn, or give up after ten seconds.
pub async fn wait_for_a_tracked_turn(resources: &ServerContext) -> bool {
    for _ in 0..100 {
        if !resources.common.turns.is_empty() {
            return true;
        }
        sleep(Duration::from_millis(100)).await;
    }
    false
}

/// Wait until every tracked turn has finished, or give up after `budget`.
pub async fn wait_for_turns_to_finish(resources: &ServerContext, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    while Instant::now() < deadline {
        if resources.common.turns.is_empty() {
            return true;
        }
        sleep(Duration::from_millis(100)).await;
    }
    resources.common.turns.is_empty()
}
