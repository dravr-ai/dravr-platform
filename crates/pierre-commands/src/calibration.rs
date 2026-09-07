// ABOUTME: Handler for /calibrate — start the difficulty-calibration interview on a conversation
// ABOUTME: Supersedes the previous interview's window, snapshots recent load, opens the flow
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use async_trait::async_trait;
use pierre_contremaitre::messaging_strings::{
    KEY_CALIBRATE_OPENER, KEY_CALIBRATE_OPENER_ROOM, KEY_CALIBRATE_START_FAILED,
};
use pierre_core::errors::AppError;
use pierre_core::models::GuidedFlow;
use pierre_messaging::commands::CommandResponse;

use crate::guided_walk::{read_profile, start_walk, WalkSpec};
use crate::{CommandHandler, PlatformCommandContext};

/// Handler for `/calibrate` — tune how hard this athlete's training should be.
///
/// A short guided interview whose answers land as facts that steer later plan
/// generation. Bare `/calibrate` is the only form: an athlete typing `/harder`
/// expects an adjustment to today's plan, not a six-question interview, so that
/// alias is deliberately absent and discovery runs through the coach instead.
///
/// The start sequence — supersession, window, load snapshot, activation,
/// opener — is [`start_walk`], shared with `/season`; what is calibration's
/// own is the flow and its three strings.
pub struct CalibrateHandler;

#[async_trait]
impl CommandHandler for CalibrateHandler {
    async fn execute(&self, ctx: &PlatformCommandContext) -> Result<CommandResponse, AppError> {
        let profile = read_profile(ctx, "/calibrate").await?;
        start_walk(
            ctx,
            profile,
            WalkSpec {
                flow: GuidedFlow::Calibration,
                command: "/calibrate",
                opener_private: KEY_CALIBRATE_OPENER,
                opener_room: KEY_CALIBRATE_OPENER_ROOM,
                start_failed: KEY_CALIBRATE_START_FAILED,
                opener_args: &[],
            },
        )
        .await
    }
}
