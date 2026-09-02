// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Turn-progress event → catalogue key for the status line both chat UIs render
// ABOUTME: Mirrors the Telegram/Slack/Discord vocabulary so progress reads the same on every surface

import type { TurnProgress } from '@pierre/shared-types';

import type { TranslatableText } from './text';

/**
 * The catalogue key of the generic placeholder shown before the first named
 * stage arrives and between tool calls.
 *
 * Symmetry with `KEY_THINKING_PLACEHOLDER` in
 * `crates/pierre-contremaitre/src/messaging_strings.rs` — Telegram, Slack and
 * Discord render the localized form of the same string at the same points in
 * the turn, so an athlete who switches devices mid-turn sees continuous
 * terminology across surfaces.
 */
export const THINKING_PLACEHOLDER_KEY = 'chat.status.thinking';

/** Stage status the server reports as a stage is entered. */
const STAGE_STARTED = 'started';

/** The ACP tool-call state that means the call is over. */
const TOOL_COMPLETED = 'Completed';

/**
 * A status line to render: a catalogue key, plus the values its placeholders
 * take.
 *
 * The key rather than the text, because this module is shared by the web and
 * mobile chats and has no locale of its own. Returning finished English here
 * is how a French athlete came to read "generating response…" under French
 * chrome while every i18n gate was green (carnet#206).
 */
export type ProgressStatus = TranslatableText;

/**
 * Map one progress event to the status line to show, or `null` when it should
 * not be surfaced.
 *
 * The vocabulary matches the channel-side renderer in
 * `pierre_services::messaging_status_bridge`, so an athlete sees the same
 * "reading your question…" whether they talk to the coach over Telegram, the
 * mobile app, or the web app.
 *
 * Keep the rendered results short (<= 60 chars) so the line fits one row of a
 * progress strip.
 */
export function statusForProgress(progress: TurnProgress): ProgressStatus | null {
  if (progress.kind === 'tool') {
    // A finished tool call clears the sticky "calling foo…" line rather than
    // leaving the athlete looking at a step that already completed.
    if (progress.status === TOOL_COMPLETED) return { key: THINKING_PLACEHOLDER_KEY };
    return progress.title
      ? { key: 'chat.status.callingTool', params: { tool: progress.title } }
      : { key: 'chat.status.runningTool' };
  }
  // A stage that has finished is a transient marker: the next event
  // overwrites it, and showing "prompt_assembly finished" says nothing the
  // athlete can act on.
  if (progress.status !== STAGE_STARTED) return null;
  if (progress.title === 'prompt_assembly') return { key: 'chat.status.readingQuestion' };
  if (progress.title === 'dispatch') return { key: 'chat.status.generatingResponse' };
  return progress.title ? { key: 'chat.status.stage', params: { stage: progress.title } } : null;
}
