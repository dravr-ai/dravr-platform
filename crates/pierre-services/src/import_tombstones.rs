// ABOUTME: The doors a deleted account's identities are refused at, and the explicit clear that reopens them
// ABOUTME: Provider connects and webhooks refuse a tombstoned provider account; admission holds a tombstoned email

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Import tombstones, as the doors read them.
//!
//! An account deletion leaves a live tombstone for each provider-side id its
//! tokens held and for its email (`pierre_database::repositories::import_tombstones`).
//! Afterwards, provider data for that provider account is refused and logged
//! wherever it would enter: a provider's push event is dropped
//! (`webhook_owner`), and an OAuth connect or an intervals.icu link is
//! refused with [`refusal`], which names the tombstone and when the data was
//! deleted. A new sign-up with the email is never admitted straight to
//! Active ([`email_held`]); an operator, or the person clearing the
//! tombstone, decides.
//!
//! Nothing reopens on its own: the person who now holds the identity clears
//! the tombstone explicitly ([`clear_for_account`]), or an operator does
//! ([`clear_by_operator`]). A provider-account tombstone is cleared by the
//! account that proved it holds that provider account: being refused after
//! completing the provider's authorization records that proof on the
//! tombstone ([`refuse_provider_account`]), and it stands for
//! [`CLAIM_VALIDITY_HOURS`].

use chrono::{DateTime, Duration, Utc};
use pierre_auth::dto::auth::EmailTombstoneInfo;
use pierre_core::errors::{AppError, AppResult, ErrorCode};
use pierre_core::models::{User, UserStatus};
use pierre_database::repositories::{
    athlete_marker, email_identity_hash, provider_identity_hash, ImportTombstone,
    ImportTombstoneRepository, TombstoneKind,
};
use pierre_database::RepositoryRegistry;
use pierre_providers::backend_resolver;
use serde_json::json;
use tracing::{info, warn};

/// The machine-readable code every tombstone refusal carries, in the error
/// details' `error` field and as the OAuth return URL's `error` parameter:
/// the clients render their text from it.
pub const TOMBSTONED_ERROR_CODE: &str = "import_tombstoned";

/// How long a refused account's proof that it holds a tombstoned provider
/// account lets it clear that tombstone.
pub const CLAIM_VALIDITY_HOURS: i64 = 24;

/// The provider name a tombstone is keyed under: the user-facing name, so
/// either half of a coalesced pair finds it.
fn tombstone_provider(provider: &str) -> &str {
    backend_resolver::user_facing_name(provider)
}

/// The refusal for an identity a live tombstone names: 409, with
/// `details` carrying [`TOMBSTONED_ERROR_CODE`], the tombstone's id and kind,
/// its provider and when the data was deleted, for a client to render.
#[must_use]
pub fn refusal(tombstone: &ImportTombstone) -> AppError {
    let deleted_on = tombstone.created_at.format("%Y-%m-%d");
    let what = match tombstone.kind {
        TombstoneKind::ProviderAccount => "The data of this provider account",
        TombstoneKind::Email => "The account registered with this email",
    };
    let mut error = AppError::new(
        ErrorCode::ResourceLocked,
        format!(
            "{what} was deleted on {deleted_on} (tombstone {}); it must be explicitly cleared before it is accepted again",
            tombstone.id
        ),
    );
    error.details = Some(Box::new(json!({
        "error": TOMBSTONED_ERROR_CODE,
        "tombstone_id": tombstone.id,
        "kind": tombstone.kind,
        "provider": tombstone.provider,
        "deleted_at": tombstone.created_at.to_rfc3339(),
    })));
    error
}

/// The `field` of a [`refusal`]'s details, or `None` for any other error.
fn refusal_detail<'e>(error: &'e AppError, field: &str) -> Option<&'e str> {
    let details = error.details.as_deref()?;
    if details.get("error")?.as_str()? != TOMBSTONED_ERROR_CODE {
        return None;
    }
    details.get(field)?.as_str()
}

/// The tombstone id a [`refusal`] names, or `None` for any other error.
#[must_use]
pub fn refused_tombstone_id(error: &AppError) -> Option<&str> {
    refusal_detail(error, "tombstone_id")
}

/// When the data a [`refusal`] names was deleted (RFC 3339), or `None` for
/// any other error: the date a client words the refusal with.
#[must_use]
pub(crate) fn refused_tombstone_deleted_at(error: &AppError) -> Option<&str> {
    refusal_detail(error, "deleted_at")
}

/// The live tombstone naming `provider_user_id` at `provider`, if any.
///
/// # Errors
/// Returns a database error when the read fails.
pub async fn provider_account_tombstone(
    tombstones: &dyn ImportTombstoneRepository,
    provider: &str,
    provider_user_id: &str,
) -> AppResult<Option<ImportTombstone>> {
    tombstones
        .find_live(
            TombstoneKind::ProviderAccount,
            Some(tombstone_provider(provider)),
            &provider_identity_hash(provider_user_id),
        )
        .await
}

/// Refuse `user_id`'s connect of a tombstoned provider account.
///
/// The caller completed the provider's authorization for it, so the refusal
/// records that proof on the tombstone, which is what lets this account
/// clear it ([`clear_for_account`]). Logged at INFO with the provider and the
/// tombstone id, never the provider-side id. Always returns the [`refusal`];
/// a claim that cannot be recorded is logged, and the refusal still stands.
pub async fn refuse_provider_account(
    tombstones: &dyn ImportTombstoneRepository,
    user_id: uuid::Uuid,
    provider: &str,
    tombstone: &ImportTombstone,
) -> AppError {
    info!(
        user_id = %user_id,
        provider = %provider,
        tombstone_id = %tombstone.id,
        "Refused a provider connect: the provider account belonged to a deleted account"
    );
    if let Err(e) = tombstones
        .record_claim(&tombstone.id, &athlete_marker(user_id), Utc::now())
        .await
    {
        warn!(
            user_id = %user_id,
            tombstone_id = %tombstone.id,
            error = %e,
            "Could not record the refused account's claim on the tombstone; it must reconnect to claim it again"
        );
    }
    refusal(tombstone)
}

/// Whether `email` is held out of automatic admission: a live tombstone
/// names it. A read that fails holds it too, so a database fault never
/// admits a deleted account's address.
pub async fn email_held(repos: &RepositoryRegistry, email: &str) -> bool {
    match repos
        .import_tombstones
        .find_live(TombstoneKind::Email, None, &email_identity_hash(email))
        .await
    {
        Ok(Some(tombstone)) => {
            info!(
                tombstone_id = %tombstone.id,
                "Sign-up admission held: the email belonged to a deleted account"
            );
            true
        }
        Ok(None) => false,
        Err(e) => {
            warn!(error = %e, "Email tombstone lookup failed; admission is held");
            true
        }
    }
}

/// The live tombstone holding a Pending `user` out of admission: the one
/// naming its email, as the pending screen reads it.
///
/// `None` for an account that is not Pending, and when no live tombstone
/// names the address. A failed read is logged and answers `None` too: the
/// screen then words the ordinary review wait, which stays true, and the
/// account is still held ([`email_held`] fails closed).
pub(crate) async fn pending_email_tombstone(
    tombstones: &dyn ImportTombstoneRepository,
    user: &User,
) -> Option<EmailTombstoneInfo> {
    if user.user_status != UserStatus::Pending {
        return None;
    }
    match tombstones
        .find_live(
            TombstoneKind::Email,
            None,
            &email_identity_hash(&user.email),
        )
        .await
    {
        Ok(tombstone) => tombstone.map(|t| EmailTombstoneInfo {
            tombstone_id: t.id,
            deleted_at: t.created_at.to_rfc3339(),
        }),
        Err(e) => {
            warn!(user_id = %user.id, error = %e, "Email tombstone lookup for the pending screen failed");
            None
        }
    }
}

/// Whether `user` holds the identity `tombstone` names at `now`: its own
/// email, or a provider account it proved it holds within
/// [`CLAIM_VALIDITY_HOURS`].
fn holds_identity(user: &User, tombstone: &ImportTombstone, now: DateTime<Utc>) -> bool {
    match tombstone.kind {
        TombstoneKind::Email => tombstone.identity_hash == email_identity_hash(&user.email),
        TombstoneKind::ProviderAccount => {
            tombstone.claimed_by.as_deref() == Some(athlete_marker(user.id).as_str())
                && tombstone
                    .claimed_at
                    .is_some_and(|at| now - at <= Duration::hours(CLAIM_VALIDITY_HOURS))
        }
    }
}

/// Clear tombstones as the account `user`.
///
/// With `tombstone_id`, that tombstone, which the account must hold the
/// identity of; without, the live tombstone naming the account's own email,
/// if there is one. Idempotent: a tombstone already cleared is returned as
/// its first clearing left it. Returns the tombstones as they now stand.
///
/// # Errors
/// Returns a not-found error for an unknown id or one whose identity the
/// account does not hold (the two read alike, so the route never confirms a
/// tombstone exists to someone it does not concern), and a database error
/// when a read or the clear fails.
pub async fn clear_for_account(
    tombstones: &dyn ImportTombstoneRepository,
    user: &User,
    tombstone_id: Option<&str>,
) -> AppResult<Vec<ImportTombstone>> {
    let now = Utc::now();
    let target = match tombstone_id {
        Some(id) => {
            let not_found = || AppError::not_found(format!("No tombstone {id} for this account"));
            let tombstone = tombstones.get(id).await?.ok_or_else(not_found)?;
            if !holds_identity(user, &tombstone, now) {
                return Err(not_found());
            }
            tombstone
        }
        None => {
            match tombstones
                .find_live(
                    TombstoneKind::Email,
                    None,
                    &email_identity_hash(&user.email),
                )
                .await?
            {
                Some(tombstone) => tombstone,
                None => return Ok(Vec::new()),
            }
        }
    };
    let cleared = clear(tombstones, &target.id, &athlete_marker(user.id), now).await?;
    info!(
        user_id = %user.id,
        tombstone_id = %cleared.id,
        kind = ?cleared.kind,
        "Tombstone cleared by the account that holds its identity"
    );
    Ok(vec![cleared])
}

/// Clear the tombstone `tombstone_id` as the operator `operator` (an admin
/// token id). Idempotent.
///
/// # Errors
/// Returns a not-found error for an unknown id and a database error when the
/// clear fails.
pub async fn clear_by_operator(
    tombstones: &dyn ImportTombstoneRepository,
    tombstone_id: &str,
    operator: &str,
) -> AppResult<ImportTombstone> {
    let cleared = clear(
        tombstones,
        tombstone_id,
        &format!("operator:{operator}"),
        Utc::now(),
    )
    .await?;
    info!(
        tombstone_id = %cleared.id,
        operator = %operator,
        "Tombstone cleared by an operator"
    );
    Ok(cleared)
}

/// Clear `id` as `by` at `now`, or report it unknown.
async fn clear(
    tombstones: &dyn ImportTombstoneRepository,
    id: &str,
    by: &str,
    now: DateTime<Utc>,
) -> AppResult<ImportTombstone> {
    tombstones
        .clear(id, by, now)
        .await?
        .ok_or_else(|| AppError::not_found(format!("No tombstone {id}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use pierre_database::repositories::subject_hash;
    use uuid::Uuid;

    fn tombstone(kind: TombstoneKind, identity_hash: String) -> ImportTombstone {
        ImportTombstone {
            id: "t-1".to_owned(),
            subject_hash: subject_hash(Uuid::new_v4()),
            kind,
            provider: (kind == TombstoneKind::ProviderAccount).then(|| "strava".to_owned()),
            identity_hash,
            created_at: DateTime::parse_from_rfc3339("2026-10-20T08:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            cleared_at: None,
            cleared_by: None,
            claimed_by: None,
            claimed_at: None,
        }
    }

    fn user(email: &str) -> User {
        User::new(email.to_owned(), "hash".to_owned(), None)
    }

    #[test]
    fn a_refusal_names_the_tombstone_and_its_date_and_carries_the_code() {
        let refused = refusal(&tombstone(
            TombstoneKind::ProviderAccount,
            provider_identity_hash("42"),
        ));
        assert_eq!(refused.code, ErrorCode::ResourceLocked);
        assert!(
            refused.message.contains("tombstone t-1"),
            "{}",
            refused.message
        );
        assert!(
            refused.message.contains("2026-10-20"),
            "{}",
            refused.message
        );
        assert!(!refused.message.contains("42"), "{}", refused.message);
        assert_eq!(refused_tombstone_id(&refused), Some("t-1"));
        assert_eq!(refused_tombstone_id(&AppError::internal("other")), None);
        assert_eq!(
            refused_tombstone_deleted_at(&refused),
            Some("2026-10-20T08:00:00+00:00")
        );
        assert_eq!(
            refused_tombstone_deleted_at(&AppError::internal("other")),
            None
        );
    }

    #[test]
    fn an_email_tombstone_is_held_by_its_own_address_only() {
        let held = tombstone(TombstoneKind::Email, email_identity_hash("ana@example.com"));
        let now = Utc::now();
        assert!(holds_identity(&user("Ana@Example.com"), &held, now));
        assert!(!holds_identity(&user("bob@example.com"), &held, now));
    }

    #[test]
    fn a_provider_tombstone_is_held_by_a_fresh_claim_of_this_account_only() {
        let claimant = user("ana@example.com");
        let other = user("bob@example.com");
        let now = Utc::now();
        let mut claimed = tombstone(TombstoneKind::ProviderAccount, provider_identity_hash("42"));
        assert!(!holds_identity(&claimant, &claimed, now));

        claimed.claimed_by = Some(athlete_marker(claimant.id));
        claimed.claimed_at = Some(now - Duration::hours(CLAIM_VALIDITY_HOURS));
        assert!(holds_identity(&claimant, &claimed, now));
        assert!(!holds_identity(&other, &claimed, now));

        claimed.claimed_at = Some(now - Duration::hours(CLAIM_VALIDITY_HOURS + 1));
        assert!(!holds_identity(&claimant, &claimed, now));
    }
}
