// ABOUTME: Every admin screen scoped by carnet#1440, through its real route: a plain admin stays home, a super admin reads the platform
// ABOUTME: Seeds two tenants' rows and pins the refusals, the content each role reads, and the audit row a cross-tenant read writes

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::Router;
use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::admin::scope::AdminScope;
use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
use pierre_core::models::{ConversationTurnId, InsertLlmUsage, Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_core::transport::TransportPolicy;
use pierre_database::repositories::{
    AuditEventFilter, AuditEventRecord, ExtractionRunOutcome, ExtractionRunRecord,
    InsertClaimVerdictParams, MemoryExtractionJobRow, UpsertUserFactParams,
};
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_memory::claims::{ClaimCategory, ClaimStatus, EvidenceStrength, VerdictLayer};
use pierre_memory::{FactKind, FactSource, MemoryScope, PredicateCode};
use pierre_routes_admin::{
    AdminApiContext, AdminApiContextInit, AdminRoutes, LlmConsumptionRoutes,
};
use pierre_routes_dashboard::DashboardRoutes;
use pierre_routes_web_admin::WebAdminRoutes;
use serde_json::Value;
use serial_test::serial;
use uuid::Uuid;

/// One scoped endpoint: its path, and the number it reports for what it read.
/// Each tenant is seeded with exactly one row of everything, so that number is
/// the count of tenants the read reached.
struct Endpoint {
    path: &'static str,
    read: fn(&Value) -> i64,
}

fn endpoints() -> Vec<Endpoint> {
    let mut all = vec![
        Endpoint {
            path: "/api/admin/memory/worker-metrics",
            read: |body| {
                assert_eq!(
                    body["metrics"]["total_facts"], body["runs"]["last_24h"]["persisted"],
                    "bug: the store and the run ledger must cover the same tenants"
                );
                assert_eq!(
                    body["runs"]["backlog"], body["runs"]["last_24h"]["persisted"],
                    "bug: the backlog must cover the same tenants"
                );
                body["metrics"]["total_facts"].as_i64().unwrap()
            },
        },
        Endpoint {
            path: "/api/admin/myth-busting/summary",
            read: |body| body["verdicts_scanned"].as_i64().unwrap(),
        },
        Endpoint {
            path: "/api/admin/agent-grading/summary",
            read: |body| body["verdicts_scanned"].as_i64().unwrap(),
        },
        Endpoint {
            path: "/api/admin/claim-verdicts",
            read: |body| body["total"].as_i64().unwrap(),
        },
        Endpoint {
            path: "/api/admin/claim-verdicts/health",
            read: |body| body["totals"]["flagged"].as_i64().unwrap(),
        },
        Endpoint {
            path: "/admin/usage/llm-consumption",
            read: |body| body["summary"]["total_calls"].as_i64().unwrap(),
        },
        Endpoint {
            path: "/admin/tool-usage",
            read: |body| body["summary"]["total_invocations"].as_i64().unwrap(),
        },
        Endpoint {
            path: "/api/dashboard/analytics",
            read: |body| {
                body["time_series"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|point| point["request_count"].as_i64().unwrap())
                    .sum()
            },
        },
        Endpoint {
            path: "/api/admin/analytics/recent-activity",
            read: |body| {
                let calls = body["summary"]["llm_calls_today"].as_i64().unwrap();
                assert_eq!(
                    i64::try_from(body["recent_llm_calls"].as_array().unwrap().len()).unwrap(),
                    calls,
                    "bug: the feed and the summary must cover the same tenants"
                );
                assert_eq!(
                    i64::try_from(body["recent_conversations"].as_array().unwrap().len()).unwrap(),
                    calls,
                    "bug: the conversation list must cover the same tenants"
                );
                assert_eq!(
                    body["summary"]["active_conversations"].as_i64().unwrap(),
                    calls,
                    "bug: the active count must cover the same tenants"
                );
                calls
            },
        },
    ];
    if cfg!(feature = "tools-verification") {
        all.push(Endpoint {
            path: "/api/admin/evals/verdict-stats",
            read: |body| body["totals"]["contradicted"].as_i64().unwrap(),
        });
    }
    all
}

struct Fixture {
    resources: Arc<ServerContext>,
    router: Router,
    /// A platform `Admin`, owner of `home`.
    plain_admin: User,
    plain_auth: String,
    /// A platform `SuperAdmin`, owner of a third tenant with no rows at all.
    super_auth: String,
    super_home: TenantId,
    /// A plain `User`, owner of a fourth tenant with no rows at all.
    athlete_auth: String,
    home: TenantId,
    foreign: TenantId,
}

async fn operator(resources: &Arc<ServerContext>, role: UserRole) -> (User, TenantId, String) {
    let email = format!("scope-{}@example.com", Uuid::new_v4());
    let password_hash = bcrypt::hash("password123", 4).unwrap();
    let mut user = User::new(email.clone(), password_hash, Some("Operator".to_owned()));
    user.is_admin = role != UserRole::User;
    user.role = role;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Workspace of {email}"),
        slug: format!("tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user.id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
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
        .update_tenant_id(user.id, tenant_id)
        .await
        .unwrap();

    let token = generate_test_token(resources, &user).await;
    (user, tenant_id, format!("Bearer {token}"))
}

/// One chat call billed to `tenant`.
async fn insert_llm_call(
    resources: &Arc<ServerContext>,
    tenant: TenantId,
    user: Uuid,
    provider: &str,
    model: &str,
    prompt_tokens: i64,
    total_tokens: i64,
) {
    resources
        .common
        .repos
        .llm_usage
        .insert_llm_usage(&InsertLlmUsage {
            tenant_id: &tenant.to_string(),
            user_id: &user.to_string(),
            conversation_id: None,
            turn_id: ConversationTurnId::from_uuid(Uuid::new_v4()),
            provider,
            model,
            prompt_tokens,
            completion_tokens: 0,
            total_tokens,
            cached_tokens: 0,
            cached_write_tokens: 0,
            cached_write_1h_tokens: 0,
            reasoning_tokens: 0,
            call_type: "chat",
            tool_calls_count: 1,
            tools_called: r#"["get_activities"]"#,
            execution_time_ms: Some(20),
            cost_usd: 0.0,
            call_sequence: Some(1),
        })
        .await
        .unwrap();
}

/// A user added to `tenant` as a plain member, as an approval does, with no
/// tenant of their own.
async fn member_of(
    resources: &Arc<ServerContext>,
    tenant: TenantId,
    role: UserRole,
) -> (User, String) {
    let email = format!("member-{}@example.com", Uuid::new_v4());
    let password_hash = bcrypt::hash("password123", 4).unwrap();
    let mut user = User::new(email, password_hash, Some("Member".to_owned()));
    user.is_admin = role != UserRole::User;
    user.role = role;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());
    let repos = &resources.common.repos;
    repos.users.create(&user).await.unwrap();
    repos.users.update_tenant_id(user.id, tenant).await.unwrap();
    assert_eq!(
        repos
            .tenants
            .get_user_role(user.id, tenant)
            .await
            .unwrap()
            .as_deref(),
        Some("member"),
        "bug: the fixture must be a plain member"
    );
    let token = generate_test_token(resources, &user).await;
    (user, format!("Bearer {token}"))
}

/// One row of everything every scoped screen reads, for one tenant, quoting
/// `sentence` in its claim verdict.
async fn seed_rows(resources: &Arc<ServerContext>, tenant: TenantId, user: Uuid, sentence: &str) {
    let repos = &resources.common.repos;
    let user_text = user.to_string();
    insert_llm_call(resources, tenant, user, "google", "gemini", 0, 10).await;
    repos
        .chat
        .create_conversation(&user_text, tenant, "A conversation", "gemini", None, None)
        .await
        .unwrap();
    repos
        .claim_verdicts
        .insert_claim_verdict(&InsertClaimVerdictParams {
            tenant_id: tenant,
            user_id: &user_text,
            agent_id: None,
            conversation_id: None,
            message_id: None,
            claim_text: sentence,
            category: ClaimCategory::Nutrition,
            status: ClaimStatus::Contradicted,
            evidence_strength: EvidenceStrength::Strong,
            confidence: 0.9,
            layer_fired: VerdictLayer::Evidence,
            explanation: None,
            evidence_refs: None,
            transport_policy: TransportPolicy::AnyTransport,
        })
        .await
        .unwrap();
    repos
        .memory
        .upsert_user_fact(&UpsertUserFactParams {
            tenant_id: tenant,
            user_id: &user_text,
            agent_id: None,
            scope: MemoryScope::User,
            kind: FactKind::Goal,
            pillar: None,
            predicate_code: PredicateCode::States,
            object: "a marathon",
            confidence: 0.9,
            source: FactSource::Conversation,
            valid_until: None,
            source_msg_id: None,
            transport_policy: TransportPolicy::AnyTransport,
        })
        .await
        .unwrap();
    let now_ms = chrono::Utc::now().timestamp_millis();
    repos
        .memory_extraction_jobs
        .record_extraction_run(&ExtractionRunRecord {
            tenant_id: tenant,
            outcome: ExtractionRunOutcome::Persisted,
            facts_persisted: 1,
            finished_at_ms: now_ms,
        })
        .await
        .unwrap();
    repos
        .memory_extraction_jobs
        .record_extraction_job(&MemoryExtractionJobRow {
            id: Uuid::new_v4().to_string(),
            tenant_id: tenant,
            usage_tenant_id: None,
            payload: "{}".to_owned(),
            created_at_ms: now_ms - 3_600_000,
            leased_until_ms: 0,
            attempts: 1,
            lease_token: None,
        })
        .await
        .unwrap();
}

fn cookie_routes(resources: &Arc<ServerContext>) -> Router {
    let context = AdminApiContext::new(AdminApiContextInit {
        database: resources.agent.database.clone(),
        repos: resources.common.repos.clone(),
        jwt_secret: resources.auth.admin_jwt_secret.to_string(),
        auth_manager: resources.auth.auth_manager.clone(),
        jwks_manager: resources.auth.jwks_manager.clone(),
        admin_api_key_monthly_limit: STARTER_MONTHLY_LIMIT,
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
        app_behavior: resources.common.config.app_behavior.clone(),
    });
    AdminRoutes::cookie_admin_routes::<ServerContext>(context, resources)
}

const DAY: i64 = 86_400_000;
const HOME_SENTENCE: &str = "Home tenant says creatine is a steroid every single day";
const FOREIGN_SENTENCE: &str = "Foreign tenant says water is optional on a long ride";

async fn fixture() -> Fixture {
    common::init_server_config();
    let resources = create_test_server_resources().await.unwrap();
    let (plain_admin, home, plain_auth) = operator(&resources, UserRole::Admin).await;
    let (foreign_admin, foreign, _) = operator(&resources, UserRole::Admin).await;
    let (_, super_home, super_auth) = operator(&resources, UserRole::SuperAdmin).await;
    let (_, _, athlete_auth) = operator(&resources, UserRole::User).await;
    seed_rows(&resources, home, plain_admin.id, HOME_SENTENCE).await;
    seed_rows(&resources, foreign, foreign_admin.id, FOREIGN_SENTENCE).await;

    let router = cookie_routes(&resources)
        .merge(LlmConsumptionRoutes::routes(resources.clone()))
        .merge(DashboardRoutes::routes::<ServerContext>().with_state(resources.clone()))
        .merge(WebAdminRoutes::routes(resources.web_admin_context()));
    Fixture {
        resources,
        router,
        plain_admin,
        plain_auth,
        super_auth,
        super_home,
        athlete_auth,
        home,
        foreign,
    }
}

impl Fixture {
    async fn get(&self, path: &str, auth: &str, tenant: Option<&str>) -> (u16, Value) {
        let query = tenant.map_or_else(String::new, |t| format!("?tenant_id={t}"));
        let response = AxumTestRequest::get(&format!("{path}{query}"))
            .header("authorization", auth)
            .send(self.router.clone())
            .await;
        let status = response.status();
        let body = if status == 200 {
            response.json()
        } else {
            Value::Null
        };
        (status, body)
    }

    async fn audit_rows(&self) -> Vec<AuditEventRecord> {
        self.resources
            .common
            .repos
            .admin_audit
            .list(
                &AuditEventFilter {
                    tenant_id: None,
                    before: None,
                    before_id: None,
                    action_prefix: Some("claim_sentences.".to_owned()),
                    outcome: None,
                    actor_user_id: None,
                },
                50,
            )
            .await
            .unwrap()
    }
}

fn tenant_scope(tenant: TenantId) -> Value {
    serde_json::json!({ "kind": "tenant", "tenant_id": tenant.to_string() })
}

#[tokio::test]
#[serial]
async fn a_plain_admin_asking_for_another_tenant_is_refused_on_every_screen() {
    let fx = fixture().await;
    let foreign = fx.foreign.to_string();
    for endpoint in endpoints() {
        let (status, _) = fx.get(endpoint.path, &fx.plain_auth, Some(&foreign)).await;
        assert_eq!(
            status, 403,
            "bug: {} let a plain admin read another tenant",
            endpoint.path
        );
    }
}

#[tokio::test]
#[serial]
async fn a_plain_admin_asking_for_all_tenants_is_refused_on_every_screen() {
    let fx = fixture().await;
    for endpoint in endpoints() {
        let (status, _) = fx.get(endpoint.path, &fx.plain_auth, Some("all")).await;
        assert_eq!(
            status, 403,
            "bug: {} let a plain admin read every tenant",
            endpoint.path
        );
    }
}

#[tokio::test]
#[serial]
async fn a_plain_admin_reads_exactly_their_own_tenant_on_every_screen() {
    let fx = fixture().await;
    let home = fx.home.to_string();
    for requested in [None, Some(home.as_str())] {
        for endpoint in endpoints() {
            let (status, body) = fx.get(endpoint.path, &fx.plain_auth, requested).await;
            assert_eq!(
                status, 200,
                "bug: {} refused the admin's own tenant",
                endpoint.path
            );
            assert_eq!(
                (endpoint.read)(&body),
                1,
                "bug: {} must count the home tenant's one row alone",
                endpoint.path
            );
            assert_eq!(
                body["scope"],
                tenant_scope(fx.home),
                "bug: {} must state the tenant it read",
                endpoint.path
            );
        }
    }
}

#[tokio::test]
#[serial]
async fn a_super_admin_reads_the_whole_platform_by_default_and_by_name() {
    let fx = fixture().await;
    for requested in [None, Some("all")] {
        for endpoint in endpoints() {
            let (status, body) = fx.get(endpoint.path, &fx.super_auth, requested).await;
            assert_eq!(status, 200, "bug: {} refused a super admin", endpoint.path);
            assert_eq!(
                (endpoint.read)(&body),
                2,
                "bug: {} must sum both seeded tenants",
                endpoint.path
            );
            assert_eq!(
                body["scope"],
                serde_json::json!({ "kind": "all_tenants" }),
                "bug: {} must state it read every tenant",
                endpoint.path
            );
        }
    }
}

#[tokio::test]
#[serial]
async fn a_super_admin_picking_one_tenant_reads_that_tenant_alone() {
    let fx = fixture().await;
    let foreign = fx.foreign.to_string();
    for endpoint in endpoints() {
        let (status, body) = fx.get(endpoint.path, &fx.super_auth, Some(&foreign)).await;
        assert_eq!(status, 200, "bug: {} refused a super admin", endpoint.path);
        assert_eq!(
            (endpoint.read)(&body),
            1,
            "bug: {} must narrow to the picked tenant",
            endpoint.path
        );
        assert_eq!(
            body["scope"],
            tenant_scope(fx.foreign),
            "bug: {} must state the picked tenant",
            endpoint.path
        );
    }
}

#[tokio::test]
#[serial]
async fn reply_sentences_never_cross_to_a_plain_admin() {
    let fx = fixture().await;
    let (_, own) = fx
        .get("/api/admin/myth-busting/summary", &fx.plain_auth, None)
        .await;
    let quoted = own["recent_claims"].to_string();
    assert!(
        quoted.contains("Home tenant"),
        "bug: own sentence missing: {quoted}"
    );
    assert!(
        !quoted.contains("Foreign tenant"),
        "bug: another tenant's sentence reached a plain admin: {quoted}"
    );

    let (_, platform) = fx
        .get("/api/admin/myth-busting/summary", &fx.super_auth, None)
        .await;
    let quoted = platform["recent_claims"].to_string();
    assert!(
        quoted.contains("Home tenant") && quoted.contains("Foreign tenant"),
        "bug: the platform view must quote both tenants: {quoted}"
    );
}

#[tokio::test]
#[serial]
async fn a_session_claiming_a_tenant_its_user_left_reads_nothing() {
    let fx = fixture().await;
    // A signed token outlives a membership: this one names a tenant the admin
    // does not belong to, as a token issued before a removal would.
    let stale = fx
        .resources
        .auth
        .auth_manager
        .generate_token_with_tenant(
            &fx.plain_admin,
            &fx.resources.auth.jwks_manager,
            Some(fx.foreign.to_string()),
        )
        .unwrap();
    let auth = format!("Bearer {stale}");
    for endpoint in endpoints() {
        let (status, _) = fx.get(endpoint.path, &auth, None).await;
        assert!(
            status == 401 || status == 403,
            "bug: {} served {status} on a stale tenant claim",
            endpoint.path
        );
    }
}

#[tokio::test]
#[serial]
async fn only_a_super_admin_lists_the_tenants_to_pick_from() {
    let fx = fixture().await;
    let (status, _) = fx.get("/api/admin/tenants", &fx.plain_auth, None).await;
    assert_eq!(
        status, 403,
        "bug: a plain admin must not learn other tenants' names"
    );
    let (status, _) = fx.get("/api/admin/tenants", &fx.athlete_auth, None).await;
    assert_eq!(status, 403, "bug: a plain user must not reach the list");

    let (status, body) = fx.get("/api/admin/tenants", &fx.super_auth, None).await;
    assert_eq!(
        status, 200,
        "bug: a super admin needs the list to pick from"
    );
    let ids: Vec<&str> = body["tenants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tenant| tenant["id"].as_str().unwrap())
        .collect();
    for tenant in [fx.home, fx.foreign, fx.super_home] {
        assert!(
            ids.contains(&tenant.to_string().as_str()),
            "bug: {tenant} missing from {ids:?}"
        );
    }
}

#[tokio::test]
#[serial]
async fn a_plain_user_stays_refused_on_the_console_screens() {
    let fx = fixture().await;
    for path in [
        "/api/admin/memory/worker-metrics",
        "/api/admin/myth-busting/summary",
        "/api/admin/agent-grading/summary",
        "/api/admin/analytics/recent-activity",
    ] {
        let (status, _) = fx.get(path, &fx.athlete_auth, None).await;
        assert_eq!(status, 403, "bug: {path} is for admins");
    }
}

#[tokio::test]
#[serial]
async fn a_tenant_member_is_refused_the_tenants_llm_series() {
    let fx = fixture().await;
    let (_, member_auth) = member_of(&fx.resources, fx.home, UserRole::User).await;
    for path in ["/api/dashboard/analytics", "/admin/usage/llm-consumption"] {
        let (status, _) = fx.get(path, &member_auth, None).await;
        assert_eq!(
            status, 403,
            "bug: a plain member must not read {path}, and a 401 would sign them out"
        );
    }
}

#[tokio::test]
#[serial]
async fn a_promoted_member_reads_their_tenants_llm_series() {
    let fx = fixture().await;
    let (_, admin_auth) = member_of(&fx.resources, fx.home, UserRole::Admin).await;
    for path in ["/api/dashboard/analytics", "/admin/usage/llm-consumption"] {
        let (status, body) = fx.get(path, &admin_auth, None).await;
        assert_eq!(status, 200, "bug: a platform admin must keep {path}");
        assert_eq!(
            body["scope"],
            tenant_scope(fx.home),
            "bug: {path} stays on their tenant"
        );
    }
}

/// What a user-usage response totals: tokens by model, tokens by day, cost.
fn usage_totals(body: &Value) -> (i64, i64, f64) {
    let sum = |key: &str, rows: &str| -> i64 {
        body[rows]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row[key].as_i64().unwrap())
            .sum()
    };
    (
        sum("total_tokens", "by_model"),
        sum("tokens", "daily"),
        body["total_cost_usd"].as_f64().unwrap(),
    )
}

#[tokio::test]
#[serial]
async fn a_plain_admin_reads_a_shared_users_usage_in_their_own_tenant_alone() {
    let fx = fixture().await;
    let (shared, _) = member_of(&fx.resources, fx.home, UserRole::User).await;
    let repos = &fx.resources.common.repos;
    repos
        .users
        .update_tenant_id(shared.id, fx.foreign)
        .await
        .unwrap();
    // A model priced call by call, so the cost read goes through the scope too.
    let (provider, model) = ("anthropic", "claude-haiku-5-5");
    insert_llm_call(
        &fx.resources,
        fx.home,
        shared.id,
        provider,
        model,
        1_000,
        1_000,
    )
    .await;
    insert_llm_call(
        &fx.resources,
        fx.foreign,
        shared.id,
        provider,
        model,
        3_000,
        3_000,
    )
    .await;
    let path = format!("/api/admin/users/{}/usage", shared.id);

    let (status, body) = fx.get(&path, &fx.plain_auth, None).await;
    assert_eq!(status, 200, "bug: the user shares the admin's tenant");
    let (by_model, daily, plain_cost) = usage_totals(&body);
    assert_eq!(by_model, 1_000, "bug: another tenant's tokens by model");
    assert_eq!(daily, 1_000, "bug: another tenant's tokens by day");

    let (status, body) = fx.get(&path, &fx.super_auth, None).await;
    assert_eq!(status, 200);
    let (by_model, daily, all_cost) = usage_totals(&body);
    assert_eq!(by_model, 4_000, "bug: a super admin reads every tenant");
    assert_eq!(daily, 4_000, "bug: a super admin reads every tenant");
    assert!(
        plain_cost > 0.0 && (all_cost / plain_cost - 4.0).abs() < 1e-9,
        "bug: the plain admin's cost must price their tenant's calls alone: \
         {plain_cost} of {all_cost}"
    );
}

#[tokio::test]
#[serial]
async fn a_plain_admin_is_refused_a_user_outside_the_tenant_they_act_in() {
    let fx = fixture().await;
    // An admin acting in `home` who also belongs to another workspace, whose
    // owner is in no tenant the admin acts in.
    let (target, other, _) = operator(&fx.resources, UserRole::Admin).await;
    let (admin, admin_auth) = member_of(&fx.resources, fx.home, UserRole::Admin).await;
    fx.resources
        .common
        .repos
        .users
        .update_tenant_id(admin.id, other)
        .await
        .unwrap();
    let target = target.id;

    let (status, _) = fx
        .get(
            &format!("/api/admin/users/{target}/usage"),
            &admin_auth,
            None,
        )
        .await;
    assert_eq!(status, 403, "bug: the read would come back as empty rows");
}

#[tokio::test]
#[serial]
async fn reading_another_tenants_sentences_writes_one_audit_row() {
    let fx = fixture().await;
    let path = "/api/admin/myth-busting/summary";

    fx.get(path, &fx.plain_auth, None).await;
    assert!(
        fx.audit_rows().await.is_empty(),
        "bug: reading one's own tenant is not a cross-tenant access"
    );
    fx.get(path, &fx.super_auth, Some(&fx.super_home.to_string()))
        .await;
    assert!(
        fx.audit_rows().await.is_empty(),
        "bug: a super admin reading their own tenant is not a cross-tenant access"
    );

    fx.get(path, &fx.super_auth, None).await;
    let rows = fx.audit_rows().await;
    assert_eq!(rows.len(), 1, "bug: the platform read must be on the trail");
    assert_eq!(rows[0].action, "claim_sentences.read_cross_tenant");
    assert_eq!(rows[0].outcome, "success");
    assert_eq!(
        rows[0].tenant_id, None,
        "bug: a plain admin of the reader's tenant would read this row"
    );
    assert_eq!(
        rows[0].metadata["target"]["scope"],
        serde_json::json!({ "kind": "all_tenants" }),
        "bug: the row must say what was read"
    );

    fx.get(path, &fx.super_auth, Some(&fx.foreign.to_string()))
        .await;
    let rows = fx.audit_rows().await;
    assert_eq!(
        rows.len(),
        2,
        "bug: reading one other tenant must be on the trail"
    );
    assert_eq!(
        rows[0].metadata["target"]["scope"],
        tenant_scope(fx.foreign)
    );
    assert_eq!(rows[0].metadata["route"], path);
    assert_eq!(
        rows[0].tenant_id, None,
        "bug: this row names another tenant, so it belongs to no tenant's trail"
    );
}

#[tokio::test]
#[serial]
async fn the_verdict_list_is_on_the_trail_when_it_reads_past_the_callers_tenant() {
    let fx = fixture().await;
    let path = "/api/admin/claim-verdicts";

    fx.get(path, &fx.plain_auth, None).await;
    assert!(
        fx.audit_rows().await.is_empty(),
        "bug: reading one's own tenant is not a cross-tenant access"
    );

    let (_, body) = fx.get(path, &fx.super_auth, None).await;
    let quoted: Vec<&str> = body["verdicts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["claim_text"].as_str().unwrap())
        .collect();
    assert!(
        quoted.contains(&HOME_SENTENCE) && quoted.contains(&FOREIGN_SENTENCE),
        "bug: every tenant's rows must come back, got {quoted:?}"
    );
    let rows = fx.audit_rows().await;
    assert_eq!(rows.len(), 1, "bug: the platform read must be on the trail");
    assert_eq!(rows[0].action, "claim_sentences.read_cross_tenant");
    assert_eq!(rows[0].metadata["route"], path);
}

#[tokio::test]
#[serial]
async fn a_verdict_filter_narrows_the_platform_read_as_it_does_a_tenants() {
    let fx = fixture().await;
    for (filter, expected) in [("status=contradicted", 2), ("status=supported", 0)] {
        let response = AxumTestRequest::get(&format!("/api/admin/claim-verdicts?{filter}"))
            .header("authorization", &fx.super_auth)
            .send(fx.router.clone())
            .await;
        assert_eq!(response.status(), 200);
        let body: Value = response.json();
        assert_eq!(body["total"], expected, "bug: {filter} over every tenant");
    }
}

#[tokio::test]
#[serial]
async fn a_refused_read_writes_no_cross_tenant_audit_row() {
    let fx = fixture().await;
    fx.get(
        "/api/admin/myth-busting/summary",
        &fx.plain_auth,
        Some("all"),
    )
    .await;
    assert!(
        fx.audit_rows().await.is_empty(),
        "bug: a refused read reached no sentence"
    );
}

#[tokio::test]
#[serial]
async fn the_run_ledger_counts_by_outcome_window_and_tenant() {
    let fx = fixture().await;
    let jobs = &fx.resources.common.repos.memory_extraction_jobs;
    let now_ms = chrono::Utc::now().timestamp_millis();
    let run = |outcome, facts, age: i64| ExtractionRunRecord {
        tenant_id: fx.home,
        outcome,
        facts_persisted: facts,
        finished_at_ms: now_ms - age,
    };
    // The fixture already seeded one fresh persisted run with one fact.
    for record in [
        run(ExtractionRunOutcome::Empty, 0, 1000),
        run(ExtractionRunOutcome::Failed, 0, 2000),
        run(ExtractionRunOutcome::Persisted, 3, 2 * DAY),
        run(ExtractionRunOutcome::Empty, 0, 40 * DAY),
    ] {
        jobs.record_extraction_run(&record).await.unwrap();
    }

    let owed_since = now_ms - 300_000;
    let stats = jobs
        .extraction_run_stats(fx.home.into(), now_ms, owed_since)
        .await
        .unwrap();
    assert_eq!(
        (
            stats.last_24h.persisted,
            stats.last_24h.empty,
            stats.last_24h.failed
        ),
        (1, 1, 1),
        "bug: the 24-hour window counts by outcome"
    );
    assert_eq!(stats.last_24h.facts_persisted, 1, "bug: 24-hour facts");
    assert_eq!(
        (
            stats.last_7d.persisted,
            stats.last_7d.empty,
            stats.last_7d.failed
        ),
        (2, 1, 1),
        "bug: the 40-day-old run is outside the 7-day window"
    );
    assert_eq!(stats.last_7d.facts_persisted, 4, "bug: 7-day facts");
    assert_eq!(
        stats.last_failure_at_ms,
        Some(now_ms - 2000),
        "bug: last failure"
    );
    assert_eq!(stats.backlog, 1, "bug: the hour-old job is owed");

    let fresh_job_only = jobs
        .extraction_run_stats(fx.home.into(), now_ms, now_ms - 7_200_000)
        .await
        .unwrap();
    assert_eq!(
        fresh_job_only.backlog, 0,
        "bug: a job younger than the lease bound is not backlog"
    );

    let other = jobs
        .extraction_run_stats(fx.foreign.into(), now_ms, owed_since)
        .await
        .unwrap();
    assert_eq!(
        (other.last_7d.empty, other.last_7d.failed),
        (0, 0),
        "bug: one tenant's runs must not reach another's stats"
    );

    let all = jobs
        .extraction_run_stats(AdminScope::AllTenants, now_ms, owed_since)
        .await
        .unwrap();
    assert_eq!(
        all.last_24h.persisted, 2,
        "bug: the platform sums both tenants"
    );
    assert_eq!(
        all.backlog, 2,
        "bug: the platform backlog sums both tenants"
    );

    let pruned = jobs.prune_extraction_runs(now_ms - 30 * DAY).await.unwrap();
    assert_eq!(pruned, 1, "bug: only the 40-day-old row is past retention");
}

/// The console refetches its screens every two minutes. Each refetch of the
/// same screen at the same scope within the hour is one access already on the
/// trail; another scope, or another screen, is a new one.
#[tokio::test]
#[serial]
async fn a_repeated_cross_tenant_read_within_the_hour_writes_one_audit_row() {
    let fx = fixture().await;
    let summary = "/api/admin/myth-busting/summary";

    fx.get(summary, &fx.super_auth, None).await;
    fx.get(summary, &fx.super_auth, None).await;
    let rows = fx.audit_rows().await;
    assert_eq!(
        rows.len(),
        1,
        "bug: a refetch of the same read within the hour flooded the trail"
    );

    fx.get(summary, &fx.super_auth, Some(&fx.foreign.to_string()))
        .await;
    fx.get(summary, &fx.super_auth, Some(&fx.foreign.to_string()))
        .await;
    let rows = fx.audit_rows().await;
    assert_eq!(rows.len(), 2, "bug: another scope is another access");
    assert_eq!(
        rows[0].metadata["target"]["scope"],
        tenant_scope(fx.foreign)
    );

    fx.get("/api/admin/claim-verdicts", &fx.super_auth, None)
        .await;
    let rows = fx.audit_rows().await;
    assert_eq!(rows.len(), 3, "bug: another screen is another access");
    assert_eq!(rows[0].metadata["route"], "/api/admin/claim-verdicts");
}

/// Every Engagement figure follows one scope: a plain admin's users, agents
/// and uses are their tenant's, a super admin's picked tenant is that tenant's,
/// and the platform read covers every user. The screen used to count every
/// tenant's users beside one tenant's agents.
#[tokio::test]
#[serial]
async fn every_engagement_figure_follows_the_one_scope() {
    let fx = fixture().await;
    let repos = &fx.resources.common.repos;
    let (member, _) = member_of(&fx.resources, fx.home, UserRole::User).await;
    let agent = repos
        .agents
        .create_system_agent(
            fx.plain_admin.id,
            fx.home,
            &CreateSystemAgentRequest {
                title: "Home agent".to_owned(),
                description: None,
                system_prompt: "You answer.".to_owned(),
                category: AgentCategory::Training,
                tags: vec![],
                sample_prompts: vec![],
                visibility: AgentVisibility::Tenant,
            },
        )
        .await
        .unwrap();
    assert!(repos
        .agents
        .record_usage(&agent.id.to_string(), member.id, fx.home)
        .await
        .unwrap());

    let (status, home) = fx.get("/api/admin/engagement", &fx.plain_auth, None).await;
    assert_eq!(status, 200, "bug: the plain admin reads their own tenant");
    assert_eq!(home["scope"], tenant_scope(fx.home));
    assert_eq!(
        home["users"]["total"], 2,
        "bug: the users are the tenant's two members, not the platform's: {home}"
    );
    assert_eq!(home["users"]["daily"], 2, "{home}");
    assert_eq!(home["total_agents"], 1, "{home}");
    assert_eq!(home["leaderboard"][0]["title"], "Home agent", "{home}");
    assert_eq!(home["leaderboard"][0]["use_count"], 1, "{home}");

    let foreign = fx.foreign.to_string();
    let (status, picked) = fx
        .get("/api/admin/engagement", &fx.super_auth, Some(&foreign))
        .await;
    assert_eq!(status, 200);
    assert_eq!(picked["scope"], tenant_scope(fx.foreign));
    assert_eq!(picked["users"]["total"], 1, "{picked}");
    assert_eq!(picked["total_agents"], 0, "{picked}");

    let (status, platform) = fx.get("/api/admin/engagement", &fx.super_auth, None).await;
    assert_eq!(status, 200);
    assert_eq!(
        platform["scope"],
        serde_json::json!({ "kind": "all_tenants" })
    );
    assert_eq!(
        platform["users"]["total"], 5,
        "every user: the four operators and the member: {platform}"
    );

    let (status, _) = fx
        .get("/api/admin/engagement", &fx.plain_auth, Some(&foreign))
        .await;
    assert_eq!(status, 403, "bug: a plain admin never reads another tenant");
}
