// ABOUTME: The one message a walk no conversation asked for posts when it ends: a history walk's year and count
// ABOUTME: Or a detail walk's count of activities whose segments it read; in the personal thread, each sent once

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use chrono::Datelike;
use pierre_contremaitre::messaging_strings::{KEY_DETAIL_WALK_COMPLETE, KEY_HISTORY_WALK_COMPLETE};
use pierre_core::models::TenantId;
use pierre_core::transport::TransportPolicy;
use pierre_database::backends::MessagingRepository;
use pierre_services::personal_thread::find_personal_thread;
use pierre_tool_runtime::runtime::{DetailWalkCompletion, HistoryWalkCompletion};
use tracing::{info, warn};

use super::ServerBackfillNotifier;
use crate::services::backfill_delivery::InAppDelivery;

impl ServerBackfillNotifier {
    /// Post the walk's end in the athlete's personal thread (B4): one short
    /// line, the oldest year their history now reaches and how many
    /// activities the walk brought in.
    ///
    /// The thread is the athlete's own, adopted when none is designated yet,
    /// never forged: an athlete with no conversation at all has never opened
    /// the app, and the history is there when they do. The send is claimed
    /// once per `(user, walked key, oldest activity)` across replicas, so a
    /// walk resumed on two instances posts once, and a walk that reached no
    /// further than the last one says nothing new.
    pub(super) async fn post_history_walk_complete(&self, completion: HistoryWalkCompletion<'_>) {
        let HistoryWalkCompletion {
            user_id,
            tenant_id,
            provider,
            oldest,
            activity_count,
        } = completion;
        let owner = user_id.to_string();
        let Some(thread_id) = self.walk_post_thread(&owner, tenant_id, provider).await else {
            return;
        };
        if !self
            .claim_walk_post(&owner, tenant_id, provider, oldest.timestamp())
            .await
        {
            return;
        }
        let locale = self.resolve_locale(user_id, tenant_id, None).await;
        let year = oldest.year().to_string();
        let count = activity_count.to_string();
        let body = self
            .strings
            .render(KEY_HISTORY_WALK_COMPLETE, &locale, &[&year, &count]);
        // The line counts the athlete's own history and names no session's
        // data, so nothing in it is stamped.
        InAppDelivery {
            repos: &self.repos,
            strings: &self.strings,
            #[cfg(feature = "client-notifications")]
            notifications: self.notifications.as_ref(),
            transport_policy: TransportPolicy::AnyTransport,
        }
        .deliver(
            user_id,
            tenant_id,
            &thread_id,
            &locale,
            body,
            activity_count,
        )
        .await;
        info!(
            provider,
            activity_count, "History walk post: delivered in the personal thread"
        );
    }

    /// Post a detail walk's end in the athlete's personal thread: one short
    /// line, how many of their activities had their segment efforts read.
    /// Counts only, never a segment's name: those were written by other
    /// athletes. No app push: the one a loaded history raises says "N
    /// activities loaded", and a detail walk loads none, it reads the
    /// efforts of activities already stored.
    ///
    /// The thread is found as a history walk's is
    /// ([`Self::post_history_walk_complete`]). The send is claimed once per
    /// walk, `(user, "<backend>:detail_walk", asked_at_ms)`, across replicas:
    /// a walk ended on two instances posts once, and a later walk, asked
    /// later, posts its own end. A catalogue that holds no line for the key
    /// posts nothing and claims nothing, rather than an empty message.
    pub(super) async fn post_detail_walk_complete(&self, completion: DetailWalkCompletion<'_>) {
        let DetailWalkCompletion {
            user_id,
            tenant_id,
            provider,
            asked_at_ms,
            activity_count,
        } = completion;
        let owner = user_id.to_string();
        let Some(thread_id) = self.walk_post_thread(&owner, tenant_id, provider).await else {
            return;
        };
        let locale = self.resolve_locale(user_id, tenant_id, None).await;
        let count = activity_count.to_string();
        let body = self
            .strings
            .render(KEY_DETAIL_WALK_COMPLETE, &locale, &[&count]);
        if body.trim().is_empty() {
            warn!(
                provider,
                locale, "Detail walk post: the catalogue holds no line for it; nothing posted"
            );
            return;
        }
        let walk = format!("{provider}:detail_walk");
        if !self
            .claim_walk_post(&owner, tenant_id, &walk, asked_at_ms)
            .await
        {
            return;
        }
        // Stored without the app push a loaded history raises: its "N
        // activities loaded" would misstate a walk that loaded none. The
        // turn's unread badge is the ping.
        let delivered = InAppDelivery {
            repos: &self.repos,
            strings: &self.strings,
            #[cfg(feature = "client-notifications")]
            notifications: self.notifications.as_ref(),
            transport_policy: TransportPolicy::AnyTransport,
        }
        .persist(user_id, tenant_id, &thread_id, body, activity_count)
        .await;
        if delivered {
            info!(
                provider,
                activity_count, "Detail walk post: delivered in the personal thread"
            );
        }
    }

    /// The athlete's personal thread a walk post lands in, or `None` (logged)
    /// when they hold none yet or it cannot be read.
    async fn walk_post_thread(
        &self,
        owner: &str,
        tenant_id: TenantId,
        provider: &str,
    ) -> Option<String> {
        match find_personal_thread(&self.repos, owner, tenant_id).await {
            Ok(Some(thread)) => Some(thread.id),
            Ok(None) => {
                info!(
                    provider,
                    "History walk post: the athlete has no personal thread yet; nothing posted"
                );
                None
            }
            Err(e) => {
                warn!(provider, error = %e, "History walk post: the personal thread could not be read");
                None
            }
        }
    }

    /// Claim the one send of a walk post for `(owner, walk, at)` across
    /// replicas: a history walk's key and oldest activity, or a detail walk's
    /// key and ask. `false` (logged) when it was sent already or the claim
    /// failed.
    async fn claim_walk_post(&self, owner: &str, tenant_id: TenantId, walk: &str, at: i64) -> bool {
        let db: &dyn MessagingRepository = self.repos.messaging.as_ref();
        match db.claim_backfill_push(tenant_id, owner, walk, at).await {
            Ok(true) => true,
            Ok(false) => {
                info!(walk, "Walk post: already sent for this walk");
                false
            }
            Err(e) => {
                warn!(walk, error = %e, "Walk post: the dedup claim failed");
                false
            }
        }
    }
}
