// ABOUTME: PHASE 2 e2e — the mobile client reads the turn's JSON body and nothing else
// ABOUTME: X-Usage-* headers, is_command_response and card_title left the wire; nothing may depend on them

import React from 'react';
import { readFileSync, readdirSync } from 'fs';
import { join } from 'path';
import { renderHook, act, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { Message } from '@pierre/shared-types';

import { installHttpStub, type HttpStub } from './helpers/httpStub';
import { CONVERSATION_ID, PROSE_OPENING, assistantTurn } from './helpers/chatFixtures';

import { useMessages } from '../../src/screens/chat/useMessages';
import { useUsageStatus } from '../../src/screens/chat/useUsageStatus';

const MESSAGES_URL = `/api/chat/conversations/${CONVERSATION_ID}/messages`;

/**
 * Headers the server stopped sending, at values that would be impossible to
 * ignore if anything still read them: quota exhausted, send blocked.
 */
const DEAD_USAGE_HEADERS = {
  'x-usage-percent': '100',
  'x-usage-remaining': '0',
  'x-usage-limit': '100',
  'x-usage-reset': '2026-08-23T00:00:00Z',
};

/** Fields the TurnEnvelope work removed from the response body. */
const DEAD_BODY_FIELDS = { is_command_response: true, card_title: 'Coaches' };

function usageStatus(current: number) {
  const counter = {
    allowed: true,
    current,
    limit: 100,
    warning: current >= 80,
    burst_zone: false,
    resets_at: '2026-08-23T00:00:00Z',
  };
  const idle = { ...counter, current: 0, warning: false };
  return {
    daily: { messages: counter, tokens: idle, tool_calls: idle },
    weekly: { messages: idle, tokens: idle, tool_calls: idle },
    resources: { conversations: 3, max_conversations: 50, coaches: 2, max_coaches: 10 },
  };
}

/**
 * A query client per test, cleared afterwards.
 *
 * Its cache keeps a garbage-collection timer alive; clearing it is what lets
 * the jest worker exit instead of idling until the timer fires.
 */
let queryClient: QueryClient | null = null;

function queryWrapper() {
  queryClient = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  const client = queryClient;
  return function Wrapper({ children }: { children: React.ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  };
}

/** Every source file the mobile bundle compiles, for the dependency scan. */
function sourceFiles(root: string): string[] {
  const found: string[] = [];
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    const full = join(root, entry.name);
    if (entry.isDirectory()) {
      found.push(...sourceFiles(full));
    } else if (/\.(ts|tsx)$/.test(entry.name)) {
      found.push(full);
    }
  }
  return found;
}

describe('PHASE 2 — the mobile client does not read the removed wire fields', () => {
  let stub: HttpStub | null = null;

  afterEach(() => {
    stub?.restore();
    stub = null;
    queryClient?.clear();
    queryClient = null;
  });

  it('renders a turn identically with and without X-Usage-* headers and is_command_response', async () => {
    const send = async () => {
      const { result } = renderHook(() => useMessages(), { wrapper: queryWrapper() });
      await act(async () => {
        await result.current.sendTurn(CONVERSATION_ID, 'Comment se presente ma semaine ?');
      });
      return result.current.messages.find((message) => message.role === 'assistant') as Message;
    };

    stub = installHttpStub({ [`POST ${MESSAGES_URL}`]: { data: assistantTurn() } });
    const bare = await send();
    stub.restore();

    stub = installHttpStub({
      [`POST ${MESSAGES_URL}`]: {
        data: { ...assistantTurn(), ...DEAD_BODY_FIELDS },
        headers: DEAD_USAGE_HEADERS,
      },
    });
    const withDeadFields = await send();

    // Concrete on both sides, so an all-empty render cannot pass this.
    expect(bare.content).toBe(PROSE_OPENING);
    expect(bare.model).toBe('claude-sonnet-4-6');
    expect(bare.execution_time_ms).toBe(4210);
    // Byte-identical: the removed header block and body fields changed nothing.
    expect(withDeadFields).toEqual(bare);
  });

  it('computes the quota banner from /api/usage/status, not from a turn response header', async () => {
    stub = installHttpStub({
      'GET /api/usage/status': { data: usageStatus(92) },
      // A turn whose headers claim the athlete is out of messages entirely.
      [`POST ${MESSAGES_URL}`]: { data: assistantTurn(), headers: DEAD_USAGE_HEADERS },
    });

    const chat = renderHook(() => useMessages(), { wrapper: queryWrapper() });
    await act(async () => {
      await chat.result.current.sendTurn(CONVERSATION_ID, 'Comment se presente ma semaine ?');
    });

    const usage = renderHook(() => useUsageStatus(), { wrapper: queryWrapper() });
    await waitFor(() => expect(usage.result.current.isLoading).toBe(false));

    // 92/100 from the JSON body — a warning, not the block the header claimed.
    expect(usage.result.current.level).toBe('warning');
    expect(usage.result.current.sendDisabled).toBe(false);
    expect(usage.result.current.message).toContain(
      "You've used 92% of your daily messages (92/100)"
    );
    expect(stub.requestsFor('GET').map((request) => request.url)).toEqual(['/api/usage/status']);
  });

  it('leaves no reference to the removed wire names anywhere the app compiles', () => {
    const roots = [
      join(__dirname, '../../src'),
      join(__dirname, '../../../packages/api-client/src'),
    ];
    const files = roots.flatMap(sourceFiles);

    // Proves the walk found the real tree rather than an empty directory.
    expect(files.length).toBeGreaterThan(120);

    const offenders: string[] = [];
    for (const file of files) {
      const contents = readFileSync(file, 'utf8').toLowerCase();
      for (const name of ['x-usage-', 'is_command_response', 'card_title']) {
        if (contents.includes(name)) {
          offenders.push(`${file}: ${name}`);
        }
      }
    }
    expect(offenders).toEqual([]);
  });
});
