// ABOUTME: Coach-access requests — a coach asks in one tap or withdraws, a super-admin grants or declines (carnet#738)
// ABOUTME: A grant sets manages_roster and attaches the requester to every coachless group they own; the coach is told

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
//!    announces itself on the notify channel with what a super-admin decides
//!    on (who asked, how the request came, the group it names), and a caller
//!    holding the email service tells every super-admin
//!    ([`email_super_admins`]). A request that names no group takes the one
//!    its coach later makes to coach ([`name_requested_group`]).
//! 2. While it waits, the coach may [`withdraw`] it: the row is deleted, so
//!    they can ask again at once, and the operators hear it on the notify
//!    channel (`coach_access.cancelled`).
//! 3. A super-admin reads the queue ([`list_requests`]) and either
//!    [`grant`]s — `manages_roster` as an operator grant, and the requester
//!    attached as coach of every active group they own that has no coach
//!    (carnet#1521), the named one first — or [`decline`]s, optionally saying
//!    why. The decision is announced on the notify channel too, so the room
//!    that saw the request sees it decided, and told to the coach through
//!    [`CoachAccessNotices`]: the room line, the in-app notice and the email.
//!
//! Every function is the single implementation behind both the REST routes
//! and the admin console, so the two cannot drift.

use std::collections::HashMap;

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::groups::CoachingGroup;
use pierre_core::models::{
    normalize_decline_reason, CoachAccessRequest, CoachAccessStatus, TenantId, User,
};
use pierre_database::RepositoryRegistry;
use pierre_email::templates::CoachAccessOrigin;
use pierre_email::ResendEmailService;
use pierre_middleware::redaction::mask_email;
use serde::Serialize;
use tracing::{field, info, warn};
use uuid::Uuid;

use crate::admin_ops::set_user_manages_roster;
use crate::analytics::cache_user_email;
pub use crate::coach_access_notice::CoachAccessNotices;
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
/// `origin` says how the request came: the coach asking, or a roster read
/// filing it for them. A request opened here is announced with it
/// ([`announce_opened`]).
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
    origin: CoachAccessOrigin,
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
        supersede_earlier_declines(repos, &request).await;
        announce_opened(repos, &user, &request, origin).await;
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

/// Asking again replaces an earlier decline (carnet#1522): stamp the coach's
/// declined requests superseded by the one just opened, so the request they
/// read skips them — after a withdrawal of the new ask too. The console's
/// history keeps every declined row and its reason.
///
/// Best-effort: the request is open whatever happens here, and a decline
/// left unstamped only shows again if the coach withdraws the new ask, so a
/// failure is logged rather than failing the ask.
async fn supersede_earlier_declines(repos: &RepositoryRegistry, request: &CoachAccessRequest) {
    if let Err(e) = repos
        .coach_access_requests
        .supersede_declined(request.user_id, request.id, request.created_at)
        .await
    {
        warn!(
            request_id = %request.id,
            error = %e,
            "Could not mark the coach's earlier declines as replaced by their new ask"
        );
    }
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

/// Announce a request just opened with what a super-admin decides on: who
/// asked, how the request came, and the group it names, when it names one.
///
/// The notify channel is the only announcement a request filed from a live
/// roster read gets (no email goes out from there), so the notice has to
/// carry the decision on its own. The requester is named by their email,
/// which reaches the notice through the identity cache the notify enricher
/// reads, warmed here because the requester may never have signed in on this
/// pod; the enricher adds the console link. Neither is a field of this event,
/// so neither reaches the log line, and their display name stays off it too:
/// the operator email and the console queue carry it. The `PostHog` sink
/// keeps the email, the group name and the link off the event.
async fn announce_opened(
    repos: &RepositoryRegistry,
    requester: &User,
    request: &CoachAccessRequest,
    origin: CoachAccessOrigin,
) {
    let group_name = requested_group_name(repos, request).await;
    cache_user_email(&requester.id.to_string(), &requester.email);
    // LIMITATION(registre#1060): coach-access events carry no referral attribution; no `coach_referral.*` event or `referral_id` exists.
    info!(
        target: "notify",
        event = "coach_access.requested",
        user_id = %requester.id,
        request_id = %request.id,
        origin = origin_label(origin),
        coached_active_athletes = coached_active_athletes(origin),
        group_id = request.group_id.map(field::display),
        group_name = group_name.as_deref(),
        "coach access requested"
    );
}

/// How a request came, as its notice names it.
const fn origin_label(origin: CoachAccessOrigin) -> &'static str {
    match origin {
        CoachAccessOrigin::Asked => "asked",
        CoachAccessOrigin::IntervalsIcuRoster { .. } => "intervals_icu_roster",
    }
}

/// The active athletes the roster a request was filed from coaches; `None`
/// for a request the coach asked for.
const fn coached_active_athletes(origin: CoachAccessOrigin) -> Option<usize> {
    match origin {
        CoachAccessOrigin::Asked => None,
        CoachAccessOrigin::IntervalsIcuRoster {
            coached_active_athletes,
        } => Some(coached_active_athletes),
    }
}

/// Warm the identity cache with the requester's email, so a decline's notice
/// names them even when they never signed in on this pod.
async fn cache_requester_email(repos: &RepositoryRegistry, user_id: Uuid) {
    match repos.users.get_global(user_id).await {
        Ok(Some(requester)) => cache_user_email(&user_id.to_string(), &requester.email),
        Ok(None) => {}
        Err(e) => {
            warn!(%user_id, error = %e, "Could not read the coach access requester for the notice");
        }
    }
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

/// The name of the group a request names, for the email and the notify
/// notice; `None` when it names none or the group is gone.
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
    /// Every group the requester now coaches because of this grant, the
    /// group the request named first when it was among them.
    pub attached_groups: Vec<Uuid>,
}

impl GrantOutcome {
    /// The first group the grant attached — the named one when it was
    /// attached — or `None` when it attached none.
    #[must_use]
    pub fn attached_group(&self) -> Option<Uuid> {
        self.attached_groups.first().copied()
    }
}

/// Grant a pending request, and attach the requester to every coachless
/// group they own.
///
/// The requester gets `manages_roster` as an operator grant (which a
/// `TrainingPeaks` disconnect never takes back) and becomes the coach of every
/// active group they own in their own tenant that still has no coach, the one
/// the request names first, so no coach invite is needed (carnet#1521). Their
/// own tenant is the named group's when the request names one, else their
/// home tenant ([`grant_tenant`]); a coachless group they own elsewhere is left
/// alone. The waiting state is
/// one per account, so every such group truthfully showed the same pending
/// request, and one grant answers all of them. Groups they make later are
/// coached on creation: they hold `manages_roster`.
///
/// The decision is claimed first, by the conditional update a concurrent
/// [`decline`] or [`withdraw`] races against, and the effects run only once
/// this grant has won it. When an effect then fails, the claim is undone —
/// the groups this grant attached are detached again and a coach access it
/// gave is taken back — so the request row and the access state cannot
/// disagree.
///
/// Once the grant stands, `notices` tells the coach (best-effort): a line in
/// every copy of each attached group's chat, the in-app notice and the email.
/// `None` tells nobody but the notify channel.
///
/// # Errors
///
/// Returns not-found for an unknown request, a conflict when it was already
/// decided, and the repository error otherwise.
pub async fn grant(
    repos: &RepositoryRegistry,
    request_id: Uuid,
    operator: Option<Uuid>,
    notices: Option<&CoachAccessNotices>,
) -> AppResult<GrantOutcome> {
    let request = pending_request(repos, request_id).await?;
    // Read before the claim: an undo restores this, so a grant an operator
    // made by hand meanwhile is never taken back by a failed attachment.
    let requester = repos.users.get_global(request.user_id).await?;
    let had_coach_access = requester.as_ref().is_some_and(|user| user.manages_roster);
    let tenant = grant_tenant(repos, &request).await;

    let decided_at = Utc::now();
    if !repos
        .coach_access_requests
        .decide(
            request_id,
            CoachAccessStatus::Granted,
            operator,
            decided_at,
            None,
        )
        .await?
    {
        return Err(already_decided());
    }

    let mut attached: Vec<CoachingGroup> = Vec::new();
    if let Err(error) = apply_grant(repos, &request, operator, tenant, &mut attached).await {
        undo_grant_claim(
            repos,
            &request,
            operator,
            had_coach_access,
            tenant,
            &attached,
        )
        .await;
        return Err(error);
    }
    let attached_groups: Vec<Uuid> = attached.iter().map(|g| g.id).collect();
    info!(
        request_id = %request_id,
        target_user_id = %request.user_id,
        operator_user_id = ?operator,
        attached_groups = ?attached_groups,
        "Coach access request granted"
    );
    // The enricher names the requester from this cache, so a grant taken on a
    // pod they never signed in on still says whose request it was.
    if let Some(requester) = &requester {
        cache_user_email(&requester.id.to_string(), &requester.email);
    }
    info!(
        target: "notify",
        event = "coach_access.granted",
        user_id = %request.user_id,
        request_id = %request_id,
        group_id = attached_groups.first().map(field::display),
        "coach access granted"
    );
    let granted = CoachAccessRequest {
        status: CoachAccessStatus::Granted,
        decided_at: Some(decided_at),
        decided_by: operator,
        decline_reason: None,
        ..request
    };
    if let Some(notices) = notices {
        notices.tell_granted(repos, &granted, &attached).await;
    }
    Ok(GrantOutcome {
        request: granted,
        attached_groups,
    })
}

/// The coach's own tenant, where a grant attaches them to their coachless
/// groups: the tenant of the group the request names, else the coach's home
/// tenant. `None` (logged) when neither can be read; the grant then gives
/// coach access and attaches no group.
async fn grant_tenant(
    repos: &RepositoryRegistry,
    request: &CoachAccessRequest,
) -> Option<TenantId> {
    match located_group(request) {
        Some((_, tenant)) => Some(tenant),
        None => home_tenant(repos, request.user_id).await,
    }
}

/// The tenant a user signs in to by default — the first of their tenants, as
/// login picks it — where their feed is read and the groups a grant attaches
/// live; `None` (logged) when it cannot be read or they hold none.
pub(crate) async fn home_tenant(repos: &RepositoryRegistry, user_id: Uuid) -> Option<TenantId> {
    match repos.tenants.first_for_user(user_id).await {
        Ok(Some(tenant)) => Some(tenant),
        Ok(None) => {
            warn!(%user_id, "Coach access: the requester holds no tenant");
            None
        }
        Err(e) => {
            warn!(%user_id, error = %e, "Coach access: the requester's tenant could not be read");
            None
        }
    }
}

/// A claimed grant's effects: coach access, then the requester attached as
/// coach of every active group they own in `tenant` that has no coach, the
/// named group first. Each group attached is pushed onto `attached` as it
/// lands, so a failure part-way leaves the caller the ones to detach again.
///
/// The attachment writes `coach_user_id` directly rather than through
/// `GroupService::set_group_coach`: that path's extra work ends the delegated
/// connections read through a coach being *replaced*, and a coachless group
/// has none.
async fn apply_grant(
    repos: &RepositoryRegistry,
    request: &CoachAccessRequest,
    operator: Option<Uuid>,
    tenant: Option<TenantId>,
    attached: &mut Vec<CoachingGroup>,
) -> AppResult<()> {
    set_user_manages_roster(repos, request.user_id, true, operator).await?;

    let Some(tenant) = tenant else {
        return Ok(());
    };
    let mut groups = repos
        .groups
        .list_coachless_groups_owned_by(request.user_id, tenant)
        .await?;
    if let Some(named) = request.group_id {
        // The group the coach asked from leads, so the first group attached
        // is the one they asked about.
        groups.sort_by_key(|g| g.id != named);
    }
    for group in groups {
        if repos
            .groups
            .set_group_coach_user(&group.id.to_string(), Some(request.user_id), tenant)
            .await?
        {
            attached.push(CoachingGroup {
                coach_user_id: Some(request.user_id),
                ..group
            });
        }
    }
    Ok(())
}

/// Undo a grant whose effects failed after it claimed the request: detach
/// the requester from every group this grant `attached` in `tenant`, put the
/// coach access back to what it was before the grant (`had_coach_access`)
/// and the request back to pending. Every step is attempted and what did not
/// undo is logged; the caller returns the original error.
async fn undo_grant_claim(
    repos: &RepositoryRegistry,
    request: &CoachAccessRequest,
    operator: Option<Uuid>,
    had_coach_access: bool,
    tenant: Option<TenantId>,
    attached: &[CoachingGroup],
) {
    for group in attached {
        // A group was attached only under the grant's tenant.
        let detached = match tenant {
            Some(tenant) => repos
                .groups
                .set_group_coach_user(&group.id.to_string(), None, tenant)
                .await
                .map(|_| ()),
            None => Err(AppError::internal("a group was attached with no tenant")),
        };
        if let Err(e) = detached {
            warn!(
                request_id = %request.id,
                group_id = %group.id,
                error = %e,
                "A failed coach access grant could not detach a group it attached"
            );
        }
    }
    let revoked = if had_coach_access {
        Ok(())
    } else {
        repos
            .users
            .set_manages_roster_by_operator(request.user_id, false, operator)
            .await
    };
    let reopened = repos.coach_access_requests.reopen(request.id).await;
    if revoked.is_err() || !matches!(reopened, Ok(true)) {
        warn!(
            request_id = %request.id,
            target_user_id = %request.user_id,
            revoke = ?revoked.err(),
            reopen = ?reopened,
            "A failed coach access grant could not be fully undone"
        );
    }
}

/// Decline a pending request, keeping the super-admin's `reason` when they
/// gave one (trimmed and bounded; blank means none). Nothing else changes;
/// the coach may ask again.
///
/// Once the decline stands, `notices` tells the coach (best-effort): the
/// in-app notice and the email, each with the reason. `None` tells nobody
/// but the notify channel.
///
/// # Errors
///
/// Returns not-found for an unknown request, a conflict when it was already
/// decided, and the repository error otherwise.
pub async fn decline(
    repos: &RepositoryRegistry,
    request_id: Uuid,
    operator: Option<Uuid>,
    reason: Option<&str>,
    notices: Option<&CoachAccessNotices>,
) -> AppResult<CoachAccessRequest> {
    let request = pending_request(repos, request_id).await?;
    let decided_at = Utc::now();
    let reason = normalize_decline_reason(reason);
    if !repos
        .coach_access_requests
        .decide(
            request_id,
            CoachAccessStatus::Declined,
            operator,
            decided_at,
            reason.as_deref(),
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
    cache_requester_email(repos, request.user_id).await;
    info!(
        target: "notify",
        event = "coach_access.declined",
        user_id = %request.user_id,
        request_id = %request_id,
        "coach access declined"
    );
    let declined = CoachAccessRequest {
        status: CoachAccessStatus::Declined,
        decided_at: Some(decided_at),
        decided_by: operator,
        decline_reason: reason,
        ..request
    };
    if let Some(notices) = notices {
        notices.tell_declined(repos, &declined).await;
    }
    Ok(declined)
}

/// Withdraw `user_id`'s waiting request, so the coach may ask again at once.
///
/// The row is deleted, and the operators hear it on the notify channel
/// (`coach_access.cancelled`), so the card they were about to decide is known
/// to be gone. Returns the request the coach reads from now on
/// ([`CoachAccessRequestRepository::current_for_user`]): an earlier grant,
/// or `None` — never a decline the withdrawn ask had replaced, so they see
/// the plain ask.
///
/// [`CoachAccessRequestRepository::current_for_user`]: pierre_database::repositories::CoachAccessRequestRepository::current_for_user
///
/// # Errors
///
/// Returns not-found when the user has no waiting request, a conflict when a
/// super-admin's decision claimed it first, and the repository error
/// otherwise.
pub async fn withdraw(
    repos: &RepositoryRegistry,
    user_id: Uuid,
) -> AppResult<Option<CoachAccessRequest>> {
    let pending = repos
        .coach_access_requests
        .latest_for_user(user_id)
        .await?
        .filter(|r| r.status == CoachAccessStatus::Pending)
        .ok_or_else(|| AppError::not_found("Waiting coach access request"))?;
    if !repos
        .coach_access_requests
        .withdraw(pending.id, user_id)
        .await?
    {
        return Err(already_decided());
    }
    info!(
        request_id = %pending.id,
        target_user_id = %user_id,
        "Coach access request withdrawn by its coach"
    );
    cache_requester_email(repos, user_id).await;
    info!(
        target: "notify",
        event = "coach_access.cancelled",
        user_id = %user_id,
        request_id = %pending.id,
        "coach access request withdrawn"
    );
    repos.coach_access_requests.current_for_user(user_id).await
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
