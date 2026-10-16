// ABOUTME: Coach-access requests — a coach asks in one tap, a super-admin grants or declines (carnet#738)
// ABOUTME: A grant sets manages_roster and attaches the requester to their still-coachless onboarding group

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Coach access, asked for and decided.
//!
//! A coach without `manages_roster` who makes a group during onboarding gets
//! it coachless (ADR-018: asking to coach never grants coaching). This module
//! is the way out of that state that does not go through a support inbox:
//!
//! 1. [`request_access`] opens a request, naming the coach's group when they
//!    asked from one, or filed for them when their Intervals.icu key coaches
//!    athletes ([`crate::coach_access_evidence`]). It grants nothing; it
//!    announces itself on the notify channel, and a caller holding the email
//!    service tells every super-admin ([`email_super_admins`]). A request
//!    that names no group takes the one its coach later makes to coach
//!    ([`name_requested_group`]).
//! 2. A super-admin reads the queue ([`list_requests`]) and either
//!    [`grant`]s — `manages_roster` as an operator grant, and the requester
//!    attached as coach of the named group when they still own it and it
//!    still has no coach — or [`decline`]s.
//!
//! Every function is the single implementation behind both the REST routes
//! and the admin console, so the two cannot drift.

use std::collections::HashMap;

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{CoachAccessRequest, CoachAccessStatus, TenantId, User};
use pierre_database::RepositoryRegistry;
use pierre_email::templates::CoachAccessOrigin;
use pierre_email::ResendEmailService;
use pierre_middleware::redaction::mask_email;
use serde::Serialize;
use tracing::{info, warn};
use uuid::Uuid;

use crate::admin_ops::set_user_manages_roster;
use crate::operator_mail::{console_users_url, super_admin_emails};

/// What [`request_access`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestOutcome {
    /// A new request was opened; the super-admins should hear about it.
    Opened(CoachAccessRequest),
    /// The user already had one waiting, returned as it is. Nobody is told
    /// again: a second tap is not a second request.
    AlreadyPending(CoachAccessRequest),
}

impl RequestOutcome {
    /// The request, whichever way it came.
    #[must_use]
    pub const fn request(&self) -> &CoachAccessRequest {
        match self {
            Self::Opened(request) | Self::AlreadyPending(request) => request,
        }
    }
}

/// Open a coach-access request for `user_id`.
///
/// `group` names the group the coach asked from (its id and tenant). It must
/// be theirs and still coachless, since that is the group a grant attaches
/// them to. A user who already holds coach access has nothing to ask for.
///
/// A user who already has a request waiting gets it back. When it names no
/// group (one filed from their Intervals.icu roster) and this ask names one,
/// the waiting request names it from now on, so a grant attaches them to it.
///
/// # Errors
///
/// Returns not-found for an unknown user or group, permission-denied when the
/// group is not the caller's, invalid-input when it already has a coach or the
/// user already holds coach access, and the repository error otherwise.
pub async fn request_access(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    group: Option<(Uuid, TenantId)>,
) -> AppResult<RequestOutcome> {
    let user = repos
        .users
        .get_global(user_id)
        .await?
        .ok_or_else(|| AppError::not_found("User"))?;
    if user.manages_roster {
        return Err(AppError::invalid_input("You already have coach access"));
    }

    let group = match group {
        Some(group) => Some(group_to_coach(repos, user_id, group).await?),
        None => None,
    };

    let request = CoachAccessRequest::pending(user_id, group.clone(), Utc::now());
    if repos.coach_access_requests.open(&request).await? {
        // LIMITATION(registre#1060): `coach_access.requested` is the only coach-access event; no referral attribution (`coach_referral.*`, `referral_id`) exists.
        info!(
            target: "notify",
            event = "coach_access.requested",
            user_id = %user_id,
            request_id = %request.id,
            group_id = ?request.group_id,
            "coach access requested"
        );
        return Ok(RequestOutcome::Opened(request));
    }
    // The partial unique index kept the request already waiting.
    let pending = repos
        .coach_access_requests
        .latest_for_user(user_id)
        .await?
        .filter(|r| r.status == CoachAccessStatus::Pending)
        .ok_or_else(|| AppError::internal("Coach access request was neither opened nor pending"))?;
    let pending = match group {
        Some(group) => name_group(repos, pending, group).await?,
        None => pending,
    };
    Ok(RequestOutcome::AlreadyPending(pending))
}

/// Name `group` (its id and tenant) on `user_id`'s waiting coach-access
/// request when that request names no group, so a grant attaches them to it.
///
/// Called when a coach without coach access makes the group they asked to
/// coach and it comes back coachless: a request filed from their
/// Intervals.icu roster before the group existed names none, and the coach is
/// shown it as already sent. Nothing is opened, granted or announced; a user
/// with no waiting request, or one that names a group already, is left as
/// they are.
///
/// # Errors
///
/// Returns not-found for an unknown group, permission-denied when the group
/// is not the user's, invalid-input when it already has a coach, and the
/// repository error otherwise.
pub async fn name_requested_group(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    group: (Uuid, TenantId),
) -> AppResult<()> {
    let Some(pending) = repos
        .coach_access_requests
        .latest_for_user(user_id)
        .await?
        .filter(|r| r.status == CoachAccessStatus::Pending && r.group_id.is_none())
    else {
        return Ok(());
    };
    let group = group_to_coach(repos, user_id, group).await?;
    name_group(repos, pending, group).await?;
    Ok(())
}

/// The group (its id and tenant) `user_id` asks to coach, once it is theirs
/// and still coachless: the group a grant attaches them to.
///
/// # Errors
///
/// Returns not-found for an unknown group, permission-denied when the group
/// is not the user's, invalid-input when it already has a coach, and the
/// repository error otherwise.
async fn group_to_coach(
    repos: &RepositoryRegistry,
    user_id: Uuid,
    (group_id, tenant_id): (Uuid, TenantId),
) -> AppResult<(Uuid, String)> {
    let found = repos
        .groups
        .get_group(&group_id.to_string(), tenant_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Group {group_id}")))?;
    if found.owner_id != user_id {
        return Err(AppError::new(
            ErrorCode::PermissionDenied,
            "Only the group's owner can ask to coach it",
        ));
    }
    if found.coach_user_id.is_some() {
        return Err(AppError::invalid_input("This group already has a coach"));
    }
    Ok((group_id, found.tenant_id))
}

/// `pending`, naming `group` when it named none; as it was otherwise.
async fn name_group(
    repos: &RepositoryRegistry,
    mut pending: CoachAccessRequest,
    (group_id, group_tenant_id): (Uuid, String),
) -> AppResult<CoachAccessRequest> {
    if pending.group_id.is_none()
        && repos
            .coach_access_requests
            .name_group(pending.id, pending.user_id, group_id, &group_tenant_id)
            .await?
    {
        info!(
            request_id = %pending.id,
            group_id = %group_id,
            "Coach access request now names the group its coach asked to coach"
        );
        pending.group_id = Some(group_id);
        pending.group_tenant_id = Some(group_tenant_id);
    }
    Ok(pending)
}

/// Email every active super-admin about the request just opened.
///
/// The email links the console queue under `frontend_url` when the server
/// has one. `origin` says how the request opened: one filed from an
/// Intervals.icu roster is never worded as the coach's asking.
///
/// Best-effort: an unconfigured email service or a failed send is logged and
/// never fails the caller; the notify channel and the console queue carry the
/// request regardless.
pub async fn email_super_admins(
    repos: &RepositoryRegistry,
    email: Option<&ResendEmailService>,
    frontend_url: Option<&str>,
    request: &CoachAccessRequest,
    origin: CoachAccessOrigin,
) {
    let Some(email) = email else {
        warn!(request_id = %request.id, "Email service not configured — skipping coach access emails");
        return;
    };
    let Some(recipients) = recipients(repos, request.id).await else {
        return;
    };
    let Some(requester) = repos.users.get_global(request.user_id).await.ok().flatten() else {
        warn!(request_id = %request.id, "Coach access requester vanished before the email");
        return;
    };
    let group_name = requested_group_name(repos, request).await;
    let review_url = frontend_url.map(console_users_url);

    for to in &recipients {
        let sent = email
            .send_coach_access_requested(
                to,
                &requester.email,
                requester.display_name.as_deref(),
                group_name.as_deref(),
                review_url.as_deref(),
                origin,
            )
            .await;
        log_email_outcome(to, request.id, sent);
    }
}

/// The addresses of every active super-admin; `None` (logged) when they
/// cannot be read.
async fn recipients(repos: &RepositoryRegistry, request_id: Uuid) -> Option<Vec<String>> {
    match super_admin_emails(repos.users.as_ref()).await {
        Ok(recipients) => Some(recipients),
        Err(e) => {
            warn!(%request_id, error = %e, "Could not list super-admins for a coach access request");
            None
        }
    }
}

/// The name of the group a request names, for the email; `None` when it names
/// none or the group is gone.
async fn requested_group_name(
    repos: &RepositoryRegistry,
    request: &CoachAccessRequest,
) -> Option<String> {
    let (group_id, tenant_id) = located_group(request)?;
    repos
        .groups
        .get_group(&group_id.to_string(), tenant_id)
        .await
        .ok()
        .flatten()
        .map(|g| g.name)
}

/// Log one operator email's outcome against the masked recipient.
fn log_email_outcome(to: &str, request_id: Uuid, sent: AppResult<()>) {
    let recipient = mask_email(to);
    match sent {
        Ok(()) => info!(%recipient, %request_id, "Coach access request email sent"),
        Err(e) => {
            warn!(%recipient, %request_id, error = %e, "Failed to send coach access request email");
        }
    }
}

/// A request as the admin queue shows it: who asked, and for which group.
#[derive(Debug, Clone, Serialize)]
pub struct CoachAccessRequestView {
    /// The request itself.
    #[serde(flatten)]
    pub request: CoachAccessRequest,
    /// The requester's email; `None` when the account is gone.
    pub email: Option<String>,
    /// The requester's display name, when they set one.
    pub display_name: Option<String>,
    /// The named group's name, when the request names one that still exists.
    pub group_name: Option<String>,
}

/// The requests in `status`, oldest first, with who asked and for which group.
///
/// # Errors
///
/// Returns the repository error if a read fails.
pub async fn list_requests(
    repos: &RepositoryRegistry,
    status: CoachAccessStatus,
) -> AppResult<Vec<CoachAccessRequestView>> {
    let requests = repos.coach_access_requests.list_by_status(status).await?;
    let user_ids: Vec<Uuid> = requests.iter().map(|r| r.user_id).collect();
    let users: HashMap<Uuid, User> = repos.users.get_global_many(&user_ids).await?;

    let mut views = Vec::with_capacity(requests.len());
    for request in requests {
        let group_name = match located_group(&request) {
            Some((group_id, tenant_id)) => repos
                .groups
                .get_group(&group_id.to_string(), tenant_id)
                .await?
                .map(|g| g.name),
            None => None,
        };
        let user = users.get(&request.user_id);
        views.push(CoachAccessRequestView {
            email: user.map(|u| u.email.clone()),
            display_name: user.and_then(|u| u.display_name.clone()),
            group_name,
            request,
        });
    }
    Ok(views)
}

/// What [`grant`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantOutcome {
    /// The request, now granted.
    pub request: CoachAccessRequest,
    /// The group the requester now coaches, when the grant attached them.
    pub attached_group: Option<Uuid>,
}

/// Grant a pending request, and attach the requester to their group.
///
/// The requester gets `manages_roster` as an operator grant (which a
/// `TrainingPeaks` disconnect never takes back) and, when the request names a
/// group they still own that still has no coach, becomes its coach — so no
/// coach invite is needed.
///
/// The attachment writes `coach_user_id` directly rather than through
/// `GroupService::set_group_coach`: that path's extra work ends the delegated
/// connections read through a coach being *replaced*, and a coachless group
/// has none.
///
/// # Errors
///
/// Returns not-found for an unknown request, a conflict when it was already
/// decided, and the repository error otherwise.
pub async fn grant(
    repos: &RepositoryRegistry,
    request_id: Uuid,
    operator: Option<Uuid>,
) -> AppResult<GrantOutcome> {
    let request = pending_request(repos, request_id).await?;

    set_user_manages_roster(repos, request.user_id, true, operator).await?;

    let mut attached_group = None;
    if let Some((group_id, tenant_id)) = located_group(&request) {
        let group_key = group_id.to_string();
        let attachable = repos
            .groups
            .get_group(&group_key, tenant_id)
            .await?
            .is_some_and(|g| {
                g.is_active && g.owner_id == request.user_id && g.coach_user_id.is_none()
            });
        if attachable
            && repos
                .groups
                .set_group_coach_user(&group_key, Some(request.user_id), tenant_id)
                .await?
        {
            attached_group = Some(group_id);
        }
    }

    let decided_at = Utc::now();
    if !repos
        .coach_access_requests
        .decide(request_id, CoachAccessStatus::Granted, operator, decided_at)
        .await?
    {
        return Err(already_decided());
    }
    info!(
        request_id = %request_id,
        target_user_id = %request.user_id,
        operator_user_id = ?operator,
        attached_group = ?attached_group,
        "Coach access request granted"
    );
    Ok(GrantOutcome {
        request: CoachAccessRequest {
            status: CoachAccessStatus::Granted,
            decided_at: Some(decided_at),
            decided_by: operator,
            ..request
        },
        attached_group,
    })
}

/// Decline a pending request. Nothing else changes; the coach may ask again.
///
/// # Errors
///
/// Returns not-found for an unknown request, a conflict when it was already
/// decided, and the repository error otherwise.
pub async fn decline(
    repos: &RepositoryRegistry,
    request_id: Uuid,
    operator: Option<Uuid>,
) -> AppResult<CoachAccessRequest> {
    let request = pending_request(repos, request_id).await?;
    let decided_at = Utc::now();
    if !repos
        .coach_access_requests
        .decide(
            request_id,
            CoachAccessStatus::Declined,
            operator,
            decided_at,
        )
        .await?
    {
        return Err(already_decided());
    }
    info!(
        request_id = %request_id,
        target_user_id = %request.user_id,
        operator_user_id = ?operator,
        "Coach access request declined"
    );
    Ok(CoachAccessRequest {
        status: CoachAccessStatus::Declined,
        decided_at: Some(decided_at),
        decided_by: operator,
        ..request
    })
}

/// The request, refused unless it is still pending.
async fn pending_request(
    repos: &RepositoryRegistry,
    request_id: Uuid,
) -> AppResult<CoachAccessRequest> {
    let request = repos
        .coach_access_requests
        .get(request_id)
        .await?
        .ok_or_else(|| AppError::not_found(format!("Coach access request {request_id}")))?;
    if request.status == CoachAccessStatus::Pending {
        Ok(request)
    } else {
        Err(already_decided())
    }
}

fn already_decided() -> AppError {
    AppError::new(
        ErrorCode::ResourceAlreadyExists,
        "This coach access request was already decided",
    )
}

/// The group a request names, with the tenant that locates it; `None` when it
/// names none or its stored tenant is unreadable.
fn located_group(request: &CoachAccessRequest) -> Option<(Uuid, TenantId)> {
    let group_id = request.group_id?;
    let tenant = request.group_tenant_id.as_deref()?.parse::<Uuid>().ok()?;
    Some((group_id, TenantId::from_uuid(tenant)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_without_a_group_locates_none() {
        let request = CoachAccessRequest::pending(Uuid::new_v4(), None, Utc::now());
        assert_eq!(located_group(&request), None);
    }

    #[test]
    fn a_request_with_a_group_locates_it_in_its_tenant() {
        let group = Uuid::new_v4();
        let tenant = Uuid::new_v4();
        let request = CoachAccessRequest::pending(
            Uuid::new_v4(),
            Some((group, tenant.to_string())),
            Utc::now(),
        );
        assert_eq!(
            located_group(&request),
            Some((group, TenantId::from_uuid(tenant)))
        );
    }

    #[test]
    fn an_unreadable_tenant_locates_none_rather_than_a_wrong_group() {
        let request = CoachAccessRequest::pending(
            Uuid::new_v4(),
            Some((Uuid::new_v4(), "not-a-uuid".to_owned())),
            Utc::now(),
        );
        assert_eq!(located_group(&request), None);
    }
}
