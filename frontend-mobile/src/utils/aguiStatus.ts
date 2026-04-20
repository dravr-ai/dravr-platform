// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: AG-UI event → user-facing status text mapping for the mobile chat UI
// ABOUTME: Mirrors the Telegram/Slack/Discord vocabulary so progress feels consistent cross-channel

/**
 * AG-UI event shape as it appears on the SSE wire.
 *
 * Only fields the progress UI actually consumes are typed; anything
 * else the server includes is preserved as `any` so future fields
 * don't force a mobile redeploy. This matches canot's
 * `#[serde(other)] Unknown` forward-compat contract on the Rust side.
 */
export interface AguiEventWire {
  type: string;
  run_id?: string;
  step_name?: string;
  tool_name?: string;
  message?: string;
}

/**
 * Generic placeholder shown at run start and between tool calls.
 *
 * Symmetry with the Rust `PLACEHOLDER_TEXT` const in
 * `crates/pierre-server/src/services/messaging_status_bridge.rs` —
 * messaging channels (Telegram/Slack/Discord) render the same string
 * when their pipeline lands on `RUN_STARTED` / `TOOL_CALL_RESULT`,
 * so a user who switches devices mid-turn sees continuous terminology
 * across surfaces.
 */
const THINKING_PLACEHOLDER = 'thinking…';

/**
 * Map an AG-UI event to a short, user-facing status line, or `null`
 * when the event should not be surfaced (e.g. step-finished — a
 * transient marker the next event will overwrite anyway).
 *
 * The vocabulary matches the channel-side renderers in
 * `dravr-canot/src/agui_status.rs::status_text_for_event`, so a user
 * sees the same "reading your question…" string whether they talk to
 * the assistant over Telegram or the mobile app.
 *
 * Keep results short (<= 60 chars) so the line fits on one row of
 * the chat progress strip.
 */
export function statusTextForAguiEvent(event: AguiEventWire): string | null {
  switch (event.type) {
    // RUN_STARTED opens the turn with the generic placeholder.
    // TOOL_CALL_RESULT clears the sticky "calling foo…" status so the
    // user sees the tool advanced rather than a stuck placeholder;
    // "thinking…" bridges back to whatever the next step emits.
    case 'RUN_STARTED':
    case 'TOOL_CALL_RESULT':
      return THINKING_PLACEHOLDER;
    case 'STEP_STARTED': {
      const name = event.step_name ?? '';
      if (name === 'prompt_assembly') return 'reading your question…';
      if (name === 'dispatch') return 'generating response…';
      return name ? `${name}…` : null;
    }
    case 'TOOL_CALL_START':
      return event.tool_name ? `calling ${event.tool_name}…` : 'running a tool…';
    case 'RUN_ERROR':
      return event.message ? `error: ${event.message}` : 'error';
    // STEP_FINISHED / RUN_FINISHED / Unknown fall through to `null`.
    default:
      return null;
  }
}

/**
 * True when the event terminates the run. The progress hook stops
 * tracking edits at this point — the UI collapses the progress strip
 * and shows the assistant's final reply (the HTTP turn response).
 */
export function isTerminalAguiEvent(event: AguiEventWire): boolean {
  return event.type === 'RUN_FINISHED' || event.type === 'RUN_ERROR';
}
