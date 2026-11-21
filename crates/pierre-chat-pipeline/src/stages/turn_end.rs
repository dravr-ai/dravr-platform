// ABOUTME: Runs, once each, the tools a turn's calls handed to its end — the calendar push a turn's plan saves queued
// ABOUTME: Through the turn's own executor on the handing call's seat, after the reply, whatever the turn came to

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

//! The turn's end (carnet#1702).
//!
//! A call that would only repeat an effect the turn's next call carries hands
//! it to the turn's end ([`pierre_tool_runtime::turn_end`]): an athlete's plan
//! saved in four batches is put on their calendar once, here, rather than
//! once per batch inside the loop, where the fourth push would meet the
//! per-turn destructive cap. Each handed-over tool runs once through the
//! turn's own executor, so the dispatch chokepoint applies every gate a
//! model's call of it meets, the Guardian's destructive charge included.
//!
//! It runs whatever the turn came to — a reply, a stop, a withheld platform
//! reply or an error — because the writes that queued it landed either way.
//! A call still running on a headless tool host when the turn ends hands
//! over into the sink the turn leaves (carnet#1519's seal), and what it hands
//! over runs on a task of its own, for as long as the server runs.
//! A failure here reaches no reply, which is already written: the tool's own
//! events report it (`sync.failed` for a push), and the next read of the plan
//! shows its calendar behind it.

use std::sync::{Arc, Weak};

use serde_json::json;
use tokio::runtime::Handle;
use tracing::{info, warn};

use pierre_core::models::{ConversationTurnId, TenantId};
use pierre_core::transport::Transport;
use pierre_providers::ai_scope;
use pierre_tool_runtime::context::{DispatchOrigin, DISPATCH_ORIGIN};
use pierre_tool_runtime::guardian::TurnKey;
use pierre_tool_runtime::protocol::{UniversalExecutor, UniversalRequest};
use pierre_tool_runtime::runtime::ToolRuntime;
use pierre_tool_runtime::turn_end::HandedOver;

use super::tool_dispatch::{self, TurnBinding};
use crate::supervised::{spawn_supervised, TaskOwner};
use crate::turn::TurnInput;
use crate::ChatPipelineContext;

/// What running a turn's end needs, captured before the turn spends its input.
pub(crate) struct TurnEndRun {
    binding: TurnBinding,
    user_id: String,
    tool_tenant_id: TenantId,
    turn_id: ConversationTurnId,
}

impl TurnEndRun {
    /// The end of `input`'s turn.
    pub(crate) fn of(input: &TurnInput) -> Self {
        Self {
            binding: TurnBinding::of(input),
            user_id: input.user_id.clone(),
            tool_tenant_id: input.tool_tenant_id,
            turn_id: input.turn_id,
        }
    }

    /// Run each tool the turn's calls handed over, once, in the order they
    /// were first handed over, and seal the turn so a call that settles
    /// after it (a headless tool host's) still has what it hands over run.
    pub(crate) async fn run(self, ctx: &ChatPipelineContext) {
        // The key every dispatch of the turn recorded under: the turn's
        // executors call tools under its tool tenant and its turn token.
        let key = TurnKey::new(
            Some(self.tool_tenant_id.as_uuid()),
            self.turn_id.0.to_string(),
        );
        let late = LateRun {
            runtime: Arc::downgrade(&ctx.tool_runtime),
            binding: self.binding.clone(),
            transport: ai_scope::declared_transport(),
            user_id: self.user_id.clone(),
            tool_tenant_id: self.tool_tenant_id,
            turn_id: self.turn_id,
        };
        let handed_over = ctx
            .tool_runtime
            .turn_end_dispatches()
            .take_and_seal(&key, move |handed| late.spawn(handed));
        for handed in handed_over {
            let executor = tool_dispatch::executor_for(ctx, &self.binding, handed.seat);
            run_handed(&executor, handed, &self.user_id, self.tool_tenant_id).await;
        }
    }
}

/// What a call that settles after its turn ended hands over, run on a task
/// of its own, outside the turn's scopes.
///
/// The sink holding it outlives the turn by up to the store's TTL, so it
/// holds the runtime weakly and keeps nothing alive: a hand-off that arrives
/// once the runtime is gone runs nothing. It carries the turn's transport,
/// which an executor built inside the turn would have read from its scope.
struct LateRun {
    runtime: Weak<dyn ToolRuntime>,
    binding: TurnBinding,
    transport: Option<Transport>,
    user_id: String,
    tool_tenant_id: TenantId,
    turn_id: ConversationTurnId,
}

impl LateRun {
    /// Run `handed_over` on a supervised task of its own, through executors
    /// built now over the runtime, while it lives.
    fn spawn(&self, handed_over: Vec<HandedOver>) {
        let Some(runtime) = self.runtime.upgrade() else {
            warn!(
                turn_id = %self.turn_id,
                "a hand-off arrived after its turn's runtime was gone; nothing runs"
            );
            return;
        };
        let runs: Vec<(UniversalExecutor, HandedOver)> = handed_over
            .into_iter()
            .map(|handed| {
                let executor =
                    tool_dispatch::executor_on(Arc::clone(&runtime), &self.binding, handed.seat);
                let executor = match self.transport {
                    Some(transport) => executor.with_transport(transport),
                    None => executor,
                };
                (executor, handed)
            })
            .collect();
        let (user_id, tool_tenant_id) = (self.user_id.clone(), self.tool_tenant_id);
        let owner = TaskOwner::Turn(self.turn_id);
        spawn_supervised(&Handle::current(), "turn_end_late_run", owner, async move {
            for (executor, handed) in runs {
                run_handed(&executor, handed, &user_id, tool_tenant_id).await;
            }
        });
    }
}

/// Run one handed-over tool on `executor`. A failure is logged: the reply it
/// would have reached is written.
async fn run_handed(
    executor: &UniversalExecutor,
    handed: HandedOver,
    user_id: &str,
    tool_tenant_id: TenantId,
) {
    let tool = handed.dispatch.tool;
    let trigger = handed.dispatch.trigger;
    let request = UniversalRequest {
        tool_name: tool.to_owned(),
        parameters: json!({}),
        user_id: user_id.to_owned(),
        protocol: "chat".to_owned(),
        tenant_id: Some(tool_tenant_id.to_string()),
    };
    let ran = DISPATCH_ORIGIN
        .scope(
            DispatchOrigin::TurnEnd { trigger },
            executor.execute_tool(request),
        )
        .await;
    match ran {
        Ok(response) if response.success => {
            info!(
                tool,
                trigger, "the turn's end ran what its calls handed over"
            );
        }
        Ok(response) => warn!(
            tool,
            trigger,
            error = response.error.as_deref().unwrap_or("no error message"),
            "the turn's end ran a handed-over tool that did not succeed"
        ),
        Err(error) => warn!(
            tool,
            trigger,
            error = %error,
            "the turn's end could not run a handed-over tool"
        ),
    }
}
