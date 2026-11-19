// ABOUTME: The server's one SessionLandedSink — what happens when a live write lands a workout for an athlete
// ABOUTME: Runs the coach's landing post when one is installed, and the per-session digest unless that post replaced it

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Where a landing goes (carnet#1486).
//!
//! The activity store decides a landing inside its own ingest
//! ([`pierre_services::live_activities::landed_session`]); every live writer
//! — a chat turn's read, the capture sweep, a webhook, a Garmin push — hands
//! it here, through the runtime's
//! [`SessionLandedSink`](pierre_tool_runtime::runtime::SessionLandedSink) or
//! the push worker. This router is that sink, in every build:
//!
//! - with the coach's landing post installed
//!   ([`crate::services::activity_landed_post::install_activity_landed`]), the
//!   post runs first; a post that was stored replaces the athlete's
//!   per-session digest unless `activity_landed.digest` says `keep_digest`;
//! - otherwise — every test resource, a build without messaging, an athlete
//!   with no coach thread, a landing the post skipped or withheld — the
//!   per-session digest is sent as it always was, so the notifications the
//!   athlete's persona held back return.

use std::fmt;

use async_trait::async_trait;
use pierre_core::models::TenantId;
use pierre_services::live_activities::LandedSession;
use pierre_tool_runtime::runtime::SessionLandedSink;
use tracing::debug;
use uuid::Uuid;

#[cfg(any(feature = "client-notifications", feature = "client-messaging"))]
use std::sync::Arc;
#[cfg(feature = "client-messaging")]
use std::sync::OnceLock;

#[cfg(feature = "client-notifications")]
use pierre_contremaitre::messaging_strings::MessagingStringsRegistry;
#[cfg(feature = "client-notifications")]
use pierre_database::RepositoryRegistry;
#[cfg(feature = "client-notifications")]
use pierre_notifications::NotificationService;
#[cfg(feature = "client-notifications")]
use pierre_services::notification_digest_scheduler;
#[cfg(feature = "client-notifications")]
use tracing::warn;

#[cfg(feature = "client-messaging")]
use crate::services::activity_landed_post::ActivityLandedPoster;
#[cfg(feature = "client-messaging")]
use crate::services::post_delivery::PostAthlete;

/// The per-session digest's handles: what the digest reads and sends with.
#[cfg(feature = "client-notifications")]
pub struct SessionDigest {
    /// The athlete's notifications and persona policy; the context's own
    /// registry, shared with every other reader.
    pub repos: Arc<RepositoryRegistry>,
    /// Sends the digest; `None` when no notification backend is wired. The
    /// context's service, shared with every other sender.
    pub notifications: Option<Arc<NotificationService>>,
    /// The digest's words: the context's hot-reloaded registry, shared so a
    /// contremaitre sync reaches the digest too.
    pub strings: Arc<MessagingStringsRegistry>,
}

#[cfg(feature = "client-notifications")]
impl fmt::Debug for SessionDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionDigest")
            .field("notifications", &self.notifications.is_some())
            .finish_non_exhaustive()
    }
}

/// The server's one [`SessionLandedSink`]. It always exists; the coach's
/// landing post is layered on at install.
#[derive(Default)]
pub struct SessionLandedRouter {
    /// The per-session digest a landing sends when no post replaced it.
    #[cfg(feature = "client-notifications")]
    digest: Option<SessionDigest>,
    /// The coach's landing post, installed once the composition root exists;
    /// shared with every landing task the router runs it on.
    #[cfg(feature = "client-messaging")]
    poster: OnceLock<Arc<ActivityLandedPoster>>,
}

impl fmt::Debug for SessionLandedRouter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SessionLandedRouter")
            .finish_non_exhaustive()
    }
}

impl SessionLandedRouter {
    /// A router that sends the per-session digest through `digest`.
    #[cfg(feature = "client-notifications")]
    #[must_use]
    pub fn with_digest(digest: SessionDigest) -> Self {
        Self {
            digest: Some(digest),
            ..Self::default()
        }
    }

    /// Layer the coach's landing post on. A second install is a startup
    /// logic bug; the first poster stays and `false` says so.
    #[cfg(feature = "client-messaging")]
    pub fn install(&self, poster: Arc<ActivityLandedPoster>) -> bool {
        self.poster.set(poster).is_ok()
    }

    /// Send the athlete's per-session digest, best-effort.
    #[cfg(feature = "client-notifications")]
    async fn send_digest(&self, user_id: Uuid, tenant_id: TenantId) {
        let Some(digest) = &self.digest else {
            return;
        };
        let Some(service) = &digest.notifications else {
            return;
        };
        if let Err(e) = notification_digest_scheduler::session_landed(
            &digest.repos,
            service,
            &digest.strings,
            user_id,
            tenant_id,
        )
        .await
        {
            warn!(
                user_id = %user_id,
                error = %e,
                "per-session persona digest failed (best-effort)"
            );
        }
    }
}

#[async_trait]
impl SessionLandedSink for SessionLandedRouter {
    async fn session_landed(&self, user_id: Uuid, tenant_id: TenantId, session: LandedSession) {
        debug!(
            user_id = %user_id,
            tenant_id = %tenant_id,
            provider = %session.provider,
            first_in_store = session.first_in_store,
            "a live write landed a session"
        );
        #[cfg(feature = "client-messaging")]
        if let Some(poster) = self.poster.get() {
            let landed = poster
                .react(PostAthlete { user_id, tenant_id }, session)
                .await;
            if !landed.sends_digest() {
                return;
            }
        }
        // A build without notifications has no digest to send.
        #[cfg(feature = "client-notifications")]
        self.send_digest(user_id, tenant_id).await;
    }
}
