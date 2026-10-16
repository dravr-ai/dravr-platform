// ABOUTME: What a member lets a confirmed coach link do beyond reading their workouts: calendar writes and wellness reads
// ABOUTME: Offered per platform, granted only by the member from Dravr's own app, revocable from anywhere; a wellness revoke purges

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Per-link consents
//!
//! Confirming a link is the member's consent to having their workouts on the
//! coaching platform read through the coach's account, and to nothing more
//! ([`crate::delegated_connections`]). Two further consents are the member's
//! alone, per link ([`LinkConsents`]):
//!
//! - `calendar_write`: the link's coach writes the member's training calendar
//!   on the platform. Offered only where the platform takes a coach's calendar
//!   writes ([`ProviderCapabilities::supports_coach_calendar_write`]).
//! - `wellness_read`: the member's sleep, HRV, resting heart rate, weight and
//!   daily note sync through the coach's account. Offered only where the
//!   coach's credential reads them
//!   ([`crate::coach_platform::CoachPlatform::reads_member_wellness`]).
//!
//! Each is off until the member grants it: at confirm, through an explicit
//! choice (a confirm that names none grants nothing), or later on the
//! confirmed link ([`set_link_consents`]). A consent the platform does not
//! offer is ignored, never stored. A grant is accepted only from Dravr's own
//! app, never over an API key or another external transport; a revoke is
//! accepted from anywhere. Revoking `wellness_read` deletes the wellness the
//! link wrote under the member's tenant for that platform, with its
//! attestation row; the workouts the link reads stay.
//!
//! No coach can change a consent: every write is scoped to the member the
//! link names.
//!
//! A wellness source of the member's own wins over the link whatever they
//! granted ([`wellness_read_skipped`]): the link then syncs none of their
//! wellness, and both apps say so beside the consent.

use chrono::Utc;
use pierre_core::constants::oauth_providers::{GARMIN, WHOOP};
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::groups::CoachingGroup;
use pierre_core::models::{DelegatedConnection, LinkConsents};
use pierre_core::transport::Transport;
use pierre_database::repositories::PurgeReason;
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope;
use pierre_providers::backend_resolver::backend_pair_for;
use pierre_providers::registry::ProviderRegistry;
#[cfg(doc)]
use pierre_providers::spi::ProviderCapabilities;
use serde_json::json;
use tracing::info;

use crate::coach_platform::coach_platform;
use crate::delegated_read::{holds_own_connection, holds_own_session, OwnLogins};

/// The reason a grant made outside Dravr's own app is refused with, in
/// `details.reason`.
const GRANT_NEEDS_APP: &str = "consent_grant_needs_app";

/// Which consents a link on one platform can carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsentOffer {
    /// The platform takes a coach's writes to the athlete's calendar.
    pub calendar_write: bool,
    /// The coach's credential reads the athlete's wellness.
    pub wellness_read: bool,
}

impl ConsentOffer {
    /// `requested`, keeping only what the platform offers.
    const fn keep(self, requested: LinkConsents) -> LinkConsents {
        LinkConsents {
            calendar_write: requested.calendar_write && self.calendar_write,
            wellness_read: requested.wellness_read && self.wellness_read,
        }
    }
}

/// The consents a link on `backend` can carry.
#[must_use]
pub fn consent_offer(registry: &ProviderRegistry, backend: &str) -> ConsentOffer {
    ConsentOffer {
        calendar_write: registry
            .get_capabilities(backend)
            .is_some_and(|caps| caps.supports_coach_calendar_write()),
        wellness_read: coach_platform(registry, backend)
            .is_some_and(|platform| platform.reads_member_wellness()),
    }
}

/// Why a link syncs none of its member's wellness, whatever they granted: a
/// source of their own already gives Dravr it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WellnessReadSkip {
    /// The member holds a credential of their own for the link's platform,
    /// in any tenant, which wins over every link.
    OwnConnection,
    /// The member holds their own Garmin or WHOOP connection: it stays the
    /// source of record for what it measures.
    OwnWellnessSource,
}

impl WellnessReadSkip {
    /// The name the apps and the logs read it by.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OwnConnection => "own_connection",
            Self::OwnWellnessSource => "own_wellness_source",
        }
    }
}

/// Why `link` syncs none of its member's wellness whether or not they
/// granted it, read from the member's own logins alone; `None` when nothing
/// of theirs keeps it from syncing.
///
/// A confirmed link checks the member's own Garmin or WHOOP connection in
/// the tenant it files under; a proposal, which names no tenant yet, checks
/// every tenant of theirs. A WHOOP record read back through another platform
/// would also pass by WHOOP's own terms gate, which keys on WHOOP as the
/// record's source.
///
/// # Errors
///
/// Returns the repository error when the member's sessions or connections
/// cannot be read.
pub async fn wellness_read_skipped(
    logins: OwnLogins<'_>,
    link: &DelegatedConnection,
) -> AppResult<Option<WellnessReadSkip>> {
    let member = link.member_user_id;
    if holds_own_session(logins.tokens, member, None, &[&link.provider]).await? {
        return Ok(Some(WellnessReadSkip::OwnConnection));
    }
    let backends = own_wellness_backends();
    let backends: Vec<&str> = backends.iter().map(String::as_str).collect();
    if holds_own_connection(logins, member, link.member_tenant_id, &backends).await? {
        return Ok(Some(WellnessReadSkip::OwnWellnessSource));
    }
    Ok(None)
}

/// Every backend a member's own Garmin or WHOOP connection is stored under.
fn own_wellness_backends() -> Vec<String> {
    [GARMIN, WHOOP]
        .into_iter()
        .flat_map(backend_pair_for)
        .collect()
}

/// The member's change to a confirmed link's consents: `Some` grants or
/// revokes one, `None` leaves it as it stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConsentChange {
    /// Grant or revoke the coach's writes to the member's calendar.
    pub calendar_write: Option<bool>,
    /// Grant or revoke the sync of the member's wellness.
    pub wellness_read: Option<bool>,
}

/// The consents the member's confirm of `link` grants, of the `requested`
/// ones: those its platform offers.
///
/// # Errors
///
/// Returns a permission-denied error (`consent_grant_needs_app`) when it
/// requests a grant outside Dravr's own app.
pub(crate) fn granted_at_confirm(
    registry: &ProviderRegistry,
    link: &DelegatedConnection,
    requested: LinkConsents,
) -> AppResult<LinkConsents> {
    require_app_for_grant(requested.calendar_write || requested.wellness_read)?;
    Ok(consent_offer(registry, &link.provider).keep(requested))
}

/// The member `link` names grants or revokes its consents, as `change` says.
///
/// `link` is a confirmed link of `group`, read for that member. A consent its
/// platform does not offer is left as it stands. Revoking `wellness_read`
/// deletes the wellness the link wrote, whatever it stood at before, so a
/// revoke repeated after a failed purge deletes what the first one left.
///
/// # Errors
///
/// Returns a permission-denied error (`consent_grant_needs_app`) when it
/// grants a consent outside Dravr's own app, a not-found error when `link`
/// is no longer a confirmed link naming the member, or a repository error.
pub async fn set_link_consents(
    repos: &RepositoryRegistry,
    registry: &ProviderRegistry,
    group: &CoachingGroup,
    link: &DelegatedConnection,
    change: ConsentChange,
) -> AppResult<DelegatedConnection> {
    require_app_for_grant(
        change.calendar_write == Some(true) || change.wellness_read == Some(true),
    )?;
    let offer = consent_offer(registry, &link.provider);
    let calendar_write = change.calendar_write.filter(|_| offer.calendar_write);
    let wellness_read = change.wellness_read.filter(|_| offer.wellness_read);
    let updated = repos
        .delegated_connections
        .set_link_consents(
            link.id,
            link.member_user_id,
            calendar_write,
            wellness_read,
            Utc::now(),
        )
        .await?
        .ok_or_else(|| AppError::not_found("Confirmed link"))?;
    info!(
        link_id = %updated.id,
        group_id = %group.id,
        calendar_write = updated.calendar_write,
        wellness_read = updated.wellness_read,
        "Delegated link consents set"
    );
    if wellness_read == Some(false) {
        purge_link_wellness(repos, group, &updated).await?;
    }
    Ok(updated)
}

/// Delete the wellness `link` wrote: every row the wellness sync filed for
/// the member from the link's platform under the tenant they confirmed in,
/// where the link's reads are filed (the sleep, recovery and health rows, the
/// data sources they reference and the sync's cursors over them).
async fn purge_link_wellness(
    repos: &RepositoryRegistry,
    group: &CoachingGroup,
    link: &DelegatedConnection,
) -> AppResult<()> {
    let tenant = link
        .member_tenant_id
        .ok_or_else(|| AppError::internal("A confirmed link carries no member tenant"))?;
    let purge = repos
        .provider_data
        .purge_user_provider_wellness(
            link.member_user_id,
            &tenant,
            &link.provider,
            PurgeReason::WellnessConsentWithdrawn,
        )
        .await?;
    info!(
        link_id = %link.id,
        group_id = %group.id,
        removed = purge.total(),
        "Deleted the wellness read through a delegated link"
    );
    Ok(())
}

/// Refuse a grant made anywhere but Dravr's own app.
///
/// `grants` says whether the request grants anything. A grant is accepted
/// only when the call is declared served over a first-party transport (the
/// member's session in Dravr's app); an API key, an MCP or A2A client, or an
/// undeclared caller cannot grant. A request that only revokes passes.
fn require_app_for_grant(grants: bool) -> AppResult<()> {
    if !grants || ai_scope::declared_transport().is_some_and(Transport::is_first_party) {
        return Ok(());
    }
    let mut error = AppError::new(
        ErrorCode::PermissionDenied,
        "Grant what a coach link shares in the Dravr app; from here it can only be revoked",
    );
    error.details = Some(Box::new(json!({ "reason": GRANT_NEEDS_APP })));
    Err(error)
}

#[cfg(test)]
mod tests {
    use pierre_core::constants::oauth_providers::SCIOTTE_GARMIN;

    use super::*;

    /// A scrape-connected Garmin athlete's session and connection are both
    /// stored under the mirror's slug, so checking Garmin by its own name
    /// alone would miss nearly every Garmin athlete.
    #[test]
    fn an_own_garmin_source_counts_under_its_scraped_mirror_too() {
        let backends = own_wellness_backends();
        for slug in [GARMIN, SCIOTTE_GARMIN, WHOOP] {
            assert!(backends.iter().any(|b| b == slug), "{slug}: {backends:?}");
        }
    }

    const BOTH: LinkConsents = LinkConsents {
        calendar_write: true,
        wellness_read: true,
    };

    #[test]
    fn a_platform_keeps_only_the_consents_it_offers() {
        let calendar_only = ConsentOffer {
            calendar_write: true,
            wellness_read: false,
        };
        assert_eq!(
            calendar_only.keep(BOTH),
            LinkConsents {
                calendar_write: true,
                wellness_read: false,
            }
        );
        let nothing = ConsentOffer {
            calendar_write: false,
            wellness_read: false,
        };
        assert_eq!(nothing.keep(BOTH), LinkConsents::default());
        let everything = ConsentOffer {
            calendar_write: true,
            wellness_read: true,
        };
        assert_eq!(
            everything.keep(LinkConsents::default()),
            LinkConsents::default()
        );
        assert_eq!(everything.keep(BOTH), BOTH);
    }

    #[tokio::test]
    async fn only_dravrs_own_app_grants_and_anywhere_revokes() {
        for transport in [Transport::WebApp, Transport::MobileApp] {
            assert!(
                ai_scope::serve_over(transport, async { require_app_for_grant(true) })
                    .await
                    .is_ok(),
                "{transport:?}"
            );
        }
        for transport in [Transport::ApiKey, Transport::McpHttp, Transport::A2a] {
            let refused = ai_scope::serve_over(transport, async { require_app_for_grant(true) })
                .await
                .expect_err("an external caller cannot grant");
            assert!(
                refused.code == ErrorCode::PermissionDenied,
                "{transport:?}: {refused:?}"
            );
            assert_eq!(
                refused.details.as_deref().map(|d| d["reason"].clone()),
                Some(json!(GRANT_NEEDS_APP)),
                "{transport:?}"
            );
            assert!(
                ai_scope::serve_over(transport, async { require_app_for_grant(false) })
                    .await
                    .is_ok(),
                "a revoke passes over {transport:?}"
            );
        }
        assert!(
            require_app_for_grant(true).is_err(),
            "a caller that declared no transport is not vouched for"
        );
        assert!(require_app_for_grant(false).is_ok());
    }
}
