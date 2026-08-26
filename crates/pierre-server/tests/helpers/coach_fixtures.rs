// ABOUTME: Catalogue-coach fixtures — publish a coach so it owns a @handle, then install it for an athlete
// ABOUTME: Shared by the @handle mention and /coach invite @handle tests so "installed" means one thing
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
#![allow(missing_docs, dead_code)]

use pierre_core::models::coaches::{
    Coach, CoachCategory, CoachVisibility, CreateSystemCoachRequest,
};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use uuid::Uuid;

/// Create a system coach carrying `system_prompt` and take it through review
/// to a published listing — the moment it is assigned its catalogue `@handle`,
/// derived from `title` ("Recovery Coach" → `recovery-coach`).
pub async fn publish_catalogue_coach(
    repos: &RepositoryRegistry,
    author_id: Uuid,
    tenant_id: TenantId,
    title: &str,
    system_prompt: &str,
) -> Uuid {
    let coach = repos
        .coaches
        .create_system_coach(
            author_id,
            tenant_id,
            &CreateSystemCoachRequest {
                title: title.to_owned(),
                description: Some(format!("Description for {title}")),
                system_prompt: system_prompt.to_owned(),
                category: CoachCategory::Training,
                tags: vec!["test".to_owned()],
                visibility: CoachVisibility::Tenant,
                sample_prompts: vec![],
            },
        )
        .await
        .unwrap();
    let id = coach.id.to_string();
    repos
        .store_listings
        .submit_for_review(&id, author_id, tenant_id)
        .await
        .unwrap();
    repos
        .store_listings
        .approve_coach(&id, tenant_id, Some(author_id))
        .await
        .unwrap();
    coach.id
}

/// Install a published coach on `user_id`'s coach list and return the copy.
///
/// The copy carries its origin's handle, which is what
/// `CoachesRepository::find_installed_by_handle` resolves for this athlete.
pub async fn install_catalogue_coach(
    repos: &RepositoryRegistry,
    origin: Uuid,
    user_id: Uuid,
    tenant_id: TenantId,
) -> Coach {
    repos
        .store_listings
        .install_from_store(&origin.to_string(), user_id, tenant_id)
        .await
        .unwrap()
}
