// ABOUTME: Boot-time sizing of the chat tool catalogue for ServerContext
// ABOUTME: Logs its wire size per seat and has Anthropic count its tokens off the boot path
//
// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

use pierre_llm::config::LlmChain;
use pierre_llm::FunctionDeclaration;
use pierre_tool_runtime::registry::ToolRegistry;
use tracing::{info, warn};

use super::ServerContext;

impl ServerContext {
    /// Log the chat tool catalogue's wire size once per boot, and have
    /// Anthropic count it off the boot path (carnet#1196).
    ///
    /// Two surfaces: the athlete's own turn, which sends every chat-callable
    /// tool — and, with the rest deferred, holds only the loaded core in
    /// context — and the coach's seat, which withholds the coach's own data
    /// tools.
    /// The contremaitre overlay has not synced yet at this point, so this is
    /// the compiled-in text; every turn's dispatch line carries the size and
    /// digest it actually sent.
    pub(super) fn report_tool_catalogue_size(registry: &ToolRegistry, chain: &LlmChain) {
        use pierre_tool_runtime::chat_catalogue::{
            build_mcp_tools, defer_beyond_core, measure_catalogue,
        };
        use pierre_tool_runtime::coach_seat::TurnSeat;

        /// How many of the largest tools the line names
        const LARGEST_LOGGED: usize = 20;

        let mut tools = build_mcp_tools(registry);
        let chat = measure_catalogue(&tools);
        // What a turn holds in context when the rest is deferred, on a
        // provider with tool search (carnet#406).
        let mut deferred = tools.clone();
        defer_beyond_core(&mut deferred);
        let loaded = measure_catalogue(&deferred);
        // The detached count owns its copy: the coach-seat filter below
        // narrows `tools` in place.
        Self::spawn_tool_catalogue_token_count(
            chain.anthropic_models(),
            tools.function_declarations.clone(),
            chat.bytes,
        );
        tools
            .function_declarations
            .retain(|decl| !TurnSeat::Coach.withholds(registry, &decl.name));
        // LIMITATION(registre#1208): `coach_seat` sizes `TurnSeat::Coach`
        // (a group's room) only; `TurnSeat::AthleteThread` is sized per turn
        // by the dispatch line, never here.
        let coach_seat = measure_catalogue(&tools);
        info!(
            event_type = "tool_catalogue_size",
            tool_count = chat.tool_count,
            bytes = chat.bytes,
            estimated_tokens = chat.estimated_tokens,
            digest = %chat.digest,
            loaded_tool_count = loaded.tool_count - loaded.deferred_count,
            loaded_bytes = loaded.loaded_bytes,
            loaded_estimated_tokens = loaded.loaded_estimated_tokens,
            coach_seat_tool_count = coach_seat.tool_count,
            coach_seat_bytes = coach_seat.bytes,
            coach_seat_estimated_tokens = coach_seat.estimated_tokens,
            largest = %chat.largest(LARGEST_LOGGED),
            "Chat tool catalogue size at startup"
        );
    }

    /// Count the chat catalogue on every model the chain runs on the Anthropic
    /// API, in a detached task (carnet#1196).
    ///
    /// `count_tokens` is free and rate-limited apart from Messages. It counts
    /// the same compiled-in set `tool_catalogue_size` sized, so `bytes_per_token`
    /// converts any turn's `tools_bytes` on that model. A chain without the
    /// Anthropic API counts nothing, and a refused count is a warning, never a
    /// failed boot.
    fn spawn_tool_catalogue_token_count(
        models: Vec<String>,
        tools: Vec<FunctionDeclaration>,
        bytes: usize,
    ) {
        use pierre_llm::http_env;

        if models.is_empty() {
            return;
        }
        let count = tokio::spawn(async move {
            for model in models {
                match http_env::anthropic_tool_tokens(&model, &tools).await {
                    Ok(tokens) => {
                        // Byte and token counts sit far below 2^52.
                        #[allow(clippy::cast_precision_loss)]
                        let bytes_per_token = bytes as f64 / f64::from(tokens.max(1));
                        info!(
                            event_type = "tool_catalogue_tokens",
                            model = %model,
                            tokens,
                            bytes,
                            bytes_per_token,
                            "Chat tool catalogue counted by Anthropic"
                        );
                    }
                    Err(error) => warn!(
                        model = %model,
                        error = %error,
                        "Chat tool catalogue token count failed"
                    ),
                }
            }
        });
        // A panic in a detached task is otherwise swallowed, leaving the
        // token line missing with nothing in the logs to explain it.
        tokio::spawn(async move {
            if let Err(e) = count.await {
                warn!(error = %e, "Chat tool catalogue token count task failed");
            }
        });
    }
}
