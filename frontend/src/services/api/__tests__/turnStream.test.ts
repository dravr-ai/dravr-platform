// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit cover for the ONE turn parser both clients read a chat reply through
// ABOUTME: Frames, the single-JSON slash-command answer, keep-alives and error frames, in one reader

import { describe, it, expect, vi } from 'vitest';
import { parseTurnBody, readEventStream } from '@pierre/api-client';
import type { TurnEnvelope } from '@pierre/shared-types';

const CONVERSATION_ID = 'conv-parser-1';

/** The finished turn the server puts in the `done` frame. */
function envelope(overrides: { content?: string; finishReason?: string } = {}): TurnEnvelope {
  const { content = 'Ta charge monte depuis trois semaines.', finishReason = 'stop' } = overrides;
  return {
    turn_id: 'turn-parser-1',
    user_message: {
      id: 'msg-user-1',
      conversation_id: CONVERSATION_ID,
      role: 'user',
      content: 'Comment se presente ma semaine ?',
      created_at: '2026-08-24T10:00:00Z',
    },
    assistant: {
      message: {
        id: 'msg-assistant-1',
        conversation_id: CONVERSATION_ID,
        role: 'assistant',
        content,
        created_at: '2026-08-24T10:00:04Z',
      },
      blocks: [
        { type: 'prose', text: content },
        { type: 'activity_list', text: '1. Sortie longue - 24 km' },
      ],
      finish_reason: finishReason,
    },
    conversation_updated_at: '2026-08-24T10:00:04Z',
    telemetry: {
      model: 'gpt-5-codex',
      provider_name: 'copilot',
      tool_calls_count: 1,
      tools_called: ['get_activities'],
      execution_time_ms: 4210,
    },
  };
}

/** Feed a body to the parser one arbitrary slice at a time. */
async function* slices(body: string, sizes: number[]): AsyncGenerator<string> {
  let at = 0;
  for (const size of sizes) {
    if (at >= body.length) return;
    yield body.slice(at, at + size);
    at += size;
  }
  if (at < body.length) yield body.slice(at);
}

/** The whole body in one piece — how React Native's fetch hands it over. */
async function* whole(body: string): AsyncGenerator<string> {
  yield body;
}

const DELTAS = ['Ta charge ', 'monte depuis ', 'trois semaines.'];

function streamedTurn(turn: TurnEnvelope): string {
  return [
    ...DELTAS.map((delta) => `event: delta\ndata: ${JSON.stringify({ delta })}\n\n`),
    `event: progress\ndata: ${JSON.stringify({
      kind: 'tool',
      id: 'call-1',
      title: 'get_activities',
      status: 'Completed',
    })}\n\n`,
    ':keepalive\n\n',
    ...turn.assistant.blocks.map((block) => `event: block\ndata: ${JSON.stringify(block)}\n\n`),
    `event: done\ndata: ${JSON.stringify(turn)}\n\n`,
  ].join('');
}

describe('parseTurnBody — the streaming shape', () => {
  it('yields every delta in order and the terminal envelope', async () => {
    const turn = envelope();
    const onDelta = vi.fn();
    const onProgress = vi.fn();
    const onBlock = vi.fn();

    const parsed = await parseTurnBody(slices(streamedTurn(turn), [7, 40, 3, 200, 9]), {
      onDelta,
      onProgress,
      onBlock,
    });

    // The delta sequence, concretely — a parser that yielded nothing fails here.
    expect(onDelta.mock.calls.map((call) => call[0])).toEqual(DELTAS);
    expect(onDelta.mock.calls.map((call) => call[0]).join('')).toBe(
      'Ta charge monte depuis trois semaines.'
    );
    // The tool-call observation, decoded, and the keep-alive comment ignored.
    expect(onProgress).toHaveBeenCalledTimes(1);
    expect(onProgress).toHaveBeenCalledWith({
      kind: 'tool',
      id: 'call-1',
      title: 'get_activities',
      status: 'Completed',
    });
    // Each block arrived as its own frame, once, in the server's order — a
    // reader that also re-walked the envelope's list would draw them twice.
    expect(onBlock).toHaveBeenCalledTimes(turn.assistant.blocks.length);
    expect(onBlock.mock.calls.map((call) => call[0])).toEqual(turn.assistant.blocks);
    // The terminal envelope, whole.
    expect(parsed.turn_id).toBe('turn-parser-1');
    expect(parsed.assistant.message.content).toBe('Ta charge monte depuis trois semaines.');
    expect(parsed.assistant.blocks).toHaveLength(2);
    expect(parsed.telemetry.model).toBe('gpt-5-codex');
  });

  it('reads the same body handed over in one piece, as React Native hands it over', async () => {
    const turn = envelope();
    const onDelta = vi.fn();

    const parsed = await parseTurnBody(whole(streamedTurn(turn)), { onDelta });

    expect(onDelta.mock.calls.map((call) => call[0])).toEqual(DELTAS);
    expect(parsed).toEqual(turn);
  });

  it('splits frames on CRLF blank lines too', async () => {
    const turn = envelope();
    const body = `event: delta\r\ndata: ${JSON.stringify({
      delta: 'Bonjour',
    })}\r\n\r\nevent: done\r\ndata: ${JSON.stringify(turn)}\r\n\r\n`;
    const onDelta = vi.fn();

    const parsed = await parseTurnBody(whole(body), { onDelta });

    expect(onDelta.mock.calls.map((call) => call[0])).toEqual(['Bonjour']);
    expect(parsed.turn_id).toBe('turn-parser-1');
  });
});

describe('parseTurnBody — the single-JSON shape', () => {
  it('reads a slash-command answer that never became an event stream', async () => {
    // A command is dispatched before the streaming branch is chosen, so the
    // whole body is the envelope. No content-type is consulted anywhere.
    const command = envelope({ content: 'Coachs disponibles', finishReason: 'command' });
    const onDelta = vi.fn();
    const onProgress = vi.fn();
    const onBlock = vi.fn();

    const parsed = await parseTurnBody(slices(JSON.stringify(command), [12, 60, 5]), {
      onDelta,
      onProgress,
      onBlock,
    });

    expect(parsed).toEqual(command);
    expect(parsed.assistant.finish_reason).toBe('command');
    expect(parsed.assistant.blocks[0]).toEqual({ type: 'prose', text: 'Coachs disponibles' });
    // Nothing on the live channel: a JSON document carries no frames.
    expect(onDelta).not.toHaveBeenCalled();
    expect(onProgress).not.toHaveBeenCalled();
    // The blocks still reach the caller, walked off the envelope — the same
    // callback, the same order, whichever shape the body arrived in.
    expect(onBlock.mock.calls.map((call) => call[0])).toEqual(command.assistant.blocks);
  });
});

describe('parseTurnBody — the failures a turn can end in', () => {
  it('throws the server message carried by a failed frame', async () => {
    const body = `event: delta\ndata: ${JSON.stringify({
      delta: 'Je regarde',
    })}\n\nevent: failed\ndata: ${JSON.stringify({
      error: 'Daily message limit reached.',
    })}\n\n`;

    await expect(parseTurnBody(whole(body))).rejects.toThrow('Daily message limit reached.');
  });

  it('throws when the body ended without a reply', async () => {
    const body = `event: delta\ndata: ${JSON.stringify({ delta: 'Je regarde' })}\n\n`;

    await expect(parseTurnBody(whole(body))).rejects.toThrow('The turn ended without a reply.');
  });

  it('throws when the done frame carried an unreadable payload', async () => {
    await expect(parseTurnBody(whole('event: done\ndata: {not json}\n\n'))).rejects.toThrow(
      'The turn finished with an unreadable payload.'
    );
  });
});

describe('readEventStream — the shared frame reader', () => {
  it('stops reading as soon as a handler reports a terminal frame', async () => {
    const seen: string[] = [];
    const body = ['event: progress\ndata: {"kind":"stage","title":"dispatch"}\n\n',
      'event: keepalive\ndata: ping\n\n',
      'event: done\ndata: {"turn_id":"t-1"}\n\n',
      'event: block\ndata: {"type":"NEVER_READ"}\n\n'].join('');

    await readEventStream(whole(body), (frame) => {
      seen.push(frame.data);
      return frame.event === 'done';
    });

    expect(seen).toEqual([
      '{"kind":"stage","title":"dispatch"}',
      'ping',
      '{"turn_id":"t-1"}',
    ]);
  });

  it('hands a non-stream body to the document callback and emits no frames', async () => {
    const frames: string[] = [];
    let document: string | null = null;

    await readEventStream(
      slices('{"turn_id":"t-1"}', [4, 4, 4, 40]),
      (frame) => {
        frames.push(frame.event);
      },
      (body) => {
        document = body;
      }
    );

    expect(frames).toEqual([]);
    expect(document).toBe('{"turn_id":"t-1"}');
  });
});
