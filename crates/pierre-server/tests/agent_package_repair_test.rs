// ABOUTME: The boot repair of stored package flavours — quality purposes closed at every ladder level with no hard session
// ABOUTME: A stale catalogue-style row comes back byte for byte as the current file; other breakage is reported, never guessed at

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! Every catalogue flavour once opened `tempo` at p1 with a hard-session cap
//! of 0, and a coach package seeded under that kernel stored the same line.
//! The kernel now refuses it, so the resolver skipped the row and the coach's
//! athletes lost the house flavour. These tests pin the rewrite on the text
//! itself and end to end against the database, including the resolver
//! lending the repaired flavour again.

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Result;
use dravr_contremaitre::training;
use pierre_core::models::agents::AgentCategory;
use pierre_core::models::periodization::{Flavour, ReadinessLevel, WorkoutPurpose};
use pierre_core::models::{sha256_hex, ArtefactKind, PackageArtefact, TenantId};
use pierre_core::permissions::scopes::OAuthScope;
use pierre_services::agent_package::load_agent_package;
use pierre_services::agent_package_repair::{close_quality_at_cap_zero, repair_stored_flavours};
use pierre_tool_runtime::protocols::UniversalToolExecutor;
use uuid::Uuid;

mod common;
mod helpers;

use helpers::agent_fixtures::publish_catalogue_agent_tagged;

/// The pinned catalogue's polarized flavour, as contremaitre ships it now.
fn current_polarized() -> &'static str {
    training::FLAVOURS
        .iter()
        .find(|(stem, _)| *stem == "polarized-classic")
        .map(|(_, text)| *text)
        .expect("the pinned catalogue carries polarized-classic")
}

/// The same file as the older catalogue shipped it: `tempo` open at p1,
/// whose cap is 0.
fn stale_polarized() -> String {
    let current = current_polarized();
    let p1 = current
        .find("  p1:\n    purposes: [")
        .expect("the ladder writes p1 as a one-line list");
    let insert_at = p1 + "  p1:\n    purposes: [".len();
    format!(
        "{}tempo, {}",
        current.get(..insert_at).unwrap(),
        current.get(insert_at..).unwrap()
    )
}

#[test]
fn the_kernel_refuses_a_stale_ladder() {
    let refused = Flavour::from_yaml(&stale_polarized()).expect_err("tempo at a cap-0 level");
    assert!(
        refused
            .to_string()
            .contains("quality purpose \"tempo\" is not allowed at p1"),
        "{refused}"
    );
}

#[test]
fn a_stale_one_line_ladder_comes_back_as_the_current_file_byte_for_byte() {
    let closed = close_quality_at_cap_zero(&stale_polarized()).expect("the ladder is repairable");
    assert_eq!(
        closed.removed,
        BTreeMap::from([(ReadinessLevel::P1, vec![WorkoutPurpose::Tempo])]),
        "only what the kernel forbids is taken out"
    );
    assert!(
        closed.layout_kept,
        "the list sat on one line, so it is edited in place"
    );
    assert_eq!(
        closed.content,
        current_polarized(),
        "comments, key order and every other byte are the author's"
    );
    Flavour::from_yaml(&closed.content).expect("the kernel accepts the rewrite");
}

#[test]
fn a_ladder_written_as_block_lists_is_reserialised_and_still_parses() {
    let doc: serde_yaml::Value = serde_yaml::from_str(&stale_polarized()).unwrap();
    let block_style = serde_yaml::to_string(&doc).unwrap();
    assert!(
        Flavour::from_yaml(&block_style).is_err(),
        "the block-style copy carries the same stale rule"
    );

    let closed = close_quality_at_cap_zero(&block_style).expect("the ladder is repairable");
    assert!(
        !closed.layout_kept,
        "a block list is not edited by line; the document is written back"
    );
    let flavour = Flavour::from_yaml(&closed.content).expect("the kernel accepts the rewrite");
    let p1 = &flavour.readiness_substitution[&ReadinessLevel::P1];
    assert!(!p1.purposes.contains(&WorkoutPurpose::Tempo), "{p1:?}");
    assert!(p1.purposes.contains(&WorkoutPurpose::Endurance), "{p1:?}");
    let p2 = &flavour.readiness_substitution[&ReadinessLevel::P2];
    assert!(
        p2.purposes.contains(&WorkoutPurpose::Tempo),
        "a level with a hard session keeps its quality purposes: {p2:?}"
    );
}

#[test]
fn every_closed_level_loses_every_quality_purpose_it_opens() {
    // A p0 that opens strides (neuromuscular) and a p1 that opens tempo and
    // uphill work: three quality purposes across two closed levels.
    let text = stale_polarized().replacen(
        "  p0:\n    purposes: [recovery]",
        "  p0:\n    purposes: [recovery, neuromuscular]",
        1,
    );
    let text = text.replacen("purposes: [tempo, ", "purposes: [tempo, uphill, ", 1);
    let closed = close_quality_at_cap_zero(&text).expect("the ladder is repairable");
    assert_eq!(
        closed.removed,
        BTreeMap::from([
            (ReadinessLevel::P0, vec![WorkoutPurpose::Neuromuscular]),
            (
                ReadinessLevel::P1,
                vec![WorkoutPurpose::Tempo, WorkoutPurpose::Uphill]
            ),
        ])
    );
    assert_eq!(closed.content, current_polarized());
}

#[test]
fn a_flavour_refused_for_another_reason_is_not_guessed_at() {
    let broken = stale_polarized().replacen("family: polarized", "family: not-a-family", 1);
    assert!(
        close_quality_at_cap_zero(&broken).is_none(),
        "closing the ladder does not make an unknown family parse"
    );
}

#[test]
fn a_flavour_the_kernel_accepts_needs_nothing() {
    assert!(close_quality_at_cap_zero(current_polarized()).is_none());
}

async fn create_executor() -> Result<Arc<UniversalToolExecutor>> {
    common::init_server_config();
    common::init_test_http_clients();
    let resources = common::create_test_server_resources().await?;
    Ok(Arc::new(
        UniversalToolExecutor::new(resources).with_scopes(OAuthScope::self_grant()),
    ))
}

/// A stored artefact written as a seeder under the older kernel wrote it —
/// `PackageArtefact::parse` would refuse it today, which is the point.
fn stored(kind: ArtefactKind, slug: &str, content: String) -> PackageArtefact {
    PackageArtefact {
        kind,
        slug: slug.to_owned(),
        sha256: sha256_hex(&content),
        content,
    }
}

#[tokio::test]
async fn the_boot_repair_rewrites_the_stored_row_and_the_package_lends_it_again() -> Result<()> {
    let executor = create_executor().await?;
    let email = format!("package_repair_{}@example.com", Uuid::new_v4());
    let (user_id, _user) =
        common::create_test_user_with_email(executor.resources.database(), &email).await?;
    let repos = executor.resources.repos();
    let tenant = repos
        .tenants
        .get_all()
        .await?
        .into_iter()
        .find(|t| t.owner_user_id == user_id)
        .ok_or_else(|| anyhow::anyhow!("user should have a tenant"))?
        .id;
    let tenant = TenantId::parse_str(&tenant.to_string())?;
    let tenant_id = tenant.to_string();

    let stale = stale_polarized().replace("id: polarized-classic", "id: house-polarized");
    let repaired_agent = publish_catalogue_agent_tagged(
        repos,
        user_id,
        tenant,
        "House Polarized",
        "You coach the house way.",
        AgentCategory::Training,
        vec!["polarized".to_owned()],
    )
    .await
    .to_string();
    repos
        .agent_artefacts
        .replace_agent_artefacts(
            &tenant_id,
            &repaired_agent,
            &[stored(
                ArtefactKind::Flavour,
                "house-polarized",
                stale.clone(),
            )],
        )
        .await?;

    let broken = stale.replacen("family: polarized", "family: not-a-family", 1);
    let broken_agent = publish_catalogue_agent_tagged(
        repos,
        user_id,
        tenant,
        "House Broken",
        "You coach the broken way.",
        AgentCategory::Training,
        vec!["polarized".to_owned()],
    )
    .await
    .to_string();
    repos
        .agent_artefacts
        .replace_agent_artefacts(
            &tenant_id,
            &broken_agent,
            &[stored(
                ArtefactKind::Flavour,
                "house-polarized",
                broken.clone(),
            )],
        )
        .await?;

    // Before the repair the resolver skips the stale row: the package lends
    // no flavour.
    let before = load_agent_package(repos, tenant, user_id, Some(repaired_agent.as_str())).await?;
    assert!(
        before.and_then(|p| p.flavour).is_none(),
        "the kernel refuses the stored ladder"
    );

    let report = repair_stored_flavours(repos).await?;
    let repaired_label = format!("{repaired_agent}/house-polarized");
    assert!(
        report.repaired.contains(&repaired_label),
        "the stale row is rewritten: {report:?}"
    );
    assert!(
        report
            .unrepaired
            .iter()
            .any(|u| u.starts_with(&format!("{broken_agent}/house-polarized: "))),
        "the row broken for another reason is named: {report:?}"
    );

    let rows = repos
        .agent_artefacts
        .list_agent_artefacts(&tenant_id, &repaired_agent)
        .await?;
    assert_eq!(rows.len(), 1);
    let expected = current_polarized().replace("id: polarized-classic", "id: house-polarized");
    assert_eq!(
        rows[0].content, expected,
        "the row reads as the current file"
    );
    assert_eq!(
        rows[0].sha256,
        sha256_hex(&expected),
        "and carries its digest"
    );
    assert_eq!(rows[0].slug, "house-polarized", "the row keeps its key");

    let after = load_agent_package(repos, tenant, user_id, Some(repaired_agent.as_str()))
        .await?
        .expect("the published package is lent");
    assert_eq!(
        after.flavour.map(|f| f.id),
        Some("house-polarized".to_owned()),
        "the resolver lends the house flavour again"
    );

    let untouched = repos
        .agent_artefacts
        .list_agent_artefacts(&tenant_id, &broken_agent)
        .await?;
    assert_eq!(untouched[0].content, broken, "left exactly as stored");

    let again = repair_stored_flavours(repos).await?;
    assert!(
        !again.repaired.contains(&repaired_label),
        "a flavour that parses is never rewritten: {again:?}"
    );
    Ok(())
}
