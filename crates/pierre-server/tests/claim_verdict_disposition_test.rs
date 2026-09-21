// ABOUTME: Support disposition on claim verdicts — the repository writes, the SQL filters, and the admin routes
// ABOUTME: A disposition lands with all five fields, overwrites, 404s on a stranger, and is gated by ManageConfiguration + tenant
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::sync::Arc;

use anyhow::Result;
use axum::body::to_bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Extension, Json};
use chrono::{TimeZone, Utc};
use helpers::axum_test::AxumTestRequest;
use pierre_contremaitre::cageux_config::CageuxConfigRegistry;
use pierre_contremaitre::evidence_registry::parse_evidence_markdown;
use pierre_contremaitre::harness_config_registry::HarnessConfigRegistry;
use pierre_contremaitre::persona_contracts::PersonaContractRegistry;
use pierre_contremaitre::EvidenceRegistry;
use pierre_core::admin::models::{AdminPermission, AdminPermissions, ValidatedAdminToken};
use pierre_core::errors::ErrorCode;
use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
use pierre_core::models::{Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_database::repositories::{
    DispositionFilter, InsertClaimVerdictParams, SetVerdictDispositionParams, VerdictListFilter,
};
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::claims::{
    ClaimCategory, ClaimStatus, ClaimVerdict, DispositionReason, EvidenceStrength,
    VerdictDisposition, VerdictLayer,
};
use pierre_routes_admin::auth::service::AdminAuthService;
use pierre_routes_admin::handlers::claim_verdicts::{
    handle_get_claim_verdict, handle_list_claim_verdicts, handle_list_verdicts_by_message,
    handle_set_verdict_disposition, knob_for, ListVerdictsQuery, SetDispositionRequest,
    TenantScopedQuery, MAX_DISPOSITION_NOTE_CHARS,
};
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit, AdminRoutes};
use pierre_tool_runtime::guardian::GuardianConfigRegistry;
use serde_json::Value;
use serial_test::serial;
use uuid::Uuid;

fn tenant() -> TenantId {
    TenantId::from_uuid(Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap())
}

fn other_tenant() -> TenantId {
    TenantId::from_uuid(Uuid::parse_str("00000000-0000-0000-0000-000000000002").unwrap())
}

/// One flagged verdict's insert, with the knobs the tests vary.
struct Seed<'a> {
    user_id: &'a str,
    agent_id: Option<&'a str>,
    message_id: Option<&'a str>,
    claim_text: &'a str,
    category: ClaimCategory,
    status: ClaimStatus,
    layer: VerdictLayer,
    evidence_refs: Option<&'a str>,
}

impl Default for Seed<'_> {
    fn default() -> Self {
        Self {
            user_id: "user-1",
            agent_id: None,
            message_id: None,
            claim_text: "Take 5 g of creatine per day",
            category: ClaimCategory::Supplement,
            status: ClaimStatus::Unsupported,
            layer: VerdictLayer::Evidence,
            evidence_refs: None,
        }
    }
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

async fn seed(repos: &RepositoryRegistry, tenant_id: TenantId, s: Seed<'_>) -> String {
    let params = InsertClaimVerdictParams {
        tenant_id,
        user_id: s.user_id,
        agent_id: s.agent_id,
        conversation_id: None,
        message_id: s.message_id,
        claim_text: s.claim_text,
        category: s.category,
        status: s.status,
        evidence_strength: EvidenceStrength::None,
        confidence: 0.5,
        layer_fired: s.layer,
        explanation: None,
        evidence_refs: s.evidence_refs,
    };
    repos
        .claim_verdicts
        .insert_claim_verdict(&params)
        .await
        .unwrap()
        .id
}

fn dispose<'a>(
    tenant_id: TenantId,
    verdict_id: &'a str,
    disposition: VerdictDisposition,
    reason: Option<DispositionReason>,
    note: Option<&'a str>,
) -> SetVerdictDispositionParams<'a> {
    SetVerdictDispositionParams {
        tenant_id,
        verdict_id,
        disposition,
        reason,
        note,
        disposed_by: "support@example.com",
        disposed_at: Utc.with_ymd_and_hms(2026, 9, 21, 12, 30, 0).unwrap(),
    }
}

// ============================================================================
// Repository
// ============================================================================

#[tokio::test]
async fn disposition_round_trips_through_the_enums() {
    // `ALL` is the vocabulary the handler's rejection messages list, so it
    // must carry every variant the migration's CHECK admits — three and seven.
    assert_eq!(VerdictDisposition::ALL.len(), 3);
    for d in VerdictDisposition::ALL {
        assert_eq!(VerdictDisposition::parse(d.as_str()), Some(*d));
    }
    assert_eq!(DispositionReason::ALL.len(), 7);
    for r in DispositionReason::ALL {
        assert_eq!(DispositionReason::parse(r.as_str()), Some(*r));
    }
    assert_eq!(
        DispositionReason::ALL
            .iter()
            .map(|r| r.as_str())
            .collect::<Vec<_>>(),
        vec![
            "missing_keyword",
            "bound_too_tight",
            "tolerance_too_tight",
            "stale_evidence",
            "extractor_misroute",
            "judge_error",
            "other"
        ]
    );
    assert_eq!(VerdictDisposition::parse("maybe"), None);
    assert_eq!(DispositionReason::parse("gremlins"), None);
    assert_eq!(
        DispositionFilter::parse("undisposed"),
        Some(DispositionFilter::Undisposed)
    );
    assert_eq!(
        DispositionFilter::parse("unsure"),
        Some(DispositionFilter::Is(VerdictDisposition::Unsure))
    );
    assert_eq!(DispositionFilter::parse("later"), None);
}

#[tokio::test]
async fn set_disposition_lands_all_five_fields_and_a_second_set_overwrites() -> Result<()> {
    let db = common::create_test_database().await?;
    let repos = db.repositories();
    let id = seed(&repos, tenant(), Seed::default()).await;

    let first = repos
        .claim_verdicts
        .set_verdict_disposition(&dispose(
            tenant(),
            &id,
            VerdictDisposition::FalsePositive,
            Some(DispositionReason::MissingKeyword),
            Some("corpus has it under a different word"),
        ))
        .await?;
    assert_eq!(first.id, id);
    assert_eq!(first.disposition, Some(VerdictDisposition::FalsePositive));
    assert_eq!(
        first.disposition_reason,
        Some(DispositionReason::MissingKeyword)
    );
    assert_eq!(
        first.disposition_note.as_deref(),
        Some("corpus has it under a different word")
    );
    assert_eq!(first.disposed_by.as_deref(), Some("support@example.com"));
    assert_eq!(
        first.disposed_at,
        Some(Utc.with_ymd_and_hms(2026, 9, 21, 12, 30, 0).unwrap())
    );
    // The pipeline's own columns are untouched by the write.
    assert_eq!(first.status, ClaimStatus::Unsupported);
    assert_eq!(first.layer_fired, VerdictLayer::Evidence);

    // A re-read sees the same five fields — the write is on the row, not on
    // the returned struct alone.
    let read = repos
        .claim_verdicts
        .get_verdict(tenant(), &id)
        .await?
        .expect("the row exists");
    assert_eq!(read.disposition, Some(VerdictDisposition::FalsePositive));
    assert_eq!(
        read.disposition_reason,
        Some(DispositionReason::MissingKeyword)
    );
    assert_eq!(read.disposed_by.as_deref(), Some("support@example.com"));

    // The disposition is a state, not a log: the second call replaces every
    // field, including clearing the reason and note.
    let second = repos
        .claim_verdicts
        .set_verdict_disposition(&dispose(
            tenant(),
            &id,
            VerdictDisposition::TrueCatch,
            None,
            None,
        ))
        .await?;
    assert_eq!(second.disposition, Some(VerdictDisposition::TrueCatch));
    assert_eq!(second.disposition_reason, None);
    assert_eq!(second.disposition_note, None);

    let read = repos
        .claim_verdicts
        .get_verdict(tenant(), &id)
        .await?
        .expect("the row exists");
    assert_eq!(read.disposition, Some(VerdictDisposition::TrueCatch));
    assert_eq!(read.disposition_reason, None);
    Ok(())
}

#[tokio::test]
async fn set_disposition_on_an_unknown_or_foreign_verdict_is_not_found() -> Result<()> {
    let db = common::create_test_database().await?;
    let repos = db.repositories();
    let id = seed(&repos, tenant(), Seed::default()).await;

    let err = repos
        .claim_verdicts
        .set_verdict_disposition(&dispose(
            tenant(),
            "no-such-verdict",
            VerdictDisposition::Unsure,
            None,
            None,
        ))
        .await
        .expect_err("an unknown id is refused");
    assert_eq!(err.code, ErrorCode::ResourceNotFound);

    // Another tenant's write on this id is the same refusal: the tenant is
    // part of the key, so the row is invisible rather than merely forbidden.
    let err = repos
        .claim_verdicts
        .set_verdict_disposition(&dispose(
            other_tenant(),
            &id,
            VerdictDisposition::Unsure,
            None,
            None,
        ))
        .await
        .expect_err("a foreign tenant is refused");
    assert_eq!(err.code, ErrorCode::ResourceNotFound);
    assert!(repos
        .claim_verdicts
        .get_verdict(other_tenant(), &id)
        .await?
        .is_none());

    // And the row is still undisposed.
    let read = repos
        .claim_verdicts
        .get_verdict(tenant(), &id)
        .await?
        .expect("the row exists");
    assert_eq!(read.disposition, None);
    Ok(())
}

#[tokio::test]
async fn list_filters_are_applied_in_sql() -> Result<()> {
    let db = common::create_test_database().await?;
    let repos = db.repositories();
    let owner = seed_tenant_owner(&repos, tenant()).await;
    let agent_a = seed_agent(&repos, owner, tenant(), "Agent A").await;
    let agent_b = seed_agent(&repos, owner, tenant(), "Agent B").await;

    let evidence_u1 = seed(
        &repos,
        tenant(),
        Seed {
            user_id: "user-1",
            agent_id: Some(&agent_a),
            layer: VerdictLayer::Evidence,
            ..Seed::default()
        },
    )
    .await;
    let deterministic_u1 = seed(
        &repos,
        tenant(),
        Seed {
            user_id: "user-1",
            agent_id: Some(&agent_b),
            category: ClaimCategory::Physiological,
            status: ClaimStatus::Contradicted,
            layer: VerdictLayer::Deterministic,
            ..Seed::default()
        },
    )
    .await;
    let judge_u2 = seed(
        &repos,
        tenant(),
        Seed {
            user_id: "user-2",
            layer: VerdictLayer::Judge,
            ..Seed::default()
        },
    )
    .await;
    let supported_u2 = seed(
        &repos,
        tenant(),
        Seed {
            user_id: "user-2",
            status: ClaimStatus::Supported,
            ..Seed::default()
        },
    )
    .await;
    // A row in another tenant never appears, whatever the filter.
    seed(&repos, other_tenant(), Seed::default()).await;

    repos
        .claim_verdicts
        .set_verdict_disposition(&dispose(
            tenant(),
            &deterministic_u1,
            VerdictDisposition::FalsePositive,
            Some(DispositionReason::BoundTooTight),
            None,
        ))
        .await?;
    repos
        .claim_verdicts
        .set_verdict_disposition(&dispose(
            tenant(),
            &judge_u2,
            VerdictDisposition::TrueCatch,
            None,
            None,
        ))
        .await?;

    let ids = |rows: Vec<ClaimVerdict>| {
        let mut ids: Vec<String> = rows.into_iter().map(|v| v.id).collect();
        ids.sort();
        ids
    };
    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };

    // No filter: everything in the tenant, and nothing from the other one.
    let all = repos
        .claim_verdicts
        .list_verdicts_filtered(tenant(), &VerdictListFilter::recent(50))
        .await?;
    assert_eq!(
        ids(all),
        sorted(vec![
            evidence_u1.clone(),
            deterministic_u1.clone(),
            judge_u2.clone(),
            supported_u2.clone()
        ])
    );

    // The provided `list_recent_verdicts` is the same scan.
    let recent = repos
        .claim_verdicts
        .list_recent_verdicts(tenant(), 50)
        .await?;
    assert_eq!(recent.len(), 4);

    let by_layer = repos
        .claim_verdicts
        .list_verdicts_filtered(
            tenant(),
            &VerdictListFilter {
                layer_fired: Some(VerdictLayer::Evidence),
                limit: 50,
                ..VerdictListFilter::default()
            },
        )
        .await?;
    // Two rows were decided by the evidence layer: the flagged one and the
    // supported one — the layer filter says nothing about status.
    assert_eq!(
        ids(by_layer),
        sorted(vec![evidence_u1.clone(), supported_u2.clone()])
    );

    let false_positives = repos
        .claim_verdicts
        .list_verdicts_filtered(
            tenant(),
            &VerdictListFilter {
                disposition: Some(DispositionFilter::Is(VerdictDisposition::FalsePositive)),
                limit: 50,
                ..VerdictListFilter::default()
            },
        )
        .await?;
    assert_eq!(ids(false_positives), vec![deterministic_u1.clone()]);

    let undisposed = repos
        .claim_verdicts
        .list_verdicts_filtered(
            tenant(),
            &VerdictListFilter {
                disposition: Some(DispositionFilter::Undisposed),
                limit: 50,
                ..VerdictListFilter::default()
            },
        )
        .await?;
    assert_eq!(
        ids(undisposed),
        sorted(vec![evidence_u1.clone(), supported_u2.clone()])
    );

    let by_user = repos
        .claim_verdicts
        .list_verdicts_filtered(
            tenant(),
            &VerdictListFilter {
                user_id: Some("user-2".to_owned()),
                limit: 50,
                ..VerdictListFilter::default()
            },
        )
        .await?;
    assert_eq!(
        ids(by_user),
        sorted(vec![judge_u2.clone(), supported_u2.clone()])
    );

    // Axes combine: status + category + agent narrow to the one row.
    let combined = repos
        .claim_verdicts
        .list_verdicts_filtered(
            tenant(),
            &VerdictListFilter {
                status: Some(ClaimStatus::Contradicted),
                category: Some(ClaimCategory::Physiological),
                agent_id: Some(agent_b.clone()),
                limit: 50,
                ..VerdictListFilter::default()
            },
        )
        .await?;
    assert_eq!(ids(combined), vec![deterministic_u1.clone()]);

    // A combination nothing satisfies is empty, not an error.
    let none = repos
        .claim_verdicts
        .list_verdicts_filtered(
            tenant(),
            &VerdictListFilter {
                layer_fired: Some(VerdictLayer::Judge),
                disposition: Some(DispositionFilter::Undisposed),
                limit: 50,
                ..VerdictListFilter::default()
            },
        )
        .await?;
    assert!(none.is_empty());

    // The limit is honoured and clamped to at least one.
    let one = repos
        .claim_verdicts
        .list_verdicts_filtered(tenant(), &VerdictListFilter::recent(0))
        .await?;
    assert_eq!(one.len(), 1);
    Ok(())
}

/// Agent grading scans up to 1000 verdicts and myth-busting up to 500
/// through `list_recent_verdicts`; the admin list's own 200 ceiling lives in
/// its handler, not in the repository, so the wide scans keep their reach.
#[tokio::test]
async fn the_recent_scan_reaches_past_the_admin_lists_page_size() -> Result<()> {
    let db = common::create_test_database().await?;
    let repos = db.repositories();
    for _ in 0..201 {
        seed(&repos, tenant(), Seed::default()).await;
    }

    let wide = repos
        .claim_verdicts
        .list_recent_verdicts(tenant(), 1000)
        .await?;
    assert_eq!(wide.len(), 201, "the caller's limit is bound as given");

    let page = repos
        .claim_verdicts
        .list_verdicts_filtered(tenant(), &VerdictListFilter::recent(200))
        .await?;
    assert_eq!(page.len(), 200, "a smaller limit is honoured");
    Ok(())
}

#[tokio::test]
async fn list_verdicts_for_message_returns_only_that_message() -> Result<()> {
    let db = common::create_test_database().await?;
    let repos = db.repositories();
    let a1 = seed(
        &repos,
        tenant(),
        Seed {
            message_id: Some("msg-a"),
            claim_text: "first claim of a",
            ..Seed::default()
        },
    )
    .await;
    let a2 = seed(
        &repos,
        tenant(),
        Seed {
            message_id: Some("msg-a"),
            claim_text: "second claim of a",
            ..Seed::default()
        },
    )
    .await;
    seed(
        &repos,
        tenant(),
        Seed {
            message_id: Some("msg-b"),
            ..Seed::default()
        },
    )
    .await;
    seed(&repos, tenant(), Seed::default()).await;
    // The same message id under another tenant is not this tenant's.
    seed(
        &repos,
        other_tenant(),
        Seed {
            message_id: Some("msg-a"),
            ..Seed::default()
        },
    )
    .await;

    let rows = repos
        .claim_verdicts
        .list_verdicts_for_message(tenant(), "msg-a")
        .await?;
    let ids: Vec<&str> = rows.iter().map(|v| v.id.as_str()).collect();
    assert_eq!(ids, vec![a1.as_str(), a2.as_str()], "oldest first");
    assert!(rows
        .iter()
        .all(|v| v.message_id.as_deref() == Some("msg-a")));

    let none = repos
        .claim_verdicts
        .list_verdicts_for_message(tenant(), "msg-none")
        .await?;
    assert!(none.is_empty());
    Ok(())
}

// ============================================================================
// Handlers
// ============================================================================

/// Build a fully-wired [`AdminApiContext`] backed by a fresh test database,
/// with the given evidence registry so the knob tests can seed propositions.
async fn build_context(
    evidence_registry: EvidenceRegistry,
) -> (Arc<AdminApiContext>, Arc<RepositoryRegistry>) {
    let database = common::create_test_database().await.unwrap();
    let auth_manager = common::create_test_auth_manager();
    let jwks_manager = common::get_shared_test_jwks();

    let database_arc = Arc::new((*database).clone());
    let repos_arc = Arc::new(database_arc.repositories());

    let context = AdminApiContext::new(AdminApiContextInit {
        database: database_arc,
        repos: repos_arc.clone(),
        jwt_secret: "test_admin_jwt_secret_for_verdict_disposition".to_owned(),
        auth_manager,
        jwks_manager,
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        admin_token_cache_ttl_secs: AdminAuthService::DEFAULT_CACHE_TTL_SECS,
        harness_config_registry: Arc::new(HarnessConfigRegistry::bootstrap()),
        guardian_config_registry: Arc::new(GuardianConfigRegistry::bootstrap()),
        prompt_registry: Arc::new(pierre_contremaitre::PromptRegistry::new()),
        tool_description_registry: Arc::new(pierre_contremaitre::ToolDescriptionRegistry::new()),
        evidence_registry: Arc::new(evidence_registry),
        messaging_strings_registry: Arc::new(pierre_contremaitre::MessagingStringsRegistry::new()),
        cageux_config_registry: Arc::new(CageuxConfigRegistry::from_env()),
        persona_contract_registry: Arc::new(PersonaContractRegistry::new()),
        training_catalogue_registry: Arc::new(pierre_contremaitre::TrainingCatalogueRegistry::new()),
        contremaitre_config: None,
    });

    (Arc::new(context), repos_arc)
}

/// Cookie-style super-admin token, mirroring what `cookie_admin_middleware`
/// synthesizes for a `SuperAdmin`.
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

/// Plain admin: passes both auth middlewares but carries only the default
/// keys-management permission set — no ViewConfiguration/ManageConfiguration.
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

/// Tenant-bound admin token: holds the permissions the handlers ask for but
/// is bound to one tenant, so a refusal can only come from the tenant gate.
fn tenant_bound_token(tenant_id: TenantId) -> ValidatedAdminToken {
    ValidatedAdminToken {
        token_id: format!("admin-token:{}", Uuid::new_v4()),
        service_name: "tenant-admin@example.com".to_owned(),
        permissions: AdminPermissions::new(vec![
            AdminPermission::ViewConfiguration,
            AdminPermission::ManageConfiguration,
        ]),
        is_super_admin: false,
        tenant_id: Some(tenant_id.as_uuid().to_string()),
        user_info: None,
    }
}

fn disposition_body(tenant_id: TenantId, disposition: &str) -> SetDispositionRequest {
    SetDispositionRequest {
        tenant_id: tenant_id.to_string(),
        disposition: disposition.to_owned(),
        reason: Some("missing_keyword".to_owned()),
        note: Some("  wording  ".to_owned()),
    }
}

async fn body_json(resp: impl IntoResponse) -> (StatusCode, Value) {
    let resp = resp.into_response();
    let status = resp.status();
    let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn put_disposition_is_refused_without_manage_configuration() {
    let (context, repos) = build_context(EvidenceRegistry::new()).await;
    let id = seed(&repos, tenant(), Seed::default()).await;

    let err = handle_set_verdict_disposition(
        State(context.clone()),
        Extension(plain_admin_token()),
        Path(id.clone()),
        Json(disposition_body(tenant(), "false_positive")),
    )
    .await
    .err()
    .expect("a plain admin cannot dispose");
    assert_eq!(err.code, ErrorCode::PermissionDenied);
    assert_eq!(
        err.into_response().status(),
        StatusCode::FORBIDDEN,
        "the refusal is a 403 on the wire"
    );

    // Nothing was written.
    let row = repos
        .claim_verdicts
        .get_verdict(tenant(), &id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.disposition, None);
}

#[tokio::test]
async fn put_disposition_is_refused_across_tenants() {
    let (context, repos) = build_context(EvidenceRegistry::new()).await;
    let id = seed(&repos, tenant(), Seed::default()).await;

    let err = handle_set_verdict_disposition(
        State(context.clone()),
        Extension(tenant_bound_token(other_tenant())),
        Path(id.clone()),
        Json(disposition_body(tenant(), "false_positive")),
    )
    .await
    .err()
    .expect("a token bound to another tenant cannot dispose");
    assert_eq!(err.code, ErrorCode::PermissionDenied);
    assert_eq!(err.into_response().status(), StatusCode::FORBIDDEN);

    // The bound token reaches its own tenant.
    let resp = handle_set_verdict_disposition(
        State(context),
        Extension(tenant_bound_token(tenant())),
        Path(id),
        Json(disposition_body(tenant(), "false_positive")),
    )
    .await
    .expect("own tenant disposes");
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["verdict"]["disposition"], "false_positive");
    assert_eq!(
        json["verdict"]["disposed_by"], "tenant-admin@example.com",
        "the row records the admin's service name"
    );
}

#[tokio::test]
async fn put_disposition_writes_the_row_and_answers_with_the_knob() {
    let (context, repos) = build_context(EvidenceRegistry::new()).await;
    let id = seed(
        &repos,
        tenant(),
        Seed {
            category: ClaimCategory::Physiological,
            status: ClaimStatus::Contradicted,
            layer: VerdictLayer::Deterministic,
            claim_text: "Your max heart rate is 250 bpm",
            ..Seed::default()
        },
    )
    .await;

    let resp = handle_set_verdict_disposition(
        State(context.clone()),
        Extension(super_admin_token()),
        Path(id.clone()),
        Json(SetDispositionRequest {
            tenant_id: tenant().to_string(),
            disposition: "true_catch".to_owned(),
            reason: Some("other".to_owned()),
            note: Some("  250 bpm is nonsense  ".to_owned()),
        }),
    )
    .await
    .expect("super-admin disposes");
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["verdict"]["id"], id);
    assert_eq!(json["verdict"]["disposition"], "true_catch");
    assert_eq!(json["verdict"]["disposition_reason"], "other");
    assert_eq!(
        json["verdict"]["disposition_note"], "250 bpm is nonsense",
        "the note is trimmed"
    );
    assert_eq!(json["verdict"]["disposed_by"], "admin@example.com");
    assert!(json["verdict"]["disposed_at"]
        .as_str()
        .is_some_and(|t| chrono::DateTime::parse_from_rfc3339(t).is_ok()));
    assert_eq!(json["knob"]["layer"], "deterministic");
    assert_eq!(json["knob"]["kind"], "deterministic_bounds");
    assert_eq!(
        json["knob"]["location"],
        "crates/pierre-evals/src/deterministic_bounds.rs"
    );
    assert!(json["knob"]["detail"]
        .as_str()
        .unwrap()
        .contains("check_physiological"));

    // The row carries it.
    let row = repos
        .claim_verdicts
        .get_verdict(tenant(), &id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.disposition, Some(VerdictDisposition::TrueCatch));
    assert_eq!(row.disposition_reason, Some(DispositionReason::Other));
    assert_eq!(row.disposition_note.as_deref(), Some("250 bpm is nonsense"));
}

#[tokio::test]
async fn put_disposition_validates_its_body_and_404s_a_stranger() {
    let (context, repos) = build_context(EvidenceRegistry::new()).await;
    let id = seed(&repos, tenant(), Seed::default()).await;

    let err = handle_set_verdict_disposition(
        State(context.clone()),
        Extension(super_admin_token()),
        Path(id.clone()),
        Json(SetDispositionRequest {
            tenant_id: tenant().to_string(),
            disposition: "maybe".to_owned(),
            reason: None,
            note: None,
        }),
    )
    .await
    .err()
    .expect("an unknown disposition is refused");
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(err.message.contains("true_catch, false_positive, unsure"));

    let err = handle_set_verdict_disposition(
        State(context.clone()),
        Extension(super_admin_token()),
        Path(id.clone()),
        Json(SetDispositionRequest {
            tenant_id: tenant().to_string(),
            disposition: "unsure".to_owned(),
            reason: Some("gremlins".to_owned()),
            note: None,
        }),
    )
    .await
    .err()
    .expect("an unknown reason is refused");
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(err.message.contains("reason must be one of"));

    let err = handle_set_verdict_disposition(
        State(context.clone()),
        Extension(super_admin_token()),
        Path(id.clone()),
        Json(SetDispositionRequest {
            tenant_id: tenant().to_string(),
            disposition: "unsure".to_owned(),
            reason: None,
            note: Some("x".repeat(MAX_DISPOSITION_NOTE_CHARS + 1)),
        }),
    )
    .await
    .err()
    .expect("an oversized note is refused");
    assert_eq!(err.code, ErrorCode::InvalidInput);

    let err = handle_set_verdict_disposition(
        State(context.clone()),
        Extension(super_admin_token()),
        Path("no-such-verdict".to_owned()),
        Json(disposition_body(tenant(), "unsure")),
    )
    .await
    .err()
    .expect("a stranger is not found");
    assert_eq!(err.code, ErrorCode::ResourceNotFound);
    assert_eq!(err.into_response().status(), StatusCode::NOT_FOUND);

    // None of the refused writes touched the row.
    let row = repos
        .claim_verdicts
        .get_verdict(tenant(), &id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.disposition, None);
}

#[tokio::test]
async fn get_detail_serves_the_row_and_knob_and_is_gated() {
    let (context, repos) = build_context(EvidenceRegistry::new()).await;
    let id = seed(
        &repos,
        tenant(),
        Seed {
            layer: VerdictLayer::Judge,
            ..Seed::default()
        },
    )
    .await;

    let err = handle_get_claim_verdict(
        State(context.clone()),
        Extension(plain_admin_token()),
        Path(id.clone()),
        Query(TenantScopedQuery {
            tenant_id: tenant().to_string(),
        }),
    )
    .await
    .err()
    .expect("a plain admin cannot read");
    assert_eq!(err.code, ErrorCode::PermissionDenied);

    let err = handle_get_claim_verdict(
        State(context.clone()),
        Extension(tenant_bound_token(other_tenant())),
        Path(id.clone()),
        Query(TenantScopedQuery {
            tenant_id: tenant().to_string(),
        }),
    )
    .await
    .err()
    .expect("a foreign tenant cannot read");
    assert_eq!(err.code, ErrorCode::PermissionDenied);

    let resp = handle_get_claim_verdict(
        State(context.clone()),
        Extension(super_admin_token()),
        Path(id.clone()),
        Query(TenantScopedQuery {
            tenant_id: tenant().to_string(),
        }),
    )
    .await
    .expect("super-admin reads");
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["verdict"]["id"], id);
    assert_eq!(json["verdict"]["disposition"], Value::Null);
    assert_eq!(json["knob"]["kind"], "judge_prompt");
    assert_eq!(json["knob"]["location"], "prompts/system/claim_judge.md");
    assert!(json["knob"]["detail"]
        .as_str()
        .unwrap()
        .contains("`claim_judge` system prompt"));

    let err = handle_get_claim_verdict(
        State(context),
        Extension(super_admin_token()),
        Path("no-such-verdict".to_owned()),
        Query(TenantScopedQuery {
            tenant_id: tenant().to_string(),
        }),
    )
    .await
    .err()
    .expect("a stranger is not found");
    assert_eq!(err.code, ErrorCode::ResourceNotFound);
}

#[tokio::test]
async fn message_lookup_serves_the_message_rows_and_is_gated() {
    let (context, repos) = build_context(EvidenceRegistry::new()).await;
    let a1 = seed(
        &repos,
        tenant(),
        Seed {
            message_id: Some("msg-a"),
            ..Seed::default()
        },
    )
    .await;
    seed(
        &repos,
        tenant(),
        Seed {
            message_id: Some("msg-b"),
            ..Seed::default()
        },
    )
    .await;

    let err = handle_list_verdicts_by_message(
        State(context.clone()),
        Extension(tenant_bound_token(other_tenant())),
        Path("msg-a".to_owned()),
        Query(TenantScopedQuery {
            tenant_id: tenant().to_string(),
        }),
    )
    .await
    .err()
    .expect("a foreign tenant cannot read");
    assert_eq!(err.code, ErrorCode::PermissionDenied);

    let resp = handle_list_verdicts_by_message(
        State(context),
        Extension(super_admin_token()),
        Path("msg-a".to_owned()),
        Query(TenantScopedQuery {
            tenant_id: tenant().to_string(),
        }),
    )
    .await
    .expect("super-admin reads");
    let (status, json) = body_json(resp).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(json["total"], 1);
    assert_eq!(json["verdicts"][0]["id"], a1);
    assert_eq!(json["verdicts"][0]["message_id"], "msg-a");
}

#[tokio::test]
async fn list_handler_filters_in_sql_and_rejects_a_typo() {
    let (context, repos) = build_context(EvidenceRegistry::new()).await;
    let evidence = seed(&repos, tenant(), Seed::default()).await;
    let deterministic = seed(
        &repos,
        tenant(),
        Seed {
            layer: VerdictLayer::Deterministic,
            status: ClaimStatus::Contradicted,
            ..Seed::default()
        },
    )
    .await;
    repos
        .claim_verdicts
        .set_verdict_disposition(&dispose(
            tenant(),
            &deterministic,
            VerdictDisposition::FalsePositive,
            None,
            None,
        ))
        .await
        .unwrap();

    let query = |layer: Option<&str>, disposition: Option<&str>| ListVerdictsQuery {
        tenant_id: tenant().to_string(),
        status: None,
        category: None,
        agent_id: None,
        layer_fired: layer.map(ToOwned::to_owned),
        disposition: disposition.map(ToOwned::to_owned),
        user_id: None,
        limit: None,
    };

    let resp = handle_list_claim_verdicts(
        State(context.clone()),
        Extension(super_admin_token()),
        Query(query(Some("deterministic"), None)),
    )
    .await
    .expect("layer filter");
    let (_, json) = body_json(resp).await;
    assert_eq!(json["total"], 1);
    assert_eq!(json["verdicts"][0]["id"], deterministic);
    assert_eq!(json["verdicts"][0]["disposition"], "false_positive");

    let resp = handle_list_claim_verdicts(
        State(context.clone()),
        Extension(super_admin_token()),
        Query(query(None, Some("undisposed"))),
    )
    .await
    .expect("undisposed filter");
    let (_, json) = body_json(resp).await;
    assert_eq!(json["total"], 1);
    assert_eq!(json["verdicts"][0]["id"], evidence);

    let err = handle_list_claim_verdicts(
        State(context),
        Extension(super_admin_token()),
        Query(query(Some("vibes"), None)),
    )
    .await
    .err()
    .expect("a layer outside the vocabulary is a 400, not an empty page");
    assert_eq!(err.code, ErrorCode::InvalidInput);
    assert!(err.message.contains("layer_fired must be one of"));
}

// ============================================================================
// The knob
// ============================================================================

const CREATINE_PROPOSITION: &str = "---
id: doi:10.1186/s12970-017-0173-z
category: supplement
strength: strong
citation: Kreider et al. 2017 ISSN position stand
---

Creatine monohydrate at 3 to 5 g per day is the most effective ergogenic supplement for high-intensity exercise performance.
";

const CAFFEINE_PROPOSITION: &str = "---
id: doi:10.1186/s12970-020-00383-4
category: supplement
strength: strong
citation: Guest et al. 2021 ISSN position stand
---

Caffeine at 3 to 6 mg/kg improves endurance performance when taken 60 minutes before exercise.
";

#[tokio::test]
async fn evidence_knob_names_the_propositions_that_match_and_marks_the_cited_one() {
    let registry = EvidenceRegistry::new();
    registry.update(
        "supplement",
        "kreider-2017-creatine",
        parse_evidence_markdown(CREATINE_PROPOSITION).unwrap(),
        "sha-creatine".to_owned(),
    );
    registry.update(
        "supplement",
        "guest-2021-caffeine",
        parse_evidence_markdown(CAFFEINE_PROPOSITION).unwrap(),
        "sha-caffeine".to_owned(),
    );
    let (context, repos) = build_context(registry).await;
    let id = seed(
        &repos,
        tenant(),
        Seed {
            claim_text: "Creatine at 5 g per day improves high-intensity performance",
            status: ClaimStatus::Supported,
            evidence_refs: Some("doi:10.1186/s12970-017-0173-z"),
            ..Seed::default()
        },
    )
    .await;

    let resp = handle_get_claim_verdict(
        State(context),
        Extension(super_admin_token()),
        Path(id),
        Query(TenantScopedQuery {
            tenant_id: tenant().to_string(),
        }),
    )
    .await
    .expect("reads");
    let (_, json) = body_json(resp).await;
    let knob = &json["knob"];
    assert_eq!(knob["kind"], "evidence_corpus");
    assert_eq!(knob["location"], "evidence/sports_science/supplement/");
    let props = knob["propositions"].as_array().unwrap();
    // Both propositions share a 4+ letter word with the claim ("improves",
    // "performance" are in both; "creatine" and "high-intensity" only in one),
    // so both are named — the cited one first, then by score.
    assert_eq!(props.len(), 2, "{props:?}");
    assert_eq!(
        props[0]["path"],
        "evidence/sports_science/supplement/kreider-2017-creatine.md"
    );
    assert_eq!(props[0]["id"], "doi:10.1186/s12970-017-0173-z");
    assert_eq!(props[0]["slug"], "kreider-2017-creatine");
    assert_eq!(props[0]["category"], "supplement");
    assert_eq!(props[0]["strength"], "strong");
    assert_eq!(props[0]["cited"], true);
    assert_eq!(
        props[1]["path"],
        "evidence/sports_science/supplement/guest-2021-caffeine.md"
    );
    assert_eq!(props[1]["cited"], false);
    assert!(
        props[0]["score"].as_u64().unwrap() > props[1]["score"].as_u64().unwrap(),
        "the cited creatine record shares more words with the claim than caffeine does"
    );
    assert!(knob["detail"]
        .as_str()
        .unwrap()
        .contains("verification_config.categories.supplement.min_strength"));
}

#[tokio::test]
async fn evidence_knob_on_an_empty_registry_points_at_the_compiled_in_corpus() {
    let (context, repos) = build_context(EvidenceRegistry::new()).await;
    let id = seed(
        &repos,
        tenant(),
        Seed {
            category: ClaimCategory::Nutrition,
            ..Seed::default()
        },
    )
    .await;
    let resp = handle_get_claim_verdict(
        State(context),
        Extension(super_admin_token()),
        Path(id),
        Query(TenantScopedQuery {
            tenant_id: tenant().to_string(),
        }),
    )
    .await
    .expect("reads");
    let (_, json) = body_json(resp).await;
    assert_eq!(
        json["knob"]["location"],
        "crates/pierre-evals/fixtures/sports_science/nutrition/"
    );
    assert!(json["knob"]["detail"]
        .as_str()
        .unwrap()
        .contains("EMBEDDED_PROPOSITIONS"));
    assert_eq!(json["knob"]["propositions"], Value::Array(vec![]));
}

#[test]
fn every_layer_has_a_knob_whose_location_is_a_path() {
    let registry = EvidenceRegistry::new();
    for layer in [
        VerdictLayer::Rhetoric,
        VerdictLayer::Deterministic,
        VerdictLayer::Personalized,
        VerdictLayer::AthleteData,
        VerdictLayer::Evidence,
        VerdictLayer::Consistency,
        VerdictLayer::Judge,
    ] {
        let verdict = ClaimVerdict {
            id: "v".to_owned(),
            tenant_id: tenant().to_string(),
            user_id: "u".to_owned(),
            agent_id: None,
            conversation_id: None,
            message_id: None,
            claim_text: "claim".to_owned(),
            category: ClaimCategory::Recovery,
            status: ClaimStatus::Unsupported,
            evidence_strength: EvidenceStrength::None,
            confidence: 0.5,
            layer_fired: layer,
            explanation: None,
            evidence_refs: None,
            created_at: Utc::now(),
            disposition: None,
            disposition_reason: None,
            disposition_note: None,
            disposed_by: None,
            disposed_at: None,
        };
        let knob = knob_for(&verdict, &registry);
        assert_eq!(knob.layer, layer.as_str());
        assert!(
            knob.location.starts_with("crates/")
                || knob.location.starts_with("evidence/")
                || knob.location.starts_with("prompts/"),
            "{layer:?}: {}",
            knob.location
        );
        assert!(!knob.detail.is_empty(), "{layer:?}");
        assert!(!knob.kind.is_empty(), "{layer:?}");
    }
}

// ============================================================================
// Route registration
// ============================================================================

/// Create an active user with the given role and its own tenant; returns the
/// bearer header.
async fn bearer_for_role(resources: &Arc<ServerContext>, email: &str, role: UserRole) -> String {
    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST).unwrap();
    let mut user = User::new(
        email.to_owned(),
        password_hash,
        Some("Test User".to_owned()),
    );
    user.is_admin = role.is_admin_or_higher();
    user.role = role;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(Utc::now());
    let user_id = user.id;
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Tenant for {email}"),
        slug: format!("tenant-{tenant_id}"),
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
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();
    let token = common::generate_test_token(resources, &user).await;
    format!("Bearer {token}")
}

fn cookie_admin_router(resources: &Arc<ServerContext>) -> axum::Router {
    let admin_context = AdminApiContext::new(AdminApiContextInit {
        database: resources.agent.database.clone(),
        repos: resources.common.repos.clone(),
        jwt_secret: resources.auth.admin_jwt_secret.to_string(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
        admin_token_cache_ttl_secs: AdminAuthService::DEFAULT_CACHE_TTL_SECS,
        harness_config_registry: resources.fitness.harness_config_registry.clone(),
        guardian_config_registry: resources.fitness.guardian_config_registry.clone(),
        prompt_registry: resources.mcp.prompt_registry.clone(),
        tool_description_registry: resources.mcp.tool_description_registry.clone(),
        evidence_registry: resources.mcp.evidence_registry.clone(),
        messaging_strings_registry: resources.mcp.messaging_strings_registry.clone(),
        cageux_config_registry: resources.fitness.cageux_config_registry.clone(),
        persona_contract_registry: resources.fitness.persona_contract_registry.clone(),
        training_catalogue_registry: resources.mcp.training_catalogue_registry.clone(),
        contremaitre_config: None,
    });
    AdminRoutes::cookie_admin_routes::<ServerContext>(admin_context, resources)
}

/// The four new routes are mounted under the cookie-admin group, and `health`
/// is read as the static segment rather than as a verdict id.
#[tokio::test]
#[serial]
async fn the_routes_are_mounted_under_the_cookie_admin_group() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let super_admin = bearer_for_role(
        &resources,
        "verdict-routes-super@example.com",
        UserRole::SuperAdmin,
    )
    .await;
    let plain_admin = bearer_for_role(
        &resources,
        "verdict-routes-plain@example.com",
        UserRole::Admin,
    )
    .await;
    let repos = resources.common.repos.clone();
    let id = seed(
        &repos,
        tenant(),
        Seed {
            message_id: Some("msg-route"),
            ..Seed::default()
        },
    )
    .await;
    let t = tenant().to_string();

    let response = AxumTestRequest::get(&format!("/api/admin/claim-verdicts/health?tenant_id={t}"))
        .header("authorization", &super_admin)
        .send(cookie_admin_router(&resources))
        .await;
    assert_eq!(response.status(), 200, "health is a static route");
    let body: Value = response.json();
    assert_eq!(body["totals"]["flagged"], 1);

    let response = AxumTestRequest::get(&format!(
        "/api/admin/claim-verdicts/messages/msg-route?tenant_id={t}"
    ))
    .header("authorization", &super_admin)
    .send(cookie_admin_router(&resources))
    .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json();
    assert_eq!(body["verdicts"][0]["id"], id);

    let response = AxumTestRequest::get(&format!("/api/admin/claim-verdicts/{id}?tenant_id={t}"))
        .header("authorization", &super_admin)
        .send(cookie_admin_router(&resources))
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json();
    assert_eq!(body["knob"]["kind"], "evidence_corpus");

    let response = AxumTestRequest::put(&format!("/api/admin/claim-verdicts/{id}/disposition"))
        .header("authorization", &plain_admin)
        .json(&serde_json::json!({
            "tenant_id": t,
            "disposition": "unsure",
        }))
        .send(cookie_admin_router(&resources))
        .await;
    assert_eq!(
        response.status(),
        403,
        "a plain admin's cookie token carries no ManageConfiguration"
    );

    let response = AxumTestRequest::put(&format!("/api/admin/claim-verdicts/{id}/disposition"))
        .header("authorization", &super_admin)
        .json(&serde_json::json!({
            "tenant_id": t,
            "disposition": "unsure",
            "note": "cannot tell from the row",
        }))
        .send(cookie_admin_router(&resources))
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json();
    assert_eq!(body["verdict"]["disposition"], "unsure");
    assert_eq!(
        body["verdict"]["disposition_note"],
        "cannot tell from the row"
    );
    assert_eq!(
        body["verdict"]["disposed_by"], "verdict-routes-super@example.com",
        "under cookie auth the service name is the admin's email"
    );

    let response = AxumTestRequest::get(&format!(
        "/api/admin/claim-verdicts?tenant_id={t}&disposition=unsure&layer_fired=evidence"
    ))
    .header("authorization", &super_admin)
    .send(cookie_admin_router(&resources))
    .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json();
    assert_eq!(body["total"], 1);
    assert_eq!(body["verdicts"][0]["id"], id);
    Ok(())
}
