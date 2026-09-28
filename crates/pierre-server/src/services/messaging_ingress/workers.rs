// ABOUTME: The messaging background workers started once per process: the outbound retry queue and the turn resume sweeper
// ABOUTME: Both build a resumed or retried send's adapter from the same stored channel config the webhook ingress uses
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use std::sync::Arc;

use pierre_services::channel_adapters::ConfigChannelAdapters;
use pierre_services::messaging_outbound::start_outbound_worker;
use tracing::info;

use super::resume;
use crate::mcp::resources::ServerContext;

/// Start the messaging background workers for the life of the process.
///
/// Two of them: the outbound retry queue, and the resume sweeper for the turns
/// a drained instance left on file — one pass now, because this instance may
/// exist only because the athlete's next message arrived, then one a minute
/// for a sibling that outlived the drained instance (registre#126). A resumed
/// turn's adapter comes from the same stored channel config the webhook
/// ingress builds from.
pub fn start_background_workers(resources: Arc<ServerContext>) {
    start_outbound_worker(
        Arc::clone(&resources.common.repos.messaging),
        Arc::clone(&resources.common.repos.provider_connections),
        Arc::new(ConfigChannelAdapters),
    );
    info!("Messaging outbound retry worker started");
    resume::start_turn_resume_sweeper(resources, Arc::new(ConfigChannelAdapters));
}
