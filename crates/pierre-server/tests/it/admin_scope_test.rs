// ABOUTME: Every admin screen scoped by carnet#1440, through its real route: an admin reads the platform, a tenant owner who is not one stays home
// ABOUTME: Seeds two tenants' rows and pins the refusals, the content each role reads, and the audit row a cross-tenant read writes

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// The role boundary these tests hold sat between a plain admin and a super
// admin. There is one admin role now (carnet#1567), so it sits between a user
// and an admin: an admin reads every tenant; a tenant owner who is not an
// admin is refused the console outright and stays on their own tenant on the
// analytics routes that admit them.

#![allow(clippy::unwrap_used)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::sync::Arc;

use axum::Router;
use common::{create_test_server_resources, generate_test_token};
use dravr_tronc::dates::Timestamp;
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

/// The scoped endpoints that admit a tenant's owner, or a member holding that
/// tenant's own admin membership, without the admin role: they keep user
/// auth, and bind such a caller to the tenant they act in. Every other scoped
/// endpoint is the console's, for admins only.
const OWNER_ENDPOINTS: [&str; 3] = [
    "/admin/usage/llm-consumption",
    "/admin/tool-usage",
    "/api/dashboard/analytics",
];

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
    /// The owner of `home`, who is not an admin.
    owner: User,
    owner_auth: String,
    /// An admin, owner of a third tenant with no rows at all.
    admin_auth: String,
    admin_home: TenantId,
    /// A plain `User`, owner of a fourth tenant with no rows at all.
    athlete_auth: String,
    home: TenantId,
    foreign: TenantId,
}

async fn admin(resources: &Arc<ServerContext>, role: UserRole) -> (User, TenantId, String) {
    let email = format!("scope-{}@example.com", Uuid::new_v4());
    let password_hash = bcrypt::hash("password123", 4).unwrap();
    let mut user = User::new(email.clone(), password_hash, Some("Admin".to_owned()));
    user.role = role;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(Timestamp::now());
    resources.common.repos.users.create(&user).await.unwrap();

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Workspace of {email}"),
        slug: format!("tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user.id,
        created_at: Timestamp::now(),
        updated_at: Timestamp::now(),
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
    user.role = role;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(Timestamp::now());
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
    let now_ms = Timestamp::now().epoch_millis();
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
    let (owner, home, owner_auth) = admin(&resources, UserRole::User).await;
    let (foreign_owner, foreign, _) = admin(&resources, UserRole::User).await;
    let (_, admin_home, admin_auth) = admin(&resources, UserRole::Admin).await;
    let (_, _, athlete_auth) = admin(&resources, UserRole::User).await;
    seed_rows(&resources, home, owner.id, HOME_SENTENCE).await;
    seed_rows(&resources, foreign, foreign_owner.id, FOREIGN_SENTENCE).await;

    let router = cookie_routes(&resources)
        .merge(LlmConsumptionRoutes::routes(resources.clone()))
        .merge(DashboardRoutes::routes::<ServerContext>().with_state(resources.clone()))
        .merge(WebAdminRoutes::routes(resources.web_admin_context()));
    Fixture {
        resources,
        router,
        owner,
        owner_auth,
        admin_auth,
        admin_home,
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
async fn a_tenant_owner_asking_for_another_tenant_is_refused_where_owners_are_admitted() {
    let fx = fixture().await;
    let foreign = fx.foreign.to_string();
    // Only the endpoints that admit an owner say anything about the tenant
    // asked for; the console refuses an owner whatever it asks
    // (`a_tenant_owner_reads_exactly_their_own_tenant_and_only_off_the_console`).
    for path in OWNER_ENDPOINTS {
        let (admitted, _) = fx.get(path, &fx.owner_auth, None).await;
        assert_eq!(
            admitted, 200,
            "{path} admits a tenant owner on its own tenant"
        );
        let (status, _) = fx.get(path, &fx.owner_auth, Some(&foreign)).await;
        assert_eq!(
            status, 403,
            "bug: {path} let a tenant owner who is not an admin read another tenant"
        );
    }
}

#[tokio::test]
#[serial]
async fn a_tenant_owner_asking_for_all_tenants_is_refused_where_owners_are_admitted() {
    let fx = fixture().await;
    for path in OWNER_ENDPOINTS {
        let (admitted, _) = fx.get(path, &fx.owner_auth, None).await;
        assert_eq!(
            admitted, 200,
            "{path} admits a tenant owner on its own tenant"
        );
        let (status, _) = fx.get(path, &fx.owner_auth, Some("all")).await;
        assert_eq!(
            status, 403,
            "bug: {path} let a tenant owner who is not an admin read every tenant"
        );
    }
}

#[tokio::test]
#[serial]
async fn a_tenant_owner_reads_exactly_their_own_tenant_and_only_off_the_console() {
    let fx = fixture().await;
    let home = fx.home.to_string();
    for requested in [None, Some(home.as_str())] {
        for endpoint in endpoints() {
            let (status, body) = fx.get(endpoint.path, &fx.owner_auth, requested).await;
            if !OWNER_ENDPOINTS.contains(&endpoint.path) {
                assert_eq!(
                    status, 403,
                    "bug: {} is the console's, and the console is for admins",
                    endpoint.path
                );
                continue;
            }
            assert_eq!(
                status, 200,
                "bug: {} refused the owner's own tenant",
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
async fn an_admin_reads_the_whole_platform_by_default_and_by_name() {
    let fx = fixture().await;
    for requested in [None, Some("all")] {
        for endpoint in endpoints() {
            let (status, body) = fx.get(endpoint.path, &fx.admin_auth, requested).await;
            assert_eq!(status, 200, "bug: {} refused an admin", endpoint.path);
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
async fn an_admin_picking_one_tenant_reads_that_tenant_alone() {
    let fx = fixture().await;
    let foreign = fx.foreign.to_string();
    for endpoint in endpoints() {
        let (status, body) = fx.get(endpoint.path, &fx.admin_auth, Some(&foreign)).await;
        assert_eq!(status, 200, "bug: {} refused an admin", endpoint.path);
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
async fn reply_sentences_reach_an_admin_and_never_a_tenant_owner() {
    let fx = fixture().await;
    // The sentences screen is the console's: a tenant owner who is not an
    // admin reads no sentence, not even their own tenant's.
    for requested in [None, Some("all")] {
        let response = AxumTestRequest::get(&format!(
            "/api/admin/claim-verdicts{}",
            requested.map_or_else(String::new, |t| format!("?tenant_id={t}"))
        ))
        .header("authorization", &fx.owner_auth)
        .send(fx.router.clone())
        .await;
        assert_eq!(response.status(), 403, "bug: the console is for admins");
        let refused = response.text();
        assert!(
            !refused.contains(HOME_SENTENCE) && !refused.contains(FOREIGN_SENTENCE),
            "bug: a refusal quoted a reply sentence: {refused}"
        );
    }

    // An admin picking one tenant reads that tenant's sentences alone.
    let (_, own) = fx
        .get(
            "/api/admin/claim-verdicts",
            &fx.admin_auth,
            Some(&fx.home.to_string()),
        )
        .await;
    let quoted = own["verdicts"].to_string();
    assert!(
        quoted.contains(HOME_SENTENCE),
        "bug: the picked tenant's sentence missing: {quoted}"
    );
    assert!(
        !quoted.contains(FOREIGN_SENTENCE),
        "bug: another tenant's sentence crossed into a one-tenant read: {quoted}"
    );

    let (_, platform) = fx
        .get("/api/admin/claim-verdicts", &fx.admin_auth, None)
        .await;
    let quoted = platform["verdicts"].to_string();
    assert!(
        quoted.contains(HOME_SENTENCE) && quoted.contains(FOREIGN_SENTENCE),
        "bug: the platform view must quote both tenants: {quoted}"
    );
}

/// A signed session outlives a membership. The endpoints that admit a tenant
/// owner bind the caller to the tenant the session names only after reading
/// the membership table, so a session naming a tenant its user is not in is
/// refused there, while the same owner's session naming their own tenant
/// reads it. The console's endpoints refuse the owner whatever it names.
#[tokio::test]
#[serial]
async fn a_session_claiming_a_tenant_its_user_left_reads_nothing() {
    let fx = fixture().await;
    let session_naming = |tenant: TenantId| {
        let token = fx
            .resources
            .auth
            .auth_manager
            .generate_token_with_tenant(
                &fx.owner,
                &fx.resources.auth.jwks_manager,
                Some(tenant.to_string()),
            )
            .unwrap();
        format!("Bearer {token}")
    };
    // This one names a tenant the owner does not belong to, as a session
    // issued before a removal would; the other names the owner's own.
    let stale = session_naming(fx.foreign);
    let current = session_naming(fx.home);
    for endpoint in endpoints() {
        let (stale_status, _) = fx.get(endpoint.path, &stale, None).await;
        let (current_status, _) = fx.get(endpoint.path, &current, None).await;
        if OWNER_ENDPOINTS.contains(&endpoint.path) {
            assert_eq!(
                current_status, 200,
                "bug: {} refused an owner whose session names their own tenant",
                endpoint.path
            );
            assert_eq!(
                stale_status, 401,
                "bug: {} served a session naming a tenant its user is not in",
                endpoint.path
            );
        } else {
            assert_eq!(
                (stale_status, current_status),
                (403, 403),
                "bug: {} is the console's, and the console is for admins",
                endpoint.path
            );
        }
    }
}

#[tokio::test]
#[serial]
async fn only_an_admin_lists_the_tenants_to_pick_from() {
    let fx = fixture().await;
    let (status, _) = fx.get("/api/admin/tenants", &fx.owner_auth, None).await;
    assert_eq!(
        status, 403,
        "bug: a tenant owner must not learn other tenants' names"
    );
    let (status, _) = fx.get("/api/admin/tenants", &fx.athlete_auth, None).await;
    assert_eq!(status, 403, "bug: a plain user must not reach the list");

    let (status, body) = fx.get("/api/admin/tenants", &fx.admin_auth, None).await;
    assert_eq!(status, 200, "bug: an admin needs the list to pick from");
    let ids: Vec<&str> = body["tenants"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tenant| tenant["id"].as_str().unwrap())
        .collect();
    for tenant in [fx.home, fx.foreign, fx.admin_home] {
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
        "/api/admin/claim-verdicts",
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
async fn a_member_promoted_to_admin_reads_the_llm_series_of_every_tenant() {
    let fx = fixture().await;
    // The same plain member the test above sees refused, once promoted: the
    // admin role, not a tenant role, is what admits them, and it reaches the
    // platform.
    let (_, admin_auth) = member_of(&fx.resources, fx.home, UserRole::Admin).await;
    for path in ["/api/dashboard/analytics", "/admin/usage/llm-consumption"] {
        let (status, body) = fx.get(path, &admin_auth, None).await;
        assert_eq!(status, 200, "bug: an admin must keep {path}");
        assert_eq!(
            body["scope"],
            serde_json::json!({ "kind": "all_tenants" }),
            "bug: {path} reads every tenant for an admin"
        );
        let home = fx.home.to_string();
        let (status, body) = fx.get(path, &admin_auth, Some(&home)).await;
        assert_eq!(status, 200);
        assert_eq!(
            body["scope"],
            tenant_scope(fx.home),
            "bug: {path} narrows to the tenant an admin picks"
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
async fn an_admin_reads_a_shared_users_usage_in_every_tenant_and_a_tenant_owner_none() {
    let fx = fixture().await;
    let (shared, _) = member_of(&fx.resources, fx.home, UserRole::User).await;
    let (single, _) = member_of(&fx.resources, fx.home, UserRole::User).await;
    let repos = &fx.resources.common.repos;
    repos
        .users
        .update_tenant_id(shared.id, fx.foreign)
        .await
        .unwrap();
    // A model priced call by call, so the cost read covers what the tokens do.
    let (provider, model) = ("anthropic", "claude-haiku-5-5");
    for (tenant, user, tokens) in [
        (fx.home, shared.id, 1_000),
        (fx.foreign, shared.id, 3_000),
        (fx.home, single.id, 1_000),
    ] {
        insert_llm_call(&fx.resources, tenant, user, provider, model, tokens, tokens).await;
    }
    let path = format!("/api/admin/users/{}/usage", shared.id);

    // The owner of a tenant the user belongs to is not an admin: per-user
    // financial data is not theirs to read, in their tenant or any other.
    let (status, _) = fx.get(&path, &fx.owner_auth, None).await;
    assert_eq!(status, 403, "bug: a tenant owner read a user's usage");

    let (status, body) = fx.get(&path, &fx.admin_auth, None).await;
    assert_eq!(status, 200);
    let (by_model, daily, all_cost) = usage_totals(&body);
    assert_eq!(by_model, 4_000, "bug: an admin reads every tenant");
    assert_eq!(daily, 4_000, "bug: an admin reads every tenant");

    // The cost prices the same calls the tokens count: four times the user
    // whose one call is a quarter of them.
    let (status, body) = fx
        .get(
            &format!("/api/admin/users/{}/usage", single.id),
            &fx.admin_auth,
            None,
        )
        .await;
    assert_eq!(status, 200);
    let (by_model, _, single_cost) = usage_totals(&body);
    assert_eq!(by_model, 1_000);
    assert!(
        single_cost > 0.0 && (all_cost / single_cost - 4.0).abs() < 1e-9,
        "bug: the cost must price every tenant's calls: {single_cost} of {all_cost}"
    );
}

#[tokio::test]
#[serial]
async fn a_user_outside_every_tenant_of_the_reader_is_an_admins_to_read_and_no_one_elses() {
    let fx = fixture().await;
    // A reader acting in `home` who also belongs to another workspace, and a
    // target who is in no tenant the reader belongs to.
    let (target, _, _) = admin(&fx.resources, UserRole::User).await;
    let (_, other, _) = admin(&fx.resources, UserRole::User).await;
    let target = target.id;
    let path = format!("/api/admin/users/{target}/usage");

    for role in [UserRole::User, UserRole::Admin] {
        let (reader, reader_auth) = member_of(&fx.resources, fx.home, role).await;
        fx.resources
            .common
            .repos
            .users
            .update_tenant_id(reader.id, other)
            .await
            .unwrap();
        let (status, _) = fx.get(&path, &reader_auth, None).await;
        if role.is_admin() {
            assert_eq!(status, 200, "bug: an admin reaches every user");
        } else {
            assert_eq!(status, 403, "bug: a user read another user's usage");
        }
    }

    // A user who does not exist is a 404 for the admin, never empty rows.
    let (status, _) = fx
        .get(
            &format!("/api/admin/users/{}/usage", Uuid::new_v4()),
            &fx.admin_auth,
            None,
        )
        .await;
    assert_eq!(status, 404, "bug: a missing user read as empty rows");
}

#[tokio::test]
#[serial]
async fn reading_another_tenants_sentences_writes_one_audit_row() {
    let fx = fixture().await;
    let path = "/api/admin/claim-verdicts";

    fx.get(path, &fx.owner_auth, None).await;
    assert!(
        fx.audit_rows().await.is_empty(),
        "bug: a read refused to a tenant owner reached no sentence"
    );
    fx.get(path, &fx.admin_auth, Some(&fx.admin_home.to_string()))
        .await;
    assert!(
        fx.audit_rows().await.is_empty(),
        "bug: an admin reading their own tenant is not a cross-tenant access"
    );

    fx.get(path, &fx.admin_auth, None).await;
    let rows = fx.audit_rows().await;
    assert_eq!(rows.len(), 1, "bug: the platform read must be on the trail");
    assert_eq!(rows[0].action, "claim_sentences.read_cross_tenant");
    assert_eq!(rows[0].outcome, "success");
    assert_eq!(
        rows[0].tenant_id, None,
        "bug: a token scoped to the reader's tenant would read this row"
    );
    assert_eq!(
        rows[0].metadata["target"]["scope"],
        serde_json::json!({ "kind": "all_tenants" }),
        "bug: the row must say what was read"
    );

    fx.get(path, &fx.admin_auth, Some(&fx.foreign.to_string()))
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

    fx.get(path, &fx.admin_auth, Some(&fx.admin_home.to_string()))
        .await;
    assert!(
        fx.audit_rows().await.is_empty(),
        "bug: reading one's own tenant is not a cross-tenant access"
    );

    let (_, body) = fx.get(path, &fx.admin_auth, None).await;
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
            .header("authorization", &fx.admin_auth)
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
    // The admin passes the console's session check, so this refusal is the
    // handler's own: the scope it was asked for names no tenant. An account
    // the session check stops never reaches the code that writes the row.
    let (status, _) = fx
        .get(
            "/api/admin/claim-verdicts",
            &fx.admin_auth,
            Some("not-a-tenant-id"),
        )
        .await;
    assert_eq!(
        status, 400,
        "bug: a scope that names no tenant is refused by the handler"
    );
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
    let now_ms = Timestamp::now().epoch_millis();
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
    let list = "/api/admin/claim-verdicts";

    fx.get(list, &fx.admin_auth, None).await;
    fx.get(list, &fx.admin_auth, None).await;
    let rows = fx.audit_rows().await;
    assert_eq!(
        rows.len(),
        1,
        "bug: a refetch of the same read within the hour flooded the trail"
    );

    fx.get(list, &fx.admin_auth, Some(&fx.foreign.to_string()))
        .await;
    fx.get(list, &fx.admin_auth, Some(&fx.foreign.to_string()))
        .await;
    let rows = fx.audit_rows().await;
    assert_eq!(rows.len(), 2, "bug: another scope is another access");
    assert_eq!(
        rows[0].metadata["target"]["scope"],
        tenant_scope(fx.foreign)
    );

    fx.get(
        "/api/admin/claim-verdicts/messages/no-such-message",
        &fx.admin_auth,
        None,
    )
    .await;
    let rows = fx.audit_rows().await;
    assert_eq!(rows.len(), 3, "bug: another screen is another access");
    assert_eq!(
        rows[0].metadata["route"],
        "/api/admin/claim-verdicts/messages/{message_id}"
    );
}

/// Every Engagement figure follows one scope: an admin's picked tenant is
/// that tenant's users, agents and uses, and the platform read covers every
/// user. The screen used to count every tenant's users beside one tenant's
/// agents. A tenant owner who is not an admin reads none of it.
#[tokio::test]
#[serial]
async fn every_engagement_figure_follows_the_one_scope() {
    let fx = fixture().await;
    let repos = &fx.resources.common.repos;
    let (member, _) = member_of(&fx.resources, fx.home, UserRole::User).await;
    let agent = repos
        .agents
        .create_system_agent(
            fx.owner.id,
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

    let (status, home) = fx
        .get(
            "/api/admin/engagement",
            &fx.admin_auth,
            Some(&fx.home.to_string()),
        )
        .await;
    assert_eq!(status, 200, "bug: an admin reads the tenant it picks");
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
        .get("/api/admin/engagement", &fx.admin_auth, Some(&foreign))
        .await;
    assert_eq!(status, 200);
    assert_eq!(picked["scope"], tenant_scope(fx.foreign));
    assert_eq!(picked["users"]["total"], 1, "{picked}");
    assert_eq!(picked["total_agents"], 0, "{picked}");

    let (status, platform) = fx.get("/api/admin/engagement", &fx.admin_auth, None).await;
    assert_eq!(status, 200);
    assert_eq!(
        platform["scope"],
        serde_json::json!({ "kind": "all_tenants" })
    );
    assert_eq!(
        platform["users"]["total"], 5,
        "every user: the four admins and the member: {platform}"
    );

    for requested in [None, Some(foreign.as_str())] {
        let (status, _) = fx
            .get("/api/admin/engagement", &fx.owner_auth, requested)
            .await;
        assert_eq!(
            status, 403,
            "bug: a tenant owner who is not an admin never reads the screen"
        );
    }
}
