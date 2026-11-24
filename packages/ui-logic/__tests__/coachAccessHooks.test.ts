// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves the shared coach-access hooks — the grant refresh re-reads the session once on a grant, the seat line reads rooms only while it moves, a platform link reads the seat back
// ABOUTME: The pending line carries the short day both clients print (no year); an athlete's list and a held coach access read nothing

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { cleanup, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { buildConversationRow, type ConversationRowModel } from '@pierre/chat-utils';
import { COACH_SEAT_CHECK_WINDOW_MS, QUERY_KEYS } from '@pierre/shared-constants';
import type { CoachAccessRequest, CoachingGroup, Conversation } from '@pierre/shared-types';
import { createCoachAccessHooks, seatLineDay, type CoachAccessViewer } from '../src/coachAccessHooks';

const getCoachAccessRequest = vi.fn();
const getSession = vi.fn();
const getGroup = vi.fn();
const updateUser = vi.fn();
const session: { user: CoachAccessViewer | null } = { user: { id: 'user-coach', manages_roster: false } };

const { useCoachAccessGrantRefresh, useGroupSeatLines, useSeatAfterPlatformLink } = createCoachAccessHooks({
  userApi: { getCoachAccessRequest },
  authApi: { getSession },
  groupsApi: { getGroup },
  useSession: () => ({ user: session.user, updateUser }),
  useWords: () => ({
    t: (key, params) => `${key}|${Object.values(params ?? {}).join('|')}`,
    language: 'fr',
  }),
});

function request(status: CoachAccessRequest['status'], overrides: Partial<CoachAccessRequest> = {}): CoachAccessRequest {
  return {
    id: 'req-1',
    user_id: 'user-coach',
    group_id: null,
    group_tenant_id: null,
    status,
    created_at: '2026-11-12T19:32:00Z',
    decided_at: status === 'pending' ? null : '2026-11-13T09:14:00Z',
    decided_by: null,
    ...overrides,
  };
}

function room(id: string, groupId: string): ConversationRowModel {
  const conversation: Conversation = {
    id,
    title: 'Room',
    agent_id: null,
    group_id: groupId,
    group_name: 'Room',
    message_count: 1,
    unread_count: 0,
    created_at: '2026-11-12T10:00:00Z',
    updated_at: '2026-11-12T10:00:00Z',
  };
  return buildConversationRow(
    conversation,
    { locale: 'fr', you: 'Toi', coach: 'Agent', untitled: 'Sans titre' },
    new Date('2026-11-13T10:00:00Z'),
  );
}

function group(id: string, overrides: Partial<CoachingGroup> = {}): CoachingGroup {
  return {
    id,
    tenant_id: 'tenant-1',
    name: 'Room',
    description: null,
    agent_id: 'agent-1',
    agent_title: null,
    agent_handle: null,
    owner_id: 'user-coach',
    coach_user_id: null,
    coach_display_name: null,
    peer_data_sharing: false,
    respond_mode: 'all',
    digest_mode: 'off',
    max_members: 10,
    is_active: true,
    created_at: '2026-11-01T00:00:00Z',
    updated_at: '2026-11-01T00:00:00Z',
    ...overrides,
  };
}

function wrapper() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return ({ children }: { children: ReactNode }) => createElement(QueryClientProvider, { client }, children);
}

beforeEach(() => {
  vi.clearAllMocks();
  session.user = { id: 'user-coach', manages_roster: false };
});

// No vitest globals here, so nothing unmounts a hook between tests: a seat
// probe left reading would keep polling past the file and wake React after
// its window is gone.
afterEach(() => {
  cleanup();
});

describe('seatLineDay', () => {
  it('prints day and month only, in the reader’s language', () => {
    expect(seatLineDay('2026-11-12T19:32:00Z', 'fr')).toBe('12 nov.');
    expect(seatLineDay('2026-11-12T19:32:00Z', 'en')).toBe('Nov 12');
    expect(seatLineDay('not-a-date', 'fr')).toBe('not-a-date');
  });
});

describe('useCoachAccessGrantRefresh', () => {
  it('re-reads the session once the request reads granted, and gives the user coach access', async () => {
    getCoachAccessRequest.mockResolvedValue({ request: request('granted') });
    getSession.mockResolvedValue({ user: { id: 'user-coach', manages_roster: true } });

    renderHook(() => useCoachAccessGrantRefresh(), { wrapper: wrapper() });

    await waitFor(() => expect(updateUser).toHaveBeenCalledWith({ manages_roster: true }));
    expect(getSession).toHaveBeenCalledTimes(1);
  });

  it('changes nothing while the request waits, after a decline, or when the session says no', async () => {
    for (const answer of [request('pending'), request('declined')]) {
      getCoachAccessRequest.mockResolvedValue({ request: answer });
      const { unmount } = renderHook(() => useCoachAccessGrantRefresh(), { wrapper: wrapper() });
      await waitFor(() => expect(getCoachAccessRequest).toHaveBeenCalled());
      unmount();
    }
    expect(getSession).not.toHaveBeenCalled();

    getCoachAccessRequest.mockResolvedValue({ request: request('granted') });
    getSession.mockResolvedValue({ user: { id: 'user-coach', manages_roster: false } });
    renderHook(() => useCoachAccessGrantRefresh(), { wrapper: wrapper() });
    await waitFor(() => expect(getSession).toHaveBeenCalled());
    expect(updateUser).not.toHaveBeenCalled();
  });

  it('reads nothing for a user who already holds coach access', async () => {
    session.user = { id: 'user-coach', manages_roster: true };
    renderHook(() => useCoachAccessGrantRefresh(), { wrapper: wrapper() });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(getCoachAccessRequest).not.toHaveBeenCalled();
  });
});

describe('useGroupSeatLines', () => {
  it('says the request waits, with the short day, on every coachless room the viewer owns', async () => {
    getCoachAccessRequest.mockResolvedValue({ request: request('pending') });
    getGroup.mockImplementation(async (id: string) =>
      id === 'group-theirs' ? group(id, { owner_id: 'someone-else' }) : group(id),
    );
    const rows = [room('conv-a', 'group-a'), room('conv-c', 'group-theirs')];
    const { result } = renderHook(() => useGroupSeatLines(rows, session.user), { wrapper: wrapper() });

    await waitFor(() => expect(result.current.size).toBe(1));
    expect(result.current.get('conv-a')).toBe('humanCoach.seat.listPending|12 nov.');
    expect(result.current.has('conv-c')).toBe(false);
  });

  it('says the viewer is the coach right after the grant', async () => {
    const decidedAt = new Date().toISOString();
    getCoachAccessRequest.mockResolvedValue({ request: request('granted', { decided_at: decidedAt }) });
    getGroup.mockResolvedValue(group('group-a', { coach_user_id: 'user-coach' }));
    const fresh = { ...room('conv-a', 'group-a'), lastActivityAt: decidedAt };
    const { result } = renderHook(() => useGroupSeatLines([fresh], { id: 'user-coach', manages_roster: true }), {
      wrapper: wrapper(),
    });

    await waitFor(() => expect(result.current.size).toBe(1));
    expect(result.current.get('conv-a')).toMatch(/^humanCoach\.seat\.listCoach\|\d{2}:\d{2}$/);
  });

  it('reads no group for a viewer with nothing waiting, nor without a viewer', async () => {
    getCoachAccessRequest.mockResolvedValue({ request: null });
    const rows = [room('conv-a', 'group-a')];
    const { result } = renderHook(() => useGroupSeatLines(rows, { id: 'user-athlete' }), { wrapper: wrapper() });
    await waitFor(() => expect(getCoachAccessRequest).toHaveBeenCalled());
    expect(result.current.size).toBe(0);
    expect(getGroup).not.toHaveBeenCalled();

    vi.clearAllMocks();
    renderHook(() => useGroupSeatLines(rows, null), { wrapper: wrapper() });
    expect(getCoachAccessRequest).not.toHaveBeenCalled();
  });
});

describe('useSeatAfterPlatformLink', () => {
  type Props = { connected: readonly string[] | undefined };

  it('reads the session back once a coaching platform connects, and gives the user the access it earned', async () => {
    getCoachAccessRequest.mockResolvedValue({ request: null });
    getSession.mockResolvedValue({ user: { id: 'user-coach', manages_roster: true } });
    const { result, rerender } = renderHook(({ connected }: Props) => useSeatAfterPlatformLink(connected), {
      wrapper: wrapper(),
      initialProps: { connected: ['strava'] },
    });
    expect(result.current).toEqual({ reading: false, ended: false });

    rerender({ connected: ['strava', 'sciotte_trainingpeaks'] });
    await waitFor(() => expect(updateUser).toHaveBeenCalledWith({ manages_roster: true }));
  });

  it('reads nothing for a platform connected before the surface opened, nor for another provider', async () => {
    getCoachAccessRequest.mockResolvedValue({ request: null });
    const { result, rerender } = renderHook(({ connected }: Props) => useSeatAfterPlatformLink(connected), {
      wrapper: wrapper(),
      initialProps: { connected: undefined },
    });
    rerender({ connected: ['intervals_icu'] });
    rerender({ connected: ['intervals_icu', 'strava'] });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(getSession).not.toHaveBeenCalled();
    expect(result.current.reading).toBe(false);
  });

  it('opens the window for a platform already there on arrival when the surface says so', async () => {
    getCoachAccessRequest.mockResolvedValue({ request: null });
    getSession.mockResolvedValue({ user: { id: 'user-coach', manages_roster: false } });
    const { result, rerender } = renderHook(
      ({ connected }: Props) => useSeatAfterPlatformLink(connected, { fromFirstRead: true }),
      { wrapper: wrapper(), initialProps: { connected: undefined } },
    );
    rerender({ connected: ['intervals_icu'] });

    await waitFor(() => expect(getSession).toHaveBeenCalled());
    await waitFor(() => expect(result.current.reading).toBe(true));
    expect(updateUser).not.toHaveBeenCalled();
  });

  it('stops reading at a waiting request — the roster filed it', async () => {
    getCoachAccessRequest.mockResolvedValue({ request: request('pending') });
    const { result, rerender } = renderHook(({ connected }: Props) => useSeatAfterPlatformLink(connected), {
      wrapper: wrapper(),
      initialProps: { connected: [] },
    });
    await waitFor(() => expect(getCoachAccessRequest).toHaveBeenCalled());
    rerender({ connected: ['intervals_icu'] });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(result.current.reading).toBe(false);
    expect(getSession).not.toHaveBeenCalled();
  });

  it('says the window ran out once the check window is over', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      getCoachAccessRequest.mockResolvedValue({ request: null });
      getSession.mockResolvedValue({ user: { id: 'user-coach', manages_roster: false } });
      const { result, rerender } = renderHook(({ connected }: Props) => useSeatAfterPlatformLink(connected), {
        wrapper: wrapper(),
        initialProps: { connected: [] },
      });
      rerender({ connected: ['intervals_icu'] });
      await waitFor(() => expect(result.current.reading).toBe(true));
      await vi.advanceTimersByTimeAsync(COACH_SEAT_CHECK_WINDOW_MS + 1);
      await waitFor(() => expect(result.current).toEqual({ reading: false, ended: true }));
    } finally {
      vi.useRealTimers();
    }
  });

  it('reads nothing once the user holds coach access', async () => {
    session.user = { id: 'user-coach', manages_roster: true };
    const { result, rerender } = renderHook(({ connected }: Props) => useSeatAfterPlatformLink(connected), {
      wrapper: wrapper(),
      initialProps: { connected: [] },
    });
    rerender({ connected: ['intervals_icu'] });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(result.current.reading).toBe(false);
    expect(getSession).not.toHaveBeenCalled();
  });

  it('never refetches a failed request read as it mounts, so a surface that hides while pending cannot loop', async () => {
    getCoachAccessRequest.mockRejectedValue(new Error('offline'));
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const wrap = ({ children }: { children: ReactNode }) => createElement(QueryClientProvider, { client }, children);
    // The surface's own read, failed: the block under it renders on the error.
    await client.prefetchQuery({ queryKey: QUERY_KEYS.user.coachAccessRequest(), queryFn: getCoachAccessRequest });
    expect(getCoachAccessRequest).toHaveBeenCalledTimes(1);

    for (let mount = 0; mount < 3; mount += 1) {
      const { unmount } = renderHook(() => useSeatAfterPlatformLink(['strava']), { wrapper: wrap });
      await new Promise((resolve) => setTimeout(resolve, 10));
      unmount();
    }
    expect(getCoachAccessRequest).toHaveBeenCalledTimes(1);
  });
});
