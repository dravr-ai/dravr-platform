// ABOUTME: Publishes a reply's chart specs as signed image URLs for channels that fetch pixels
// ABOUTME: The pipeline's ScenePublisher seam, backed by the existing viz_delivery negotiation
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The messaging surface's chart publisher.
//!
//! A messaging channel cannot draw a chart spec, but most of them accept a
//! media URL and fetch it server-side at send time. The pipeline asks for that
//! through [`ScenePublisher`] once the assistant message is durable — the
//! specs are addressed by message id, so there is nothing to sign before the
//! row exists — and the answers arrive on the envelope as
//! [`pierre_chat_pipeline::ReplyBlock::SceneImage`] blocks, positioned among
//! the prose rather than assembled again at the egress.

use std::sync::Arc;

use pierre_chat_pipeline::{RenderCapabilities, SceneImage, ScenePublishRequest, ScenePublisher};
use pierre_core::models::ColorScheme;
use tracing::warn;
use uuid::Uuid;

use super::viz_delivery::{plan_media, target as viz_target, VizDelivery};
use crate::mcp::resources::ServerContext;

/// The colour scheme a messaging chart is painted in for `user_id`.
///
/// A messaging chart is fetched by the channel's servers, not the athlete's
/// device, so nothing on the wire can report the scheme the athlete is looking
/// at — the `users.theme` pin is the only signal there is. An athlete who
/// pinned nothing, or whose row cannot be read, gets
/// [`ColorScheme::Dark`]: messaging clients overwhelmingly draw media bubbles
/// on dark, and a chart in the wrong scheme still beats no chart.
///
/// Shared by every path that mints a chart for a channel — the live reply and
/// the backfill push that re-asks a question once the history has loaded — so
/// the two paint the same athlete the same way.
pub async fn athlete_color_scheme(resources: &ServerContext, user_id: Uuid) -> ColorScheme {
    match resources.common.repos.users.get_global(user_id).await {
        Ok(Some(user)) => ColorScheme::resolve(user.theme.as_deref()),
        Ok(None) => ColorScheme::default(),
        Err(e) => {
            warn!(error = %e, "theme lookup failed for chart minting; painting dark");
            ColorScheme::default()
        }
    }
}

/// Mints one signed image URL per stored chart spec.
pub struct MessagingScenePublisher {
    /// Server context supplying the public base URL, the signing secret and
    /// whether a press service is configured at all.
    resources: Arc<ServerContext>,
    /// What the turn's channel can render. Carried so the fidelity
    /// negotiation stays one decision made in one place.
    render: RenderCapabilities,
    /// The athlete's pinned colour scheme, resolved once when the turn starts.
    /// Read here rather than at mint time because `publish` is synchronous —
    /// and because one turn's charts should agree with each other even if the
    /// athlete flips the pin mid-answer.
    theme: ColorScheme,
}

impl MessagingScenePublisher {
    /// Build a publisher for one turn's channel and athlete.
    #[must_use]
    pub const fn new(
        resources: Arc<ServerContext>,
        render: RenderCapabilities,
        theme: ColorScheme,
    ) -> Self {
        Self {
            resources,
            render,
            theme,
        }
    }
}

impl ScenePublisher for MessagingScenePublisher {
    fn publish(&self, request: &ScenePublishRequest<'_>) -> Vec<SceneImage> {
        plan_media(
            &VizDelivery {
                target: viz_target(
                    request.conversation_id.to_owned(),
                    request.user_id.to_owned(),
                    // The tenant the conversation was written under, which the
                    // render route re-reads the message with — not the tenant
                    // owning the channel webhook. The two differ on every Slack
                    // channel chat, and naming the wrong one 404s every chart.
                    request.tenant_id,
                    request.message_id.to_owned(),
                ),
                stored_blocks: Some(request.specs),
                render: &self.render,
                locale: request.locale,
                theme: self.theme,
                base_url: &self.resources.common.config.base_url,
                press_enabled: self.resources.common.photograveur.is_enabled(),
            },
            &self.resources.auth.admin_jwt_secret,
        )
        .into_iter()
        .map(|media| SceneImage {
            url: media.url,
            mime_type: media.mime_type,
            caption: media.caption,
        })
        .collect()
    }
}
