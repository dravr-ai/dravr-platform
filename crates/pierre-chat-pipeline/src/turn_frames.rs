// ABOUTME: The turn's deepest futures, each built and boxed in a plain function of its own
// ABOUTME: A debug build lays a future out on the stack of the frame that builds it; built here, that slot is gone before a poll

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! Frames of their own for the turn's deepest futures.
//!
//! A debug build lays a future out on the stack of the frame that builds it
//! before moving it into its box, and an async caller keeps that slot for as
//! long as it is polled. Built in these plain functions, the slot is freed
//! once the box holds it, so the turn's deepest polls do not stack on it: a
//! coach's roster of two members over the API-key route overflowed the 2 MB
//! test thread with it.

use std::future::Future;
use std::pin::Pin;

use pierre_core::errors::AppResult;
use pierre_llm::ChatMessage;
use pierre_providers::ai_scope;
use pierre_tool_runtime::context::TURN_DIRECT_MESSAGE;
use pierre_tool_runtime::tool_loop_io::ToolLoopResult;

use crate::envelope::{ReconnectPrompt, TurnEnvelope};
use crate::hooks::PipelineHooks;
use crate::recovery::{run_recovery_and_post_process, RecoveryAndPostProcessInputs};
use crate::stages;
use crate::surface_profile::SurfaceProfile;
use crate::turn::{TurnInput, UserMessageResult};
use crate::{answer_stored_turn, run_turn, ChatPipelineContext, TurnEventSink};

/// [`run_recovery_and_post_process`], built and boxed here for the reason
/// [`scoped_turn`] is.
pub(crate) fn recovered_and_post_processed<'a>(
    inputs: RecoveryAndPostProcessInputs<'a>,
    result: &'a mut ToolLoopResult,
    hooks: &'a PipelineHooks<'_>,
) -> BoxedStage<
    'a,
    (
        stages::post_process::PostProcessedReply,
        Option<ReconnectPrompt>,
    ),
> {
    Box::pin(run_recovery_and_post_process(inputs, result, hooks))
}

/// A stage's future, boxed from a frame of its own ([`scoped_turn`]).
pub(crate) type BoxedStage<'stage, T> = Pin<Box<dyn Future<Output = T> + Send + 'stage>>;

/// The turn's tool loop, built and boxed here (`dispatch_stage`).
pub(crate) fn dispatch_llm<'a>(
    inputs: stages::tool_dispatch::DispatchLlmInputs<'a>,
    llm_messages: &'a mut Vec<ChatMessage>,
    max_iterations: usize,
    stream_sink: Option<TurnEventSink>,
) -> BoxedStage<'a, AppResult<stages::tool_dispatch::DispatchedTurn>> {
    Box::pin(stages::tool_dispatch::dispatch_llm_with_tools(
        inputs,
        llm_messages,
        max_iterations,
        stream_sink,
    ))
}

/// A turn's future, boxed from a frame of its own.
pub(crate) type BoxedTurn<'a> = BoxedStage<'a, AppResult<TurnEnvelope>>;

/// `run_turn` under the turn's group-chat and direct-message scopes,
/// built and boxed here.
///
/// A debug build lays a future it builds out on the stack of the frame that
/// builds it before moving it into its box, and an async caller keeps that
/// slot for as long as it is polled. Built in this plain function, the slot
/// is freed once the box holds it, so the turn's deepest polls do not stack
/// on it: a roster turn over the API-key route overflowed the 2 MB test
/// thread with it.
pub(crate) fn scoped_turn<'a>(
    ctx: &'a ChatPipelineContext,
    input: TurnInput,
    profile: &'a SurfaceProfile,
    hooks: &'a PipelineHooks<'_>,
    group_chat: bool,
) -> BoxedTurn<'a> {
    let direct_message = input.is_direct_message;
    Box::pin(ai_scope::for_group_chat(
        group_chat,
        TURN_DIRECT_MESSAGE.scope(Some(direct_message), run_turn(ctx, input, profile, hooks)),
    ))
}

/// The rest of the turn once its prompt is stored, built and boxed here for
/// the reason [`scoped_turn`] is. Erased behind `dyn`: it is the deepest
/// future the pipeline builds, and every caller proving its own `Send` would
/// walk it.
pub(crate) fn answered_turn<'a>(
    ctx: &'a ChatPipelineContext,
    input: TurnInput,
    profile: &'a SurfaceProfile,
    hooks: &'a PipelineHooks<'_>,
    msg_result: UserMessageResult,
) -> BoxedTurn<'a> {
    Box::pin(answer_stored_turn(ctx, input, profile, hooks, msg_result))
}
