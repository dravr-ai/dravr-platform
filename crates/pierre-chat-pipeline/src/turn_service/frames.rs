// ABOUTME: The turn service's outer futures, each built and boxed in a plain function of its own
// ABOUTME: Keeps the turn's polls off the slot a debug build lays the future out in (see crate::turn_frames)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_core::errors::AppResult;
use pierre_providers::ai_scope;
use pierre_tool_runtime::context::TURN_LOCALE;

use super::{execute_turn, ServedTurn, TurnRequest};
use crate::envelope::TurnEnvelope;
use crate::hooks::PipelineHooks;
use crate::surface_profile::SurfaceProfile;
use crate::turn::TurnInput;
use crate::turn_frames::BoxedStage;
use crate::ChatPipelineContext;

/// [`crate::run`] in the language the turn resolved, built and boxed in a
/// frame of its own, as [`tracked_turn`] is.
pub(super) fn localized_run<'a>(
    ctx: &'a ChatPipelineContext,
    input: TurnInput,
    profile: &'a SurfaceProfile,
    hooks: &'a PipelineHooks<'_>,
) -> BoxedStage<'a, AppResult<TurnEnvelope>> {
    Box::pin(TURN_LOCALE.scope(
        Some(profile.locale.clone()),
        crate::run(ctx, input, profile, hooks),
    ))
}

/// [`super::execute_turn`] over its transport and under a fresh provenance, built
/// and boxed in a frame of its own: a debug build lays the future out on the
/// stack of the frame that builds it, and an async caller would keep that
/// slot under every poll of the turn (`crate::turn_frames::scoped_turn` says why).
pub(super) fn tracked_turn<'a>(
    ctx: &'a ChatPipelineContext,
    request: TurnRequest<'a>,
    profile: &'a SurfaceProfile,
) -> BoxedStage<'a, AppResult<ServedTurn>> {
    let transport = request.transport;
    let provenance = request.hooks.supplied_findings.clone().unwrap_or_default();
    Box::pin(ai_scope::serve_over(
        transport,
        ai_scope::tracking(provenance, execute_turn(ctx, request, profile)),
    ))
}
