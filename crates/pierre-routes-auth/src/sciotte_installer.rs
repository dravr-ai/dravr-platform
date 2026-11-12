// ABOUTME: The super admin's session clone onto a test account: a sciotte login's store and warm-up, unannounced
// ABOUTME: No provider.connected event, since copying a session is not an athlete connecting
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use dravr_sciotte::models::AuthSession;
use pierre_core::errors::AppResult;
use pierre_core::models::TenantId;
use pierre_services::scrape_session::{persist_sciotte_session, ScrapeSessionInstaller};
use uuid::Uuid;

use crate::sciotte::warm_from_session;
use crate::AuthRoutesContext;

/// The super admin clone's install: the same store and warm-up as a login,
/// with no `provider.connected` event, since a super admin copying a session
/// onto a test account is not an athlete connecting and must not count as one.
#[async_trait::async_trait]
impl ScrapeSessionInstaller for AuthRoutesContext {
    async fn install_session(
        &self,
        user_id: Uuid,
        tenant_id: TenantId,
        backend: &str,
        session: &AuthSession,
    ) -> AppResult<()> {
        let stored = persist_sciotte_session(
            &self.repos,
            self.data.provider_purge()?,
            &self.cache,
            user_id,
            tenant_id,
            session,
            backend,
        )
        .await?;
        warm_from_session(
            self,
            user_id,
            tenant_id.as_uuid(),
            session,
            &stored,
            backend,
        )
        .await;
        Ok(())
    }
}
