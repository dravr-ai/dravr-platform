// ABOUTME: GET /api/surfaces/capabilities — the SurfaceProfile::resolve table as one readable document
// ABOUTME: The catalogue the shared-constants generator reads, so no client restates what a surface renders
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The surface-capability catalogue.
//!
//! Six phases converged the surfaces onto one profile, one envelope and one
//! transport. What none of that gave anyone is a way to *read* the result:
//! `SurfaceProfile::resolve` answers per turn, in Rust, and every client that
//! needed to know what a surface renders learned it by looking at the other
//! client.
//!
//! This endpoint answers for every surface at once. It is the same relationship
//! `packages/mcp-types/src/tools.ts` has with the tool registry: the server is
//! the source, a generator writes the TypeScript, and the generated file is the
//! only thing a client reads. Nothing here is hand-maintained — every row comes
//! out of [`SurfaceProfile::resolve`] itself, so a capability that changes in
//! the pipeline changes here on the next request.
//!
//! Unauthenticated on purpose. The document is the compiled-in product
//! capability table — no user, no tenant, no configuration, the same bytes for
//! every caller — and keeping it open is what lets the generator run against a
//! bare dev server with no credentials to arrange.

use axum::routing::get;
use axum::{Json, Router};
use pierre_chat_pipeline::{
    ModelPolicy, ProgressiveSupport, ProseFormat, ProviderStreaming, ReplyBlockKind, SurfaceId,
    SurfaceProfile, SurfaceRequest, TurnBudget,
};
use pierre_core::models::messaging::ChannelType;
use pierre_core::models::NotificationScreen;
use serde::{Deserialize, Serialize};

use crate::services::messaging_ingress::surface::{surface_id, transport_caps};

/// Locale the catalogue resolves its rows under.
///
/// A profile carries the locale of the turn it was resolved for; the catalogue
/// describes the surface, not a turn, so it resolves every row under one fixed
/// locale and never reports it. Nothing in a row varies with it.
const CATALOGUE_LOCALE: &str = "en";

/// The messaging channels this server compiles in.
///
/// Enumerated here rather than derived from [`SurfaceId`] so there is no
/// inverse of [`surface_id`] to keep in step: each channel names itself, and
/// the surfaces left over are the in-app ones.
const MESSAGING_CHANNELS: [ChannelType; 5] = [
    ChannelType::Telegram,
    ChannelType::WhatsApp,
    ChannelType::Discord,
    ChannelType::Slack,
    ChannelType::Messenger,
];

/// Everything the clients generate their capability constants from.
#[derive(Debug, Serialize, Deserialize)]
pub struct SurfaceCapabilitiesResponse {
    /// Every reply-block kind the envelope can produce, in reply order.
    pub block_kinds: Vec<String>,
    /// The `data.screen` vocabulary, paired with the surface each token opens.
    pub notification_screens: Vec<NotificationScreenRow>,
    /// One row per surface, in [`SurfaceId::ALL`] order.
    pub surfaces: Vec<SurfaceCapabilityRow>,
}

/// One entry of the notification `data.screen` vocabulary.
#[derive(Debug, Serialize, Deserialize)]
pub struct NotificationScreenRow {
    /// The token as it travels on the wire.
    pub screen: String,
    /// The `USER_SURFACES` id it opens.
    pub surface: String,
}

/// What one surface renders.
#[derive(Debug, Serialize, Deserialize)]
pub struct SurfaceCapabilityRow {
    /// The surface's telemetry id, e.g. `"web_chat"`.
    pub id: String,
    /// The `call_type` stamped on this surface's `llm_usage` rows.
    pub call_type: String,
    /// `"markdown"` or `"plain_text"`.
    pub prose: String,
    /// Per-message character ceiling, or `null` where the transport imposes
    /// none.
    pub max_reply_chars: Option<usize>,
    /// Whether a rendered control's press reaches the platform.
    pub interactive: bool,
    /// `"complete"` or `"delta_channel"` — whether the transport can carry a
    /// reply before the turn finishes.
    pub progressive: String,
    /// Whether a turn on this surface puts partial text on the wire when the
    /// provider emits deltas. Both halves of the streaming question, crossed
    /// once here so a reader does not have to.
    pub streams_text_deltas: bool,
    /// Fixed tool-loop budget, or `null` when it resolves from coach/admin
    /// configuration.
    pub max_tool_iterations: Option<usize>,
    /// `"use_stored"` or `"override_with_env"`.
    pub model_policy: String,
    /// Reply-block kinds this surface can be handed, in reply order.
    pub blocks: Vec<String>,
}

/// Router for the capability catalogue.
pub struct SurfaceRoutes;

impl SurfaceRoutes {
    /// Mount `GET /api/surfaces/capabilities`.
    ///
    /// Takes no server state: every byte it serves is compiled in.
    pub fn routes() -> Router {
        Router::new().route("/api/surfaces/capabilities", get(capabilities))
    }
}

/// Serve the catalogue.
async fn capabilities() -> Json<SurfaceCapabilitiesResponse> {
    Json(catalogue())
}

/// Build the catalogue from the pipeline's own resolver.
///
/// Nothing is transcribed: every row is a real [`SurfaceProfile::resolve`]
/// call, so the document describes the profiles turns actually run under.
fn catalogue() -> SurfaceCapabilitiesResponse {
    let mut surfaces = Vec::with_capacity(SurfaceId::ALL.len());
    for surface in SurfaceId::ALL {
        // In-app surfaces are the ones no compiled-in channel claims, so a new
        // surface of either kind lands in exactly one branch and the table
        // stays total.
        let transport = MESSAGING_CHANNELS
            .iter()
            .copied()
            .find(|channel| surface_id(*channel) == surface)
            .map(transport_caps);
        surfaces.push(row(&SurfaceProfile::resolve(&SurfaceRequest {
            surface,
            locale: CATALOGUE_LOCALE.to_owned(),
            transport,
            prose_contract: None,
        })));
    }

    SurfaceCapabilitiesResponse {
        block_kinds: ReplyBlockKind::ALL
            .iter()
            .map(|kind| kind.as_str().to_owned())
            .collect(),
        notification_screens: NotificationScreen::all()
            .iter()
            .map(|screen| NotificationScreenRow {
                screen: screen.as_str().to_owned(),
                surface: screen.surface().to_owned(),
            })
            .collect(),
        surfaces,
    }
}

/// Serialize one resolved profile.
fn row(profile: &SurfaceProfile) -> SurfaceCapabilityRow {
    let render = profile.render;
    SurfaceCapabilityRow {
        id: profile.surface.as_str().to_owned(),
        call_type: profile.surface.call_type().to_owned(),
        prose: match render.prose {
            ProseFormat::Markdown => "markdown",
            ProseFormat::PlainText => "plain_text",
        }
        .to_owned(),
        // `usize::MAX` is the pipeline's way of saying "no transport ceiling";
        // on the wire that is an absence, not a number a client might pack
        // against.
        max_reply_chars: (render.max_reply_chars != usize::MAX).then_some(render.max_reply_chars),
        interactive: render.interactive,
        progressive: match render.progressive {
            ProgressiveSupport::Complete => "complete",
            ProgressiveSupport::DeltaChannel => "delta_channel",
        }
        .to_owned(),
        streams_text_deltas: render
            .progressive
            .delivers_partial_text(ProviderStreaming::TextDeltas),
        max_tool_iterations: match profile.budget {
            TurnBudget::Fixed(iterations) => Some(iterations),
            TurnBudget::CoachOrAdminDefault => None,
        },
        model_policy: match profile.model_policy {
            ModelPolicy::UseStored => "use_stored",
            ModelPolicy::OverrideWithEnv => "override_with_env",
        }
        .to_owned(),
        blocks: render
            .renderable_blocks()
            .into_iter()
            .map(|kind| kind.as_str().to_owned())
            .collect(),
    }
}
