// ABOUTME: Handler for /season — start the season walk on a conversation: the calendar and what it demands
// ABOUTME: Quotes back /calibrate's availability rather than re-asking it, then opens the fixed-list flow
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_contremaitre::messaging_strings::{
    KEY_SEASON_OPENER, KEY_SEASON_OPENER_AVAILABILITY, KEY_SEASON_OPENER_ROOM,
    KEY_SEASON_START_FAILED,
};
use pierre_core::errors::AppError;
use pierre_core::models::{Dossier, GuidedFlow, Pillar};
use pierre_memory::{FactKind, FactSource, PredicateCode};
use pierre_messaging::commands::CommandResponse;
use pierre_services::memory_facts::SentenceRenderer;
use tracing::warn;

use crate::guided_walk::{read_profile, start_walk, WalkSpec};
use crate::{CommandHandler, PlatformCommandContext};

/// Handler for `/season` — lay out what this athlete's season is for.
///
/// A short guided walk whose answers land as facts the flavour rule and the
/// season layout read: the race calendar and the event that matters, goals
/// on two horizons, best performances, background, what they can steer
/// intensity by, and what they want from coaching. Availability is
/// `/calibrate`'s and is quoted back in the opener for correction, never
/// asked cold again — the design rule is *infer, then confirm*.
///
/// The start sequence is [`start_walk`], shared with `/calibrate`.
pub struct SeasonHandler;

/// The availability `/calibrate` recorded, as one sentence per fact in the
/// athlete's locale — the current, onboarding-sourced `schedule` facts of the
/// training pillar. Empty when nothing is on file, in which case the opener
/// asks its first question without a quote.
fn availability_on_file(dossier: &Dossier, renderer: SentenceRenderer<'_>) -> Vec<String> {
    dossier
        .pillars
        .get(&Pillar::TrainingAndMovement)
        .into_iter()
        .flatten()
        .filter(|f| {
            !f.stale
                && f.kind == FactKind::Schedule.as_str()
                && f.source == FactSource::Onboarding.as_str()
        })
        .map(|f| {
            let code = PredicateCode::parse(&f.predicate_code).unwrap_or(PredicateCode::States);
            renderer.render(code, &f.object)
        })
        .collect()
}

#[async_trait]
impl CommandHandler for SeasonHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let profile = read_profile(ctx, "/season").await?;

        // The quote-back reads the dossier under the athlete's own tenant —
        // where `/calibrate` landed the answer. A dossier that cannot be
        // composed costs the quote, not the walk: the opener asks cold, and
        // the miss is logged so it is not mistaken for an athlete who never
        // calibrated.
        let reg = ctx.ctx.messaging_strings_registry();
        let quoted = match ctx
            .ctx
            .repos()
            .dossier
            .compose_dossier(ctx.tenant_id, ctx.user_id)
            .await
        {
            Ok(dossier) => availability_on_file(&dossier, SentenceRenderer::new(reg, &ctx.locale)),
            Err(e) => {
                warn!(error = %e, user_id = %ctx.user_id, "/season could not read the dossier; opening without the availability quote");
                Vec::new()
            }
        };
        let joined = quoted.join("; ");
        let (opener_private, opener_args): (&str, &[&str]) = if joined.is_empty() {
            (KEY_SEASON_OPENER, &[])
        } else {
            (KEY_SEASON_OPENER_AVAILABILITY, &[joined.as_str()])
        };

        start_walk(
            ctx,
            profile,
            WalkSpec {
                flow: GuidedFlow::Season,
                command: "/season",
                opener_private,
                opener_room: KEY_SEASON_OPENER_ROOM,
                start_failed: KEY_SEASON_START_FAILED,
                opener_args,
            },
        )
        .await
    }
}
