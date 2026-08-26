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

use super::viz_delivery::{plan_media, target as viz_target, VizDelivery};
use crate::mcp::resources::ServerContext;

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
