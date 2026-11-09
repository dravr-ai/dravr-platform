// ABOUTME: Operator reset of a whole account: every provider disconnected through the chokepoint, every owned row wiped
// ABOUTME: The account, its sign-in, role, tier and operator overrides stay; no tombstone, no receipt, no Firebase deletion

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Starting an account over, from an operator surface (carnet#1463).
//!
//! A delete ([`crate::user_removal`]) ends the account; an onboarding reset
//! ([`crate::onboarding_reset`]) only sends it back through the wizard. This
//! is between the two: the account stays — its id, email, role, tier, sign-in
//! and every operator override — and everything it owns goes, so the next
//! sign-in finds an account with no history, no provider and no memory.
//!
//! In order:
//!
//! 1. Refuse, touching nothing, while the account holds something other
//!    people use: an active group it owns or coaches, an invite it created
//!    to one, an agent it authored that others rely on
//!    (`AccountResetRepository::reset_blockers`). With
//!    [`OwnedGroups::Archive`] the groups it owns do not refuse: they are
//!    read as already archived, and a group it only coaches, or an invite it
//!    made to someone else's group, still refuses.
//!
//!    Then, with [`OwnedGroups::Archive`], archive every active group the
//!    account owns through [`GroupService::delete_group`], the path the owner's own
//!    *Delete group* runs: the group stops being active, its invites stop
//!    working, its members are released, the delegated links it carried end
//!    and the owner's billed athlete count drops. Its room history stays, as
//!    it does for the owner's delete; the wipe then takes the account's own
//!    part of it. The archive runs before any provider is disconnected and,
//!    like a disconnect, stands if a later step fails. The actor is the
//!    account itself, since the operator acts on its behalf.
//! 2. Disconnect every held provider through the disconnect chokepoint
//!    ([`ProviderDisconnector::disconnect`]), coaching platforms included —
//!    unlike an onboarding reset, which keeps them for the groups riding on
//!    them. The chokepoint revokes the grant upstream, deletes the token and
//!    connection rows, purges the provider's data and raises
//!    `provider.disconnected` itself, so this module raises no event of its
//!    own. A coaching platform's disconnect also ends the delegated links on
//!    both sides of it and, for a `TrainingPeaks` coach connection, revokes
//!    the `manages_roster` it earned: the one write a reset makes to the
//!    `users` row, through the chokepoint's own invariant (an operator-made
//!    grant is kept). A provider this build cannot revoke is kept and
//!    reported, as an onboarding reset keeps it, since only the chokepoint
//!    may clear a provider's rows.
//! 3. Erase the history dravr-barrage imported for the account and every
//!    file it uploaded ([`AccountArchiveEraser`]), now rather than through
//!    the delete's erasure outbox: the account stays live, so a sweep
//!    reaching it minutes later could not tell those bytes from what the
//!    athlete uploads or imports after the reset, and would erase the new
//!    ones under rows that still name them. Erasing barrage also ends an
//!    import still running, so a step it delivers later finds no job and
//!    projects nothing into the tables the wipe clears.
//! 4. Wipe every row the account owns in ONE transaction
//!    (`AccountResetRepository::reset_account`), which reads the blockers
//!    again first, and refuse to commit while a wiped table still holds a
//!    row.
//! 5. Evict the account's cached responses and cached email.
//!
//! A failure in steps 2 to 4 is an [`AccountResetInterruption`] naming what
//! was already disconnected and whether the imported history was already
//! erased; the wipe is one transaction and runs last, so it either all
//! happened or none of it did. A blocker the second read finds is a refusal
//! naming the same.

use std::collections::BTreeMap;
use std::fmt;

use async_trait::async_trait;
use pierre_cache::Cache;
use pierre_core::errors::{AppError, AppResult};
use pierre_core::models::UserReference;
use pierre_database::repositories::{AccountResetCommit, OwnedGroup};
use pierre_database::RepositoryRegistry;
use pierre_groups::GroupService;
use serde::Serialize;
use tracing::info;
use uuid::Uuid;

use crate::provider_revocation::DisconnectReason;
use crate::user_removal::{
    self, DisconnectedProvider, HeldProvider, Interruption, ProviderDisconnector,
};

/// What erasing an account's imported history removed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ArchiveErasure {
    /// Activity and wellness records barrage held for the account.
    pub records: u64,
    /// Import jobs, a running one included.
    pub jobs: u64,
    /// Blobs: barrage's originals and spools, and the files the account
    /// uploaded.
    pub blobs: u64,
}

/// Erases what lives outside the database for one account.
///
/// That is the history dravr-barrage imported for it in every tenant
/// (records, jobs, originals, spools) and every file it uploaded. The server
/// implements it over its importer and originals store; this crate sees
/// neither.
#[async_trait]
pub trait AccountArchiveEraser: Send + Sync {
    /// Erase everything named above for `user_id`; idempotent, so a retry
    /// after a failure part-way finishes the job.
    ///
    /// # Errors
    ///
    /// When barrage or the blob store fails; what was erased before stays
    /// erased.
    async fn erase(&self, user_id: Uuid) -> AppResult<ArchiveErasure>;
}

/// What a reset does with the active groups the account owns.
#[derive(Clone, Copy)]
pub enum OwnedGroups<'svc> {
    /// Refuse the reset while the account owns one.
    Refuse,
    /// Archive each one first, through the owner's own delete path.
    Archive(&'svc GroupService),
}

impl fmt::Debug for OwnedGroups<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Refuse => "Refuse",
            Self::Archive(_) => "Archive",
        })
    }
}

/// A group the reset archived.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArchivedGroup {
    /// The group's id.
    pub group_id: String,
    /// The group's name.
    pub name: String,
}

/// Everything an account reset did, for the operator's response.
#[derive(Debug, Clone, Default, Serialize)]
pub struct AccountResetReport {
    /// Active groups the account owned, archived before anything else ran.
    pub groups_archived: Vec<ArchivedGroup>,
    /// Providers disconnected through the chokepoint, coaching platforms
    /// included, each with what the provider said about its grant.
    pub disconnected: Vec<DisconnectedProvider>,
    /// Providers this build cannot revoke, left in place because only the
    /// disconnect chokepoint may clear a provider's rows.
    pub not_revocable: Vec<HeldProvider>,
    /// What erasing the account's imported history and uploads removed.
    pub archive_erased: ArchiveErasure,
    /// Rows the wipe deleted, by table.
    pub rows_changed: BTreeMap<String, u64>,
    /// Cache entries removed after the commit; `None` when no cache was given
    /// or it could not be reached (logged).
    pub cache_entries_removed: Option<u64>,
    /// Whether this process held the account's email for notify events and
    /// evicted it.
    pub identity_cache_evicted: bool,
}

/// A reset refused because the account holds something other people use.
#[derive(Debug, Clone)]
pub struct AccountResetRefusal {
    /// What must be reassigned or removed first.
    pub blockers: Vec<UserReference>,
    /// Providers disconnected before the refusal; empty when the first read
    /// refused, before anything was touched.
    pub disconnected: Vec<DisconnectedProvider>,
    /// What was erased before the refusal; `None` when nothing was.
    pub archive_erased: Option<ArchiveErasure>,
    /// Groups archived before the refusal; empty when none were.
    pub groups_archived: Vec<ArchivedGroup>,
}

/// A reset that failed part-way: the failure, what was disconnected before
/// it, and whether the imported history was already erased.
#[derive(Debug, Clone)]
pub struct AccountResetInterruption {
    /// The failing step and the providers disconnected before it.
    pub interruption: Interruption,
    /// What was erased before the failure; `None` when the erasure did not
    /// run or failed itself.
    pub archive_erased: Option<ArchiveErasure>,
    /// Groups archived before the failure; empty when none were.
    pub groups_archived: Vec<ArchivedGroup>,
}

/// The outcome of an account reset attempt.
#[derive(Debug, Clone)]
pub enum AccountResetOutcome {
    /// The account holds these, which other people use; no row the wipe owns
    /// was changed.
    Blocked(AccountResetRefusal),
    /// Every revocable provider was disconnected, the imported history
    /// erased and every owned row wiped.
    Reset(AccountResetReport),
    /// A disconnect, the erasure or the wipe failed; what was done first
    /// stands, and no row the wipe owns was changed.
    Interrupted(AccountResetInterruption),
}

/// The operator-facing refusal naming each reference that blocks a reset,
/// and what was already done when the second read refused it.
#[must_use]
pub fn blockers_message(refusal: &AccountResetRefusal) -> String {
    let mut message = format!(
        "User cannot be reset until these are reassigned or removed: {}",
        user_removal::name_blockers(&refusal.blockers)
    );
    if refusal.archive_erased.is_none()
        && refusal.disconnected.is_empty()
        && refusal.groups_archived.is_empty()
    {
        return message;
    }
    // The second read, inside the wipe's transaction, refused: say what the
    // steps before it already did.
    message.push_str(". Found after the reset began: ");
    if let Some(archived) = describe_archived(&refusal.groups_archived) {
        message.push_str(&archived);
        message.push_str("; ");
    }
    if refusal.disconnected.is_empty() {
        message.push_str("no provider was disconnected");
    } else {
        let each: Vec<String> = refusal
            .disconnected
            .iter()
            .map(DisconnectedProvider::describe)
            .collect();
        message.push_str("already disconnected: ");
        message.push_str(&each.join("; "));
    }
    if refusal.archive_erased.is_some() {
        message.push_str("; the imported history and uploaded files were already erased");
    }
    message.push_str(". No other row the account owns was changed.");
    message
}

/// The groups already archived, as an operator reads it; `None` when none
/// were.
fn describe_archived(groups: &[ArchivedGroup]) -> Option<String> {
    if groups.is_empty() {
        return None;
    }
    let names: Vec<String> = groups.iter().map(|g| format!("'{}'", g.name)).collect();
    Some(format!("already archived the groups {}", names.join(", ")))
}

/// What an interrupted reset already did and what it did not, as an operator
/// reads it; `cause` is the failure as the caller may show it.
#[must_use]
pub fn describe_interruption(interrupted: &AccountResetInterruption, cause: &str) -> String {
    let interruption = &interrupted.interruption;
    let done = interruption.describe_disconnected();
    let mut erased = String::new();
    if let Some(archived) = describe_archived(&interrupted.groups_archived) {
        erased.push_str(" The reset ");
        erased.push_str(&archived);
        erased.push('.');
    }
    if interrupted.archive_erased.is_some() {
        erased.push_str(" The imported history and uploaded files were already erased.");
    }
    interruption.failed.as_ref().map_or_else(
        || {
            format!(
                "The account reset failed: {cause}. {done}.{erased} No other row the account owns was changed."
            )
        },
        |failed| {
            format!(
                "Disconnecting {} failed: {cause}. Its grant may already have been revoked at the provider. {done}.{erased} No other row the account owns was changed.",
                failed.describe()
            )
        },
    )
}

/// Reset a whole account, as the module documentation describes.
///
/// # Errors
///
/// Returns a database error from the reads that precede any change, and a
/// `ResourceUnavailable` error, touching nothing, when no archive eraser is
/// wired (the wipe would leave the imported history and uploads behind) or
/// the account holds a provider and no disconnector is wired (wiping would
/// strand its grant upstream). A failed archive, disconnect, erasure or wipe
/// is [`AccountResetOutcome::Interrupted`], not an error.
pub async fn reset_account(
    repos: &RepositoryRegistry,
    disconnector: Option<&dyn ProviderDisconnector>,
    eraser: Option<&dyn AccountArchiveEraser>,
    cache: Option<&Cache>,
    user_id: Uuid,
    owned_groups: OwnedGroups<'_>,
) -> AppResult<AccountResetOutcome> {
    let archive_owned = matches!(owned_groups, OwnedGroups::Archive(_));
    let blockers = repos
        .account_reset
        .reset_blockers(user_id, archive_owned)
        .await?;
    if !blockers.is_empty() {
        return Ok(AccountResetOutcome::Blocked(AccountResetRefusal {
            blockers,
            disconnected: Vec::new(),
            archive_erased: None,
            groups_archived: Vec::new(),
        }));
    }
    let Some(eraser) = eraser else {
        return Err(AppError::resource_unavailable(
            "Archive erasure is not wired on this server; resetting would leave the user's imported history and uploaded files behind",
        ));
    };

    let held = user_removal::held_providers(repos, user_id).await?;
    let disconnector = match (held.is_empty(), disconnector) {
        (true, _) => None,
        (false, Some(disconnector)) => Some(disconnector),
        (false, None) => {
            return Err(AppError::resource_unavailable(
                "Provider disconnect is not wired on this server; resetting would leave the user's grants authorized at the provider",
            ));
        }
    };

    let mut report = AccountResetReport::default();
    if let OwnedGroups::Archive(groups) = owned_groups {
        let owned = repos.account_reset.owned_active_groups(user_id).await?;
        if let Err(error) = archive_each(groups, user_id, owned, &mut report.groups_archived).await
        {
            return Ok(AccountResetOutcome::Interrupted(AccountResetInterruption {
                interruption: Interruption {
                    disconnected: Vec::new(),
                    failed: None,
                    error,
                },
                archive_erased: None,
                groups_archived: report.groups_archived,
            }));
        }
    }

    if let Some(disconnector) = disconnector {
        let (revocable, not_revocable): (Vec<HeldProvider>, Vec<HeldProvider>) = held
            .into_iter()
            .partition(|target| disconnector.supports(&target.provider));
        report.not_revocable = not_revocable;
        report.disconnected = match user_removal::disconnect_each(
            disconnector,
            user_id,
            revocable,
            DisconnectReason::Operator,
        )
        .await
        {
            Ok(disconnected) => disconnected,
            Err(interruption) => {
                return Ok(AccountResetOutcome::Interrupted(AccountResetInterruption {
                    interruption: *interruption,
                    archive_erased: None,
                    groups_archived: report.groups_archived,
                }))
            }
        };
    }

    report.archive_erased = match eraser.erase(user_id).await {
        Ok(removed) => removed,
        Err(error) => {
            return Ok(AccountResetOutcome::Interrupted(AccountResetInterruption {
                interruption: Interruption {
                    disconnected: report.disconnected,
                    failed: None,
                    error,
                },
                archive_erased: None,
                groups_archived: report.groups_archived,
            }))
        }
    };

    let reset = match repos.account_reset.reset_account(user_id).await {
        Ok(AccountResetCommit::Reset(reset)) => reset,
        Ok(AccountResetCommit::Blocked(blockers)) => {
            return Ok(AccountResetOutcome::Blocked(AccountResetRefusal {
                blockers,
                disconnected: report.disconnected,
                archive_erased: Some(report.archive_erased),
                groups_archived: report.groups_archived,
            }))
        }
        Err(error) => {
            return Ok(AccountResetOutcome::Interrupted(AccountResetInterruption {
                interruption: Interruption {
                    disconnected: report.disconnected,
                    failed: None,
                    error,
                },
                archive_erased: Some(report.archive_erased),
                groups_archived: report.groups_archived,
            }))
        }
    };
    report.rows_changed = reset.rows_changed;
    (report.cache_entries_removed, report.identity_cache_evicted) =
        user_removal::evict_user_caches(cache, user_id).await;

    info!(
        user_id = %user_id,
        groups_archived = report.groups_archived.len(),
        disconnected = report.disconnected.len(),
        not_revocable = report.not_revocable.len(),
        tables_changed = report.rows_changed.len(),
        archive_records_erased = report.archive_erased.records,
        archive_blobs_erased = report.archive_erased.blobs,
        "Account reset by operator"
    );
    Ok(AccountResetOutcome::Reset(report))
}

/// Archive each group through the owner's delete path, recording each one
/// archived into `archived` so a failure part-way still names them. A group
/// archived since it was listed is skipped.
async fn archive_each(
    groups: &GroupService,
    user_id: Uuid,
    owned: Vec<OwnedGroup>,
    archived: &mut Vec<ArchivedGroup>,
) -> AppResult<()> {
    for group in owned {
        if groups
            .delete_group(&group.group_id, group.tenant_id, user_id)
            .await?
        {
            archived.push(ArchivedGroup {
                group_id: group.group_id,
                name: group.name,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use pierre_core::models::{TenantId, UserReferenceKind};

    use super::*;
    use crate::provider_revocation::RevocationOutcome;

    fn shared_agent() -> Vec<UserReference> {
        vec![UserReference {
            kind: UserReferenceKind::AuthoredAgent,
            detail: "Hill coach".to_owned(),
        }]
    }

    #[test]
    fn a_refusal_before_anything_ran_names_only_the_blockers() {
        let message = blockers_message(&AccountResetRefusal {
            blockers: shared_agent(),
            disconnected: Vec::new(),
            archive_erased: None,
            groups_archived: Vec::new(),
        });
        assert_eq!(
            message,
            "User cannot be reset until these are reassigned or removed: authored agent 'Hill coach', which other users rely on",
            "bug: a refusal before any step ran names more than its blockers"
        );
    }

    /// The second read refuses after the disconnects and the erasure ran: the
    /// operator is told both stand, so the retry is not a surprise.
    #[test]
    fn a_refusal_inside_the_wipe_says_what_already_ran() {
        let tenant_id = TenantId::generate();
        let message = blockers_message(&AccountResetRefusal {
            blockers: shared_agent(),
            disconnected: vec![DisconnectedProvider {
                tenant_id,
                provider: "strava".to_owned(),
                revocation: RevocationOutcome::Revoked,
            }],
            archive_erased: Some(ArchiveErasure::default()),
            groups_archived: vec![ArchivedGroup {
                group_id: "g1".to_owned(),
                name: "Hill repeats".to_owned(),
            }],
        });
        assert!(
            message.contains("already archived the groups 'Hill repeats'"),
            "{message}"
        );
        assert!(
            message.contains(&format!(
                "already disconnected: strava (tenant {tenant_id}) revoked at the provider"
            )),
            "{message}"
        );
        assert!(
            message.contains("the imported history and uploaded files were already erased"),
            "{message}"
        );
        assert!(
            message.ends_with("No other row the account owns was changed."),
            "{message}"
        );
    }
}
