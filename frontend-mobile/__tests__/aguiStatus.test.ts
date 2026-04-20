// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: Unit tests for statusTextForAguiEvent + isTerminalAguiEvent mapping
// ABOUTME: Asserts vocabulary parity with dravr-canot so the mobile progress strip matches Telegram/Slack/Discord

import {
  statusTextForAguiEvent,
  isTerminalAguiEvent,
} from '../src/utils/aguiStatus';

describe('statusTextForAguiEvent', () => {
  it('maps RUN_STARTED to the generic placeholder', () => {
    expect(statusTextForAguiEvent({ type: 'RUN_STARTED' })).toBe('thinking…');
  });

  it('maps TOOL_CALL_RESULT to the generic placeholder so sticky "calling foo…" clears', () => {
    // Parity with canot: a tool result clears the previous sticky
    // "calling foo…" so the user sees the pipeline advancing rather
    // than a stuck placeholder until the next STEP_STARTED.
    expect(
      statusTextForAguiEvent({
        type: 'TOOL_CALL_RESULT',
        run_id: 'r',
      }),
    ).toBe('thinking…');
  });

  it('maps STEP_STARTED("prompt_assembly") to "reading your question…"', () => {
    expect(
      statusTextForAguiEvent({ type: 'STEP_STARTED', step_name: 'prompt_assembly' }),
    ).toBe('reading your question…');
  });

  it('maps STEP_STARTED("dispatch") to "generating response…"', () => {
    expect(
      statusTextForAguiEvent({ type: 'STEP_STARTED', step_name: 'dispatch' }),
    ).toBe('generating response…');
  });

  it('falls back to a generic "<step_name>…" for unknown step names', () => {
    expect(
      statusTextForAguiEvent({ type: 'STEP_STARTED', step_name: 'hydration_check' }),
    ).toBe('hydration_check…');
  });

  it('maps TOOL_CALL_START to "calling <tool_name>…"', () => {
    expect(
      statusTextForAguiEvent({ type: 'TOOL_CALL_START', tool_name: 'get_activities' }),
    ).toBe('calling get_activities…');
  });

  it('maps RUN_ERROR to "error: <message>"', () => {
    expect(
      statusTextForAguiEvent({ type: 'RUN_ERROR', message: 'rate_limited' }),
    ).toBe('error: rate_limited');
  });

  it('returns null for STEP_FINISHED / RUN_FINISHED / unknown types so the strip does not flicker', () => {
    expect(statusTextForAguiEvent({ type: 'STEP_FINISHED' })).toBeNull();
    expect(statusTextForAguiEvent({ type: 'RUN_FINISHED' })).toBeNull();
    expect(statusTextForAguiEvent({ type: 'FUTURE_EVENT_NOT_YET_DEFINED' })).toBeNull();
  });
});

describe('isTerminalAguiEvent', () => {
  it('treats RUN_FINISHED and RUN_ERROR as terminal', () => {
    expect(isTerminalAguiEvent({ type: 'RUN_FINISHED' })).toBe(true);
    expect(isTerminalAguiEvent({ type: 'RUN_ERROR', message: 'x' })).toBe(true);
  });

  it('treats non-terminal events as non-terminal', () => {
    expect(isTerminalAguiEvent({ type: 'RUN_STARTED' })).toBe(false);
    expect(isTerminalAguiEvent({ type: 'STEP_STARTED', step_name: 'x' })).toBe(false);
    expect(isTerminalAguiEvent({ type: 'TOOL_CALL_START' })).toBe(false);
  });
});
