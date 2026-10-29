// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit cover for the body sendTurn posts — the message, its origin and the room entry a coach quotes (carnet#1249)
// ABOUTME: Runs the real chat domain over a stubbed fetch and a stub adapter, so web and mobile share what is pinned here

import { afterEach, describe, expect, it, vi } from 'vitest';
import axios from 'axios';
import type { SendMessageRequest, TurnEnvelope } from '@pierre/shared-types';
import { createChatApi } from '../src/domains/chat';
import { TurnRequestError } from '../src/core/turn-stream';
import type { PlatformAdapter } from '../src/types/platform';

const BASE_URL = 'https://api.test';
const THREAD_ID = 'conv-work-alice';
const ENTRY_ID = '8f0c6a2e-4c1b-4d7e-9a39-2b5f1f1d9e10';

/**
 * A mobile-shaped adapter: a bearer token, no cookie jar. Only what a turn
 * reads is real; the sign-out path is spied on, since a quote refusal must
 * never reach it.
 */
function stubAdapter() {
  const onAuthFailure = vi.fn();
  const adapter: PlatformAdapter = {
    httpConfig: { baseURL: BASE_URL },
    authStorage: {
      getToken: async () => 'token-coach',
      setToken: async () => undefined,
      removeToken: async () => undefined,
      getCsrfToken: async () => null,
      setCsrfToken: async () => undefined,
      getUser: async () => null,
      setUser: async () => undefined,
      getRefreshToken: async () => null,
      setRefreshToken: async () => undefined,
      clear: async () => undefined,
    },
    authFailure: { onAuthFailure, subscribe: () => () => undefined },
    turnCredentials: 'omit',
    platform: 'mobile',
  };
  return { adapter, onAuthFailure };
}

function chatApi() {
  const { adapter, onAuthFailure } = stubAdapter();
  return { api: createChatApi(axios.create({ baseURL: BASE_URL }), adapter), onAuthFailure };
}

/** The turn a coach's quoted question comes back as: the server's copy of the athlete's words rides the question. */
function envelope(): TurnEnvelope {
  return {
    turn_id: 'turn-quote-1',
    user_message: {
      id: 'msg-coach-1',
      conversation_id: THREAD_ID,
      role: 'user',
      content: 'Move Thursday to Saturday',
      created_at: '2026-10-28T20:40:00Z',
      quote: {
        entry_id: ENTRY_ID,
        author_user_id: 'user-alice',
        author_display_name: 'Alice Runner',
        content: 'Legs felt heavy this morning.',
        created_at: '2026-10-28T07:12:00+00:00',
      },
    },
    assistant: {
      message: {
        id: 'msg-agent-1',
        conversation_id: THREAD_ID,
        role: 'assistant',
        content: 'Done. Two days change.',
        created_at: '2026-10-28T20:40:05Z',
      },
      blocks: [{ type: 'prose', text: 'Done. Two days change.' }],
      finish_reason: 'stop',
    },
    conversation_updated_at: '2026-10-28T20:40:05Z',
    telemetry: {
      model: 'gpt-5-codex',
      provider_name: 'copilot',
      tool_calls_count: 0,
      tools_called: [],
      execution_time_ms: 900,
    },
  };
}

function json(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), { status, headers: { 'Content-Type': 'application/json' } });
}

/** The body the stubbed fetch was handed, decoded. */
function postedBody(fetchSpy: ReturnType<typeof vi.spyOn>): SendMessageRequest {
  const init = fetchSpy.mock.calls[0]?.[1] as RequestInit | undefined;
  return JSON.parse(String(init?.body)) as SendMessageRequest;
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe('sendTurn — the body it posts', () => {
  it('names the quoted room entry as quote_entry_id, beside the message and its origin', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(json(envelope()));
    const { api } = chatApi();
    const onDone = vi.fn();

    await api.sendTurn(THREAD_ID, 'Move Thursday to Saturday', {
      origin: 'typed',
      quoteEntryId: ENTRY_ID,
      onDone,
    });

    expect(fetchSpy.mock.calls[0]?.[0]).toBe(`${BASE_URL}/api/chat/conversations/${THREAD_ID}/messages`);
    expect(postedBody(fetchSpy)).toEqual({
      content: 'Move Thursday to Saturday',
      origin: 'typed',
      quote_entry_id: ENTRY_ID,
    });
    // The quote the server copied from the room comes back on the question.
    const turn = onDone.mock.calls[0]?.[0] as TurnEnvelope;
    expect(turn.user_message.quote?.entry_id).toBe(ENTRY_ID);
    expect(turn.user_message.quote?.content).toBe('Legs felt heavy this morning.');
  });

  it('posts the message alone when nothing is quoted and no origin is named', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(json(envelope()));
    const { api } = chatApi();

    await api.sendTurn(THREAD_ID, 'How is Alice doing?', {});

    expect(postedBody(fetchSpy)).toEqual({ content: 'How is Alice doing?' });
  });

  it('carries the quote without an origin', async () => {
    const fetchSpy = vi.spyOn(globalThis, 'fetch').mockResolvedValue(json(envelope()));
    const { api } = chatApi();

    await api.sendTurn(THREAD_ID, 'Ease Sunday off', { quoteEntryId: ENTRY_ID });

    expect(postedBody(fetchSpy)).toEqual({ content: 'Ease Sunday off', quote_entry_id: ENTRY_ID });
  });

  it("hands a refused quote to onError with the server's reason, and signs no one out", async () => {
    vi.spyOn(globalThis, 'fetch').mockResolvedValue(
      json(
        {
          code: 'PermissionDenied',
          message: 'That message is not shared with you',
          details: { reason: 'quote_withheld' },
          timestamp: '2026-10-28T20:40:00Z',
        },
        403,
      ),
    );
    const { api, onAuthFailure } = chatApi();
    const onError = vi.fn();
    const onDone = vi.fn();

    await api.sendTurn(THREAD_ID, 'Move Thursday to Saturday', { quoteEntryId: ENTRY_ID, onError, onDone });

    expect(onDone).not.toHaveBeenCalled();
    const error = onError.mock.calls[0]?.[0];
    expect(error).toBeInstanceOf(TurnRequestError);
    expect((error as TurnRequestError).status).toBe(403);
    expect((error as TurnRequestError).response.data).toMatchObject({ details: { reason: 'quote_withheld' } });
    expect(onAuthFailure).not.toHaveBeenCalled();
  });
});
