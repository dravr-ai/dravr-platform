// ABOUTME: The super-admin fuel-product review routes — queue, stored label image, approve with set/clear edits, reject, live catalogue
// ABOUTME: Plain admins are refused, each approval outcome has its own status, and the image is served only under its stored, allowlisted type (carnet#155)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Drives the routes through the real cookie-admin group (bearer header, the
//! way the console's session reaches the same middleware), so the mount, the
//! middleware's role-derived token and the JSON bodies are all exercised. The
//! two cases a console session cannot produce — a service token naming no
//! operator, an image stored under a type the console must not be served —
//! call the handlers directly.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

use crate::common;
use crate::helpers;

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Result;
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::{Extension, Json};
use chrono::Utc;
use helpers::axum_test::AxumTestRequest;
use pierre_core::admin::models::{AdminPermissions, ValidatedAdminToken};
use pierre_core::errors::ErrorCode;
use pierre_core::models::{
    normalize_gtin, proposal_source_hash, FactSource, FieldProvenance, FuelFacts, FuelForm,
    NewFuelProposal, ProposalSource, ProposalSourceImage, Tenant, TenantId, User, UserStatus,
};
use pierre_core::permissions::UserRole;
use pierre_database::RepositoryRegistry;
use pierre_mcp_server::constants::system_config::STARTER_MONTHLY_LIMIT;
use pierre_mcp_server::mcp::resources::ServerContext;
use pierre_routes_admin::handlers::fuel_products::{
    handle_approve_fuel_proposal, handle_get_fuel_proposal_image, ApproveFuelProposalRequest,
    MAX_REJECT_NOTE_CHARS,
};
use pierre_routes_admin::{AdminApiContext, AdminApiContextInit, AdminRoutes};
use serde_json::{json, Value};
use serial_test::serial;
use uuid::Uuid;

const PANEL: &[u8] = b"\x89PNG\r\n\x1a\n label panel bytes";

/// A consistent gel label: 40 g serving, 25 g carbs, 1:0.8.
fn gel_facts() -> FuelFacts {
    FuelFacts {
        serving_label: Some("1 gel (40 g)".to_owned()),
        serving_g: Some(40.0),
        energy_kcal: Some(100.0),
        carbs_g: Some(25.0),
        sugars_g: Some(12.0),
        sodium_mg: Some(34.0),
        carbs_g_per_100g: Some(62.5),
        sodium_mg_per_100g: Some(85.0),
        salt_g_per_100g: Some(0.21),
        glucose_fructose_ratio: Some(0.8),
        ..FuelFacts::default()
    }
}

/// A proposal with a distinct name and source, read off a PNG label by OCR.
fn proposal(mime: &str) -> NewFuelProposal {
    let url = format!("https://shop.example/{}", Uuid::new_v4());
    let mut provenance = BTreeMap::new();
    for field in ["carbs_g", "energy_kcal", "glucose_fructose_ratio"] {
        provenance.insert(
            field.to_owned(),
            FieldProvenance {
                source: FactSource::LabelOcr,
                source_url: Some(url.clone()),
                confidence: Some(0.9),
            },
        );
    }
    NewFuelProposal {
        product_id: None,
        brand: "Review Labs".to_owned(),
        name: format!("Gel {}", Uuid::new_v4()),
        variant: String::new(),
        form: FuelForm::Gel,
        gtin: None,
        source_kind: ProposalSource::ShopifyOcr,
        source_hash: Some(proposal_source_hash(Some(&url), PANEL)),
        source_url: Some(url),
        source_image: Some(ProposalSourceImage {
            mime: mime.to_owned(),
            bytes: PANEL.to_vec(),
        }),
        facts: gel_facts(),
        provenance,
        gate_flags: Vec::new(),
        proposed_by_user_id: None,
    }
}

async fn queue(repos: &RepositoryRegistry, proposal: &NewFuelProposal) -> Uuid {
    repos
        .fuel_catalogue
        .create_fuel_proposal(proposal)
        .await
        .unwrap()
}

/// An active user of `role` in its own tenant; returns its id and bearer
/// header.
async fn operator(resources: &Arc<ServerContext>, role: UserRole) -> (Uuid, String) {
    let email = format!("fuel-review-{}@example.com", Uuid::new_v4());
    let mut user = User::new(email.clone(), "not-a-real-hash".to_owned(), None);
    user.is_admin = role.is_admin_or_higher();
    user.role = role;
    user.user_status = UserStatus::Active;
    user.approved_by = Some(user.id);
    user.approved_at = Some(Utc::now());
    let user_id = user.id;
    let repos = &resources.common.repos;
    repos.users.create(&user).await.unwrap();
    let tenant_id = TenantId::generate();
    let now = Utc::now();
    repos
        .tenants
        .create(&Tenant {
            id: tenant_id,
            name: format!("Tenant for {email}"),
            slug: format!("tenant-{tenant_id}"),
            domain: None,
            plan: "starter".to_owned(),
            owner_user_id: user_id,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    repos
        .users
        .update_tenant_id(user_id, tenant_id)
        .await
        .unwrap();
    let token = common::generate_test_token(resources, &user).await;
    (user_id, format!("Bearer {token}"))
}

fn admin_context(resources: &Arc<ServerContext>) -> AdminApiContext {
    AdminApiContext::new(AdminApiContextInit {
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
    })
}

fn router(resources: &Arc<ServerContext>) -> axum::Router {
    AdminRoutes::cookie_admin_routes::<ServerContext>(admin_context(resources), resources)
}

async fn get(resources: &Arc<ServerContext>, auth: &str, uri: &str) -> (u16, Value) {
    let response = AxumTestRequest::get(uri)
        .header("authorization", auth)
        .send(router(resources))
        .await;
    let status = response.status();
    let body = if response.body_text().starts_with('{') {
        response.json()
    } else {
        Value::Null
    };
    (status, body)
}

async fn post(resources: &Arc<ServerContext>, auth: &str, uri: &str, body: &Value) -> (u16, Value) {
    let response = AxumTestRequest::post(uri)
        .header("authorization", auth)
        .json(body)
        .send(router(resources))
        .await;
    let status = response.status();
    // A body axum refuses before the handler runs answers in plain text.
    let body = if response.body_text().starts_with('{') {
        response.json()
    } else {
        Value::Null
    };
    (status, body)
}

fn base(id: Uuid) -> String {
    format!("/api/admin/fuel-products/proposals/{id}")
}

#[tokio::test]
#[serial]
async fn a_plain_admin_is_refused_on_every_review_route() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let (_, plain) = operator(&resources, UserRole::Admin).await;
    let (_, super_admin) = operator(&resources, UserRole::SuperAdmin).await;
    let id = queue(&resources.common.repos, &proposal("image/png")).await;

    for uri in [
        "/api/admin/fuel-products".to_owned(),
        "/api/admin/fuel-products/proposals".to_owned(),
        base(id),
        format!("{}/source-image", base(id)),
    ] {
        let (status, _) = get(&resources, &plain, &uri).await;
        assert_eq!(status, 403, "GET {uri}");
        let (status, _) = get(&resources, &super_admin, &uri).await;
        assert_eq!(status, 200, "GET {uri}");
    }
    let (status, _) = post(
        &resources,
        &plain,
        &format!("{}/approve", base(id)),
        &json!({}),
    )
    .await;
    assert_eq!(status, 403);
    let (status, _) = post(
        &resources,
        &plain,
        &format!("{}/reject", base(id)),
        &json!({ "reason": "illegible" }),
    )
    .await;
    assert_eq!(status, 403);

    // Neither refused write touched the proposal.
    let (_, detail) = get(&resources, &super_admin, &base(id)).await;
    assert_eq!(detail["proposal"]["status"], "pending");
    Ok(())
}

#[tokio::test]
#[serial]
async fn the_queue_lists_proposals_and_serves_the_stored_image() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let (_, auth) = operator(&resources, UserRole::SuperAdmin).await;
    let repos = &resources.common.repos;
    let png = queue(repos, &proposal("image/png")).await;
    let mut no_image = proposal("image/png");
    no_image.source_image = None;
    no_image.source_hash = None;
    let bare = queue(repos, &no_image).await;

    let (status, list) = get(&resources, &auth, "/api/admin/fuel-products/proposals").await;
    assert_eq!(status, 200);
    assert_eq!(list["status"], "pending");
    let rows = list["proposals"].as_array().unwrap();
    let row = rows.iter().find(|r| r["id"] == png.to_string()).unwrap();
    assert_eq!(row["has_source_image"], true);
    assert_eq!(row["source_kind"], "shopify_ocr");
    assert_eq!(row["facts"]["carbs_g"], 25.0);
    assert_eq!(row["provenance"]["carbs_g"]["source"], "label_ocr");

    let (status, _) = get(
        &resources,
        &auth,
        "/api/admin/fuel-products/proposals?status=withdrawn",
    )
    .await;
    assert_eq!(status, 400, "an unknown status is refused");
    let (status, approved) = get(
        &resources,
        &auth,
        "/api/admin/fuel-products/proposals?status=approved&limit=500",
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(approved["limit"], 100, "the page is clamped");

    let (status, detail) = get(&resources, &auth, &base(png)).await;
    assert_eq!(status, 200);
    assert_eq!(detail["source_image_bytes"], PANEL.len());

    let response = AxumTestRequest::get(&format!("{}/source-image", base(png)))
        .header("authorization", &auth)
        .send(router(&resources))
        .await;
    assert_eq!(response.status(), 200);
    assert_eq!(response.header("content-type"), Some("image/png"));
    assert_eq!(response.header("x-content-type-options"), Some("nosniff"));
    assert_eq!(response.header("cache-control"), Some("no-store"));
    assert_eq!(response.bytes(), PANEL);

    let (status, _) = get(&resources, &auth, &format!("{}/source-image", base(bare))).await;
    assert_eq!(status, 404, "no image stored");
    let (status, _) = get(&resources, &auth, &base(Uuid::new_v4())).await;
    assert_eq!(status, 404);
    let (status, _) = get(
        &resources,
        &auth,
        "/api/admin/fuel-products/proposals/not-a-uuid",
    )
    .await;
    assert_eq!(status, 400);
    Ok(())
}

#[tokio::test]
#[serial]
async fn edit_then_approve_sets_and_clears_facts_as_admin_edits() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let (reviewer, auth) = operator(&resources, UserRole::SuperAdmin).await;
    let id = queue(&resources.common.repos, &proposal("image/png")).await;

    // Energy corrected, the ratio OCR invented cleared, carbs restated as
    // printed: only the first two are edits.
    let (status, body) = post(
        &resources,
        &auth,
        &format!("{}/approve", base(id)),
        &json!({ "overrides": {
            "energy_kcal": 110.0,
            "glucose_fructose_ratio": null,
            "carbs_g": 25.0,
        }}),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["outcome"], "approved");
    let current = &body["product"]["current"];
    assert_eq!(current["facts"]["energy_kcal"], 110.0);
    assert!(current["facts"].get("glucose_fructose_ratio").is_none());
    assert_eq!(current["provenance"]["energy_kcal"]["source"], "admin_edit");
    assert_eq!(
        current["provenance"]["glucose_fructose_ratio"]["source"],
        "admin_edit"
    );
    assert_eq!(current["provenance"]["carbs_g"]["source"], "label_ocr");
    assert_eq!(current["approved_by_user_id"], reviewer.to_string());
    let product_id = body["product"]["product"]["id"].clone();

    // The live catalogue lists it with its one version.
    let (status, catalogue) = get(&resources, &auth, "/api/admin/fuel-products?limit=100").await;
    assert_eq!(status, 200);
    let listed = catalogue["products"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["product"]["id"] == product_id)
        .unwrap();
    assert_eq!(listed["version_count"], 1);
    assert_eq!(listed["current"]["facts"]["energy_kcal"], 110.0);

    // Reviewed once: a second approval is refused and says why.
    let (status, again) = post(
        &resources,
        &auth,
        &format!("{}/approve", base(id)),
        &json!({}),
    )
    .await;
    assert_eq!(status, 410);
    assert_eq!(again["outcome"], "not_pending");
    assert_eq!(again["status"], "approved");

    // An override naming no fact is refused before anything is read.
    let other = queue(&resources.common.repos, &proposal("image/png")).await;
    let (status, _) = post(
        &resources,
        &auth,
        &format!("{}/approve", base(other)),
        &json!({ "overrides": { "carb_g": 1.0 } }),
    )
    .await;
    assert_eq!(status, 422, "an unknown field is refused by the extractor");
    let (_, detail) = get(&resources, &auth, &base(other)).await;
    assert_eq!(detail["proposal"]["status"], "pending");
    Ok(())
}

#[tokio::test]
#[serial]
async fn implausible_merged_facts_answer_422_with_their_codes() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let (_, auth) = operator(&resources, UserRole::SuperAdmin).await;
    let id = queue(&resources.common.repos, &proposal("image/png")).await;

    let (status, body) = post(
        &resources,
        &auth,
        &format!("{}/approve", base(id)),
        &json!({ "overrides": { "sugars_g": 30.0 } }),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert_eq!(body["outcome"], "implausible");
    let codes: Vec<&str> = body["violations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["code"].as_str().unwrap())
        .collect();
    assert!(codes.contains(&"sugars_exceed_carbs"), "{codes:?}");

    // Clearing the carbs a gel must state is just as implausible.
    let (status, body) = post(
        &resources,
        &auth,
        &format!("{}/approve", base(id)),
        &json!({ "overrides": { "carbs_g": null } }),
    )
    .await;
    assert_eq!(status, 422, "{body}");
    assert!(body["violations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v["code"] == "carbs_missing"));

    let (_, detail) = get(&resources, &auth, &base(id)).await;
    assert_eq!(detail["proposal"]["status"], "pending", "nothing written");
    Ok(())
}

#[tokio::test]
#[serial]
async fn reject_checks_its_reason_and_answers_each_outcome() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let (reviewer, auth) = operator(&resources, UserRole::SuperAdmin).await;
    let id = queue(&resources.common.repos, &proposal("image/png")).await;
    let reject = format!("{}/reject", base(id));

    let (status, _) = post(&resources, &auth, &reject, &json!({ "reason": "blurry" })).await;
    assert_eq!(status, 400, "a reason outside the CHECK list");
    let (status, _) = post(&resources, &auth, &reject, &json!({ "reason": "other" })).await;
    assert_eq!(status, 400, "`other` needs a note");
    let (status, _) = post(
        &resources,
        &auth,
        &reject,
        &json!({ "reason": "illegible", "note": "x".repeat(MAX_REJECT_NOTE_CHARS + 1) }),
    )
    .await;
    assert_eq!(status, 400, "an over-long note");

    let (status, body) = post(
        &resources,
        &auth,
        &reject,
        &json!({ "reason": "wrong_product", "note": "This is the bar's panel" }),
    )
    .await;
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["outcome"], "rejected");
    assert_eq!(body["proposal"]["status"], "rejected");
    assert_eq!(body["proposal"]["reject_reason"], "wrong_product");
    assert_eq!(body["proposal"]["reject_note"], "This is the bar's panel");
    assert_eq!(
        body["proposal"]["reviewed_by_user_id"],
        reviewer.to_string()
    );

    let (status, again) = post(
        &resources,
        &auth,
        &reject,
        &json!({ "reason": "duplicate" }),
    )
    .await;
    assert_eq!(status, 410);
    assert_eq!(again["status"], "rejected");
    let (status, missing) = post(
        &resources,
        &auth,
        &format!("{}/reject", base(Uuid::new_v4())),
        &json!({ "reason": "duplicate" }),
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(missing["outcome"], "not_found");

    let (_, rejected) = get(
        &resources,
        &auth,
        "/api/admin/fuel-products/proposals?status=rejected&limit=100",
    )
    .await;
    assert!(rejected["proposals"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["id"] == id.to_string()));
    Ok(())
}

/// A valid GTIN-14 unique to this run: 13 random digits and their check digit.
fn fresh_gtin() -> String {
    let body: String = Uuid::new_v4()
        .as_u128()
        .to_string()
        .chars()
        .take(13)
        .collect();
    (0..=9)
        .map(|d| format!("{body}{d}"))
        .find(|candidate| normalize_gtin(candidate).as_deref() == Some(candidate))
        .unwrap()
}

#[tokio::test]
#[serial]
async fn approve_answers_not_found_and_identity_conflict_apart() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let (_, auth) = operator(&resources, UserRole::SuperAdmin).await;
    let repos = &resources.common.repos;

    let (status, body) = post(
        &resources,
        &auth,
        &format!("{}/approve", base(Uuid::new_v4())),
        &json!({}),
    )
    .await;
    assert_eq!(status, 404);
    assert_eq!(body["outcome"], "not_found");

    // One product carries the barcode; a proposal naming another product
    // with that barcode collides.
    let gtin = fresh_gtin();
    let mut with_gtin = proposal("image/png");
    with_gtin.gtin = Some(gtin.clone());
    let first = queue(repos, &with_gtin).await;
    let (status, _) = post(
        &resources,
        &auth,
        &format!("{}/approve", base(first)),
        &json!({}),
    )
    .await;
    assert_eq!(status, 200);
    let named = proposal("image/png");
    let second = queue(repos, &named).await;
    let (status, _) = post(
        &resources,
        &auth,
        &format!("{}/approve", base(second)),
        &json!({}),
    )
    .await;
    assert_eq!(status, 200);
    let mut clash = proposal("image/png");
    clash.name.clone_from(&named.name);
    clash.gtin = Some(gtin);
    let clash_id = queue(repos, &clash).await;

    let (status, body) = post(
        &resources,
        &auth,
        &format!("{}/approve", base(clash_id)),
        &json!({}),
    )
    .await;
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["outcome"], "identity_conflict");
    assert!(body["product_id"].is_string());
    let (_, detail) = get(&resources, &auth, &base(clash_id)).await;
    assert_eq!(detail["proposal"]["status"], "pending");
    Ok(())
}

fn super_admin_service_token() -> ValidatedAdminToken {
    ValidatedAdminToken {
        token_id: format!("svc:{}", Uuid::new_v4()),
        service_name: "ingest".to_owned(),
        permissions: AdminPermissions::super_admin(),
        is_super_admin: true,
        tenant_id: None,
        user_info: None,
        operator_user_id: None,
    }
}

#[tokio::test]
#[serial]
async fn a_service_token_cannot_review_and_an_unservable_image_type_is_refused() -> Result<()> {
    let resources = common::create_test_server_resources().await?;
    let context = Arc::new(admin_context(&resources));
    let id = queue(&resources.common.repos, &proposal("image/png")).await;

    let error = handle_approve_fuel_proposal(
        State(Arc::clone(&context)),
        Extension(super_admin_service_token()),
        Path(id.to_string()),
        Json(ApproveFuelProposalRequest::default()),
    )
    .await
    .expect_err("a service token names no reviewer");
    assert_eq!(error.code, ErrorCode::PermissionDenied);

    let html = queue(&resources.common.repos, &proposal("text/html")).await;
    let response = handle_get_fuel_proposal_image(
        State(context),
        Extension(super_admin_service_token()),
        Path(html.to_string()),
    )
    .await
    .unwrap()
    .into_response();
    assert_eq!(response.status().as_u16(), 415);
    // Never sent under the stored type: the refusal is a JSON body.
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|t| t.to_str().ok())
        .unwrap_or_default();
    assert!(
        content_type.starts_with("application/json"),
        "{content_type}"
    );
    Ok(())
}
