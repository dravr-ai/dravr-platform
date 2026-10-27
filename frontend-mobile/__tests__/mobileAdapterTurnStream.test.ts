// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins that the phone's adapter reads a chat turn as it streams — each frame on arrival, not the body at the end
// ABOUTME: Drives @pierre/api-client's real mobile adapter and sendTurn over a body stream the test holds open

import type { TurnEnvelope, TurnProgress } from '@pierre/shared-types';
import {
  TurnIdleAbortedError,
  createMobileAdapter,
  createPierreApi,
  type AsyncStorageLike,
} from '@pierre/api-client';

const CONVERSATION_ID = 'conv-mobile-stream';
const BASE_URL = 'http://pierre.test';

function memoryStorage(): AsyncStorageLike {
  const store = new Map<string, string>();
  return {
    getItem: async (key) => store.get(key) ?? null,
    setItem: async (key, value) => {
      store.set(key, value);
    },
    removeItem: async (key) => {
      store.delete(key);
    },
    multiRemove: async (keys) => {
      keys.forEach((key) => store.delete(key));
    },
  };
}

function mobileAdapter() {
  return createMobileAdapter({ asyncStorage: memoryStorage(), baseURL: BASE_URL });
}

/** The body stream `expo/fetch` hands back: the Web Streams global Expo's runtime installs. */
type BodyStream = NonNullable<Response['body']>;

/** A body the test writes chunk by chunk, as `expo/fetch` delivers a turn. */
function heldStream() {
  const encoder = new TextEncoder();
  const sink: {
    enqueue: (chunk: Uint8Array<ArrayBuffer>) => void;
    close: () => void;
    error: (reason: unknown) => void;
  } = { enqueue: () => undefined, close: () => undefined, error: () => undefined };
  const stream: BodyStream = new globalThis.ReadableStream<Uint8Array<ArrayBuffer>>({
    start(controller) {
      sink.enqueue = chunk => controller.enqueue(chunk);
      sink.close = () => controller.close();
      sink.error = reason => controller.error(reason);
    },
  });
  return {
    stream,
    write: (text: string) => sink.enqueue(encoder.encode(text)),
    writeBytes: (bytes: Uint8Array<ArrayBuffer>) => sink.enqueue(bytes),
    close: () => sink.close(),
    fail: (reason: unknown) => sink.error(reason),
  };
}

/** What `expo/fetch` answers: a response whose body is a stream. */
function streamingResponse(stream: BodyStream): Response {
  return new Response(stream, { status: 200, headers: { 'Content-Type': 'text/event-stream' } });
}

function frame(event: string, data: unknown): string {
  return `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`;
}

/** Let every pending read and callback settle. */
async function settle(): Promise<void> {
  for (let i = 0; i < 10; i += 1) {
    await Promise.resolve();
  }
  await new Promise((resolve) => setTimeout(resolve, 0));
}

function envelope(): TurnEnvelope {
  return {
    turn_id: 'turn-1',
    user_message: {
      id: 'msg-user-1',
      conversation_id: CONVERSATION_ID,
      role: 'user',
      content: 'Ma semaine ?',
      created_at: '2026-10-26T08:00:00Z',
    },
    assistant: {
      message: {
        id: 'msg-assistant-1',
        conversation_id: CONVERSATION_ID,
        role: 'assistant',
        content: 'Bonne semaine.',
        created_at: '2026-10-26T08:00:40Z',
      },
      blocks: [{ type: 'prose', text: 'Bonne semaine.' }],
      finish_reason: 'stop',
    },
    conversation_updated_at: '2026-10-26T08:00:40Z',
    telemetry: {
      model: 'claude-sonnet-4-6',
      provider_name: 'claude_code',
      tool_calls_count: 1,
      tools_called: ['get_activities'],
      execution_time_ms: 40_000,
    },
  };
}

const TOOL_STARTED: TurnProgress = { kind: 'tool', id: 'call-1', title: 'get_activities', status: 'InProgress' };

describe('the mobile adapter reads a turn as it streams', () => {
  const realFetch = globalThis.fetch;

  afterEach(() => {
    globalThis.fetch = realFetch;
  });

  it('hands on each chunk the moment it lands, before the body ends', async () => {
    const held = heldStream();
    globalThis.fetch = jest.fn(async () => streamingResponse(held.stream)) as typeof fetch;
    const api = createPierreApi(mobileAdapter());

    const deltas: string[] = [];
    const onDone = jest.fn();
    const sent = api.chat.sendTurn(CONVERSATION_ID, 'Ma semaine ?', {
      onDelta: (delta) => deltas.push(delta),
      onDone,
    });

    held.write(frame('delta', { delta: 'Je regarde ta semaine' }));
    await settle();
    // The body is still open, and its first chunk has already been read.
    expect(deltas).toEqual(['Je regarde ta semaine']);
    expect(onDone).not.toHaveBeenCalled();

    held.write(frame('done', envelope()));
    held.close();
    await sent;
    expect(deltas).toEqual(['Je regarde ta semaine']);
    expect(onDone).toHaveBeenCalledTimes(1);
  });

  it('keeps a multi-byte character whole when the network splits it across chunks', async () => {
    const held = heldStream();
    globalThis.fetch = jest.fn(async () => streamingResponse(held.stream)) as typeof fetch;
    const api = createPierreApi(mobileAdapter());

    const deltas: string[] = [];
    const onError = jest.fn();
    const sent = api.chat.sendTurn(CONVERSATION_ID, 'Ma semaine ?', {
      onDelta: (delta) => deltas.push(delta),
      onError,
    });

    const bytes = new TextEncoder().encode(frame('delta', { delta: 'réflexion' }));
    const split = bytes.indexOf(0xc3) + 1; // between the two bytes of `é`
    held.writeBytes(bytes.slice(0, split));
    await settle();
    held.writeBytes(bytes.slice(split));
    held.write(frame('done', envelope()));
    held.close();
    await sent;
    expect(onError).not.toHaveBeenCalled();
    expect(deltas).toEqual(['réflexion']);
  });

  it('reads a body with no stream whole — React Native\'s own fetch, if a build opts back into it', async () => {
    const whole = {
      ok: true,
      status: 200,
      body: null,
      text: async () => frame('progress', TOOL_STARTED) + frame('done', envelope()),
    } as unknown as Response;
    globalThis.fetch = jest.fn(async () => whole) as typeof fetch;
    const api = createPierreApi(mobileAdapter());

    const steps: TurnProgress[] = [];
    let done: TurnEnvelope | null = null;
    await api.chat.sendTurn(CONVERSATION_ID, 'Ma semaine ?', {
      onProgress: (progress) => steps.push(progress),
      onDone: (turn) => {
        done = turn;
      },
    });

    expect(steps).toEqual([TOOL_STARTED]);
    expect(done).toEqual(envelope());
  });

  it('fires a step on the phone while the turn still runs, and the reply only at its end', async () => {
    const held = heldStream();
    globalThis.fetch = jest.fn(async () => streamingResponse(held.stream)) as typeof fetch;
    const api = createPierreApi(mobileAdapter());

    const steps: TurnProgress[] = [];
    let done: TurnEnvelope | null = null;
    const sent = api.chat.sendTurn(CONVERSATION_ID, 'Ma semaine ?', {
      onProgress: (progress) => steps.push(progress),
      onDone: (turn) => {
        done = turn;
      },
    });

    held.write(frame('progress', TOOL_STARTED));
    await settle();
    expect(steps).toEqual([TOOL_STARTED]);
    expect(done).toBeNull();

    held.write(frame('done', envelope()));
    held.close();
    await sent;
    expect(done).not.toBeNull();
    expect(steps).toHaveLength(1);
  });

  it('ends a turn the idle watch aborts mid-stream as an idle abort, not a failure', async () => {
    const held = heldStream();
    const idle = new AbortController();
    // `expo/fetch` errors the body stream when the request's signal aborts.
    globalThis.fetch = jest.fn(async (...[, init]: Parameters<typeof fetch>) => {
      init?.signal?.addEventListener('abort', () => held.fail(new Error('The operation was aborted.')));
      return streamingResponse(held.stream);
    }) as typeof fetch;
    const api = createPierreApi(mobileAdapter());

    const steps: TurnProgress[] = [];
    const onError = jest.fn();
    const onDone = jest.fn();
    const sent = api.chat.sendTurn(CONVERSATION_ID, 'Ma semaine ?', {
      signal: idle.signal,
      onProgress: (progress) => steps.push(progress),
      onError,
      onDone,
    });

    held.write(frame('progress', TOOL_STARTED));
    await settle();
    expect(steps).toEqual([TOOL_STARTED]);

    idle.abort();
    await sent;
    expect(onDone).not.toHaveBeenCalled();
    expect(onError).toHaveBeenCalledTimes(1);
    expect(onError.mock.calls[0][0]).toBeInstanceOf(TurnIdleAbortedError);
  });
});
