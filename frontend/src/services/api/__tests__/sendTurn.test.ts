// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit cover for sendTurn itself — the request it builds and the callbacks it drives, in order
// ABOUTME: Runs the real web adapter's progressive body reader over a stubbed fetch, no component tree

import { describe, it, expect, vi, afterEach } from 'vitest';
import type { ReplyBlock, TurnEnvelope, TurnProgress } from '@pierre/shared-types';
import { TurnRequestError } from '@pierre/api-client';

import { chatApi, pierreApi } from '../index';

const CONVERSATION_ID = 'conv-send-1';
const MESSAGES_PATH = `/api/chat/conversations/${CONVERSATION_ID}/messages`;

const ACTIVITY_TEXT = '1. Sortie longue - 24 km';

function envelope(): TurnEnvelope {
  return {
    turn_id: 'turn-send-1',
    user_message: {
      id: 'msg-user-1',
      conversation_id: CONVERSATION_ID,
      role: 'user',
      content: 'How did the block go?',
      created_at: '2026-08-24T10:00:00Z',
    },
    assistant: {
      message: {
        id: 'msg-assistant-1',
        conversation_id: CONVERSATION_ID,
        role: 'assistant',
        content: 'Solid block.',
        created_at: '2026-08-24T10:00:04Z',
      },
      blocks: [
        { type: 'prose', text: 'Solid block.' },
        { type: 'activity_list', text: ACTIVITY_TEXT },
        {
          type: 'actions',
          title: 'Your coaches',
          actions: [{ label: 'Coach Tempo', action_type: 'postback', value: '/coach select x' }],
        },
      ],
      finish_reason: 'stop',
    },
    conversation_updated_at: '2026-08-24T10:00:04Z',
    telemetry: {
      model: 'gpt-5-codex',
      provider_name: 'copilot',
      tool_calls_count: 1,
      tools_called: ['get_activities'],
      execution_time_ms: 1200,
    },
  };
}

/** A `Response` whose body streams the given text in several chunks. */
function streamedResponse(body: string, status = 200): Response {
  const encoder = new TextEncoder();
  const chunkSize = 24;
  const stream = new ReadableStream<Uint8Array>({
    start(controller) {
      for (let at = 0; at < body.length; at += chunkSize) {
        controller.enqueue(encoder.encode(body.slice(at, at + chunkSize)));
      }
      controller.close();
    },
  });
  return new Response(stream, { status, headers: { 'Content-Type': 'text/event-stream' } });
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('sendTurn — the one request every surface sends', () => {
  it('drives progress, deltas, blocks and the finished turn, in that order', async () => {
    const turn = envelope();
    const body = [
      `event: progress\ndata: ${JSON.stringify({
        kind: 'stage',
        id: 'dispatch',
        title: 'dispatch',
        status: 'started',
      })}\n\n`,
      `event: progress\ndata: ${JSON.stringify({
        kind: 'tool',
        id: 'call-1',
        title: 'get_activities',
        status: 'InProgress',
      })}\n\n`,
      `event: delta\ndata: ${JSON.stringify({ delta: 'Solid ' })}\n\n`,
      `event: delta\ndata: ${JSON.stringify({ delta: 'block.' })}\n\n`,
      ':keepalive\n\n',
      ...turn.assistant.blocks.map(
        (block) => `event: block\ndata: ${JSON.stringify(block)}\n\n`,
      ),
      `event: done\ndata: ${JSON.stringify(turn)}\n\n`,
    ].join('');

    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(streamedResponse(body));

    const order: string[] = [];
    const deltas: string[] = [];
    const blocks: ReplyBlock[] = [];
    const progress: TurnProgress[] = [];
    let done: TurnEnvelope | null = null;
    const onError = vi.fn();

    await chatApi.sendTurn(CONVERSATION_ID, 'How did the block go?', {
      onProgress: (p) => {
        order.push('progress');
        progress.push(p);
      },
      onDelta: (d) => {
        order.push('delta');
        deltas.push(d);
      },
      onBlock: (b) => {
        order.push('block');
        blocks.push(b);
      },
      onDone: (t) => {
        order.push('done');
        done = t;
      },
      onError,
    });

    // Live events land while the body is still arriving; the block walk and
    // the finished turn come after it. A stub that reported nothing fails on
    // every one of these.
    expect(order).toEqual([
      'progress',
      'progress',
      'delta',
      'delta',
      'block',
      'block',
      'block',
      'done',
    ]);
    expect(progress).toEqual([
      { kind: 'stage', id: 'dispatch', title: 'dispatch', status: 'started' },
      { kind: 'tool', id: 'call-1', title: 'get_activities', status: 'InProgress' },
    ]);
    expect(deltas.join('')).toBe('Solid block.');
    expect(blocks.map((b) => b.type)).toEqual(['prose', 'activity_list', 'actions']);
    expect(blocks[1]).toEqual({ type: 'activity_list', text: ACTIVITY_TEXT });
    expect(done).toEqual(turn);
    expect(onError).not.toHaveBeenCalled();

    // The request itself: one POST, to the shared endpoint, through the
    // configured base URL, naming the client surface.
    expect(fetchSpy).toHaveBeenCalledTimes(1);
    const [url, init] = fetchSpy.mock.calls[0];
    expect(url).toBe(`${pierreApi.axios.defaults.baseURL ?? ''}${MESSAGES_PATH}`);
    expect(init?.method).toBe('POST');
    const headers = init?.headers as Record<string, string>;
    expect(headers['X-Client-Platform']).toBe('web');
    expect(headers.Accept).toBe('text/event-stream, application/json');
    expect(init?.credentials).toBe('include');
    expect(JSON.parse(String(init?.body))).toEqual({ content: 'How did the block go?' });
  });

  it('reports a refusal through onError, carrying the status and body for formatting', async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response(
        JSON.stringify({
          message: 'Daily message limit reached.',
          details: { limit_type: 'daily_messages', current: 50, limit: 50 },
        }),
        { status: 429, headers: { 'Content-Type': 'application/json' } },
      ),
    );

    const onDone = vi.fn();
    let failure: Error | null = null;
    await chatApi.sendTurn(CONVERSATION_ID, 'Another one?', {
      onDone,
      onError: (error) => {
        failure = error;
      },
    });

    expect(onDone).not.toHaveBeenCalled();
    const refusal = failure as unknown as TurnRequestError;
    expect(refusal).toBeInstanceOf(TurnRequestError);
    expect(refusal.message).toBe('Daily message limit reached.');
    expect(refusal.status).toBe(429);
    // The structured details survive, so a caller can name the limit it hit.
    expect((refusal.body as { details: { limit_type: string } }).details.limit_type).toBe(
      'daily_messages',
    );
  });

  it('reads a slash-command answer, which is one JSON document and no stream', async () => {
    const turn = envelope();
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      new Response(JSON.stringify(turn), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }),
    );

    const onDelta = vi.fn();
    const blocks: ReplyBlock[] = [];
    let done: TurnEnvelope | null = null;

    await chatApi.sendTurn(CONVERSATION_ID, '/coach', {
      onDelta,
      onBlock: (b) => blocks.push(b),
      onDone: (t) => {
        done = t;
      },
    });

    expect(onDelta).not.toHaveBeenCalled();
    expect(blocks.map((b) => b.type)).toEqual(['prose', 'activity_list', 'actions']);
    expect(done).toEqual(turn);
  });
});
