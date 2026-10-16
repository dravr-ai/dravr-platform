// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the shared coach-group onboarding sequence — group, thread, then a 30-day athlete invite
// ABOUTME: A retry after the invite failed reuses the group and thread already made, and the chat list is refreshed

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { act, renderHook } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type {
  CoachingGroup,
  Conversation,
  CreateGroupRequest,
  CreateInviteRequest,
  GroupInvite,
} from '@pierre/shared-types';
import type { CreateConversationOptions } from '@pierre/api-client';
import { createCoachGroupSetupHook } from '../src/coachGroupSetupHook';

const GROUP = { id: 'group-1', name: 'Morning Crew', agent_id: 'agent-7', coach_user_id: 'coach-1' } as CoachingGroup;

const createGroup = vi.fn<(request: CreateGroupRequest) => Promise<CoachingGroup>>();
const createInvite = vi.fn<(groupId: string, request?: CreateInviteRequest) => Promise<GroupInvite>>();
const createConversation = vi.fn<(options?: CreateConversationOptions) => Promise<Conversation>>();
const useCoachGroupSetup = createCoachGroupSetupHook({
  groupsApi: { createGroup, createInvite },
  chatApi: { createConversation },
});

function setup() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const invalidate = vi.spyOn(client, 'invalidateQueries');
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
  return { invalidate, wrapper };
}

describe('createCoachGroupSetupHook', () => {
  beforeEach(() => {
    createGroup.mockReset();
    createInvite.mockReset();
    createConversation.mockReset();
    createGroup.mockResolvedValue(GROUP);
    createConversation.mockResolvedValue({ id: 'thread-1' } as Conversation);
  });

  it('makes the group as its coach, opens its thread with its agent, and issues a 30-day invite', async () => {
    createInvite.mockResolvedValue({ code: 'JOIN-1' } as GroupInvite);
    const { invalidate, wrapper } = setup();
    const { result } = renderHook(() => useCoachGroupSetup(), { wrapper });

    await act(() => result.current.create('  Morning Crew  ', 'agent-7'));

    expect(createGroup).toHaveBeenCalledWith({ name: 'Morning Crew', agent_id: 'agent-7', coach_is_me: true });
    expect(createConversation).toHaveBeenCalledWith({ group_id: 'group-1', agent_id: 'agent-7' });
    expect(createInvite).toHaveBeenCalledWith('group-1', { expires_in_days: 30 });
    expect(invalidate.mock.calls.map(([filters]) => filters?.queryKey)).toEqual([['chat-conversations']]);
    expect(result.current.group).toEqual(GROUP);
    expect(result.current.conversationId).toBe('thread-1');
    expect(result.current.inviteCode).toBe('JOIN-1');
    expect(result.current.failed).toBe(false);
    expect(result.current.working).toBe(false);
  });

  it('resumes after a failed invite without a second group or thread', async () => {
    createInvite.mockRejectedValueOnce(new Error('network down')).mockResolvedValueOnce({ code: 'JOIN-2' } as GroupInvite);
    const { wrapper } = setup();
    const { result } = renderHook(() => useCoachGroupSetup(), { wrapper });

    await act(() => result.current.create('Morning Crew', 'agent-7'));
    expect(result.current.failed).toBe(true);
    expect(result.current.inviteCode).toBeNull();
    expect(result.current.group).toEqual(GROUP);

    await act(() => result.current.create('Morning Crew', 'agent-7'));

    expect(createGroup).toHaveBeenCalledTimes(1);
    expect(createConversation).toHaveBeenCalledTimes(1);
    expect(createInvite).toHaveBeenCalledTimes(2);
    expect(createInvite).toHaveBeenNthCalledWith(2, 'group-1', { expires_in_days: 30 });
    expect(result.current.failed).toBe(false);
    expect(result.current.inviteCode).toBe('JOIN-2');
  });

  it('makes nothing for a blank name', async () => {
    const { wrapper } = setup();
    const { result } = renderHook(() => useCoachGroupSetup(), { wrapper });

    await act(() => result.current.create('   ', 'agent-7'));

    expect(createGroup).not.toHaveBeenCalled();
    expect(result.current.group).toBeNull();
  });
});
