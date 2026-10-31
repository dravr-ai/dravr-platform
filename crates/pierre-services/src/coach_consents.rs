// ABOUTME: What a member lets their group's human coach do beyond reading their training: calendar writes and wellness reads
// ABOUTME: Held on the member's coach relationship, granted only from Dravr's own app, revocable from anywhere; a wellness revoke purges

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # Coach consents
//!
//! Joining a group is the member's consent to share their training with its
//! human coach (`coach_sharing_consent`). Two further consents are the
//! member's alone, held on that same coach relationship, their membership row
//! ([`CoachConsents`]), never on a link:
//!
//! - `calendar_write`: the group's coach writes the member's training
//!   calendar on a coaching platform, through the coach's own account there.
//!   It holds whether the member reaches the platform through a link to the
//!   coach's account or through a connection of their own. Offered where a
//!   coaching platform takes a coach's calendar writes
//!   ([`ProviderCapabilities::supports_coach_calendar_write`]).
//! - `wellness_read`: the member's sleep, HRV, resting heart rate, weight and
//!   daily note sync through the coach's account. Offered while the member
//!   reads a platform whose coach credential reads them
//!   ([`crate::coach_platform::CoachPlatform::reads_member_wellness`])
//!   through a confirmed link in the group.
//!
//! A link only binds a platform athlete to the member; it is how a member with
//! no connection of their own is onboarded, and it carries no consent.
//!
//! Each consent is off until the member grants it: when they confirm a link,
//! through an explicit choice (a confirm that names none grants nothing), or
//! at any time on their membership ([`set_coach_consents`]). A grant that is
//! not offered is ignored, never stored. A grant is accepted only from
//! Dravr's own app, never over an API key or another external transport; a
//! revoke is accepted from anywhere. Revoking `wellness_read` deletes the
//! wellness every confirmed link of the member's in the group wrote under
//! their tenant, with its attestation row; the workouts the links read stay.
//!
//! No coach can change a consent: every write is scoped to the member's own
//! membership.
//!
//! A wellness source of the member's own wins over a link whatever they
//! granted ([`wellness_read_skipped`]): the link then syncs none of their
//! wellness, and both apps say so beside the consent.

use std::cmp::Ordering;

use chrono::Utc;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::groups::{CoachConsents, CoachingGroup, GroupMember};
use pierre_core::models::{DelegatedConnection, DelegationStatus};
use pierre_core::transport::Transport;
use pierre_database::repositories::{CoachingGroupRepository, PurgeReason};
use pierre_database::RepositoryRegistry;
use pierre_providers::ai_scope;
use pierre_providers::backend_resolver::backend_pair_for;
use pierre_providers::precedence::{rank, wellness_source, Kind};
use pierre_providers::registry::{global_registry, ProviderRegistry};
#[cfg(doc)]
use pierre_providers::spi::ProviderCapabilities;
use serde_json::json;
use tracing::info;
use uuid::Uuid;

use crate::coach_platform::{coach_platform, coach_platforms};
use crate::delegated_read::{holds_own_connection, holds_own_session, OwnLogins};

/// The reason a grant made outside Dravr's own app is refused with, in
/// `details.reason`.
const GRANT_NEEDS_APP: &str = "consent_grant_needs_app";

/// Which coach consents are offered: by a member's coach relationship right
/// now ([`consent_offer`]), or by one link's platform ([`link_offer`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsentOffer {
    /// A coaching platform takes a coach's writes to an athlete's calendar.
    pub calendar_write: bool,
    /// The member reads a platform through a confirmed link whose coach
    /// credential reads their wellness.
    pub wellness_read: bool,
}

/// Whether a coaching platform Dravr serves takes a coach's writes to an
/// athlete's calendar.
#[must_use]
pub fn calendar_write_offered(registry: &ProviderRegistry) -> bool {
    coach_platforms(registry).iter().any(|platform| {
        registry
            .get_capabilities(platform.backend())
            .is_some_and(|caps| caps.supports_coach_calendar_write())
    })
}

/// Whether `link`'s platform reads its member's wellness through the coach's
/// credential.
#[must_use]
pub fn link_reads_wellness(registry: &ProviderRegistry, link: &DelegatedConnection) -> bool {
    coach_platform(registry, &link.provider)
        .is_some_and(|platform| platform.reads_member_wellness())
}

/// The consents `link`'s own platform takes: calendar writes where it takes a
/// coach's, wellness reads where the coach's credential reads it.
#[must_use]
pub fn link_offer(registry: &ProviderRegistry, link: &DelegatedConnection) -> ConsentOffer {
    ConsentOffer {
        calendar_write: registry
            .get_capabilities(&link.provider)
            .is_some_and(|caps| caps.supports_coach_calendar_write()),
        wellness_read: link_reads_wellness(registry, link),
    }
}

/// The consents `member`'s relationship with the coach of `group` can carry.
///
/// None while the group has no coach; else calendar writes where a platform
/// takes them, wellness reads while one of the member's confirmed links in
/// the group reads wellness.
///
/// # Errors
///
/// Returns the repository error when the member's links cannot be read.
pub async fn consent_offer(
    repos: &RepositoryRegistry,
    registry: &ProviderRegistry,
    group: &CoachingGroup,
    member: Uuid,
) -> AppResult<ConsentOffer> {
    if group.coach_user_id.is_none() {
        return Ok(ConsentOffer {
            calendar_write: false,
            wellness_read: false,
        });
    }
    let links = repos
        .delegated_connections
        .list_live_for_member_in_group(group.id, member)
        .await?;
    Ok(ConsentOffer {
        calendar_write: calendar_write_offered(registry),
        wellness_read: links.iter().any(|link| {
            link.status == DelegationStatus::Confirmed && link_reads_wellness(registry, link)
        }),
    })
}

/// The consents `member` gave the coach of the group `group_id`: those on
/// their live membership, none when they are no longer a member.
///
/// # Errors
///
/// Returns the repository error when the membership cannot be read.
pub async fn member_consents(
    groups: &dyn CoachingGroupRepository,
    group_id: Uuid,
    member: Uuid,
) -> AppResult<CoachConsents> {
    Ok(groups
        .get_member(&group_id.to_string(), member)
        .await?
        .map(|membership| membership.coach_consents)
        .unwrap_or_default())
}

/// The consents `link`'s member gave the coach of the link's group.
///
/// # Errors
///
/// Returns the repository error when the membership cannot be read.
pub async fn link_consents(
    groups: &dyn CoachingGroupRepository,
    link: &DelegatedConnection,
) -> AppResult<CoachConsents> {
    member_consents(groups, link.group_id, link.member_user_id).await
}

/// Why a link syncs none of its member's wellness, whatever they granted: a
/// source of their own already gives Dravr it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WellnessReadSkip {
    /// The member holds a credential of their own for the link's platform,
    /// in any tenant, which wins over every link.
    OwnConnection,
    /// The member holds a wellness connection of their own whose service
    /// ranks above the link's (a Garmin, a WHOOP...): it stays the source of
    /// record for what it measures.
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
/// A confirmed link checks the member's own wellness connections that rank
/// above the link's platform for recovery under the one precedence rule
/// ([`own_wellness_backends`]) in the tenant it files under; a proposal, which names no tenant yet, checks
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
    let backends = own_wellness_backends(&global_registry(), &link.provider);
    let backends: Vec<&str> = backends.iter().map(String::as_str).collect();
    if holds_own_connection(logins, member, link.member_tenant_id, &backends).await? {
        return Ok(Some(WellnessReadSkip::OwnWellnessSource));
    }
    Ok(None)
}

/// Every backend a member's own wellness connection that outranks `link`'s
/// platform is stored under: each connection the registry knows to sync
/// sleep or recovery whose source [`rank`] puts above the link's for
/// recovery, with its usr/pwd mirror.
///
/// A link's wellness is then not stored at all (a write-side skip): the
/// member's own service is the source of record for what it measures, and a
/// lower-ranked relay of it would only be a second copy.
fn own_wellness_backends(registry: &ProviderRegistry, link: &str) -> Vec<String> {
    let link = wellness_source(link, None);
    let mut keys: Vec<&str> = registry
        .supported_providers()
        .into_iter()
        .filter(|key| registry.supports_sleep(key) || registry.supports_recovery(key))
        .filter(|key| rank(Kind::Recovery, &wellness_source(key, None), &link) == Ordering::Less)
        .collect();
    keys.sort_unstable();
    let mut backends: Vec<String> = keys.into_iter().flat_map(backend_pair_for).collect();
    backends.dedup();
    backends
}

/// The member's change to their coach consents: `Some` grants or revokes
/// one, `None` leaves it as it stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ConsentChange {
    /// Grant or revoke the coach's writes to the member's calendar.
    pub calendar_write: Option<bool>,
    /// Grant or revoke the sync of the member's wellness.
    pub wellness_read: Option<bool>,
}

impl ConsentChange {
    /// The grants a link confirm asks for: each consent named is granted,
    /// each one left out is left as it stands.
    #[must_use]
    pub const fn grants(calendar_write: bool, wellness_read: bool) -> Self {
        Self {
            calendar_write: if calendar_write { Some(true) } else { None },
            wellness_read: if wellness_read { Some(true) } else { None },
        }
    }

    /// Whether the change grants anything.
    pub(crate) const fn grants_any(self) -> bool {
        matches!(self.calendar_write, Some(true)) || matches!(self.wellness_read, Some(true))
    }

    /// The change, with every grant `offer` does not carry dropped. A revoke
    /// always stands: withdrawing what is not offered is harmless.
    pub(crate) const fn within(self, offer: ConsentOffer) -> Self {
        Self {
            calendar_write: keep(self.calendar_write, offer.calendar_write),
            wellness_read: keep(self.wellness_read, offer.wellness_read),
        }
    }
}

/// `change`, unless it grants what is not `offered`.
const fn keep(change: Option<bool>, offered: bool) -> Option<bool> {
    match change {
        Some(true) if !offered => None,
        other => other,
    }
}

/// `member` grants or revokes their consents to the coach of `group`, as
/// `change` says.
///
/// A grant the relationship does not offer is ignored ([`consent_offer`]).
/// Revoking `wellness_read` deletes the wellness the member's confirmed
/// links in the group wrote, whatever it stood at before, so a revoke
/// repeated after a failed purge deletes what the first one left.
///
/// # Errors
///
/// Returns a permission-denied error (`consent_grant_needs_app`) when it
/// grants a consent outside Dravr's own app, a not-found error when `member`
/// is no longer a live member of `group`, or a repository error.
pub async fn set_coach_consents(
    repos: &RepositoryRegistry,
    registry: &ProviderRegistry,
    group: &CoachingGroup,
    member: Uuid,
    change: ConsentChange,
) -> AppResult<GroupMember> {
    require_app_for_grant(change.grants_any())?;
    let offer = consent_offer(repos, registry, group, member).await?;
    let change = change.within(offer);
    let updated = repos
        .groups
        .set_coach_consents(
            &group.id.to_string(),
            member,
            change.calendar_write,
            change.wellness_read,
            Utc::now(),
        )
        .await?
        .ok_or_else(|| AppError::not_found("Group member"))?;
    info!(
        group_id = %group.id,
        member_user_id = %member,
        calendar_write = updated.coach_consents.calendar_write,
        wellness_read = updated.coach_consents.wellness_read,
        "Coach consents set"
    );
    if change.wellness_read == Some(false) {
        purge_member_wellness(repos, group, member).await?;
    }
    Ok(updated)
}

/// Delete the wellness `member`'s confirmed links in `group` wrote: every row
/// the wellness sync filed for them from each link's platform under the
/// tenant they confirmed in, where the link's reads are filed (the sleep,
/// recovery and health rows, the data sources they reference and the sync's
/// cursors over them).
async fn purge_member_wellness(
    repos: &RepositoryRegistry,
    group: &CoachingGroup,
    member: Uuid,
) -> AppResult<()> {
    let links = repos
        .delegated_connections
        .list_live_for_member_in_group(group.id, member)
        .await?;
    for link in links
        .iter()
        .filter(|link| link.status == DelegationStatus::Confirmed)
    {
        let tenant = link
            .member_tenant_id
            .ok_or_else(|| AppError::internal("A confirmed link carries no member tenant"))?;
        let purge = repos
            .provider_data
            .purge_user_provider_wellness(
                member,
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
    }
    Ok(())
}

/// Refuse a grant made anywhere but Dravr's own app.
///
/// `grants` says whether the request grants anything. A grant is accepted
/// only when the call is declared served over a first-party transport (the
/// member's session in Dravr's app); an API key, an MCP or A2A client, or an
/// undeclared caller cannot grant. A request that only revokes passes.
///
/// # Errors
///
/// Returns a permission-denied error (`consent_grant_needs_app`) for a grant
/// made outside Dravr's own app.
pub(crate) fn require_app_for_grant(grants: bool) -> AppResult<()> {
    if !grants || ai_scope::declared_transport().is_some_and(Transport::is_first_party) {
        return Ok(());
    }
    let mut error = AppError::new(
        ErrorCode::PermissionDenied,
        "Grant what your human coach may do in the Dravr app; from here it can only be revoked",
    );
    error.details = Some(Box::new(json!({ "reason": GRANT_NEEDS_APP })));
    Err(error)
}

#[cfg(test)]
mod tests {
    use pierre_core::constants::oauth_providers::{
        GARMIN, INTERVALS_ICU, SCIOTTE_GARMIN, TRAININGPEAKS, WHOOP,
    };

    use super::*;

    /// A scrape-connected Garmin athlete's session and connection are both
    /// stored under the mirror's slug, so checking Garmin by its own name
    /// alone would miss nearly every Garmin athlete.
    #[test]
    fn an_own_garmin_source_counts_under_its_scraped_mirror_too() {
        let registry = ProviderRegistry::new();
        for link in [INTERVALS_ICU, TRAININGPEAKS] {
            let backends = own_wellness_backends(&registry, link);
            for slug in [GARMIN, SCIOTTE_GARMIN, WHOOP] {
                assert!(
                    backends.iter().any(|b| b == slug),
                    "{link} {slug}: {backends:?}"
                );
            }
            assert!(
                !backends.iter().any(|b| b == link),
                "a link never outranks itself: {backends:?}"
            );
        }
    }

    /// The skip follows the one precedence rule, not a list: a source that
    /// ranks below the link's platform for recovery keeps nothing from it.
    #[test]
    fn only_a_source_ranking_above_the_link_keeps_its_wellness_from_syncing() {
        let registry = ProviderRegistry::new();
        let backends = own_wellness_backends(&registry, GARMIN);
        assert!(backends.iter().any(|b| b == WHOOP), "{backends:?}");
        assert!(!backends.iter().any(|b| b == INTERVALS_ICU), "{backends:?}");
    }

    #[test]
    fn a_relationship_keeps_only_the_grants_it_offers_and_every_revoke() {
        let both = ConsentChange::grants(true, true);
        let calendar_only = ConsentOffer {
            calendar_write: true,
            wellness_read: false,
        };
        assert_eq!(
            both.within(calendar_only),
            ConsentChange {
                calendar_write: Some(true),
                wellness_read: None,
            }
        );
        let nothing = ConsentOffer {
            calendar_write: false,
            wellness_read: false,
        };
        assert_eq!(both.within(nothing), ConsentChange::default());
        let revokes = ConsentChange {
            calendar_write: Some(false),
            wellness_read: Some(false),
        };
        assert_eq!(revokes.within(nothing), revokes);
        assert_eq!(
            ConsentChange::grants(false, false),
            ConsentChange::default()
        );
        assert!(!revokes.grants_any());
        assert!(both.grants_any());
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
