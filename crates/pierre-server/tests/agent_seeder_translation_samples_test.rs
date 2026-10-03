// ABOUTME: The agent seeder stores each locale's Example Inputs as that locale's sample prompts
// ABOUTME: French guillemets are stripped, and the store overlay hands a French athlete the French examples

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs)]

//! carnet#735: an agent's welcome offers its first examples as starter
//! questions. The seeder used to carry only the English `## Example Inputs`
//! into `agents.sample_prompts`; a translation's own examples never reached
//! the database, so a French athlete would read English questions under a
//! French greeting.

mod common;

use std::fs;
use std::path::Path;
use std::sync::Arc;

use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_seeders::agents::{self, SeedArgs};
use pierre_seeders::bootstrap::{self, SeedArgs as BootstrapArgs};
use tempfile::TempDir;
use uuid::Uuid;

const SLUG: &str = "fuelling-sample-agent";

async fn seeded_repos() -> (Arc<RepositoryRegistry>, Uuid, TenantId) {
    let database = common::create_test_database().await.unwrap();
    let repos = Arc::clone(database.repositories());
    bootstrap::run(
        BootstrapArgs {
            admin_email: "operator@dravr.ai".to_owned(),
            admin_password: "OperatorPass123!".to_owned(),
        },
        &repos,
    )
    .await
    .unwrap();
    let admin = repos.seeder.seed_get_admin_user().await.unwrap().unwrap();
    let tenant = repos
        .seeder
        .seed_get_user_tenant(admin.id)
        .await
        .unwrap()
        .unwrap();
    (repos, admin.id, TenantId::parse_str(&tenant).unwrap())
}

fn write(dir: &Path, file: &str, title: &str, purpose: &str, examples: &str) {
    fs::write(
        dir.join(file),
        format!(
            "---\nname: {SLUG}\ntitle: {title}\ncategory: nutrition\ntags: [fuelling]\n\
             prerequisites:\n  providers: []\n  min_activities: 0\n  activity_types: []\n\
             visibility: tenant\n---\n\n## Purpose\n{purpose}\n\n\
             ## Instructions\nYou are the fuelling agent.\n\n## Example Inputs\n{examples}\n"
        ),
    )
    .unwrap();
}

#[tokio::test]
async fn a_translation_s_example_inputs_become_its_sample_prompts() {
    let (repos, admin, tenant) = seeded_repos().await;
    let checkout = TempDir::new().unwrap();
    let dir = checkout.path().join("nutrition").join(SLUG);
    fs::create_dir_all(&dir).unwrap();
    write(
        &dir,
        "en.md",
        "Fuelling Agent",
        "Fuelling specialist.",
        "- \"What should I eat before a 6am run?\"\n- \"How do I carb load?\"",
    );
    write(
        &dir,
        "fr.md",
        "Agent Ravitaillement",
        "Spécialiste du ravitaillement.",
        "- « Que manger avant une course à 6 h du matin ? »\n- « Comment faire ma charge glucidique ? »",
    );

    agents::run(
        SeedArgs {
            agents_dir: checkout.path().to_path_buf(),
            dry_run: false,
        },
        &repos,
    )
    .await
    .unwrap();

    let (id, _) = repos
        .seeder
        .seed_find_agent_by_slug(SLUG, &tenant.to_string())
        .await
        .unwrap()
        .expect("the seeder wrote the agent");
    let canonical = repos
        .agents
        .get_by_id(&id, admin, tenant)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        canonical.sample_prompts,
        vec!["What should I eat before a 6am run?", "How do I carb load?"],
        "the English examples stay the canonical samples"
    );

    let mut french = [canonical];
    repos
        .agents
        .translate_agents(&mut french, "fr")
        .await
        .unwrap();
    assert_eq!(french[0].title, "Agent Ravitaillement");
    assert_eq!(
        french[0].sample_prompts,
        vec![
            "Que manger avant une course à 6 h du matin ?",
            "Comment faire ma charge glucidique ?"
        ],
        "the French examples, guillemets and their spaces stripped"
    );
}
