// ABOUTME: Whether a member's coaching-platform read goes through their human coach's own credential, decided once
// ABOUTME: Own credential wins, then the confirmed link the group still backs, bound by email, through the coach's live session

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # The delegated read decision
//!
//! A member whose human coach linked them on a coaching platform, and who
//! confirmed the link, reads their platform data through the coach's own
//! stored credential, naming the member's athlete id on every read
//! ([`crate::delegated_connections`]). Every path that serves such a read
//! decides it here, in this order ([`member_read`]):
//!
//! 1. a credential of the member's own for the platform wins over any link;
//! 2. the member's confirmed link must be one the group relation still backs:
//!    the group is active, the member is in it, and its coach is the link's;
//! 3. the roster athlete the link names must be the member by email
//!    ([`link_binding`]);
//! 4. the coach's own session must be live, and its credential one that
//!    reads athletes ([`CoachPlatform::coach_credentials`]), counted against
//!    the coach's own key ([`with_key_budget`]).
//!
//! The live read path (the tool runtime's delegated subject), the scheduled
//! wellness sync (`crate::delegated_wellness`) and a human coach's write to
//! the member's calendar (the tool runtime's coach calendar writer, which
//! also requires the link to be the writing coach's own) all go through it.
//! What a refusal does — the words a read answers, ending a link whose coach
//! holds no credential any more — stays with each caller.

use pierre_core::errors::AppResult;
use pierre_core::models::{ConnectionType, DelegatedConnection, DelegationStatus, TenantId};
use pierre_database::repositories::{OAuthTokenRepository, ProviderConnectionRepository};
use pierre_database::RepositoryRegistry;
use pierre_providers::registry::ProviderRegistry;
use pierre_providers::OAuth2Credentials;
use uuid::Uuid;

use crate::coach_platform::{with_key_budget, CoachPlatform};
use crate::delegated_connections::{coach_session_state, CoachSession};
use crate::trainingpeaks_accounts::{link_binding, EmailBinding};

/// Why a member's confirmed link serves no read through the coach's
/// credential.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoachReadRefusal {
    /// The roster athlete the link names is not the member by email; never
    /// [`EmailBinding::Bound`].
    Unbound(EmailBinding),
    /// The coach's session needs the coach to sign in again, or holds a
    /// credential that reads no athletes: only the coach can renew it.
    CoachNeedsReconnect,
    /// The coach holds no session of their own for the platform any more.
    CoachDisconnected,
}

/// A member's read served through their coach's credential.
pub struct CoachRead {
    /// The confirmed link the read goes through.
    pub link: DelegatedConnection,
    /// The coach's credential, counted against the coach's own key.
    pub credentials: OAuth2Credentials,
    /// The tenant the coach's credential was read under, the link's own:
    /// every roster read on this path uses it, never the request's tenant.
    pub coach_tenant: TenantId,
}

/// How a member's read of a coaching platform is served.
pub enum MemberRead {
    /// A credential of the member's own serves it: no link is read.
    OwnCredential,
    /// The member holds no confirmed link the group relation backs.
    NotLinked,
    /// The member's confirmed link serves no read, for `refusal`.
    Refused {
        /// The link that was refused.
        link: Box<DelegatedConnection>,
        /// Why it serves no read.
        refusal: CoachReadRefusal,
    },
    /// The read goes through the coach's credential.
    ThroughCoach(Box<CoachRead>),
}

/// How `member`'s read of `platform` is served: by a credential of their
/// own, through their confirmed link and the coach's credential, or not at
/// all, and why.
///
/// `tenant` is the tenant the read runs in: the member's own credential and
/// the link are looked up there. `None` reads across the member's tenants, as
/// a scheduled pass that serves no one tenant does: any own credential of the
/// member's wins, and the link is the member's one confirmed link on the
/// platform.
///
/// # Errors
///
/// Returns the repository error when the member's credentials, the link, the
/// member's verification or the coach's session cannot be read.
pub async fn member_read(
    repos: &RepositoryRegistry,
    registry: &ProviderRegistry,
    platform: &dyn CoachPlatform,
    member: Uuid,
    tenant: Option<TenantId>,
) -> AppResult<MemberRead> {
    let backend = platform.backend();
    if holds_own_session(repos.oauth_tokens.as_ref(), member, tenant, &[backend]).await? {
        return Ok(MemberRead::OwnCredential);
    }
    let link = match tenant {
        Some(tenant) => {
            repos
                .delegated_connections
                .find_active_for_member(member, tenant, backend)
                .await?
        }
        None => repos
            .delegated_connections
            .list_backed_for_member(member, backend)
            .await?
            .into_iter()
            .find(|link| link.status == DelegationStatus::Confirmed),
    };
    let Some(link) = link else {
        return Ok(MemberRead::NotLinked);
    };
    let binding = link_binding(repos, &link).await?;
    if binding != EmailBinding::Bound {
        return Ok(refused(link, CoachReadRefusal::Unbound(binding)));
    }
    let token = match coach_session_state(
        repos,
        link.coach_user_id,
        link.coach_tenant_id,
        &link.provider,
    )
    .await?
    {
        CoachSession::Live { token, .. } => token,
        CoachSession::NeedsReconnect => {
            return Ok(refused(link, CoachReadRefusal::CoachNeedsReconnect));
        }
        CoachSession::Missing => return Ok(refused(link, CoachReadRefusal::CoachDisconnected)),
    };
    let Some(credentials) = platform.coach_credentials(&token) else {
        return Ok(refused(link, CoachReadRefusal::CoachNeedsReconnect));
    };
    let coach_tenant = link.coach_tenant_id;
    Ok(MemberRead::ThroughCoach(Box::new(CoachRead {
        credentials: with_key_budget(registry, &token, credentials),
        coach_tenant,
        link,
    })))
}

/// `link`, refused for `refusal`.
fn refused(link: DelegatedConnection, refusal: CoachReadRefusal) -> MemberRead {
    MemberRead::Refused {
        link: Box::new(link),
        refusal,
    }
}

/// The rows a member's own logins to providers live in: their stored
/// sessions and their connections.
#[derive(Clone, Copy)]
pub struct OwnLogins<'a> {
    /// The member's stored provider sessions.
    pub(crate) tokens: &'a dyn OAuthTokenRepository,
    /// The member's provider connections, delegated ones among them.
    pub(crate) connections: &'a dyn ProviderConnectionRepository,
}

impl<'a> OwnLogins<'a> {
    /// The logins `repos` stores.
    #[must_use]
    pub fn of(repos: &'a RepositoryRegistry) -> Self {
        Self {
            tokens: repos.oauth_tokens.as_ref(),
            connections: repos.provider_connections.as_ref(),
        }
    }
}

/// Whether `member` holds a stored session of their own for one of
/// `providers` in `tenant`, or in any tenant when `tenant` is `None`.
pub(crate) async fn holds_own_session(
    tokens: &dyn OAuthTokenRepository,
    member: Uuid,
    tenant: Option<TenantId>,
    providers: &[&str],
) -> AppResult<bool> {
    Ok(tokens
        .get_tokens(member, tenant)
        .await?
        .iter()
        .any(|token| providers.contains(&token.provider.as_str())))
}

/// Whether the member reads one of `providers` through a login of their own
/// in `tenant`, or in any tenant when `tenant` is `None`: their own session,
/// or a connection that is not a delegated one.
pub(crate) async fn holds_own_connection(
    logins: OwnLogins<'_>,
    member: Uuid,
    tenant: Option<TenantId>,
    providers: &[&str],
) -> AppResult<bool> {
    if holds_own_session(logins.tokens, member, tenant, providers).await? {
        return Ok(true);
    }
    Ok(logins
        .connections
        .get_for_user(member, tenant)
        .await?
        .iter()
        .any(|c| {
            providers.contains(&c.provider.as_str())
                && c.connection_type != ConnectionType::Delegated
        }))
}
