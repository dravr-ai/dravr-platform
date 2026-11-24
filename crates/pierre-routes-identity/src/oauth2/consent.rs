// ABOUTME: The OAuth consent form's submission — the athlete's decision on an authorization request
// ABOUTME: An approval proves the form's synchronizer token, records the grant and mints the code; a denial reaches the client
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::collections::HashMap;

use axum::{
    extract::{Form, State},
    http::HeaderMap,
    response::{IntoResponse, Redirect, Response},
};
use dravr_tronc::dates::{Timestamp, Zone};
use pierre_auth::oauth2_server::authorization_details::{athlete_zone, ResolvedDetails};
use pierre_auth::oauth2_server::endpoints::{AdminAccess, OAuth2AuthorizationServer};
use pierre_auth::oauth2_server::models::{AuthorizeRequest, OAuth2Error};
use pierre_core::errors::AppError;
use pierre_core::models::{OAuthClientGrant, User, UserStatus};
use tracing::{error, warn};
use uuid::Uuid;

use super::admin_access;
use super::consent_text::ConsentText;
use super::consent_version::ConsentSubject;
use super::picker;
use super::{OAuth2Context, OAuth2Routes};
use crate::authorize_redirect::{error_redirect, rejection_response};

/// What a Pending account that waits only on its confirmation link is told
/// at consent: no admin reviews it, so the link is the next step.
pub(super) const AWAITS_EMAIL_PROOF: &str = "Your Dravr account is waiting for you to confirm your email address. Open the confirmation link we emailed you, then connect this app again.";
/// What a Pending account an admin has yet to approve is told at consent.
pub(super) const AWAITS_APPROVAL: &str =
    "Your Dravr account is waiting for approval. Connect this app again once it is approved.";
/// What a suspended account is told at consent.
pub(super) const SUSPENDED: &str = "Your Dravr account is suspended";
/// What a session naming a deleted account is told.
pub(super) const ACCOUNT_GONE: &str =
    "The account you are signed in as no longer exists; sign in again";
/// What a session is told when its account could not be read.
pub(super) const ACCOUNT_UNAVAILABLE: &str = "The account could not be loaded";
/// The refusal of an approval that did not come from a form this server rendered.
pub(super) const CONSENT_EXPIRED: &str =
    "The consent screen expired or did not come from Dravr; start the connection again";
/// The refusal when the athlete's approval could not be recorded.
pub(super) const CONSENT_NOT_RECORDED: &str =
    "Your approval could not be recorded, so this app was not connected; try again";
/// The refusal when the consent form's token could not be minted.
pub(super) const CONSENT_UNAVAILABLE: &str = "The consent screen could not be prepared";

/// What the athlete approved on the consent screen: a client, the scope the
/// authorization was checked to carry, the details it grants, and the
/// language and review digest of the screen they were shown.
struct Approval<'form> {
    client_id: &'form str,
    scope: &'form str,
    text: &'form ConsentText<'form>,
    /// The details approved; `None` for a plain grant
    details: Option<&'form ResolvedDetails>,
    /// The review digest the screen was rendered and approved under
    review_digest: String,
    /// The athlete's zone, which the details' window is frozen in
    zone: Zone,
}

impl OAuth2Routes {
    /// Handle the OAuth consent form submission (POST /oauth2/consent).
    ///
    /// On approval, records a durable grant (so future authorizations for the same
    /// client + scope skip the prompt) and mints the authorization code. On denial,
    /// returns `access_denied` to the client's registered redirect URI.
    pub(super) async fn handle_consent_submit(
        State(context): State<OAuth2Context>,
        headers: HeaderMap,
        Form(form): Form<HashMap<String, String>>,
    ) -> Response {
        // A pushed request (RFC 9126) is read again from the store: the
        // form's `request_uri` names it, and its other fields are ignored.
        let request = match Self::resolve_authorize_params(&context, form.clone())
            .await
            .and_then(|resolved| Self::parse_authorize_request(&resolved.params))
        {
            Ok(req) => req,
            Err(error) => {
                let text = Self::reader_text(&context, &headers).await;
                return Self::render_oauth_error_response(&error, &text);
            }
        };
        // The form echoes the request the consent screen was rendered for, so
        // it is checked again: only a verified client and redirect_uri may be
        // redirected to, whether the user approves or denies.
        let server = Self::authorization_server(&context);
        let checked = match server.check_authorize_request(&request).await {
            Ok(checked) => checked,
            Err(rejection) => {
                let text = Self::reader_text(&context, &headers).await;
                return rejection_response(&context.config.issuer_url, rejection, &request, &text);
            }
        };
        let redirect_uri = request.redirect_uri.clone();

        // The consent decision must belong to the authenticated user.
        let (user_id, tenant_id) = Self::extract_authenticated_user(&headers, &context).await;
        let Some(user_id) = user_id else {
            // Session lapsed between render and submit — restart through login.
            return Redirect::to(&Self::build_login_url_with_oauth_params(&request))
                .into_response();
        };

        let approved = form
            .get("decision")
            .is_some_and(|decision| decision == "approve");
        if !approved {
            return error_redirect(
                &context.config.issuer_url,
                &redirect_uri,
                request.state.as_deref(),
                &OAuth2Error::access_denied("The user denied the authorization request"),
            );
        }

        // An approval must come from the consent screen this server rendered
        // for this user, not a form another site posts with the user's cookie;
        // a denial needs no proof, and must reach the client (RFC 6749 §4.1.2.1).
        let form_token = form.get("csrf_token").map_or("", String::as_str);
        if let Err(e) = context.csrf_manager.validate_token(form_token, user_id) {
            warn!(user_id = %user_id, "OAuth consent refused: {e}");
            let text = Self::reader_text(&context, &headers).await;
            return Self::render_oauth_error_response(
                &OAuth2Error::access_denied(CONSENT_EXPIRED),
                &text,
            );
        }

        // The account may have been suspended since the screen was rendered.
        let user = match Self::account_gate(&context, user_id, &headers).await {
            Ok(user) => user,
            Err(refused) => return *refused,
        };
        // The text the screen was rendered in for this user, which the
        // approval is recorded under.
        let text = ConsentText::for_reader(&context.strings, Some(&user.locale), &headers);
        // What the athlete chose to grant, and the screen must still read as
        // it did when they chose it.
        let subject = ConsentSubject {
            client_id: &request.client_id,
            redirect_uri: &redirect_uri,
            first_party_only_sources: &context.first_party_only_sources,
        };
        let (details, digest_field) = picker::chosen(&form, &checked, &context.strings);
        let review_digest = match picker::check_review_digest(
            &form,
            &digest_field,
            details.as_ref(),
            &text,
            subject,
        ) {
            Ok(digest) => digest,
            Err(error) => return Self::render_oauth_error_response(&error, &text),
        };
        // Admin access, read from the box only for an admin now, on a request
        // it may be granted on, beside Everything; a preset grant never holds
        // it. The grant and the code carry the same scope.
        let access = if details.is_some() {
            AdminAccess::Withheld
        } else {
            admin_access::decided(&form, &user, &checked, &text)
        };
        let scope = OAuth2AuthorizationServer::granted_scope(&checked, access);

        // A client first seen in this flow gets its row before the grant naming it.
        if let Err(error) = server.enroll_checked_client(&checked).await {
            return error_redirect(
                &context.config.issuer_url,
                &redirect_uri,
                request.state.as_deref(),
                &error,
            );
        }

        // No record, no code: the athlete can neither see nor revoke a grant
        // that was not written, and a code would connect the client anyway.
        // The code names the grant actually stored, so its tokens stand and
        // fall with it.
        let client_grant_id = match Self::record_consent(
            &context,
            Approval {
                client_id: &request.client_id,
                scope: &scope,
                text: &text,
                details: details.as_ref(),
                review_digest,
                zone: athlete_zone(user.timezone.as_deref()),
            },
            user_id,
            tenant_id.clone(),
        )
        .await
        {
            Ok(client_grant_id) => client_grant_id,
            Err(error) => {
                return error_redirect(
                    &context.config.issuer_url,
                    &redirect_uri,
                    request.state.as_deref(),
                    &error,
                );
            }
        };

        Self::mint_authorization_code(
            &context,
            request,
            user_id,
            tenant_id,
            redirect_uri,
            access,
            Some(client_grant_id),
        )
        .await
    }

    /// Record the athlete's approval of `scope`, the grant the authorization
    /// was checked to carry, with the review digest and language of the
    /// consent text it was given under, and the details it grants with their
    /// window frozen now and their expiry. A live grant with the same terms
    /// keeps its row and takes the new digest at the storage layer. A grant's
    /// details never change under it, so an approval of other details, or one
    /// onto a row past its expiry, revokes that row and stores a new grant.
    /// Returns the id of the row that holds the grant.
    ///
    /// # Errors
    /// `server_error` when the grant's tenant cannot be resolved or the grant
    /// cannot be written.
    async fn record_consent(
        context: &OAuth2Context,
        approval: Approval<'_>,
        user_id: Uuid,
        tenant_id: Option<String>,
    ) -> Result<String, OAuth2Error> {
        let Approval {
            client_id,
            scope,
            text,
            details,
            review_digest,
            zone,
        } = approval;
        let Some(tid) = Self::resolve_grant_tenant(context, user_id, tenant_id).await else {
            error!(user_id = %user_id, client_id, "OAuth consent not recorded: the grant has no tenant");
            return Err(OAuth2Error::server_error(CONSENT_NOT_RECORDED));
        };
        let not_recorded = |e: AppError| {
            error!(user_id = %user_id, client_id, "Failed to persist OAuth client grant: {e}");
            OAuth2Error::server_error(CONSENT_NOT_RECORDED)
        };
        let granted_at = Timestamp::now();
        let (document, expires_at) = match details {
            Some(details) => {
                let (frozen, expires_at) =
                    details.freeze(granted_at, &zone).map_err(not_recorded)?;
                (
                    Some(frozen.to_document().map_err(not_recorded)?),
                    Some(expires_at),
                )
            }
            None => (None, None),
        };
        let grant = OAuthClientGrant {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            tenant_id: tid.clone(),
            client_id: client_id.to_owned(),
            scope: scope.to_owned(),
            granted_at,
            revoked_at: None,
            consent_version: Some(review_digest),
            consent_locale: Some(text.lang().to_owned()),
            authorization_details: document,
            expires_at,
        };
        let live = context
            .oauth2_server
            .find_active_client_grant(&grant.user_id, &tid, client_id, scope)
            .await
            .map_err(not_recorded)?;
        let stored = match live {
            Some(live) if !same_terms(&live, details) || !live.is_live_at(grant.granted_at) => {
                context
                    .oauth2_server
                    .replace_client_grant(&live.id, &grant)
                    .await
            }
            _ => context.oauth2_server.store_client_grant(&grant).await,
        };
        let client_grant_id = stored.map_err(not_recorded)?;
        // A resource server may hold this client as withdrawn from before: the
        // token this consent is about to mint must not be refused on that.
        context
            .accounts
            .grant_standings()
            .forget(user_id, &tid, client_id)
            .await;
        Ok(client_grant_id)
    }

    /// The signed-in account, when it may authorize a client: it exists and
    /// is active. A pending account is told what it waits for — its
    /// confirmation link when proving the address is what activates it, an
    /// admin's approval otherwise — and a suspended one is refused; neither
    /// is shown consent nor given a code.
    pub(super) async fn account_gate(
        context: &OAuth2Context,
        user_id: Uuid,
        headers: &HeaderMap,
    ) -> Result<User, Box<Response>> {
        let user = Self::signed_in_account(context, user_id)
            .await
            .map_err(|error| {
                let text = ConsentText::for_browser(&context.strings, headers);
                Box::new(Self::render_oauth_error_response(&error, &text))
            })?;
        let text = ConsentText::for_reader(&context.strings, Some(&user.locale), headers);
        let refuse =
            |error: OAuth2Error| Box::new(Self::render_oauth_error_response(&error, &text));
        if context.accounts.awaits_email_proof(&user).await {
            warn!(user_id = %user.id, "OAuth authorization refused: the account waits for its email to be confirmed");
            return Err(refuse(OAuth2Error::access_denied(AWAITS_EMAIL_PROOF)));
        }
        Self::account_refusal(&user).map_or_else(|| Ok(user), |error| Err(refuse(error)))
    }

    /// The text of a refusal page for whoever this request's session names:
    /// their stored locale, else the browser's. It reads the account, so it
    /// is for a refusal already decided, and never for a throttled request,
    /// whose answer must cost no query.
    pub(super) async fn reader_text<'a>(
        context: &'a OAuth2Context,
        headers: &HeaderMap,
    ) -> ConsentText<'a> {
        let stored = match Self::extract_authenticated_user(headers, context).await.0 {
            Some(user_id) => context
                .users
                .get_global(user_id)
                .await
                .ok()
                .flatten()
                .map(|user| user.locale),
            None => None,
        };
        ConsentText::for_reader(&context.strings, stored.as_deref(), headers)
    }

    /// Sign the athlete in to Dravr's own web or mobile app: a code with no
    /// consent screen, which the app redeems for a first-party session.
    ///
    /// A pending account signs in too, as it always has, so the app can show
    /// it the approval it awaits; the session it gets is refused by every
    /// route a pending account may not use. A suspended one is refused here.
    pub(super) async fn sign_in_first_party(
        context: &OAuth2Context,
        request: AuthorizeRequest,
        user_id: Uuid,
        tenant_id: Option<String>,
        redirect_uri: String,
        headers: &HeaderMap,
    ) -> Response {
        let refused = match Self::signed_in_account(context, user_id).await {
            Ok(user) if user.user_status == UserStatus::Suspended => Self::account_refusal(&user),
            Ok(_) => None,
            Err(error) => Some(error),
        };
        if let Some(error) = refused {
            let text = Self::reader_text(context, headers).await;
            return Self::render_oauth_error_response(&error, &text);
        }
        Self::mint_authorization_code(
            context,
            request,
            user_id,
            tenant_id,
            redirect_uri,
            AdminAccess::Withheld,
            None,
        )
        .await
    }

    /// The account a session names.
    async fn signed_in_account(
        context: &OAuth2Context,
        user_id: Uuid,
    ) -> Result<User, OAuth2Error> {
        match context.users.get_global(user_id).await {
            Ok(Some(user)) => Ok(user),
            Ok(None) => {
                warn!(user_id = %user_id, "OAuth authorization refused: the session names no account");
                Err(OAuth2Error::access_denied(ACCOUNT_GONE))
            }
            Err(e) => {
                error!(user_id = %user_id, "OAuth authorization could not load the account: {e}");
                Err(OAuth2Error::server_error(ACCOUNT_UNAVAILABLE))
            }
        }
    }

    /// Why `user` may not authorize a client, if it may not.
    ///
    /// A Pending account is the admin's approval policy, not a dead end:
    /// with sign-up open to verified addresses (`pierre-cli settings signup
    /// set --activate-on-verified-email true`), a first Google sign-in through
    /// the connector is Active before it reaches consent, and an account
    /// still waiting on its email is told so by [`Self::account_gate`]. Only
    /// an admin who keeps approval manual sees this refusal, and its text
    /// says when to connect again.
    fn account_refusal(user: &User) -> Option<OAuth2Error> {
        match user.user_status {
            UserStatus::Active => None,
            UserStatus::Pending => {
                warn!(user_id = %user.id, "OAuth authorization refused: the account awaits approval");
                Some(OAuth2Error::access_denied(AWAITS_APPROVAL))
            }
            UserStatus::Suspended => {
                warn!(user_id = %user.id, "OAuth authorization refused: the account is suspended");
                Some(OAuth2Error::access_denied(SUSPENDED))
            }
        }
    }
}

/// Whether the live grant `live` grants exactly `details` (none for a plain
/// grant), whenever it was approved. A stored document that does not read
/// grants nothing anyone can name, so it is replaced.
fn same_terms(live: &OAuthClientGrant, details: Option<&ResolvedDetails>) -> bool {
    match (picker::stored_details(live), details) {
        (Ok(None), None) => true,
        (Ok(Some(stored)), Some(details)) => stored.same_terms(details),
        _ => false,
    }
}
