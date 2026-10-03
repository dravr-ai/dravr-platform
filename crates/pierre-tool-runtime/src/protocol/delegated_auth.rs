// ABOUTME: Whose session a TrainingPeaks read goes through, decided before the provider is built
// ABOUTME: A coach account's own read is refused, a linked member reads through the coach's session, refusals end or flag the link
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! # `TrainingPeaks` read subject
//!
//! A `TrainingPeaks` coach account keeps no calendar of its own, so a read of
//! the account's own workouts has nothing to return. Once the connection is
//! known to be a coach account, [`AuthService::trainingpeaks_subject`] refuses
//! that read before any scrape, with the same words the scraper's refusal is
//! given ([`SciotteTarget::coach_account_refusal`]). Until then the scraper
//! answers the read with its `athlete_required` refusal, and
//! [`AuthService::react_to_trainingpeaks_refusal`] records the role, so the
//! next read refuses at the first step.
//!
//! A group member whose coach linked them, and who confirmed the link, reads
//! their `TrainingPeaks` workouts through the coach's own stored session,
//! naming the member's athlete id on every read. The member's own login wins
//! over a link. The link is found through its group, so a member who left, an
//! archived group or a replaced coach reads nothing even if the eager end of
//! the link was missed. It reads only while the roster athlete it names is
//! the member by email ([`link_binding`]): a link that stored no athlete email,
//! as one confirmed before links stored it, or one whose email is not the
//! member's verified email, is refused with the reason until the coach
//! proposes it again and the member confirms. The link is left standing, so
//! the coach and the member see why on the link itself.
//!
//! None of these refusals is the member's authentication failure: signing in
//! again leaves a coach account a coach account, and a link's reader cannot
//! renew the coach's session. So none carries the reconnect tag that would
//! send the reader to a `TrainingPeaks` login. A coach session the scraper
//! reports dead flags the coach's connection and tells the coach instead.

use std::str::FromStr;

use chrono::{DateTime, Utc};
use dravr_sciotte::models::AthleteId;
use dravr_sciotte::wire::{ATHLETE_NOT_ACCESSIBLE, ATHLETE_REQUIRED};
use pierre_core::constants::oauth_providers::SCIOTTE_TRAININGPEAKS;
use pierre_core::errors::AppError;
use pierre_core::models::{
    ConnectionType, DelegatedConnection, DelegationEndReason, ProviderAccountRole,
    ProviderConnection, TenantId,
};
use pierre_core::untrusted::display_line;
use pierre_groups::delegation::{DelegationStore, UnbackedLink};
use pierre_providers::sciotte_error::sciotte_refusal;
use pierre_providers::sciotte_provider::{is_delegated_session_expired, SciotteTarget};
use pierre_providers::{CoreFitnessProvider, CredentialKind, OAuth2Credentials};
use pierre_services::delegated_connections::{
    coach_session_state, end_off_roster, person_name, unbound_link_reason, CoachSession,
};
use pierre_services::trainingpeaks_accounts::{
    link_binding, record_trainingpeaks_role, EmailBinding,
};
use tracing::{info, warn};
use uuid::Uuid;

use crate::protocol::auth::AuthService;
use crate::protocol::reauth_notice::flag_needs_reauth;
use crate::protocol::types::UniversalResponse;

/// Longest coach name a refusal carries. A coach's name is their own Dravr
/// display name, typed by them, so it reaches the model as one defanged line.
const COACH_NAME_MAX_CHARS: usize = 60;

/// The coach a refusal names when their name cannot be read.
const UNNAMED_COACH: &str = "the coach";

/// The error class recorded on a coach's connection when a member's read
/// found the coach's session dead. Token-free, as `mark_needs_reauth` needs.
const DELEGATED_SESSION_EXPIRED: &str = "delegated_session_expired";

/// What a linked member's read answers once the link it went through ended.
const LINK_ENDED: &str = "The TrainingPeaks link through the athlete's coach has ended. The \
                          athlete can ask the coach to link again, or connect TrainingPeaks \
                          themselves.";

/// What a linked member's read answers when the link cannot be read just now.
const LINK_UNREADABLE: &str = "The athlete's TrainingPeaks link through their coach could not be \
                               read just now because of a temporary failure on our side; a \
                               later request retries it.";

/// A refusal in words, with no reconnect tag.
const fn refusal(text: String) -> UniversalResponse {
    UniversalResponse {
        success: false,
        result: None,
        error: Some(text),
        metadata: None,
    }
}

/// What a linked member's read answers while the roster athlete the link
/// names is not the member by email, worded for `binding`; `None` for a link
/// that binds.
fn unbound_link_refusal(binding: EmailBinding) -> Option<UniversalResponse> {
    let why = match binding {
        EmailBinding::Bound => return None,
        EmailBinding::ProviderEmailMissing => {
            "the link lists no TrainingPeaks email for the roster athlete it names, so nothing \
             shows that athlete is this one"
        }
        EmailBinding::Mismatch => {
            "the roster athlete the link names has a TrainingPeaks email that is not this \
             athlete's verified Dravr email"
        }
        EmailBinding::DravrEmailUnverified => {
            "this athlete's Dravr email is not verified, so it cannot show the roster athlete \
             the link names is them"
        }
    };
    Some(refusal(format!(
        "This athlete's TrainingPeaks workouts are not read through their coach's account: \
         {why}. The coach links the athlete again from the group, and the athlete confirms."
    )))
}

/// What a read of a `TrainingPeaks` coach account's own calendar answers: the
/// refusal the scraper's own is worded as, with no reconnect tag.
fn coach_account_refusal() -> UniversalResponse {
    refusal(SciotteTarget::TrainingPeaks.coach_account_refusal())
}

/// What a linked member's read answers while the coach's session needs the
/// coach to sign in again.
fn coach_reconnect_refusal(coach: &str) -> UniversalResponse {
    refusal(format!(
        "This athlete's TrainingPeaks workouts are read through {coach}'s TrainingPeaks \
         connection, which {coach} needs to reconnect. Nothing is wrong with the athlete's own \
         account."
    ))
}

impl AuthService {
    /// Decide how a read of `provider_name` for `user_id` in `tenant_id` is
    /// served, when the answer is not the user's own stored session.
    ///
    /// Returns `Some` with the outcome when this step answers the read, and
    /// `None` when the ordinary path — the user's own token — serves it. Only
    /// the `TrainingPeaks` mirror is decided here:
    ///
    /// 1. a read of the user's own connection whose account is a coach's is
    ///    refused before any scrape;
    /// 2. the user's own stored session wins over any link;
    /// 3. a confirmed link, found through its group, is read through the
    ///    coach's session ([`Self::serve_through_coach`]) while its athlete is
    ///    the member by email; a delegated connection whose link no longer
    ///    reads is released and refused;
    /// 4. with neither, the ordinary path answers its missing-token refusal.
    ///
    /// A failed read of the user's connections or token falls through as
    /// well: the ordinary path reads them again and answers its own failure.
    pub(crate) async fn trainingpeaks_subject(
        &self,
        provider_name: &str,
        user_id: Uuid,
        tenant_id: Option<TenantId>,
    ) -> Option<Result<Box<dyn CoreFitnessProvider>, Box<UniversalResponse>>> {
        if provider_name != SCIOTTE_TRAININGPEAKS {
            return None;
        }
        let tenant = tenant_id?;
        let connections = self.trainingpeaks_connections(user_id, tenant).await?;
        let own_coach_account = connections.iter().any(|connection| {
            connection.connection_type != ConnectionType::Delegated
                && connection.account_role == Some(ProviderAccountRole::Coach)
        });
        if own_coach_account {
            return Some(Err(Box::new(coach_account_refusal())));
        }
        if !matches!(
            self.runtime()
                .repos()
                .oauth_tokens
                .get_token(user_id, tenant, SCIOTTE_TRAININGPEAKS)
                .await,
            Ok(None)
        ) {
            return None;
        }
        let delegated_row = connections
            .iter()
            .any(|connection| connection.connection_type == ConnectionType::Delegated);
        self.linked_read(user_id, tenant, delegated_row).await
    }

    /// The user's `TrainingPeaks` connections in `tenant` — their own and a
    /// delegated one — or `None` (logged) when they cannot be read.
    async fn trainingpeaks_connections(
        &self,
        user_id: Uuid,
        tenant: TenantId,
    ) -> Option<Vec<ProviderConnection>> {
        match self
            .runtime()
            .repos()
            .provider_connections
            .get_for_user(user_id, Some(tenant))
            .await
        {
            Ok(connections) => Some(
                connections
                    .into_iter()
                    .filter(|connection| connection.provider == SCIOTTE_TRAININGPEAKS)
                    .collect(),
            ),
            Err(e) => {
                warn!(
                    user_id = %user_id,
                    error = %e,
                    "Could not read the TrainingPeaks connection; the ordinary path decides"
                );
                None
            }
        }
    }

    /// Serve a read by `member_user_id`, who holds no session of their own,
    /// through their confirmed link while its athlete is the member by email,
    /// or `None` when they have no link and no `delegated_row` says they had
    /// one.
    async fn linked_read(
        &self,
        member_user_id: Uuid,
        tenant: TenantId,
        delegated_row: bool,
    ) -> Option<Result<Box<dyn CoreFitnessProvider>, Box<UniversalResponse>>> {
        match self
            .runtime()
            .repos()
            .delegated_connections
            .find_active_for_member(member_user_id, tenant, SCIOTTE_TRAININGPEAKS)
            .await
        {
            Ok(Some(link)) => Some(match self.refuse_unbound_link(&link).await {
                Ok(()) => self.serve_through_coach(&link).await,
                Err(refused) => Err(refused),
            }),
            Ok(None) if delegated_row => {
                self.release_unreachable_link(member_user_id, tenant).await;
                Some(Err(Box::new(refusal(LINK_ENDED.to_owned()))))
            }
            Ok(None) => None,
            Err(e) => {
                warn!(
                    user_id = %member_user_id,
                    error = %e,
                    "Could not read the member's TrainingPeaks link"
                );
                Some(Err(Box::new(refusal(LINK_UNREADABLE.to_owned()))))
            }
        }
    }

    /// Build the provider that reads `link`'s member through the coach's own
    /// stored session, naming the member's athlete id on every read.
    ///
    /// A coach session that needs the coach to sign in again is refused in
    /// words naming the coach. A coach who holds no session any more ended
    /// the link's only way to read, so the link ends
    /// ([`DelegationEndReason::CoachDisconnected`]) and the read is refused.
    async fn serve_through_coach(
        &self,
        link: &DelegatedConnection,
    ) -> Result<Box<dyn CoreFitnessProvider>, Box<UniversalResponse>> {
        let repos = self.runtime().repos();
        let token = match coach_session_state(
            repos,
            link.coach_user_id,
            link.coach_tenant_id,
            &link.provider,
        )
        .await
        {
            Ok(CoachSession::Live { token, .. }) => token,
            Ok(CoachSession::NeedsReconnect) => {
                return Err(Box::new(coach_reconnect_refusal(
                    &self.coach_name(link).await,
                )));
            }
            Ok(CoachSession::Missing) => {
                if let Err(e) = DelegationStore::new(repos)
                    .end_one(
                        link.id,
                        link.member_user_id,
                        None,
                        DelegationEndReason::CoachDisconnected,
                    )
                    .await
                {
                    warn!(
                        link_id = %link.id,
                        error = %e,
                        "Could not end a TrainingPeaks link whose coach holds no session"
                    );
                }
                return Err(Box::new(refusal(LINK_ENDED.to_owned())));
            }
            Err(e) => {
                warn!(
                    link_id = %link.id,
                    error = %e,
                    "Could not read the coach's TrainingPeaks session"
                );
                return Err(Box::new(refusal(LINK_UNREADABLE.to_owned())));
            }
        };

        let provider = AthleteId::from_str(&link.provider_athlete_id)
            .map_err(|e| AppError::invalid_input(e.to_string()))
            .and_then(|athlete| {
                self.runtime()
                    .provider_registry()
                    .create_delegated_provider(&link.provider, athlete)
            })
            .map_err(|e| Box::new(refusal(format!("Failed to create provider: {e}"))))?;
        provider
            .set_credentials(OAuth2Credentials {
                client_id: String::new(),
                client_secret: String::new(),
                access_token: Some(token.access_token),
                refresh_token: None,
                expires_at: token.expires_at,
                scopes: vec![],
                kind: CredentialKind::OAuthBearer,
                request_budget: None,
            })
            .await
            .map_err(|e| Box::new(refusal(format!("Failed to set provider credentials: {e}"))))?;
        info!(
            link_id = %link.id,
            user_id = %link.member_user_id,
            "TrainingPeaks read served through the coach's session"
        );
        Ok(provider)
    }

    /// Refuse a read through `link` while the roster athlete it names is not
    /// the member by email ([`link_binding`]), in words saying why. The link
    /// is left standing: it reads again once the coach proposes it anew and
    /// the member confirms.
    async fn refuse_unbound_link(
        &self,
        link: &DelegatedConnection,
    ) -> Result<(), Box<UniversalResponse>> {
        let binding = link_binding(self.runtime().repos(), link)
            .await
            .map_err(|e| {
                warn!(
                    link_id = %link.id,
                    error = %e,
                    "Could not check the TrainingPeaks link's athlete against the member"
                );
                Box::new(refusal(LINK_UNREADABLE.to_owned()))
            })?;
        unbound_link_refusal(binding).map_or(Ok(()), |refused| {
            info!(
                link_id = %link.id,
                user_id = %link.member_user_id,
                reason = unbound_link_reason(binding).unwrap_or_default(),
                "TrainingPeaks link refused: its athlete is not the member by email"
            );
            Err(Box::new(refused))
        })
    }

    /// Release a member's delegated connection whose link no longer reads.
    ///
    /// Either the link is still confirmed but its group, the member's place
    /// in it or its coach no longer backs it — it ends now, for whichever of
    /// those broke — or it ended earlier and its release did not finish, and
    /// the connection row is removed on its own. A link the relation backs
    /// again by now is left as it is. Best-effort: the read is refused either
    /// way.
    async fn release_unreachable_link(&self, member_user_id: Uuid, tenant: TenantId) {
        let repos = self.runtime().repos();
        let released = match DelegationStore::new(repos)
            .end_unbacked_for_member(member_user_id, tenant, SCIOTTE_TRAININGPEAKS)
            .await
        {
            Ok(UnbackedLink::Ended(link)) => {
                info!(
                    user_id = %member_user_id,
                    link_id = %link.id,
                    reason = link.revoke_reason.map_or("", |reason| reason.as_str()),
                    "Ended a delegated TrainingPeaks link its group no longer backs"
                );
                Ok(())
            }
            Ok(UnbackedLink::Backed) => Ok(()),
            Ok(UnbackedLink::Gone) => repos
                .provider_connections
                .remove_delegated_connection(member_user_id, tenant, SCIOTTE_TRAININGPEAKS)
                .await
                .map(|_| {
                    info!(
                        user_id = %member_user_id,
                        "Released a delegated TrainingPeaks connection whose link had ended"
                    );
                }),
            Err(e) => Err(e),
        };
        if let Err(e) = released {
            warn!(
                user_id = %member_user_id,
                error = %e,
                "Could not release a delegated TrainingPeaks connection whose link no longer reads"
            );
        }
    }

    /// The coach's name as a refusal carries it.
    async fn coach_name(&self, link: &DelegatedConnection) -> String {
        match self
            .runtime()
            .repos()
            .users
            .get_global(link.coach_user_id)
            .await
        {
            Ok(Some(coach)) => display_line(&person_name(&coach), COACH_NAME_MAX_CHARS),
            Ok(None) | Err(_) => UNNAMED_COACH.to_owned(),
        }
    }

    /// React to a failed `TrainingPeaks` read by `user_id` in `tenant_id`,
    /// whose credential was read from `attempt_started_at` on.
    ///
    /// - The coach's session behind a linked member's read is dead: the
    ///   coach's own connection is flagged and, the first time, the coach is
    ///   told to reconnect. The member's connection stays as it is — the
    ///   member cannot renew the coach's session.
    /// - The scraper refused the athlete a linked read named
    ///   (`athlete_not_accessible`): `TrainingPeaks` no longer lists the
    ///   athlete on the coach's roster, so the link ends and both sides are
    ///   told.
    /// - The scraper refused a read that named no athlete because the session
    ///   is a coach account's (`athlete_required`): the connection is recorded
    ///   as a coach account — which removes the workouts misfiled under it
    ///   (see [`record_trainingpeaks_role`]) — so the next read is refused
    ///   before any scrape. The refusal carries no email, so it grants no
    ///   `manages_roster`: that waits for a profile read binding the account
    ///   to the user.
    ///
    /// Best-effort: the read has already failed with the scraper's own words,
    /// and a write that fails here is logged.
    pub(crate) async fn react_to_trainingpeaks_refusal(
        &self,
        user_id: Uuid,
        tenant_id: &str,
        provider: &dyn CoreFitnessProvider,
        error: &AppError,
        attempt_started_at: DateTime<Utc>,
    ) {
        if provider.name() != SCIOTTE_TRAININGPEAKS {
            return;
        }
        let Ok(tenant) = TenantId::parse_str(tenant_id) else {
            return;
        };
        if is_delegated_session_expired(error) {
            self.flag_coach_session(user_id, tenant, attempt_started_at)
                .await;
            return;
        }
        match sciotte_refusal(error) {
            Some(ATHLETE_NOT_ACCESSIBLE) => self.end_off_roster_link(user_id, tenant).await,
            Some(ATHLETE_REQUIRED) => self.record_coach_account(user_id, tenant).await,
            _ => {}
        }
    }

    /// Flag the coach's connection behind `member_user_id`'s link after the
    /// scraper found the coach's session dead, and tell the coach once.
    async fn flag_coach_session(
        &self,
        member_user_id: Uuid,
        tenant: TenantId,
        attempt_started_at: DateTime<Utc>,
    ) {
        let link = match self
            .runtime()
            .repos()
            .delegated_connections
            .find_active_for_member(member_user_id, tenant, SCIOTTE_TRAININGPEAKS)
            .await
        {
            Ok(Some(link)) => link,
            Ok(None) => return,
            Err(e) => {
                warn!(
                    user_id = %member_user_id,
                    error = %e,
                    "Could not read the link behind a dead coach session"
                );
                return;
            }
        };
        // The coach is the one who must sign in again, so the flag and its
        // notice are the coach's, under the coach's tenant.
        match flag_needs_reauth(
            self.runtime(),
            link.coach_user_id,
            link.coach_tenant_id,
            SCIOTTE_TRAININGPEAKS,
            DELEGATED_SESSION_EXPIRED,
            attempt_started_at,
        )
        .await
        {
            Ok((mark, notice)) => info!(
                link_id = %link.id,
                coach_user_id = %link.coach_user_id,
                ?mark,
                notified = notice.claimed,
                "Coach's TrainingPeaks session found dead on a linked read"
            ),
            Err(e) => warn!(
                link_id = %link.id,
                error = %e,
                "Could not flag the coach's dead TrainingPeaks session"
            ),
        }
    }

    /// End `member_user_id`'s link because the coach's roster no longer lists
    /// the athlete, telling both sides.
    async fn end_off_roster_link(&self, member_user_id: Uuid, tenant: TenantId) {
        #[cfg(feature = "client-notifications")]
        let notifications = self.runtime().notification_service();
        #[cfg(not(feature = "client-notifications"))]
        let notifications = None;
        match end_off_roster(
            self.runtime().repos(),
            notifications,
            member_user_id,
            tenant,
            SCIOTTE_TRAININGPEAKS,
        )
        .await
        {
            Ok(ended) => info!(
                user_id = %member_user_id,
                ended = ended.len(),
                "TrainingPeaks no longer lists the linked athlete on the coach's roster; link ended"
            ),
            Err(e) => warn!(
                user_id = %member_user_id,
                error = %e,
                "Could not end a TrainingPeaks link whose athlete left the coach's roster"
            ),
        }
    }

    /// Record `user_id`'s own `TrainingPeaks` connection as a coach account.
    async fn record_coach_account(&self, user_id: Uuid, tenant: TenantId) {
        match record_trainingpeaks_role(
            self.runtime().repos(),
            user_id,
            tenant,
            ProviderAccountRole::Coach,
        )
        .await
        {
            Ok(()) => info!(
                user_id = %user_id,
                "TrainingPeaks read refused as a coach account's; role recorded"
            ),
            Err(e) => warn!(
                user_id = %user_id,
                error = %e,
                "Could not record the TrainingPeaks coach account the scraper reported"
            ),
        }
    }
}
