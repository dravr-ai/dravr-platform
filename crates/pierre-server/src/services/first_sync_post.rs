// ABOUTME: The coach's first post once an athlete's data lands — the production FirstSyncHook
// ABOUTME: Claims the post once per athlete, picks where it lands, runs it as a platform post, reports it
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The coach speaks first (carnet#831).
//!
//! Once a provider connect lands the athlete's data, the coach posts about it
//! without waiting to be asked: a verified pipeline turn ([`PlatformPost`])
//! answering the catalogue's first-sync instruction, which names the provider
//! the data came from.
//!
//! ## Once per athlete
//!
//! Every connect reports its landing — a second provider, a reconnect, a
//! backfill finishing on two replicas — so the post is claimed in the
//! `platform_post_claims` ledger before any model call, and only the trigger
//! that takes the claim posts. A trigger that ends up posting nothing releases
//! it, so the next landing tries again.
//!
//! ## Where it lands
//!
//! Both destinations [`crate::services::post_delivery`] decides before any
//! turn runs: the athlete's own in-app thread, and the messaging DM they last
//! wrote from when its channel may still be written to unprompted. Each is its
//! own turn, laid out for its surface and stored in its own conversation, so a
//! reply in either thread reads the post as history.
//!
//! ## Strava and Wahoo
//!
//! A provider the health sync does not backfill has landed nothing at connect.
//! Its activity list is read once ([`fetch_provider_head`], which writes the
//! read through to the cache the post's turn then answers from), and the post
//! follows only when that read lands activities.
//!
//! ## The history Home needs
//!
//! What a connect reads at once is recent ([`CONNECT_RECENT_DAYS`]). Home's
//! training status needs one chronic window plus its warm-up — 114 days at
//! the defaults — and never reaches a provider itself, so the recent read
//! alone would leave a new athlete's panel empty while the chat answers from
//! their last month (carnet#1043). Every connect therefore asks for that depth
//! ([`request_status_depth`]) once its recent data is in and the post is
//! done, on every provider alike: the capture rail pages each connection
//! back to the status's warm-up floor, deduplicated per provider and bounded
//! by the rail's own limits.

use std::sync::{Arc, Weak};

use async_trait::async_trait;
use chrono::{Duration, Utc};
use pierre_contremaitre::messaging_strings::KEY_PLATFORM_POST_FIRST_SYNC_PROMPT;
use pierre_core::models::TenantId;
use pierre_providers::backend_resolver;
use pierre_providers::core::ActivityQueryParams;
use pierre_providers::registry::global_registry;
use pierre_routes_auth::{FirstSyncHook, CONNECT_RECENT_DAYS};
use pierre_services::provider_refresh::token_tenant;
use pierre_tool_runtime::activity_fetch::fetch_provider_head;
use pierre_tool_runtime::runtime::ToolRuntime;
use tracing::{info, warn};
use uuid::Uuid;

use crate::mcp::resources::ServerContext;
use crate::services::adapter_resolver::{config_adapter_resolver, AdapterResolver};
use crate::services::platform_post::{PlatformPost, PostPurpose, PostSurface};
use crate::services::post_delivery::{linked_dms, post_in_app, post_to_dm, PostAthlete};
use crate::services::training_status::request_status_depth;

/// How many activities the first read of an on-demand provider asks for —
/// one provider page, enough to tell whether anything landed and to warm the
/// cache the post's turn answers from.
const FIRST_READ_LIMIT: usize = 50;

/// The production [`FirstSyncHook`]: has the coach speak first once an
/// athlete's data lands.
pub struct FirstSyncPoster {
    /// Weak handle to the composition root — repositories, strings and the
    /// tool runtime the first read of an on-demand provider runs on. Weak for
    /// the reason [`crate::services::platform_post::PipelinePlatformPost`]
    /// gives: the context owns the slot this poster is installed into.
    ctx: Weak<ServerContext>,
    /// Runs each post through the chat pipeline.
    post: Arc<dyn PlatformPost>,
    /// Resolves a DM's channel to its adapter and config.
    resolver: Arc<dyn AdapterResolver>,
}

impl FirstSyncPoster {
    /// A poster over the composition root, posting through `post` and
    /// delivering DM copies through the adapters `resolver` builds.
    #[must_use]
    pub fn new(
        ctx: Weak<ServerContext>,
        post: Arc<dyn PlatformPost>,
        resolver: Arc<dyn AdapterResolver>,
    ) -> Self {
        Self {
            ctx,
            post,
            resolver,
        }
    }

    /// Post the coach's first message to the athlete about `provider`'s data,
    /// once per athlete.
    async fn post_first_sync(&self, user_id: Uuid, tenant_id: TenantId, provider: &str) {
        let Some(resources) = self.ctx.upgrade() else {
            return;
        };
        if !claim_first_post(&resources, user_id, tenant_id, provider).await {
            return;
        }
        let display = backend_resolver::brand_name(&global_registry(), provider)
            .unwrap_or(provider)
            .to_owned();
        let athlete = PostAthlete { user_id, tenant_id };
        let subject = FirstPostSubject {
            user_id,
            tenant_id,
            provider,
        };
        let prompt = |locale: &str| first_sync_prompt(&resources, locale, &display);

        // Decided before any turn runs: the DM the athlete last wrote from,
        // when its window is open; a held DM gets no turn.
        let dms = linked_dms(&resources, athlete, Utc::now()).await;

        let posted_in_app = post_in_app(&resources, self.post.as_ref(), athlete, &prompt).await;
        if let Some(posted) = posted_in_app {
            report_first_post(&subject, PostSurface::InApp, posted);
        }
        let posted_to_dm = match dms.open {
            Some(dm) => {
                let surface = PostSurface::Channel(dm.channel_type);
                let posted = post_to_dm(
                    &resources,
                    self.post.as_ref(),
                    self.resolver.as_ref(),
                    athlete,
                    dm,
                    &prompt,
                )
                .await;
                if let Some(posted) = posted {
                    report_first_post(&subject, surface, posted);
                }
                posted
            }
            None => None,
        };

        if posted_in_app != Some(true) && posted_to_dm != Some(true) {
            release_first_post(&resources, user_id, tenant_id).await;
        }
    }

    /// Read `provider`'s activity list once for an athlete who just connected
    /// it, and post when the read lands activities.
    async fn first_read(&self, user_id: Uuid, provider: &str) {
        let Some(resources) = self.ctx.upgrade() else {
            return;
        };
        let backend = backend_resolver::sync_backend(provider);
        let Some(tenant_id) =
            token_tenant(&resources.common.repos.auth_repos(), user_id, &backend).await
        else {
            info!(
                provider,
                "First read: no token for the provider just connected"
            );
            return;
        };
        if first_read_landed(Arc::clone(&resources), user_id, tenant_id, provider).await {
            self.post_first_sync(user_id, tenant_id, provider).await;
        }
        // Asked whether or not the recent read landed anything: an athlete
        // with no activity in the last month may still have a season behind it.
        request_history_depth(resources, user_id, tenant_id).await;
    }
}

#[async_trait]
impl FirstSyncHook for FirstSyncPoster {
    async fn data_landed(&self, user_id: Uuid, tenant_id: TenantId, provider: &str) {
        self.post_first_sync(user_id, tenant_id, provider).await;
        // After the post, not beside it: on a scrape provider the capture and
        // the post's turn would read the one browser session at once.
        if let Some(resources) = self.ctx.upgrade() {
            request_history_depth(resources, user_id, tenant_id).await;
        }
    }

    async fn connected_on_demand(&self, user_id: Uuid, provider: &str) {
        self.first_read(user_id, provider).await;
    }
}

/// Claim the first post to the athlete. `false` when an earlier trigger holds
/// it — the post was made, or is being made — or the claim failed (logged).
async fn claim_first_post(
    resources: &ServerContext,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> bool {
    match resources
        .common
        .repos
        .platform_post_claims
        .claim_platform_post(
            tenant_id,
            user_id,
            PostPurpose::FirstSync.as_str(),
            Utc::now(),
        )
        .await
    {
        Ok(claimed) => {
            if !claimed {
                info!(provider, "First post already made or under way; skipping");
            }
            claimed
        }
        Err(e) => {
            warn!(error = %e, "First post: the once-claim failed");
            false
        }
    }
}

/// Release the claim of a first post that ended up posting nothing, so the
/// next landing tries again.
async fn release_first_post(resources: &ServerContext, user_id: Uuid, tenant_id: TenantId) {
    if let Err(e) = resources
        .common
        .repos
        .platform_post_claims
        .release_platform_post(tenant_id, user_id, PostPurpose::FirstSync.as_str())
        .await
    {
        warn!(error = %e, "First post: releasing the claim of an unmade post failed");
    }
}

/// Read an on-demand `provider`'s activity list once, writing it through to
/// the cache, and say whether the read landed at least one activity.
async fn first_read_landed(
    resources: Arc<ServerContext>,
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &str,
) -> bool {
    let runtime: Arc<dyn ToolRuntime> = resources;
    let params = ActivityQueryParams {
        after: Some((Utc::now() - Duration::days(i64::from(CONNECT_RECENT_DAYS))).timestamp()),
        before: None,
        limit: Some(FIRST_READ_LIMIT),
        offset: None,
    };
    match fetch_provider_head(&runtime, provider, user_id, &tenant_id.to_string(), &params).await {
        Ok(activities) if !activities.is_empty() => true,
        Ok(_) => {
            info!(
                provider,
                "First read landed no activity; nothing to post about"
            );
            false
        }
        Err(e) => {
            warn!(provider, error = %e, "First read of the provider just connected failed");
            false
        }
    }
}

/// Ask for the stored history Home's training status needs, logging the
/// outcome: a connect that cannot ask is told nothing, and the next read that
/// computes the history asks again.
async fn request_history_depth(resources: Arc<ServerContext>, user_id: Uuid, tenant_id: TenantId) {
    let runtime: Arc<dyn ToolRuntime> = resources;
    match request_status_depth(&runtime, tenant_id, user_id).await {
        Ok(computed) => info!(
            user_id = %user_id,
            complete = computed.is_complete(),
            capture_requested = computed.capture_requested,
            "Connect: training-status history depth checked"
        ),
        Err(e) => warn!(
            user_id = %user_id,
            error = %e,
            "Connect: asking for the training-status history depth failed"
        ),
    }
}

/// The first-sync instruction in `locale`, naming the provider, or `None`
/// (logged) when the catalogue does not carry it: an empty prompt would have
/// the coach answer nothing in particular.
fn first_sync_prompt(resources: &ServerContext, locale: &str, display: &str) -> Option<String> {
    let prompt = resources.mcp.messaging_strings_registry.render(
        KEY_PLATFORM_POST_FIRST_SYNC_PROMPT,
        locale,
        &[display],
    );
    if prompt.trim().is_empty() {
        warn!(
            locale,
            "First post: the catalogue carries no first-sync instruction; nothing posted"
        );
        return None;
    }
    Some(prompt)
}

/// The athlete and provider a first post is about, for its notify event.
struct FirstPostSubject<'a> {
    user_id: Uuid,
    tenant_id: TenantId,
    provider: &'a str,
}

/// Report one destination's first post: whether the agent's reply grounded in
/// the athlete's data and was posted.
fn report_first_post(subject: &FirstPostSubject<'_>, surface: PostSurface, grounded: bool) {
    info!(
        target: "notify",
        event = "coach.first_post",
        user_id = %subject.user_id,
        tenant_id = %subject.tenant_id,
        provider = %subject.provider,
        destination = %surface.destination(),
        grounded,
        "coach first post"
    );
}

/// Install the production [`FirstSyncPoster`] on the context's first-sync slot.
///
/// The auth routes report connects to that slot. Called once the
/// composition-root `Arc<ServerContext>` exists; a second call is a startup
/// logic bug, logged not fatal.
pub fn install_first_sync_post(resources: &Arc<ServerContext>, post: Arc<dyn PlatformPost>) {
    let poster: Arc<dyn FirstSyncHook> = Arc::new(FirstSyncPoster::new(
        Arc::downgrade(resources),
        post,
        config_adapter_resolver(Arc::clone(&resources.common.repos)),
    ));
    if resources.mcp.first_sync.set(poster).is_err() {
        warn!("First-sync hook already installed; skipping");
    }
}
