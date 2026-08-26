// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: Turn-progress event → user-facing status text, shared by the web and mobile chat UIs
// ABOUTME: Mirrors the Telegram/Slack/Discord vocabulary so progress reads the same on every surface

import type { TurnProgress } from '@pierre/shared-types';

/**
 * Generic placeholder shown before the first named stage arrives and between
 * tool calls.
 *
 * Symmetry with `KEY_THINKING_PLACEHOLDER` in
 * `crates/pierre-contremaitre/src/messaging_strings.rs` — Telegram, Slack and
 * Discord render the localized form of the same string at the same points in
 * the turn, so an athlete who switches devices mid-turn sees continuous
 * terminology across surfaces.
 */
export const THINKING_PLACEHOLDER = 'thinking…';

/** Stage status the server reports as a stage is entered. */
const STAGE_STARTED = 'started';

/** The ACP tool-call state that means the call is over. */
const TOOL_COMPLETED = 'Completed';

/**
 * Map one progress event to a short, user-facing status line, or `null` when
 * it should not be surfaced.
 *
 * The vocabulary matches the channel-side renderer in
 * `pierre_services::messaging_status_bridge`, so an athlete sees the same
 * "reading your question…" whether they talk to the coach over Telegram, the
 * mobile app, or the web app.
 *
 * Keep results short (<= 60 chars) so the line fits one row of a progress
 * strip.
 */
export function statusTextForProgress(progress: TurnProgress): string | null {
  if (progress.kind === 'tool') {
    // A finished tool call clears the sticky "calling foo…" line rather than
    // leaving the athlete looking at a step that already completed.
    if (progress.status === TOOL_COMPLETED) return THINKING_PLACEHOLDER;
    return progress.title ? `calling ${progress.title}…` : 'running a tool…';
  }
  // A stage that has finished is a transient marker: the next event
  // overwrites it, and showing "prompt_assembly finished" says nothing the
  // athlete can act on.
  if (progress.status !== STAGE_STARTED) return null;
  if (progress.title === 'prompt_assembly') return 'reading your question…';
  if (progress.title === 'dispatch') return 'generating response…';
  return progress.title ? `${progress.title}…` : null;
}
