// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai
// ABOUTME: Unit tests for the web useAgUiProgress hook
// ABOUTME: Drives a mocked fetch ReadableStream of SSE frames and asserts status-text transitions + terminal close

import { describe, it, expect, vi, afterEach } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';
import { useAgUiProgress } from '../useAgUiProgress';

/**
 * Build a `Response`-like object whose body is a `ReadableStream`
 * yielding the supplied SSE wire text in one or more chunks. Mirrors
 * the frame shape the backend emits: `event: <name>\ndata: <json>\n\n`.
 */
function sseResponse(frames: string[]): Response {
  const encoder = new TextEncoder();
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      for (const frame of frames) {
        controller.enqueue(encoder.encode(frame));
      }
      controller.close();
    },
  });
  return { ok: true, body } as unknown as Response;
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('useAgUiProgress', () => {
  it('does not subscribe when runId is null', () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch');
    const { result } = renderHook(() => useAgUiProgress(null, 'jwt'));
    expect(result.current.statusText).toBeNull();
    expect(result.current.isActive).toBe(false);
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it('does not subscribe when token is missing', () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch');
    const { result } = renderHook(() => useAgUiProgress('run-1', null));
    expect(result.current.isActive).toBe(false);
    expect(fetchSpy).not.toHaveBeenCalled();
  });

  it('maps agui frames to status text and closes on RUN_FINISHED', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      sseResponse([
        'event: connection\ndata: connected\n\n',
        'event: agui\ndata: {"type":"STEP_STARTED","step_name":"prompt_assembly"}\n\n',
        'event: agui\ndata: {"type":"TOOL_CALL_START","tool_name":"get_activities"}\n\n',
        'event: agui\ndata: {"type":"RUN_FINISHED"}\n\n',
      ]),
    );

    const { result } = renderHook(() => useAgUiProgress('run-1', 'jwt'));

    await waitFor(() =>
      expect(result.current.statusText).toBe('calling get_activities…'),
    );
    await waitFor(() => expect(result.current.isActive).toBe(false));
  });

  it('sends the Bearer token to the run stream URL', async () => {
    const fetchSpy = vi
      .spyOn(globalThis, 'fetch')
      .mockResolvedValue(sseResponse(['event: agui\ndata: {"type":"RUN_FINISHED"}\n\n']));

    renderHook(() => useAgUiProgress('run-abc', 'jwt-xyz'));

    await waitFor(() => expect(fetchSpy).toHaveBeenCalled());
    // Find this run's call explicitly — a prior test's hook can leave an
    // unaborted fetch in the spy's history, so calls[0] is not reliably ours.
    const call = fetchSpy.mock.calls.find(
      ([url]) => url === '/api/agui/runs/run-abc/stream',
    );
    expect(call).toBeDefined();
    const init = call?.[1];
    expect((init?.headers as Record<string, string>).Authorization).toBe('Bearer jwt-xyz');
  });

  it('collapses silently when the response is not ok', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue({
      ok: false,
      body: null,
    } as unknown as Response);

    const { result } = renderHook(() => useAgUiProgress('run-1', 'jwt'));

    await waitFor(() => expect(result.current.isActive).toBe(false));
    expect(result.current.statusText).toBeNull();
  });
});
