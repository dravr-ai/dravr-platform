// ABOUTME: Aggregate health of the claim detector — flagged verdicts and their dispositions by layer, category, agent, reason and day
// ABOUTME: A known mix yields the exact false-positive rate and breakdowns; the handler clamps its window and is gated
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, clippy::float_cmp)]

mod common;

use std::sync::Arc;

use anyhow::Result;
use axum::body::to_bytes;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Extension;
use chrono::Utc;
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_core::admin::models::{AdminPermission, AdminPermissions, ValidatedAdminToken};
use pierre_core::errors::ErrorCode;
use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_database::repositories::{
    InsertClaimVerdictParams, SetVerdictDispositionParams, VerdictHealthTotals,
};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_memory::claims::{
    ClaimCategory, ClaimStatus, DispositionReason, EvidenceStrength, VerdictDisposition,
    VerdictLayer,
};
use pierre_routes_admin::auth::service::AdminAuthService;
use pierre_routes_admin::handlers::claim_verdicts::{handle_verdict_health, VerdictHealthQuery};
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit};
use pierre_tool_runtime::guardian::GuardianConfigRegistry;
use serde_json::Value;
use uuid::Uuid;

fn tenant() -> TenantId {
    TenantId::from_uuid(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap())
}

fn other_tenant() -> TenantId {
    TenantId::from_uuid(Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap())
}

/// A tenant row and its owner, so the agents table's foreign keys hold.
async fn seed_tenant_owner(repos: &RepositoryRegistry, tenant_id: TenantId) -> Uuid {
    let now = Utc::now();
    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap();
    let mut user = User::new(
        format!("owner+{tenant_id}@example.com"),
        password_hash,
        None,
    );
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(now);
    let user_id = user.id;
    repos.users.create(&user).await.unwrap();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Tenant {tenant_id}"),
        slug: format!("tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: now,
        updated_at: now,
    };
    repos.tenants.create(&tenant).await.unwrap();
    repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();
    user_id
}

/// A system agent the verdict can name: `claim_verdicts.agent_id` references
/// the agents table, so a made-up id fails the insert.
async fn seed_agent(
    repos: &RepositoryRegistry,
    owner: Uuid,
    tenant_id: TenantId,
    title: &str,
) -> String {
    repos
        .agents
        .create_system_agent(
            owner,
            tenant_id,
            &CreateSystemAgentRequest {
                title: title.to_owned(),
                description: None,
                system_prompt: "You are a coach.".to_owned(),
                category: AgentCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                visibility: AgentVisibility::Tenant,
            },
        )
        .await
        .unwrap()
        .id
        .to_string()
}

async fn insert(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    agent_id: Option<&str>,
    category: ClaimCategory,
    status: ClaimStatus,
    layer: VerdictLayer,
) -> String {
    let params = InsertClaimVerdictParams {
        tenant_id,
        user_id: "user-1",
        agent_id,
        conversation_id: None,
        message_id: None,
        claim_text: "claim",
        category,
        status,
        evidence_strength: EvidenceStrength::None,
        confidence: 0.5,
        layer_fired: layer,
        explanation: None,
        evidence_refs: None,
    };
    repos
        .claim_verdicts
        .insert_claim_verdict(&params)
        .await
        .unwrap()
        .id
}

async fn dispose(
    repos: &RepositoryRegistry,
    tenant_id: TenantId,
    id: &str,
    disposition: VerdictDisposition,
    reason: Option<DispositionReason>,
) {
    repos
        .claim_verdicts
        .set_verdict_disposition(&SetVerdictDispositionParams {
            tenant_id,
            verdict_id: id,
            disposition,
            reason,
            note: None,
            disposed_by: "support@example.com",
            disposed_at: Utc::now(),
        })
        .await
        .unwrap();
}

/// The mix every test below reads: six flagged verdicts across two layers
/// (four evidence, two deterministic), two agents, two categories; four
/// disposed — three false positives (two `missing_keyword`, one
/// `bound_too_tight`) and one true catch — plus one supported row and one
/// row in another tenant, neither of which may count.
async fn seed_mix(repos: &RepositoryRegistry) -> (String, String) {
    let owner = seed_tenant_owner(repos, tenant()).await;
    let agent_a = seed_agent(repos, owner, tenant(), "Agent A").await;
    let agent_b = seed_agent(repos, owner, tenant(), "Agent B").await;

    let e1 = insert(
        repos,
        tenant(),
        Some(&agent_a),
        ClaimCategory::Nutrition,
        ClaimStatus::Unsupported,
        VerdictLayer::Evidence,
    )
    .await;
    let e2 = insert(
        repos,
        tenant(),
        Some(&agent_a),
        ClaimCategory::Nutrition,
        ClaimStatus::Unsupported,
        VerdictLayer::Evidence,
    )
    .await;
    let e3 = insert(
        repos,
        tenant(),
        Some(&agent_b),
        ClaimCategory::Supplement,
        ClaimStatus::Unsupported,
        VerdictLayer::Evidence,
    )
    .await;
    let _e4 = insert(
        repos,
        tenant(),
        None,
        ClaimCategory::Supplement,
        ClaimStatus::Unsupported,
        VerdictLayer::Evidence,
    )
    .await;
    let d1 = insert(
        repos,
        tenant(),
        Some(&agent_b),
        ClaimCategory::Physiological,
        ClaimStatus::Contradicted,
        VerdictLayer::Deterministic,
    )
    .await;
    let _d2 = insert(
        repos,
        tenant(),
        Some(&agent_b),
        ClaimCategory::Physiological,
        ClaimStatus::Contradicted,
        VerdictLayer::Deterministic,
    )
    .await;
    // Not flagged: a supported row, disposed or not, never counts.
    let supported = insert(
        repos,
        tenant(),
        Some(&agent_a),
        ClaimCategory::Nutrition,
        ClaimStatus::Supported,
        VerdictLayer::Evidence,
    )
    .await;
    // Another tenant's flagged, disposed row never counts either.
    let foreign = insert(
        repos,
        other_tenant(),
        None,
        ClaimCategory::Nutrition,
        ClaimStatus::Unsupported,
        VerdictLayer::Evidence,
    )
    .await;

    dispose(
        repos,
        tenant(),
        &e1,
        VerdictDisposition::FalsePositive,
        Some(DispositionReason::MissingKeyword),
    )
    .await;
    dispose(
        repos,
        tenant(),
        &e2,
        VerdictDisposition::FalsePositive,
        Some(DispositionReason::MissingKeyword),
    )
    .await;
    dispose(repos, tenant(), &e3, VerdictDisposition::TrueCatch, None).await;
    dispose(
        repos,
        tenant(),
        &d1,
        VerdictDisposition::FalsePositive,
        Some(DispositionReason::BoundTooTight),
    )
    .await;
    dispose(
        repos,
        tenant(),
        &supported,
        VerdictDisposition::FalsePositive,
        Some(DispositionReason::Other),
    )
    .await;
    dispose(
        repos,
        other_tenant(),
        &foreign,
        VerdictDisposition::FalsePositive,
        Some(DispositionReason::JudgeError),
    )
    .await;

    (agent_a, agent_b)
}

#[test]
fn false_positive_rate_guards_the_division() {
    let nothing = VerdictHealthTotals::default();
    assert_eq!(nothing.false_positive_rate(), 0.0);
    let undisposed = VerdictHealthTotals {
        flagged: 9,
        ..VerdictHealthTotals::default()
    };
    assert_eq!(undisposed.false_positive_rate(), 0.0);
    let some = VerdictHealthTotals {
        flagged: 9,
        disposed: 4,
        true_catches: 1,
        false_positives: 3,
        unsure: 0,
    };
    assert_eq!(some.false_positive_rate(), 0.75);
}

#[tokio::test]
async fn health_rolls_a_known_mix_into_exact_totals_and_breakdowns() -> Result<()> {
    let db = common::create_test_database().await?;
    let repos = db.repositories();
    let (agent_a, agent_b) = seed_mix(&repos).await;

    let stats = repos
        .claim_verdicts
        .aggregate_verdict_health(tenant(), 30)
        .await?;

    assert_eq!(stats.window_days, 30);
    assert_eq!(
        stats.totals,
        VerdictHealthTotals {
            flagged: 6,
            disposed: 4,
            true_catches: 1,
            false_positives: 3,
            unsure: 0,
        }
    );
    assert_eq!(stats.false_positive_rate, 0.75);

    // By layer, sorted by name.
    assert_eq!(stats.by_layer.len(), 2, "{:?}", stats.by_layer);
    let deterministic = &stats.by_layer[0];
    assert_eq!(deterministic.layer, "deterministic");
    assert_eq!(deterministic.flagged, 2);
    assert_eq!(deterministic.disposed, 1);
    assert_eq!(deterministic.false_positives, 1);
    assert_eq!(deterministic.rate, 1.0);
    let evidence = &stats.by_layer[1];
    assert_eq!(evidence.layer, "evidence");
    assert_eq!(evidence.flagged, 4);
    assert_eq!(evidence.disposed, 3);
    assert_eq!(evidence.false_positives, 2);
    assert_eq!(evidence.rate, 2.0 / 3.0);

    // By category, sorted by name.
    let categories: Vec<(&str, i64, i64)> = stats
        .by_category
        .iter()
        .map(|c| (c.category.as_str(), c.flagged, c.false_positives))
        .collect();
    assert_eq!(
        categories,
        vec![
            ("nutrition", 2, 2),
            ("physiological", 2, 1),
            ("supplement", 2, 0)
        ]
    );

    // By agent: the unattributed group first, then by id.
    let mut expected_agents = vec![(Some(agent_a.clone()), 2, 2), (Some(agent_b.clone()), 3, 1)];
    expected_agents.sort();
    let mut expected = vec![(None, 1, 0)];
    expected.extend(expected_agents);
    let agents: Vec<(Option<String>, i64, i64)> = stats
        .by_agent
        .iter()
        .map(|a| (a.agent_id.clone(), a.flagged, a.false_positives))
        .collect();
    assert_eq!(agents, expected);
    let unattributed = &stats.by_agent[0];
    assert_eq!(unattributed.disposed, 0);
    assert_eq!(unattributed.rate, 0.0, "nothing disposed divides to zero");

    // By reason: the supported row's `other` and the foreign tenant's
    // `judge_error` are absent.
    let reasons: Vec<(&str, i64)> = stats
        .by_reason
        .iter()
        .map(|r| (r.reason.as_str(), r.count))
        .collect();
    assert_eq!(
        reasons,
        vec![("bound_too_tight", 1), ("missing_keyword", 2)]
    );

    // Daily: everything landed now, so one bucket carrying the whole window.
    assert_eq!(stats.daily.len(), 1, "{:?}", stats.daily);
    assert_eq!(
        stats.daily[0].date,
        Utc::now().format("%Y-%m-%d").to_string()
    );
    assert_eq!(stats.daily[0].flagged, 6);
    assert_eq!(stats.daily[0].false_positives, 3);
    Ok(())
}

#[tokio::test]
async fn health_of_an_empty_window_is_all_zeros() -> Result<()> {
    let db = common::create_test_database().await?;
    let repos = db.repositories();
    // A supported row alone: not flagged, so the window is empty.
    insert(
        &repos,
        tenant(),
        None,
        ClaimCategory::Recovery,
        ClaimStatus::Supported,
        VerdictLayer::Evidence,
    )
    .await;

    let stats = repos
        .claim_verdicts
        .aggregate_verdict_health(tenant(), 7)
        .await?;
    assert_eq!(stats.totals, VerdictHealthTotals::default());
    assert_eq!(stats.false_positive_rate, 0.0);
    assert!(stats.by_layer.is_empty());
    assert!(stats.by_category.is_empty());
    assert!(stats.by_agent.is_empty());
    assert!(stats.by_reason.is_empty());
    assert!(stats.daily.is_empty());
    Ok(())
}

#[tokio::test]
async fn health_clamps_the_window() -> Result<()> {
    let db = common::create_test_database().await?;
    let repos = db.repositories();
    let low = repos
        .claim_verdicts
        .aggregate_verdict_health(tenant(), 0)
        .await?;
    assert_eq!(low.window_days, 1);
    let high = repos
        .claim_verdicts
        .aggregate_verdict_health(tenant(), 10_000)
        .await?;
    assert_eq!(high.window_days, 365);
    Ok(())
}

// ============================================================================
// Handler
// ============================================================================

async fn build_context() -> (Arc<AdminApiContext>, Arc<RepositoryRegistry>) {
    let database = common::create_test_database().await.unwrap();
    let auth_manager = common::create_test_auth_manager();
    let jwks_manager = common::get_shared_test_jwks();

    let database_arc = Arc::new((*database).clone());
    let repos_arc = Arc::new(database_arc.repositories());

    let context = AdminApiContext::new(AdminApiContextInit {
        database: database_arc,
        repos: repos_arc.clone(),
        jwt_secret: "test_admin_jwt_secret_for_verdict_health".to_owned(),
        auth_manager,
        jwks_manager,
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        admin_token_cache_ttl_secs: AdminAuthService::DEFAULT_CACHE_TTL_SECS,
        harness_config_registry: Arc::new(HarnessConfigRegistry::bootstrap()),
        guardian_config_registry: Arc::new(GuardianConfigRegistry::bootstrap()),
        prompt_registry: Arc::new(pierre_contremaitre::PromptRegistry::new()),
        tool_description_registry: Arc::new(pierre_contremaitre::ToolDescriptionRegistry::new()),
        evidence_registry: Arc::new(pierre_contremaitre::EvidenceRegistry::new()),
        messaging_strings_registry: Arc::new(pierre_contremaitre::MessagingStringsRegistry::new()),
        cageux_config_registry: Arc::new(CageuxConfigRegistry::from_env()),
        persona_contract_registry: Arc::new(PersonaContractRegistry::new()),
        training_catalogue_registry: Arc::new(pierre_contremaitre::TrainingCatalogueRegistry::new()),
        contremaitre_config: None,
    });

    (Arc::new(context), repos_arc)
}

fn super_admin_token() -> ValidatedAdminToken {
    ValidatedAdminToken {
        token_id: format!("cookie:{}", Uuid::new_v4()),
        service_name: "admin@example.com".to_owned(),
        permissions: AdminPermissions::super_admin(),
        is_super_admin: true,
        tenant_id: None,
        user_info: None,
    }
}

fn plain_admin_token() -> ValidatedAdminToken {
    ValidatedAdminToken {
        token_id: format!("cookie:{}", Uuid::new_v4()),
        service_name: "plain-admin@example.com".to_owned(),
        permissions: AdminPermissions::default_admin(),
        is_super_admin: false,
        tenant_id: None,
        user_info: None,
    }
}

fn tenant_bound_token(tenant_id: TenantId) -> ValidatedAdminToken {
    ValidatedAdminToken {
        token_id: format!("admin-token:{}", Uuid::new_v4()),
        service_name: "tenant-admin@example.com".to_owned(),
        permissions: AdminPermissions::new(vec![AdminPermission::ViewConfiguration]),
        is_super_admin: false,
        tenant_id: Some(tenant_id.as_uuid().to_string()),
        user_info: None,
    }
}

async fn body_json(resp: impl IntoResponse) -> (StatusCode, Value) {
    let resp = resp.into_response();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn health_route_serves_the_rollup_clamps_the_window_and_is_gated() {
    let (context, repos) = build_context().await;
    seed_mix(&repos).await;

    let err = handle_verdict_health(
        State(context.clone()),
        Extension(plain_admin_token()),
        Query(VerdictHealthQuery {
            tenant_id: tenant().to_string(),
            window_days: None,
        }),
    )
    .await
    .err()
    .expect("a plain admin cannot read");
    assert_eq!(err.code, ErrorCode::PermissionDenied);
    assert_eq!(err.into_response().status(), StatusCode::FORBIDDEN);

    let err = handle_verdict_health(
        State(context.clone()),
        Extension(tenant_bound_token(other_tenant())),
        Query(VerdictHealthQuery {
            tenant_id: tenant().to_string(),
            window_days: None,
        }),
    )
    .await
    .err()
    .expect("a foreign tenant cannot read");
    assert_eq!(err.code, ErrorCode::PermissionDenied);

    let resp = handle_verdict_health(
        State(context.clone()),
        Extension(tenant_bound_token(tenant())),
        Query(VerdictHealthQuery {
            tenant_id: tenant().to_string(),
            window_days: Some(9_999),
        }),
    )
    .await
    .expect("own tenant reads");
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["window_days"], 365, "clamped");
    assert_eq!(json["totals"]["flagged"], 6);
    assert_eq!(json["totals"]["disposed"], 4);
    assert_eq!(json["totals"]["false_positives"], 3);
    assert_eq!(json["false_positive_rate"], 0.75);
    assert_eq!(json["by_layer"][0]["layer"], "deterministic");
    assert_eq!(json["by_layer"][1]["layer"], "evidence");
    assert_eq!(json["by_layer"][1]["false_positives"], 2);
    assert_eq!(json["by_reason"][1]["reason"], "missing_keyword");
    assert_eq!(json["by_reason"][1]["count"], 2);
    assert_eq!(json["daily"][0]["flagged"], 6);

    let resp = handle_verdict_health(
        State(context),
        Extension(super_admin_token()),
        Query(VerdictHealthQuery {
            tenant_id: tenant().to_string(),
            window_days: Some(0),
        }),
    )
    .await
    .expect("super-admin reads any tenant");
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["window_days"], 1, "clamped up to one day");
    assert_eq!(json["totals"]["flagged"], 6);
}
