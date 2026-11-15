// ABOUTME: The check every impersonation token passes on each request: its session, the admin who opened it, and its target
// ABOUTME: Split from auth.rs; refuses a session that ended, an admin no longer in standing, and a target that became an admin
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::{AppError, AppResult};
use pierre_logging::record_in_scope;
use tracing::{info, warn};
use uuid::Uuid;

use super::McpAuthMiddleware;

impl McpAuthMiddleware {
    /// Refuse an impersonation token whose session was ended, that names a
    /// session of another user, whose admin is no longer one, or whose target
    /// has become an admin, and attribute the request to the admin acting
    /// through it.
    ///
    /// The token acts on the authority of the admin who opened the session,
    /// so that admin's row is read on every request: the session of an admin
    /// since demoted, suspended or deleted is refused whether or not anything
    /// ended it. The target's row is read with it: an admin is never
    /// impersonated, and that is refused when a session starts, so a target
    /// promoted under an open session stops being acted as on its next
    /// request, whichever writer promoted it.
    ///
    /// The impersonator is written to the request span and to an audit event
    /// by id, so a request made under impersonation is never recorded as the
    /// target user acting alone.
    ///
    /// # Errors
    /// Returns `AuthInvalid` when the session is unknown, ended, for another
    /// target, opened by an account that is no longer an active admin, or on
    /// an account that now holds the admin role, and a database error when a
    /// lookup fails.
    pub(super) async fn require_live_impersonation(
        &self,
        session_id: &str,
        user_id: Uuid,
    ) -> AppResult<Uuid> {
        let session = self
            .repos
            .impersonation
            .get_session(session_id)
            .await?
            .filter(|session| session.is_active && session.target_user_id == user_id)
            .ok_or_else(|| {
                warn!(
                    user_id = %user_id,
                    session_id = %session_id,
                    "Impersonation token refused: its session has ended"
                );
                AppError::auth_invalid("Impersonation session has ended")
            })?;
        // SECURITY: Global lookup — the admin acts across tenants by design
        let admin_in_standing = self
            .repos
            .users
            .get_global(session.impersonator_id)
            .await?
            .is_some_and(|admin| admin.is_active_admin());
        if !admin_in_standing {
            warn!(
                user_id = %user_id,
                session_id = %session_id,
                impersonator_id = %session.impersonator_id,
                "Impersonation token refused: the account that opened it is no longer an active admin"
            );
            return Err(AppError::auth_invalid("Impersonation session has ended"));
        }
        // SECURITY: Global lookup — the target can be in any tenant
        let target_is_admin = self
            .repos
            .users
            .get_global(user_id)
            .await?
            .is_some_and(|target| target.role.is_admin());
        if target_is_admin {
            warn!(
                user_id = %user_id,
                session_id = %session_id,
                impersonator_id = %session.impersonator_id,
                "Impersonation token refused: its target now holds the admin role"
            );
            return Err(AppError::auth_invalid("Impersonation session has ended"));
        }
        let impersonator_id = session.impersonator_id.to_string();
        record_in_scope("impersonator_id", &impersonator_id.as_str());
        info!(
            user_id = %user_id,
            impersonator_id = %impersonator_id,
            session_id = %session_id,
            "Request authenticated under impersonation"
        );
        Ok(session.impersonator_id)
    }
}
