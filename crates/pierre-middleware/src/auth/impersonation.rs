// ABOUTME: The check every impersonation token passes on each request: its session, the admin who opened it, and its target
// ABOUTME: Split from auth.rs; refuses an ended session, an admin no longer in standing, a target now an admin; serves a request its session
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_auth::auth::{Claims, ImpersonationGrant};
use pierre_core::auth_header::extract_bearer_token;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_logging::record_in_scope;
use tracing::{info, warn};
use uuid::Uuid;

use super::{token_user_id, McpAuthMiddleware};

impl McpAuthMiddleware {
    /// The admin's support session a request's session credential acts in:
    /// `auth_value` (an `Authorization` value, as
    /// [`crate::extractors::request_credential`] picks it) when it is a valid
    /// impersonation token whose session is live, else `None`.
    ///
    /// For a layer that serves the whole request inside the session
    /// (`pierre_tool_runtime::context::SUPPORT_SESSION`) before the handler
    /// authenticates it. It writes nothing — no usage row, no activity, no
    /// budget — so the handler's own authentication stays the request's one
    /// admission, and it derives the session as that admission does
    /// ([`Self::live_impersonation`]). A credential that is no session, or
    /// that the admission would refuse, is `None`: the handler refuses it, and
    /// a route that needs no credential is served as usual.
    ///
    /// # Errors
    ///
    /// A database error from the session's lookup: the request is refused
    /// rather than served as no support session when one may be live.
    pub async fn support_session_of(
        &self,
        auth_value: &str,
    ) -> AppResult<Option<ImpersonationGrant>> {
        let Ok(token) = extract_bearer_token(auth_value) else {
            return Ok(None);
        };
        let Ok(claims) = self
            .auth_manager
            .validate_token_detailed(token, &self.jwks_manager)
        else {
            return Ok(None);
        };
        if claims.impersonation_session_id.is_none() {
            return Ok(None);
        }
        let Ok(user_id) = token_user_id(&claims) else {
            return Ok(None);
        };
        match self.live_impersonation(&claims, user_id).await {
            Err(refused) if refused.code == ErrorCode::AuthInvalid => Ok(None),
            served => served,
        }
    }

    /// The support session a valid token for `user_id` acts in, or `None`
    /// when it is no impersonation token.
    ///
    /// An impersonation token acts only while its session is live: ending the
    /// session revokes it, rather than leaving it good until its hour runs
    /// out ([`Self::require_live_impersonation`]).
    ///
    /// # Errors
    ///
    /// `AuthInvalid` when the token names a session that is no longer live,
    /// and a database error when a lookup fails.
    pub(super) async fn live_impersonation(
        &self,
        claims: &Claims,
        user_id: Uuid,
    ) -> AppResult<Option<ImpersonationGrant>> {
        let Some(session_id) = claims.impersonation_session_id.as_deref() else {
            return Ok(None);
        };
        Ok(Some(ImpersonationGrant {
            session_id: session_id.to_owned(),
            impersonator_id: self.require_live_impersonation(session_id, user_id).await?,
            expires_at: claims.exp,
        }))
    }

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
    async fn require_live_impersonation(&self, session_id: &str, user_id: Uuid) -> AppResult<Uuid> {
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
