// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The list's coach-seat line on the phone (carnet#1521) — pending on every coachless room the viewer owns, nothing read for anyone else
// ABOUTME: Pins the phone binding of the shared hook: rooms read only while a request waits, the short day, no read for an athlete

import React from 'react';
import { renderHook, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { buildConversationRow, type ConversationRowModel } from '@pierre/chat-utils';
import type { CoachingGroup, Conversation } from '@pierre/shared-types';

const mockGetRequest = jest.fn();
const mockGetGroup = jest.fn();
jest.mock('../src/services/api', () => ({
  userApi: { getCoachAccessRequest: () => mockGetRequest() },
  groupsApi: { getGroup: (id: string) => mockGetGroup(id) },
  authApi: { getSession: jest.fn() },
}));

import { useGroupSeatLines } from '../src/hooks/coachAccessHooks';

const LABELS = { locale: 'en-US', you: 'You', coach: 'Agent', untitled: 'Untitled chat' };

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
  return buildConversationRow(conversation, LABELS, new Date('2026-11-13T10:00:00Z'));
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

function wrapper({ children }: { children: React.ReactNode }) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

describe('useGroupSeatLines', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockGetGroup.mockImplementation(async (id: string) =>
      id === 'group-theirs' ? group(id, { owner_id: 'someone-else' }) : group(id),
    );
  });

  it('says the request waits on every coachless room the viewer owns, and on no other', async () => {
    mockGetRequest.mockResolvedValue({
      request: {
        id: 'req-1',
        user_id: 'user-coach',
        group_id: null,
        group_tenant_id: null,
        status: 'pending',
        created_at: '2026-11-12T19:32:00Z',
        decided_at: null,
        decided_by: null,
      },
    });
    const rows = [room('conv-a', 'group-a'), room('conv-b', 'group-b'), room('conv-c', 'group-theirs')];
    const { result } = renderHook(() => useGroupSeatLines(rows, { id: 'user-coach', manages_roster: false }), {
      wrapper,
    });

    await waitFor(() => expect(result.current.size).toBe(2));
    // The short day both clients print, no year (Phil, 2026-11-13).
    expect(result.current.get('conv-a')).toBe('Coach access pending · requested Nov 12');
    expect(result.current.get('conv-b')).toBe(result.current.get('conv-a'));
    expect(result.current.has('conv-c')).toBe(false);
  });

  it('reads no group for a viewer with nothing waiting, nor without a viewer', async () => {
    mockGetRequest.mockResolvedValue({ request: null });
    const rows = [room('conv-a', 'group-a')];
    const { result } = renderHook(() => useGroupSeatLines(rows, { id: 'user-athlete' }), { wrapper });
    await waitFor(() => expect(mockGetRequest).toHaveBeenCalled());
    expect(result.current.size).toBe(0);
    expect(mockGetGroup).not.toHaveBeenCalled();

    jest.clearAllMocks();
    renderHook(() => useGroupSeatLines(rows, null), { wrapper });
    expect(mockGetRequest).not.toHaveBeenCalled();
    expect(mockGetGroup).not.toHaveBeenCalled();
  });
});
