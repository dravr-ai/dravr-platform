// ABOUTME: Gives a seeded athlete Garmin's AI consent, the way the connect and the settings toggle record it
// ABOUTME: For suites whose model reads must see Garmin data: until it is given, every model read withholds it (carnet#1157)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
// Only the suites a feature set compiles call it.
#![allow(dead_code)]

use pierre_core::constants::oauth_providers::{NoticeKind, GARMIN};
use pierre_core::models::TenantId;
use pierre_database::RepositoryRegistry;
use pierre_services::provider_notice::{consent_state, grant_consent};
use uuid::Uuid;

/// The athlete gives Garmin's AI consent in `tenant`, through the service the
/// connect and the settings toggle share, so a model reads their Garmin data
/// (its `garmin` and `sciotte_garmin` backends alike).
pub(crate) async fn give_garmin_ai_consent(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    tenant: TenantId,
) {
    grant_consent(
        repos,
        tenant.as_uuid(),
        user_id,
        GARMIN,
        NoticeKind::AiConsent,
        "Garmin",
    )
    .await
    .unwrap();
    assert_eq!(
        consent_state(
            repos,
            tenant.as_uuid(),
            user_id,
            GARMIN,
            NoticeKind::AiConsent
        )
        .await,
        Some(true),
        "the Garmin AI consent the fixture gave is not standing"
    );
}
