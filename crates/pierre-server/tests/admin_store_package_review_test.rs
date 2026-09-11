// ABOUTME: The admin store review surfaces carry each agent's package — every artefact with what the review could not resolve
// ABOUTME: /api/admin/store/review-queue and /published over HTTP, against the live catalogue and evidence registries
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

mod common;
mod helpers;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::Result;
use axum::Router;
use common::{create_test_server_resources, generate_test_token};
use helpers::axum_test::AxumTestRequest;
use pierre_core::models::agents::{AgentCategory, AgentVisibility, CreateSystemAgentRequest};
use pierre_core::models::{ArtefactKind, PackageArtefact, Tenant, TenantId, User, UserStatus};
use pierre_core::permissions::UserRole;
use pierre_evals::evidence_retriever::EvidenceCorpus;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_agents::build_agents_admin_router;
use pierre_tool_runtime::runtime::ToolRuntime;
use serde_json::Value;
use serial_test::serial;
use uuid::Uuid;

const CATALOGUE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../training_catalogue");

/// A router is consumed by each `send`, so build one per request.
fn router(resources: &Arc<ServerContext>) -> Router {
    Router::new().nest(
        "/api/admin",
        build_agents_admin_router::<ServerContext>().with_state(Arc::clone(resources)),
    )
}

/// An active admin with a tenant, returning the id, the tenant and `"Bearer <jwt>"`.
async fn admin(resources: &Arc<ServerContext>) -> Result<(Uuid, TenantId, String)> {
    let email = format!("store-admin-{}@example.com", Uuid::new_v4());
    let password_hash = bcrypt::hash("password123", bcrypt::DEFAULT_COST)?;
    let mut user = User::new(email.clone(), password_hash, Some("Store Admin".to_owned()));
    user.is_admin = true;
    user.role = UserRole::Admin;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(chrono::Utc::now());
    let user_id = user.id;
    resources.common.repos.users.create(&user).await?;

    let tenant_id = TenantId::generate();
    let tenant = Tenant {
        id: tenant_id,
        name: format!("Tenant for {email}"),
        slug: format!("tenant-{tenant_id}"),
        domain: None,
        plan: "starter".to_owned(),
        owner_user_id: user_id,
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    resources.common.repos.tenants.create(&tenant).await?;
    resources
        .common
        .repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await?;
    let token = generate_test_token(resources, &user).await;
    Ok((user_id, tenant_id, format!("Bearer {token}")))
}

/// An agent submitted for review, carrying a house flavour and one workout,
/// both citing propositions.
async fn pending_agent_with_package(
    resources: &Arc<ServerContext>,
    author: Uuid,
    tenant: TenantId,
) -> Result<String> {
    let repos = &resources.common.repos;
    let agent = repos
        .agents
        .create_system_agent(
            author,
            tenant,
            &CreateSystemAgentRequest {
                title: "House Polarized".to_owned(),
                description: Some("The house way".to_owned()),
                system_prompt: "You coach the house way.".to_owned(),
                category: AgentCategory::Training,
                tags: vec!["polarized".to_owned()],
                visibility: AgentVisibility::Tenant,
                sample_prompts: vec![],
            },
        )
        .await?;
    let id = agent.id.to_string();
    repos
        .store_listings
        .submit_for_review(&id, author, tenant)
        .await?;

    let flavour =
        fs::read_to_string(Path::new(CATALOGUE_DIR).join("flavours/polarized-classic.yaml"))?
            .replace("id: polarized-classic", "id: house-polarized");
    let workout = fs::read_to_string(Path::new(CATALOGUE_DIR).join("workouts/threshold_4x8.toml"))?
        .replace("slug = \"threshold_4x8\"", "slug = \"house_4x8\"")
        .replace(
            "id = \"00000000-0000-0000-0000-000000000002\"",
            &format!("id = \"{}\"", Uuid::new_v4()),
        );
    let artefacts = vec![
        PackageArtefact::parse(ArtefactKind::Flavour, &flavour)?,
        PackageArtefact::parse(ArtefactKind::Workout, &workout)?,
    ];
    repos
        .agent_artefacts
        .replace_agent_artefacts(&tenant.to_string(), &id, &artefacts)
        .await?;
    Ok(id)
}

fn agent_named<'a>(body: &'a Value, id: &str) -> &'a Value {
    body["agents"]
        .as_array()
        .and_then(|agents| agents.iter().find(|c| c["id"] == id))
        .unwrap_or_else(|| panic!("coach {id} listed: {body}"))
}

#[tokio::test]
#[serial]
async fn the_review_queue_lists_the_package_and_what_it_cannot_resolve() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (admin_id, tenant, auth) = admin(&resources).await?;
    let agent_id = pending_agent_with_package(&resources, admin_id, tenant).await?;

    // The evidence registry holds one proposition, so every other path the
    // package cites is unresolved and the reviewer sees each one.
    resources.evidence_registry().update(
        "training_prescription",
        "some-other-proposition",
        EvidenceCorpus::default(),
        "0".repeat(64),
    );

    let response = AxumTestRequest::get("/api/admin/store/review-queue")
        .header("Authorization", &auth)
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json();
    let agent = agent_named(&body, &agent_id);
    let package = &agent["package"];
    assert_eq!(package["evidence_checked"], Value::Bool(true), "{package}");
    let artefacts = package["artefacts"].as_array().expect("artefacts");
    assert_eq!(artefacts.len(), 2, "{package}");

    let flavour = &artefacts[0];
    assert_eq!(flavour["kind"], "flavour");
    assert_eq!(flavour["slug"], "house-polarized");
    assert_eq!(flavour["cites_no_evidence"], Value::Bool(false));
    let unresolved = flavour["unresolved"].as_array().expect("unresolved");
    assert!(
        !unresolved.is_empty(),
        "the flavour's citations are absent from the registry"
    );
    assert!(unresolved.iter().all(|u| u["reference"]
        .as_str()
        .unwrap_or("")
        .starts_with("evidence/sports_science/")));
    assert!(unresolved[0]["key"]
        .as_str()
        .unwrap_or("")
        .starts_with("evidence_refs["));

    let workout = &artefacts[1];
    assert_eq!(workout["kind"], "workout");
    assert_eq!(workout["slug"], "house_4x8");
    assert_eq!(
        workout["unresolved"].as_array().map(Vec::len),
        Some(1),
        "{workout}"
    );
    Ok(())
}

#[tokio::test]
#[serial]
async fn an_empty_evidence_registry_is_reported_rather_than_read_as_clean() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (admin_id, tenant, auth) = admin(&resources).await?;
    let agent_id = pending_agent_with_package(&resources, admin_id, tenant).await?;

    let response = AxumTestRequest::get("/api/admin/store/review-queue")
        .header("Authorization", &auth)
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json();
    let package = &agent_named(&body, &agent_id)["package"];
    assert_eq!(package["evidence_checked"], Value::Bool(false), "{package}");
    assert!(package["artefacts"]
        .as_array()
        .expect("artefacts")
        .iter()
        .all(|a| a["unresolved"].as_array().is_some_and(Vec::is_empty)));
    Ok(())
}

#[tokio::test]
#[serial]
async fn a_published_agent_without_a_package_lists_an_empty_one() -> Result<()> {
    let resources = create_test_server_resources().await?;
    let (admin_id, tenant, auth) = admin(&resources).await?;
    let agent_id = pending_agent_with_package(&resources, admin_id, tenant).await?;
    let repos = &resources.common.repos;
    repos
        .agent_artefacts
        .replace_agent_artefacts(&tenant.to_string(), &agent_id, &[])
        .await?;
    repos
        .store_listings
        .approve_agent(&agent_id, tenant, Some(admin_id))
        .await?;

    let response = AxumTestRequest::get("/api/admin/store/published")
        .header("Authorization", &auth)
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 200);
    let body: Value = response.json();
    let package = &agent_named(&body, &agent_id)["package"];
    assert_eq!(package["artefacts"], Value::Array(vec![]), "{package}");
    Ok(())
}
