// ABOUTME: The step-up approval of a one-off T2 question: a fresh sign-in on Dravr's own page, then a child grant
// ABOUTME: The child sits under the client's live grant, never edits it, and records owner_verified and its Host Profile basis
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Step-up (carnet#1544 slice 4b, owl PDPP §6.1, §7.6)
//!
//! A request whose details name a step-up preset (`one_off_question`) is
//! approved only by an athlete who signed in on Dravr's own hosted page
//! (`/oauth2/login`, password or Google verified by Dravr) at most
//! [`STEP_UP_MAX_AGE_SECS`] before they approve: the sign-in token's
//! `auth_time` says when. The web app's session cookie never satisfies it,
//! and neither does a sign-in older than that: the authorize step sends the
//! athlete to sign in again, and the consent submission refuses.
//!
//! The approval records a **child** of the client's live parent grant
//! ([`ChildGrant`]): the most recent live, non-child grant the athlete gave
//! the client in the tenant, whatever its scope. None: refused, "step-up
//! needs a live grant". The code is minted naming the parent, so the
//! client's token keeps the parent's reach plus the child's.
//!
//! The Host Profile basis is the client's CIMD declaration
//! (`mcp_host_profile`, `declared_cimd`): a client declaring no T2 host,
//! and every client registered by DCR, is refused. The per-request claim is
//! checked on every call besides (the stricter carrier wins), and the
//! athlete's `sensitive_t2` switch at every read.

use axum::http::{header, HeaderMap};
use dravr_tronc::dates::Timestamp;
use pierre_auth::oauth2_server::authorization_details::ResolvedDetails;
use pierre_auth::oauth2_server::cimd::host_profile_tier;
use pierre_auth::oauth2_server::models::OAuth2Error;
use pierre_auth::security::cookies::auth_cookie_name_on;
use pierre_core::models::{
    ChildGrant, GrantApproval, HostProfileBasis, OAuthClientGrant, StepUpTerms,
    OWNER_VERIFIED_ACR, STEP_UP_MAX_AGE_SECS,
};
use tracing::{error, info, warn};
use uuid::Uuid;

use super::session::SignInCookie;
use super::{OAuth2Context, OAuth2Routes};

/// The refusal of a step-up whose client holds no live grant to step up.
pub(super) const STEP_UP_NEEDS_A_GRANT: &str = "step-up needs a live grant";

/// The refusal of a step-up whose host declares no Host Profile T2.
const HOST_PROFILE_MISSING: &str =
    "this application declares no Host Profile T2, so it is never given health records";

/// The refusal of a step-up approved without a fresh sign-in on Dravr's page.
pub(super) const STEP_UP_SIGN_IN_STALE: &str =
    "approving health records needs a sign-in on Dravr within the last five minutes";

/// Whether `details` ask for a step-up: a one-off question.
pub(super) fn asks_step_up(details: Option<&ResolvedDetails>) -> bool {
    details.is_some_and(|details| details.preset().requires_step_up())
}

/// A step-up approval to record.
pub(super) struct StepUpApproval<'a> {
    /// The client approved
    pub(super) client_id: &'a str,
    /// The one-off details approved
    pub(super) details: &'a ResolvedDetails,
    /// The review digest the approval was given under
    pub(super) review_digest: String,
    /// The language of the consent text it was given under
    pub(super) consent_locale: String,
    /// The athlete's zone, which the window is frozen in
    pub(super) zone: dravr_tronc::dates::Zone,
    /// When the athlete signed in on the hosted page
    pub(super) auth_time: Timestamp,
}

impl OAuth2Routes {
    /// When the athlete signed in on Dravr's hosted page, if the sign-in
    /// cookie is the authorization server's own and that sign-in is at most
    /// [`STEP_UP_MAX_AGE_SECS`] old at `now`; `None` otherwise (the web
    /// app's cookie, a refreshed session, a sign-in too old).
    pub(super) async fn fresh_sign_in(
        headers: &HeaderMap,
        context: &OAuth2Context,
        now: Timestamp,
    ) -> Option<Timestamp> {
        let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
        let (cookie, token) = Self::extract_session_token(
            cookies,
            &Self::session_cookie_name(context),
            &auth_cookie_name_on(Self::cookies_secure(context)),
        )?;
        if cookie != SignInCookie::AuthorizationServer {
            return None;
        }
        let claims = Self::sign_in_claims(cookie, &token, context).await.ok()?;
        let auth_time = claims.auth_time?;
        let age = now.epoch_seconds() - auth_time;
        if !(0..=STEP_UP_MAX_AGE_SECS).contains(&age) {
            return None;
        }
        Timestamp::from_epoch_seconds(auth_time).ok()
    }

    /// Record `approval` as a child of the client's live grant for
    /// `user_id` in `tenant_id`. Returns the **parent's** id, the grant the
    /// code and its tokens name.
    ///
    /// # Errors
    /// `access_denied` when the sign-in is not fresh, the client holds no
    /// live grant, or declares no Host Profile T2; `server_error` when the
    /// grant cannot be read or written.
    pub(super) async fn record_step_up(
        context: &OAuth2Context,
        approval: StepUpApproval<'_>,
        user_id: Uuid,
        tenant_id: &str,
    ) -> Result<String, OAuth2Error> {
        let StepUpApproval {
            client_id,
            details,
            review_digest,
            consent_locale,
            zone,
            auth_time,
        } = approval;
        let now = Timestamp::now();
        if now.epoch_seconds() - auth_time.epoch_seconds() > STEP_UP_MAX_AGE_SECS {
            return Err(OAuth2Error::access_denied(STEP_UP_SIGN_IN_STALE));
        }
        let failed = |e: pierre_core::errors::AppError| {
            error!(user_id = %user_id, client_id, "Failed to record a step-up grant: {e}");
            OAuth2Error::server_error(super::consent::CONSENT_NOT_RECORDED)
        };
        let user = user_id.to_string();
        let parent = context
            .oauth2_server
            .list_client_grants(&user, tenant_id)
            .await
            .map_err(failed)?
            .into_iter()
            .filter(|grant| grant.client_id == client_id && grant.is_live_at(now))
            .max_by_key(|grant| grant.granted_at)
            .ok_or_else(|| OAuth2Error::access_denied(STEP_UP_NEEDS_A_GRANT))?;
        let host_tier = context
            .oauth2_server
            .client_host_profile(client_id)
            .await
            .map_err(failed)?
            .as_deref()
            .and_then(host_profile_tier)
            .filter(|tier| tier.eq_ignore_ascii_case("T2"))
            .ok_or_else(|| {
                warn!(client_id, "step-up refused: the client declares no Host Profile T2");
                OAuth2Error::access_denied(HOST_PROFILE_MISSING)
            })?;
        let (frozen, expires_at) = details.freeze(now, &zone).map_err(failed)?;
        let child = ChildGrant {
            grant: OAuthClientGrant {
                id: Uuid::new_v4().to_string(),
                user_id: user.clone(),
                tenant_id: tenant_id.to_owned(),
                client_id: client_id.to_owned(),
                scope: parent.scope.clone(),
                granted_at: now,
                revoked_at: None,
                consent_version: Some(review_digest),
                consent_locale: Some(consent_locale),
                authorization_details: Some(frozen.to_document().map_err(failed)?),
                expires_at: Some(expires_at),
            },
            terms: StepUpTerms {
                parent_grant_id: parent.id.clone(),
                auth_time,
                acr: OWNER_VERIFIED_ACR.to_owned(),
                host_profile_basis: HostProfileBasis::DeclaredCimd,
                host_profile_tier: host_tier.to_ascii_uppercase(),
            },
        };
        let stored = context
            .oauth2_server
            .store_child_grant(&child, &GrantApproval::by_user())
            .await
            .map_err(failed)?
            .ok_or_else(|| OAuth2Error::access_denied(STEP_UP_NEEDS_A_GRANT))?;
        // The token about to be minted reads the parent's standing: drop the
        // cached one so the child is in it.
        context
            .accounts
            .grant_standings()
            .forget(user_id, tenant_id, client_id)
            .await;
        info!(user_id = %user_id, client_id, child_grant_id = %stored, parent_grant_id = %parent.id, "Step-up grant recorded");
        Ok(parent.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_auth::oauth2_server::authorization_details::{preset_defaults, Preset};

    #[test]
    fn only_a_one_off_question_asks_step_up() {
        let mcp = "https://mcp.example.test/mcp";
        assert!(
            !asks_step_up(Some(&preset_defaults(Preset::TrainingCoach, mcp))),
            "bug: a T1 preset asked step-up"
        );
        assert!(
            asks_step_up(Some(&preset_defaults(Preset::OneOffQuestion, mcp))),
            "bug: a one-off question did not ask step-up"
        );
        assert!(!asks_step_up(None), "bug: a plain grant asked step-up");
    }
}
