// ABOUTME: Records whose data an A2A tool task read, the members the dispatch noted, on the task's row
// ABOUTME: The data export and every A2A task read decide the task's row by them (carnet#1511 design calls 7 and 28)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! A tool task carries no conversation: its `contextId` is the client's own,
//! so no transcript's rows say whose data its result holds. The dispatch
//! notes each group member it reads ([`ai_scope::note_member_read`]), as it
//! does for a chat turn's tool rounds; the reads noted while the task's tool
//! ran are recorded on the task's row (`about_members`, a JSON array of
//! reads, each with how it was made: carnet#1511 call 19), so the data
//! export and A2A `GetTask`, `ListTasks`, `SubscribeToTask` and `CancelTask`
//! apply the per-reader rule to the task as to a conversation's rows. A
//! task whose record could not be written keeps no stamp, and the export
//! leaves an unstamped task out.

use std::future::Future;
use std::sync::Arc;

use pierre_providers::ai_scope;
use pierre_runtime_context::A2ACtx;
use pierre_services::task_reading::SupportStamp;
use tracing::warn;

use super::A2AServer;

impl A2AServer {
    /// Run `tool`, the task `task_id`'s tool call, recording on the task's
    /// row the members whose data it read, and the admin's support session
    /// it ran in, if any (`support_session`): such a task is read only while
    /// that session lives (carnet#1511 call 28).
    pub(super) async fn recording_members<F: Future>(
        ctx: &Arc<dyn A2ACtx>,
        task_id: &str,
        support_session: Option<String>,
        tool: F,
    ) -> F::Output {
        let (output, members) = ai_scope::reading_members(tool).await;
        let reads: Vec<_> = members.into_iter().collect();
        let encoded = match support_session {
            Some(support_session) => serde_json::to_string(&SupportStamp {
                support_session,
                reads,
            }),
            None => serde_json::to_string(&reads),
        };
        // Unencodable, the task keeps no stamp and is left out of exports.
        let Ok(stamp) = encoded else {
            warn!(task_id, "A2A task: whose data it read could not be encoded");
            return output;
        };
        if let Err(e) = ctx.repos().a2a.record_task_members(task_id, &stamp).await {
            // Unstamped, the task is left out of every export: nothing is
            // served that cannot be decided.
            warn!(error = %e, task_id, "A2A task: whose data it read was not recorded");
        }
        output
    }
}
