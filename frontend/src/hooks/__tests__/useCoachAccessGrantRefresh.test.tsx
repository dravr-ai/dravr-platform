// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves a coach-access grant reaches the signed-in user without signing out — the session is re-read once the request reads granted
// ABOUTME: A waiting or declined request, or a user who already holds coach access, re-reads nothing

import type { ReactNode } from 'react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CoachAccessRequest } from '@pierre/shared-types';
import { useCoachAccessGrantRefresh } from '../coachAccessHooks';

const api = vi.hoisted(() => ({
  getCoachAccessRequest: vi.fn(),
  getSession: vi.fn(),
  updateUser: vi.fn(),
}));
const viewer = vi.hoisted(() => ({ id: 'user-1', manages_roster: false }));

vi.mock('../../services/api', () => ({
  userApi: { getCoachAccessRequest: () => api.getCoachAccessRequest() },
  authApi: { getSession: () => api.getSession() },
  groupsApi: { getGroup: vi.fn() },
}));
vi.mock('../useAuth', () => ({ useAuth: () => ({ user: viewer, updateUser: api.updateUser }) }));

function request(status: CoachAccessRequest['status']): CoachAccessRequest {
  return {
    id: 'req-1',
    user_id: 'user-1',
    group_id: null,
    group_tenant_id: null,
    status,
    created_at: '2026-11-12T19:32:00Z',
    decided_at: status === 'pending' ? null : '2026-11-13T09:14:00Z',
    decided_by: null,
  };
}

function wrapper({ children }: { children: ReactNode }) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

describe('useCoachAccessGrantRefresh', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    viewer.manages_roster = false;
  });

  it('re-reads the session once the request reads granted, and gives the user coach access', async () => {
    api.getCoachAccessRequest.mockResolvedValue({ request: request('granted') });
    api.getSession.mockResolvedValue({ user: { id: 'user-1', manages_roster: true } });

    renderHook(() => useCoachAccessGrantRefresh(), { wrapper });

    await waitFor(() => expect(api.updateUser).toHaveBeenCalledWith({ manages_roster: true }));
    expect(api.getSession).toHaveBeenCalledTimes(1);
  });

  it('changes nothing while the request waits or after a decline', async () => {
    for (const status of ['pending', 'declined'] as const) {
      api.getCoachAccessRequest.mockResolvedValue({ request: request(status) });
      const { unmount } = renderHook(() => useCoachAccessGrantRefresh(), { wrapper });
      await waitFor(() => expect(api.getCoachAccessRequest).toHaveBeenCalled());
      unmount();
    }
    expect(api.getSession).not.toHaveBeenCalled();
    expect(api.updateUser).not.toHaveBeenCalled();
  });

  it('reads nothing for a user who already holds coach access', async () => {
    viewer.manages_roster = true;
    renderHook(() => useCoachAccessGrantRefresh(), { wrapper });

    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(api.getCoachAccessRequest).not.toHaveBeenCalled();
  });

  it('leaves the user as they are when the session says the access was taken back', async () => {
    api.getCoachAccessRequest.mockResolvedValue({ request: request('granted') });
    api.getSession.mockResolvedValue({ user: { id: 'user-1', manages_roster: false } });

    renderHook(() => useCoachAccessGrantRefresh(), { wrapper });

    await waitFor(() => expect(api.getSession).toHaveBeenCalled());
    expect(api.updateUser).not.toHaveBeenCalled();
  });
});
