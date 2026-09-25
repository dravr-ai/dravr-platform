// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the shared group hooks — the keys they read, the freshness each client passes, what a write refreshes
// ABOUTME: Both clients bind these, so a divergence here would be a divergence on web and mobile at once

import { describe, it, expect, vi, beforeEach } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { renderHook, waitFor, act } from '@testing-library/react';
import { QueryClient, QueryClientProvider, type QueryObserverOptions } from '@tanstack/react-query';
import type { GroupsApi } from '@pierre/api-client';
import { createGroupHooks, type GroupFreshness } from '../src/groupHooks';

const WEB: GroupFreshness = { detail: 30_000, members: 30_000, stats: 60_000 };
const MOBILE: GroupFreshness = { detail: 60_000, members: 60_000, stats: 5 * 60_000 };

function fakeGroupsApi() {
  return {
    getGroup: vi.fn().mockResolvedValue({ id: 'g1', name: 'Tempo Crew' }),
    listMembers: vi.fn().mockResolvedValue({ members: [{ user_id: 'u1' }, { user_id: 'u2' }] }),
    getStats: vi.fn().mockResolvedValue({ stats: { member_count: 2 } }),
    listInvites: vi.fn().mockResolvedValue({ invites: [] }),
    getPermissions: vi.fn().mockResolvedValue({ can_create: false, policy: 'admins', weekly_digest: true }),
    updateGroup: vi.fn().mockResolvedValue({ id: 'g1', name: 'Renamed' }),
    removeMember: vi.fn().mockResolvedValue(undefined),
    leaveGroup: vi.fn().mockResolvedValue(undefined),
    getDelegationRoster: vi.fn(),
    confirmDelegatedConnection: vi.fn().mockResolvedValue(undefined),
  };
}

function setup(freshness: GroupFreshness) {
  const api = fakeGroupsApi();
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
  const hooks = createGroupHooks(api as unknown as GroupsApi, freshness);
  return { api, client, wrapper, hooks };
}

function staleTimeOf(client: QueryClient, queryKey: readonly unknown[]): unknown {
  return (client.getQueryCache().find({ queryKey, exact: true })?.options as QueryObserverOptions | undefined)?.staleTime;
}

describe('createGroupHooks — reads', () => {
  it('reads the group under its shared key and returns what the API answered', async () => {
    const { api, client, wrapper, hooks } = setup(WEB);

    const { result } = renderHook(() => hooks.useGroup('g1'), { wrapper });

    await waitFor(() => expect(result.current.group).toEqual({ id: 'g1', name: 'Tempo Crew' }));
    expect(api.getGroup).toHaveBeenCalledWith('g1');
    expect(client.getQueryCache().find({ queryKey: ['groups', 'g1'], exact: true })).toBeDefined();
  });

  it('keeps the web freshness the web client passes: 30 s, 30 s, 60 s', async () => {
    const { client, wrapper, hooks } = setup(WEB);

    renderHook(
      () => [hooks.useGroup('g1'), hooks.useGroupMembers('g1'), hooks.useGroupStats('g1')],
      { wrapper },
    );

    await waitFor(() => expect(staleTimeOf(client, ['groups', 'g1', 'stats'])).toBeDefined());
    expect(staleTimeOf(client, ['groups', 'g1'])).toBe(30_000);
    expect(staleTimeOf(client, ['groups', 'g1', 'members'])).toBe(30_000);
    expect(staleTimeOf(client, ['groups', 'g1', 'stats'])).toBe(60_000);
  });

  it('keeps the mobile freshness the mobile client passes: 60 s, 60 s, 5 min', async () => {
    const { client, wrapper, hooks } = setup(MOBILE);

    renderHook(
      () => [hooks.useGroup('g1'), hooks.useGroupMembers('g1'), hooks.useGroupStats('g1')],
      { wrapper },
    );

    await waitFor(() => expect(staleTimeOf(client, ['groups', 'g1', 'stats'])).toBeDefined());
    expect(staleTimeOf(client, ['groups', 'g1'])).toBe(60_000);
    expect(staleTimeOf(client, ['groups', 'g1', 'members'])).toBe(60_000);
    expect(staleTimeOf(client, ['groups', 'g1', 'stats'])).toBe(300_000);
  });

  it('never asks for stats the coach is not admitted to', async () => {
    const { api, wrapper, hooks } = setup(WEB);

    const { result } = renderHook(() => hooks.useGroupStats('g1', false), { wrapper });

    expect(result.current.stats).toBeNull();
    expect(api.getStats).not.toHaveBeenCalled();
  });

  it('returns the member list, and the permissions the server resolved', async () => {
    const { wrapper, hooks } = setup(WEB);

    const { result } = renderHook(
      () => ({ members: hooks.useGroupMembers('g1'), permissions: hooks.useGroupPermissions() }),
      { wrapper },
    );

    await waitFor(() => expect(result.current.members.members).toHaveLength(2));
    await waitFor(() => expect(result.current.permissions.isLoading).toBe(false));
    expect(result.current.permissions.canCreate).toBe(false);
    expect(result.current.permissions.policy).toBe('admins');
    expect(result.current.permissions.weeklyDigest).toBe(true);
  });

  it('does not retry a roster refusal: it is an answer, not a blip', async () => {
    const { api, hooks } = setup(WEB);
    // The client default here is `retry: false`; the roster must refuse to
    // retry on its own, so give the default a retry to override.
    const client = new QueryClient({ defaultOptions: { queries: { retry: 3, retryDelay: 1 } } });
    const retrying = ({ children }: { children: ReactNode }) =>
      createElement(QueryClientProvider, { client }, children);
    api.getDelegationRoster.mockRejectedValue(new Error('trainingpeaks_not_connected'));

    const { result } = renderHook(() => hooks.useDelegationRoster('g1', true), { wrapper: retrying });

    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(api.getDelegationRoster).toHaveBeenCalledTimes(1);
  });
});

describe('createGroupHooks — writes', () => {
  let ctx: ReturnType<typeof setup>;

  beforeEach(() => {
    ctx = setup(WEB);
  });

  it('a rename refetches the group and the conversation row that names it before it resolves', async () => {
    const { api, client, wrapper, hooks } = ctx;
    const invalidate = vi.spyOn(client, 'invalidateQueries');
    const { result } = renderHook(
      () => ({ group: hooks.useGroup('g1'), update: hooks.useUpdateGroup('g1') }),
      { wrapper },
    );
    await waitFor(() => expect(result.current.group.group).not.toBeNull());
    api.getGroup.mockResolvedValue({ id: 'g1', name: 'Renamed' });

    await act(() => result.current.update.updateGroup({ name: 'Renamed' }));

    expect(api.updateGroup).toHaveBeenCalledWith('g1', { name: 'Renamed' });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['groups', 'g1'] });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['chat-conversations'] });
    // Awaited: the open group row was refetched by the time the write resolved.
    expect(api.getGroup).toHaveBeenCalledTimes(2);
    expect(client.getQueryData(['groups', 'g1'])).toEqual({ id: 'g1', name: 'Renamed' });
  });

  it('removing a member refreshes the member list and the stats aggregated over it', async () => {
    const { api, client, wrapper, hooks } = ctx;
    const invalidate = vi.spyOn(client, 'invalidateQueries');
    const { result } = renderHook(() => hooks.useRemoveMember('g1'), { wrapper });

    await act(() => result.current.removeMember('u2'));

    expect(api.removeMember).toHaveBeenCalledWith('g1', 'u2');
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['groups', 'g1', 'members'] });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['groups', 'g1', 'stats'] });
  });

  it('leaving refreshes every group read and the conversation list', async () => {
    const { api, client, wrapper, hooks } = ctx;
    const invalidate = vi.spyOn(client, 'invalidateQueries');
    const { result } = renderHook(() => hooks.useLeaveGroup(), { wrapper });

    await act(() => result.current.leaveGroup('g1'));

    expect(api.leaveGroup).toHaveBeenCalledWith('g1');
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['groups'] });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['chat-conversations'] });
  });

  it('a confirmed TrainingPeaks link refreshes the links, the roster and the provider rows', async () => {
    const { api, client, wrapper, hooks } = ctx;
    const invalidate = vi.spyOn(client, 'invalidateQueries');
    const { result } = renderHook(() => hooks.useConfirmDelegatedConnection('g1'), { wrapper });

    await act(() => result.current.confirmLink('conn-1'));

    expect(api.confirmDelegatedConnection).toHaveBeenCalledWith('g1', 'conn-1');
    expect(invalidate.mock.calls.map(([filters]) => filters?.queryKey)).toEqual([
      ['groups', 'g1', 'delegated-connections'],
      ['groups', 'g1', 'delegation-roster'],
      ['providers'],
      ['providers-status'],
      ['provider-connections'],
    ]);
  });
});
