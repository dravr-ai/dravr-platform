// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The activity-to-conversation link both views read — the server's detail names it, the view links a new thread there
// ABOUTME: Red if a thread is not written to the server, a refused link stays shown, or a read racing the link drops it

import { describe, it, expect, vi } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider, useQuery } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { ActivityDetailResponse } from '@pierre/shared-types';
import { createActivityConversationHook } from '../src/activityConversation';
import { createActivityDetailHook } from '../src/activityDetailHook';

function detail(conversationId: string | null): ActivityDetailResponse {
  return {
    activity: {
      id: 'act-4',
      provider: 'strava',
      name: 'Tempo 10k',
      sport_type: 'run',
      start_date: '2026-09-29T10:00:00Z',
      duration_seconds: 2_700,
      distance_meters: 10_000,
      elevation_gain_meters: null,
      has_gps: true,
      summary_polyline: null,
      attribution: null,
    },
    average_heart_rate: null,
    max_heart_rate: null,
    average_speed_mps: null,
    max_speed_mps: null,
    average_power: null,
    calories: null,
    splits: [],
    laps: [],
    conversation_id: conversationId,
  };
}

/**
 * The hook as a view drives it: the detail comes from the query cache, as
 * `useActivityDetail` reads it, and the link goes to a recorded API.
 */
function setup(server: ActivityDetailResponse) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  client.setQueryData(QUERY_KEYS.home.activityDetail('strava', 'act-4'), server);
  // The server as the view sees it: the link it takes is what it answers next.
  let held = server;
  const linkActivityConversation = vi.fn(
    async (_provider: string, _id: string, conversationId: string | null): Promise<void> => {
      held = { ...held, conversation_id: conversationId };
    },
  );
  const useActivityConversation = createActivityConversationHook({ linkActivityConversation });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
  const hook = renderHook(
    () => {
      const { data } = useQuery<ActivityDetailResponse>({
        queryKey: QUERY_KEYS.home.activityDetail('strava', 'act-4'),
        queryFn: () => held,
        staleTime: Infinity,
      });
      return useActivityConversation('strava', 'act-4', data ?? server);
    },
    { wrapper },
  );
  return { client, hook, linkActivityConversation };
}

describe('useActivityConversation', () => {
  it('shows the thread the server names for the activity', () => {
    const { hook, linkActivityConversation } = setup(detail('conv-tempo'));
    expect(hook.result.current.conversationId).toBe('conv-tempo');
    expect(linkActivityConversation).not.toHaveBeenCalled();
  });

  it('has no thread before the first question', () => {
    const { hook } = setup(detail(null));
    expect(hook.result.current.conversationId).toBeNull();
  });

  it('links the thread the view opens on the server, and shows it at once', async () => {
    const { client, hook, linkActivityConversation } = setup(detail(null));
    act(() => hook.result.current.setConversationId('conv-new'));
    await waitFor(() => expect(hook.result.current.conversationId).toBe('conv-new'));
    await waitFor(() => expect(linkActivityConversation).toHaveBeenCalledWith('strava', 'act-4', 'conv-new'));
    expect(
      client.getQueryData<ActivityDetailResponse>(QUERY_KEYS.home.activityDetail('strava', 'act-4'))
        ?.conversation_id,
    ).toBe('conv-new');
  });

  it('writes nothing when told the thread it already shows, and forgets it on null', async () => {
    const { hook, linkActivityConversation } = setup(detail('conv-tempo'));
    act(() => hook.result.current.setConversationId('conv-tempo'));
    expect(linkActivityConversation).not.toHaveBeenCalled();

    act(() => hook.result.current.setConversationId(null));
    await waitFor(() => expect(hook.result.current.conversationId).toBeNull());
    await waitFor(() => expect(linkActivityConversation).toHaveBeenCalledWith('strava', 'act-4', null));
  });

  it('restores the thread the server holds when the link is refused', async () => {
    const { client, hook, linkActivityConversation } = setup(detail(null));
    linkActivityConversation.mockRejectedValue(new Error('503 Service Unavailable'));
    act(() => hook.result.current.setConversationId('conv-new'));
    await waitFor(() => expect(hook.result.current.conversationId).toBe('conv-new'));
    // Refused on the first write and on both of its retries.
    await waitFor(() => expect(linkActivityConversation).toHaveBeenCalledTimes(3), { timeout: 6_000 });
    await waitFor(() => expect(hook.result.current.conversationId).toBeNull());
    expect(
      client.getQueryData<ActivityDetailResponse>(QUERY_KEYS.home.activityDetail('strava', 'act-4'))
        ?.conversation_id,
    ).toBeNull();
  }, 10_000);

  it('keeps the thread it is linking when the detail is read again before the link lands', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
    let serverThread: string | null = null;
    const getActivityDetail = vi.fn(async () => detail(serverThread));
    let landLink: () => void = () => {};
    const linkActivityConversation = vi.fn(
      (_provider: string, _id: string, conversationId: string | null) =>
        new Promise<void>((resolve) => {
          landLink = () => {
            serverThread = conversationId;
            resolve();
          };
        }),
    );
    const useActivityDetail = createActivityDetailHook({ getActivityDetail });
    const useActivityConversation = createActivityConversationHook({ linkActivityConversation });
    const wrapper = ({ children }: { children: ReactNode }) =>
      createElement(QueryClientProvider, { client }, children);
    const hook = renderHook(
      () => {
        const read = useActivityDetail('strava', 'act-4');
        const thread = useActivityConversation('strava', 'act-4', read.data ?? detail(null));
        return { read, thread };
      },
      { wrapper },
    );
    await waitFor(() => expect(hook.result.current.read.data).toBeDefined());

    act(() => hook.result.current.thread.setConversationId('conv-new'));
    await waitFor(() => expect(linkActivityConversation).toHaveBeenCalledTimes(1));
    // The view is opened again while the PUT is still on its way: the read
    // answers the server's old null, which must not overwrite the thread.
    await act(async () => {
      await client.refetchQueries({ queryKey: QUERY_KEYS.home.activityDetail('strava', 'act-4') });
    });
    expect(getActivityDetail.mock.calls.length).toBeGreaterThanOrEqual(2);
    expect(
      client.getQueryData<ActivityDetailResponse>(QUERY_KEYS.home.activityDetail('strava', 'act-4'))
        ?.conversation_id,
    ).toBe('conv-new');
    expect(hook.result.current.thread.conversationId).toBe('conv-new');

    act(() => landLink());
    await waitFor(() => expect(getActivityDetail.mock.calls.length).toBeGreaterThanOrEqual(3));
    expect(hook.result.current.thread.conversationId).toBe('conv-new');
  });
});
