// ABOUTME: Pins the hand-written Debug impls of the server's contexts, which hide the admin JWT secret
// ABOUTME: Covers contexts built over a test server: pipeline, auth, admin, command, tool-loop and messaging
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Every context below is built over a real test server with a distinctive
//! sentinel in each hidden field, formatted with `{:?}` and `{:#?}`, and
//! checked for the sentinel. A future `#[derive(Debug)]` on any of them, or a
//! field added to one of these impls without redaction, fails here.

use std::fmt::Debug;
use std::sync::Arc;

use chrono::Utc;
use pierre_chat_pipeline::stages::auth_recovery::AuthRecoveryDeps;
use pierre_chat_pipeline::stages::capability_recovery::CapabilityRecoveryDeps;
use pierre_chat_pipeline::stages::prefetch::ActivityRefreshInputs;
use pierre_chat_pipeline::ChatPipelineContext;
use pierre_commands::dispatch::DispatchRequest;
use pierre_commands::{ConversationRotation, PlatformCommandContext};
use pierre_config::mcp::AppBehaviorConfig;
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_contremaitre::{
    ContremaitreConfig, EvidenceRegistry, MessagingStringsRegistry, PromptRegistry,
    ToolDescriptionRegistry, TrainingCatalogueRegistry,
};
use pierre_core::models::messaging::{ChannelConfig, ChannelType, MessageContent};
use pierre_core::models::{ConversationTurnId, MessageRecord, TenantId, TurnOrigin};
use pierre_core::transport::TransportPolicy;
use pierre_llm::call_record::LlmCallRecorder;
use pierre_llm::{
    ChatMessage, ChatProvider, FunctionDeclaration, McpHeader, McpServerConfig, McpTransport, Tool,
};
use pierre_mcp_server::context::AuthContext;
use pierre_mcp_server::mcp::resources::{AuthSlice, ServerContext};
use pierre_mcp_server::services::backfill_notifier::ServerBackfillNotifier;
use pierre_mcp_server::services::messaging_ingress::block_render::RenderedReply;
use pierre_mcp_server::services::messaging_ingress::ChannelConfigLookup;
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit};
use pierre_routes_auth::AuthRoutesContext;
use pierre_runtime_context::CommandCtx;
use pierre_services::conversation_compaction::CompactionContext;
use pierre_services::llm_usage_recorder::{LlmUsageSink, UsageRepoCallRecorder};
use pierre_tool_runtime::coach_seat::TurnSeat;
use pierre_tool_runtime::guardian::GuardianConfigRegistry;
use pierre_tool_runtime::protocol::UniversalExecutor;
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::tool_loop_io::ToolLoopParams;
use tokio_util::task::TaskTracker;
use uuid::Uuid;

use crate::common::{create_test_server_resources, TestLlmProvider};
use crate::messaging_fixtures::{strings, CapturingChannel, FakeResolver};

/// The admin JWT secret every context below is given.
const ADMIN_SECRET: &str = "sentinel-admin-jwt-7f3a";

/// The secret [`create_test_server_resources`] signs admin tokens with.
const TEST_SERVER_SECRET: &str = "test_admin_secret";

/// Formats `value` both ways, asserts none of `secrets` appears in either,
/// and returns the compact form for further checks.
fn printed_without(value: &impl Debug, secrets: &[&str]) -> String {
    let compact = format!("{value:?}");
    let pretty = format!("{value:#?}");
    for secret in secrets {
        assert!(!compact.contains(secret), "{secret} leaked: {compact}");
        assert!(!pretty.contains(secret), "{secret} leaked: {pretty}");
    }
    compact
}

/// A channel config carrying a sentinel in every credential it holds.
fn sentinel_channel_config() -> ChannelConfig {
    ChannelConfig {
        id: "cfg-1".to_owned(),
        tenant_id: "tenant-1".to_owned(),
        channel_type: ChannelType::Telegram,
        api_key: Some("sentinel-api-key-7f3a".to_owned()),
        api_secret: Some("sentinel-api-secret-7f3a".to_owned()),
        webhook_secret: Some("sentinel-webhook-7f3a".to_owned()),
        verify_token: Some("sentinel-verify-7f3a".to_owned()),
        account_id: Some("account-1".to_owned()),
        phone_number: Some("+15550007f3a".to_owned()),
        bot_token: Some("sentinel-bot-token-7f3a".to_owned()),
        is_active: true,
    }
}

/// Every secret [`sentinel_channel_config`] holds.
const CHANNEL_SECRETS: [&str; 6] = [
    "sentinel-api-key-7f3a",
    "sentinel-api-secret-7f3a",
    "sentinel-webhook-7f3a",
    "sentinel-verify-7f3a",
    "+15550007f3a",
    "sentinel-bot-token-7f3a",
];

/// The pipeline context over `resources`, holding [`ADMIN_SECRET`].
fn pipeline_context(resources: &Arc<ServerContext>) -> ChatPipelineContext {
    ChatPipelineContext {
        admin_jwt_secret: Arc::from(ADMIN_SECRET),
        ..resources.chat_pipeline_context()
    }
}

#[tokio::test]
async fn the_server_context_and_its_auth_views_never_print_the_admin_secret() {
    let resources = create_test_server_resources().await.unwrap();

    let printed = printed_without(&*resources, &[TEST_SERVER_SECRET]);
    assert!(printed.contains("[REDACTED]"), "{printed}");
    printed_without(&resources.auth(), &[TEST_SERVER_SECRET]);

    let slice = AuthSlice {
        admin_jwt_secret: Arc::from(ADMIN_SECRET),
        ..resources.auth.clone()
    };
    let printed = printed_without(&slice, &[ADMIN_SECRET]);
    assert!(
        printed.contains("admin_jwt_secret: \"[REDACTED]\""),
        "{printed}"
    );

    let context = AuthContext::new(
        Arc::clone(&resources.auth.auth_manager),
        Arc::clone(&resources.auth.auth_middleware),
        Arc::from(ADMIN_SECRET),
        Arc::clone(&resources.auth.jwks_manager),
        None,
    );
    let printed = printed_without(&context, &[ADMIN_SECRET]);
    assert!(
        printed.contains("admin_jwt_secret: \"[REDACTED]\""),
        "{printed}"
    );

    let routes = AuthRoutesContext {
        admin_jwt_secret: Arc::from(ADMIN_SECRET),
        ..resources.auth_routes_context()
    };
    let printed = printed_without(&routes, &[ADMIN_SECRET]);
    assert!(
        printed.contains("admin_jwt_secret: \"[REDACTED]\""),
        "{printed}"
    );
}

#[tokio::test]
async fn the_chat_pipeline_context_never_prints_the_admin_secret() {
    let resources = create_test_server_resources().await.unwrap();
    let pipeline = pipeline_context(&resources);
    let printed = printed_without(&pipeline, &[ADMIN_SECRET]);
    assert!(
        printed.contains("admin_jwt_secret: \"[REDACTED]\""),
        "{printed}"
    );
}

#[tokio::test]
async fn the_admin_api_context_prints_neither_its_jwt_secret_nor_the_contremaitre_pat() {
    let resources = create_test_server_resources().await.unwrap();
    let init = AdminApiContextInit {
        database: Arc::clone(&resources.agent.database),
        repos: Arc::clone(&resources.common.repos),
        jwt_secret: ADMIN_SECRET.to_owned(),
        auth_manager: Arc::clone(&resources.auth.auth_manager),
        jwks_manager: Arc::clone(&resources.auth.jwks_manager),
        admin_api_key_monthly_limit: 1000,
        harness_config_registry: Arc::new(HarnessConfigRegistry::bootstrap()),
        guardian_config_registry: Arc::new(GuardianConfigRegistry::bootstrap()),
        prompt_registry: Arc::new(PromptRegistry::new()),
        tool_description_registry: Arc::new(ToolDescriptionRegistry::new()),
        evidence_registry: Arc::new(EvidenceRegistry::new()),
        messaging_strings_registry: Arc::new(MessagingStringsRegistry::new()),
        cageux_config_registry: Arc::new(CageuxConfigRegistry::from_env()),
        persona_contract_registry: Arc::new(PersonaContractRegistry::new()),
        training_catalogue_registry: Arc::new(TrainingCatalogueRegistry::new()),
        contremaitre_config: Some(ContremaitreConfig {
            repo: "dravr-ai/dravr-contremaitre".to_owned(),
            branch: "main".to_owned(),
            github_pat: "github_pat_sentinel-7f3a".to_owned(),
            gcs_bucket: None,
            webhook_secret: "sentinel-webhook-7f3a".to_owned(),
        }),
        app_behavior: AppBehaviorConfig::default(),
    };
    let secrets = [
        ADMIN_SECRET,
        "github_pat_sentinel-7f3a",
        "sentinel-webhook-7f3a",
    ];
    let printed = printed_without(&init, &secrets);
    assert!(printed.contains("jwt_secret: \"[REDACTED]\""), "{printed}");

    let context = AdminApiContext::new(init);
    let printed = printed_without(&context, &secrets);
    assert!(
        printed.contains("admin_jwt_secret: \"[REDACTED]\""),
        "{printed}"
    );
}

#[tokio::test]
async fn a_command_context_never_prints_the_command_line_or_the_sender() {
    let resources = create_test_server_resources().await.unwrap();
    let context = PlatformCommandContext {
        user_id: Uuid::new_v4(),
        tenant_id: TenantId::generate(),
        channel_type: "telegram".to_owned(),
        args: vec!["sentinel-invite-7f3a".to_owned()],
        raw_text: "/join sentinel-invite-7f3a".to_owned(),
        ctx: Arc::<ServerContext>::clone(&resources),
        locale: "en".to_owned(),
        is_direct_message: true,
        ambient_group_fallback: true,
        conversation_id: Some("conv-1".to_owned()),
        conversation_tenant_id: TenantId::generate(),
        sender_id: Some("+15550007f3a".to_owned()),
        rotation: ConversationRotation::default(),
        tool_runtime: Arc::<ServerContext>::clone(&resources),
    };
    let printed = printed_without(
        &context,
        &["sentinel-invite-7f3a", "+15550007f3a", TEST_SERVER_SECRET],
    );
    assert!(printed.contains("raw_text: \"[REDACTED]\""), "{printed}");
    assert!(
        printed.contains("sender_id: Some(\"[REDACTED]\")"),
        "{printed}"
    );
}

#[tokio::test]
async fn a_dispatch_request_prints_the_size_of_the_text_never_the_text_or_sender() {
    let resources = create_test_server_resources().await.unwrap();
    let ctx: Arc<dyn CommandCtx> = Arc::<ServerContext>::clone(&resources);
    let tool_runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(&resources);
    let text = "/confirm sentinel-confirm-7f3a";
    let request = DispatchRequest {
        ctx: &ctx,
        command_registry: resources.common.command_registry.as_ref().unwrap(),
        command_handler_registry: resources.common.command_handler_registry.as_ref().unwrap(),
        user_id: Uuid::new_v4(),
        tenant_id: TenantId::generate(),
        channel_type: "telegram",
        locale: "en",
        is_direct_message: true,
        ambient_group_fallback: true,
        conversation_id: Some("conv-1"),
        conversation_tenant_id: TenantId::generate(),
        sender_id: Some("+15550007f3a"),
        text,
        tool_runtime: &tool_runtime,
        origin: TurnOrigin::Athlete,
    };
    let printed = printed_without(&request, &["sentinel-confirm-7f3a", "+15550007f3a"]);
    assert!(
        printed.contains(&format!("text_len: {}", text.len())),
        "{printed}"
    );
    assert!(
        printed.contains("sender_id: Some(\"[REDACTED]\")"),
        "{printed}"
    );
}

#[tokio::test]
async fn the_backfill_notifier_never_prints_the_link_signing_secret() {
    let resources = create_test_server_resources().await.unwrap();
    let notifier = ServerBackfillNotifier::with_resolver(
        Arc::clone(&resources.common.repos),
        strings(),
        Arc::new(FakeResolver::new(Arc::new(CapturingChannel::default()))),
        Arc::from(ADMIN_SECRET),
        "https://api.dravr.test".to_owned(),
    );
    let printed = printed_without(&notifier, &[ADMIN_SECRET]);
    assert!(
        printed.contains("admin_jwt_secret: \"[REDACTED]\""),
        "{printed}"
    );
}

#[test]
fn a_rendered_reply_prints_how_many_messages_it_holds_never_their_content() {
    let reply = RenderedReply {
        prose: vec!["sentinel-prose-7f3a your load is high".to_owned()],
        attachments: vec![
            MessageContent::Media {
                url: "https://charts.dravr.ai/c?sig=sentinel-chart-7f3a".to_owned(),
                mime_type: "image/png".to_owned(),
                caption: None,
            },
            MessageContent::Text {
                body: "Reconnect: https://api.dravr.ai/r/sentinel-link-7f3a".to_owned(),
            },
        ],
    };
    let printed = printed_without(
        &reply,
        &[
            "sentinel-prose-7f3a",
            "sentinel-chart-7f3a",
            "sentinel-link-7f3a",
        ],
    );
    assert!(printed.contains("prose_messages: 1"), "{printed}");
    assert!(printed.contains("attachments: 2"), "{printed}");
}

#[test]
fn a_loaded_channel_config_never_prints_its_credentials() {
    let lookup = ChannelConfigLookup::Loaded(Box::new(sentinel_channel_config()));
    let printed = printed_without(&lookup, &CHANNEL_SECRETS);
    assert!(printed.contains("cfg-1"), "{printed}");
}

#[tokio::test]
async fn tool_loop_params_print_mcp_server_names_never_their_bearer_headers() {
    let resources = create_test_server_resources().await.unwrap();
    let provider = ChatProvider::Custom(Arc::new(TestLlmProvider::valid()));
    let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(&resources);
    let executor = Arc::new(UniversalExecutor::new(runtime));
    let tools = Tool {
        function_declarations: vec![FunctionDeclaration::new(
            "get_activities".to_owned(),
            "Retrieve the athlete's activities".to_owned(),
            None,
        )],
    };
    let params = ToolLoopParams {
        provider: &provider,
        executor,
        tools: &tools,
        model: "test-model-v1",
        user_id: "user-1",
        tenant_id: TenantId::generate(),
        max_iterations: 3,
        call_recorder: None,
        tool_message_recorder: None,
        temperature: None,
        stream_sink: None,
        mcp_servers: vec![McpServerConfig {
            name: "dravr".to_owned(),
            transport: McpTransport::Http {
                url: "http://127.0.0.1:9/mcp".to_owned(),
                headers: vec![McpHeader {
                    name: "Authorization".to_owned(),
                    value: "Bearer sentinel-session-7f3a".to_owned(),
                }],
            },
        }],
    };
    let printed = printed_without(&params, &["sentinel-session-7f3a", TEST_SERVER_SECRET]);
    assert!(printed.contains("mcp_servers: [\"dravr\"]"), "{printed}");
}

#[tokio::test]
async fn auth_recovery_deps_never_print_the_admin_secret() {
    let resources = create_test_server_resources().await.unwrap();
    let secret: Arc<str> = Arc::from(ADMIN_SECRET);
    let registry = strings();
    let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(&resources);
    let short_links = Arc::clone(&resources.common.repos.short_links);
    let deps = AuthRecoveryDeps {
        admin_jwt_secret: &secret,
        base_url: "https://api.dravr.test",
        messaging_strings_registry: &registry,
        tool_runtime: &runtime,
        short_links: &short_links,
    };
    let printed = printed_without(&deps, &[ADMIN_SECRET]);
    assert!(
        printed.contains("admin_jwt_secret: \"[REDACTED]\""),
        "{printed}"
    );
}

#[tokio::test]
async fn capability_recovery_deps_print_the_size_of_the_transcript_never_its_text() {
    let resources = create_test_server_resources().await.unwrap();
    let pipeline = pipeline_context(&resources);
    let recorder: Arc<dyn LlmCallRecorder> = Arc::new(UsageRepoCallRecorder::new(
        LlmUsageSink::new(
            Arc::clone(&resources.common.repos.llm_usage),
            TaskTracker::new(),
        ),
        "tenant-1".to_owned(),
        "user-1".to_owned(),
        None,
        ConversationTurnId::new(),
        "chat",
    ));
    let messages = [
        ChatMessage::system("sentinel-system-7f3a persona"),
        ChatMessage::user("sentinel-question-7f3a my knee hurts"),
    ];
    let deps = CapabilityRecoveryDeps {
        ctx: &pipeline,
        llm_messages: &messages,
        active_model: "test-model-v1",
        peer_roster: &[],
        peer_blocks: &[],
        locale: "en",
        guided_walk: None,
        seat: TurnSeat::Subject,
        reask_recorder: &recorder,
        verifier_recorder: &recorder,
    };
    let printed = printed_without(
        &deps,
        &[
            "sentinel-system-7f3a",
            "sentinel-question-7f3a",
            ADMIN_SECRET,
        ],
    );
    assert!(printed.contains("llm_messages: 2"), "{printed}");
}

#[tokio::test]
async fn activity_refresh_inputs_print_the_size_of_the_history_never_its_text() {
    let resources = create_test_server_resources().await.unwrap();
    let runtime: Arc<dyn ToolRuntime> = Arc::<ServerContext>::clone(&resources);
    let executor = Arc::new(UniversalExecutor::new(runtime));
    let history = [MessageRecord {
        id: "msg-1".to_owned(),
        conversation_id: "conv-1".to_owned(),
        role: "user".to_owned(),
        content: "sentinel-history-7f3a I slept four hours".to_owned(),
        token_count: None,
        prompt_tokens: None,
        model: None,
        finish_reason: None,
        content_blocks: Some(r#"[{"title":"sentinel-blocks-7f3a"}]"#.to_owned()),
        created_at: Utc::now().to_rfc3339(),
        transport_policy: TransportPolicy::AnyTransport,
        channel_type: Some("web".to_owned()),
        about_user_id: None,
    }];
    let inputs = ActivityRefreshInputs {
        executor: &executor,
        history: &history,
        agent_ctx: None,
        user_id: "user-1",
        tenant_id: TenantId::generate(),
        guided_flow_active: false,
    };
    let printed = printed_without(&inputs, &["sentinel-history-7f3a", "sentinel-blocks-7f3a"]);
    assert!(printed.contains("history_len: 1"), "{printed}");
}

#[tokio::test]
async fn a_compaction_context_prints_the_size_of_the_transcript_never_its_text() {
    let resources = create_test_server_resources().await.unwrap();
    let provider = ChatProvider::Custom(Arc::new(TestLlmProvider::valid()));
    let source_ids = [Some("msg-1".to_owned()), Some("msg-2".to_owned())];
    let mut llm_messages = vec![
        ChatMessage::user("sentinel-question-7f3a my knee hurts"),
        ChatMessage::assistant("sentinel-reply-7f3a rest for two days"),
    ];
    let ctx = CompactionContext {
        repo: resources.common.repos.memory.as_ref(),
        provider: &provider,
        summary_prompt: "Summarize the conversation.",
        recorder: None,
        tenant_id: TenantId::generate(),
        conversation_id: "conv-1",
        source_ids: &source_ids,
        source_members: &vec![None; source_ids.len()],
        llm_messages: &mut llm_messages,
        summary_for_prompt: str::to_owned,
    };
    let printed = printed_without(&ctx, &["sentinel-question-7f3a", "sentinel-reply-7f3a"]);
    assert!(printed.contains("llm_messages: 2"), "{printed}");
}
