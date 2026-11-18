// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the app-root settling of a group invite: the carried code's hold, then the join of what the server holds
// ABOUTME: Pins carnet#1648 — the carry outlives a failed hold and a reload, and an active account joins once per page load

import { StrictMode, type ReactNode } from 'react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { HoldGroupInviteResponse, JoinHeldInviteResponse } from '@pierre/shared-types';
import { readCarriedGroupInvite } from '../../utils/groupInvite';

const holdGroupInvite = vi.fn<(code: string) => Promise<HoldGroupInviteResponse>>();
const joinHeldInvite = vi.fn<() => Promise<JoinHeldInviteResponse>>();
const addToast = vi.fn();

interface AuthState {
  user: { id: string; user_status: 'pending' | 'active' | 'suspended' } | null;
  isAuthenticated: boolean;
  isLoading: boolean;
  impersonation: { isImpersonating: boolean };
}
let auth: AuthState;

vi.mock('../useAuth', () => ({ useAuth: () => auth }));
vi.mock('../../services/api', () => ({
  groupInviteApi: {
    holdGroupInvite: (code: string) => holdGroupInvite(code),
    joinHeldInvite: () => joinHeldInvite(),
  },
}));
vi.mock('../../components/ui', () => ({ useToast: () => ({ addToast }) }));

import { useHeldGroupInvite } from '../useHeldGroupInvite';

const CARRY_KEY = 'dravr.pending_invite_code';

function signedIn(user_status: 'pending' | 'active' | 'suspended'): AuthState {
  return {
    user: { id: 'u-1', user_status },
    isAuthenticated: true,
    isLoading: false,
    impersonation: { isImpersonating: false },
  };
}

/** A refusal as axios hands it on: the response's status is what the hook reads. */
function refusal(status: number) {
  return { isAxiosError: true, response: { status, data: {} } };
}

let client: QueryClient;

function renderHeldInvite({ strict = false }: { strict?: boolean } = {}) {
  const wrapper = ({ children }: { children: ReactNode }) => {
    const tree = <QueryClientProvider client={client}>{children}</QueryClientProvider>;
    return strict ? <StrictMode>{tree}</StrictMode> : tree;
  };
  return renderHook(() => useHeldGroupInvite(), { wrapper });
}

/** Let every pending promise in the hook settle. */
async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0));
  });
}

describe('useHeldGroupInvite — the carried code (carnet#1648)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sessionStorage.clear();
    client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    joinHeldInvite.mockResolvedValue({ outcome: 'none' });
  });

  it('hands the code to the server for a pending account and drops it once held', async () => {
    sessionStorage.setItem(CARRY_KEY, 'MRT2026X');
    auth = signedIn('pending');
    holdGroupInvite.mockResolvedValue({ held: true, kind: 'member' });

    const { result } = renderHeldInvite();

    await waitFor(() => expect(holdGroupInvite).toHaveBeenCalledWith('MRT2026X'));
    await waitFor(() => expect(readCarriedGroupInvite()).toBeNull());
    // A pending account cannot join yet: the server keeps the invite for it.
    expect(joinHeldInvite).not.toHaveBeenCalled();
    expect(result.current.invitedAsAthlete).toBe(true);
  });

  it('says nothing of the role until the hold names the kind of invite', async () => {
    sessionStorage.setItem(CARRY_KEY, 'MRT2026X');
    auth = signedIn('pending');
    let answer: (response: HoldGroupInviteResponse) => void = () => {};
    holdGroupInvite.mockReturnValue(new Promise((resolve) => (answer = resolve)));

    const { result } = renderHeldInvite();

    await waitFor(() => expect(holdGroupInvite).toHaveBeenCalledTimes(1));
    // Carried, but the kind is the server's to say: a coach seat is no athlete.
    expect(result.current.invitedAsAthlete).toBe(false);

    await act(async () => answer({ held: true, kind: 'member' }));
    expect(result.current.invitedAsAthlete).toBe(true);
  });

  it('never marks a coach-seat invite as an athlete', async () => {
    sessionStorage.setItem(CARRY_KEY, 'COACH123');
    auth = signedIn('pending');
    holdGroupInvite.mockResolvedValue({ held: true, kind: 'coach' });

    const { result } = renderHeldInvite();

    await waitFor(() => expect(readCarriedGroupInvite()).toBeNull());
    expect(result.current.invitedAsAthlete).toBe(false);
  });

  it('keeps the code through a failed hold and an aborted one, and sends it again after a reload', async () => {
    sessionStorage.setItem(CARRY_KEY, 'MRT2026X');
    auth = signedIn('pending');
    holdGroupInvite.mockRejectedValueOnce(refusal(503));

    const first = renderHeldInvite();
    await waitFor(() => expect(holdGroupInvite).toHaveBeenCalledTimes(1));
    await settle();
    expect(readCarriedGroupInvite()).toBe('MRT2026X');
    first.unmount();

    // The reload a service worker update forces: no answer at all this time.
    holdGroupInvite.mockRejectedValueOnce({ code: 'ERR_NETWORK', message: 'Network Error' });
    const second = renderHeldInvite();
    await waitFor(() => expect(holdGroupInvite).toHaveBeenCalledTimes(2));
    await settle();
    expect(readCarriedGroupInvite()).toBe('MRT2026X');
    second.unmount();

    holdGroupInvite.mockResolvedValueOnce({ held: true, kind: 'member' });
    renderHeldInvite();
    await waitFor(() => expect(holdGroupInvite).toHaveBeenCalledTimes(3));
    expect(holdGroupInvite.mock.calls.map(([code]) => code)).toEqual(['MRT2026X', 'MRT2026X', 'MRT2026X']);
    await waitFor(() => expect(readCarriedGroupInvite()).toBeNull());
  });

  it.each([400, 404])('drops a code the server calls dead (%i), and the page is no invite', async (status) => {
    sessionStorage.setItem(CARRY_KEY, 'EXPIRED1');
    auth = signedIn('pending');
    holdGroupInvite.mockRejectedValue(refusal(status));

    const { result } = renderHeldInvite();

    await waitFor(() => expect(readCarriedGroupInvite()).toBeNull());
    expect(result.current.invitedAsAthlete).toBe(false);
  });

  it('keeps a code a throttled hold (429) could not place', async () => {
    sessionStorage.setItem(CARRY_KEY, 'MRT2026X');
    auth = signedIn('pending');
    holdGroupInvite.mockRejectedValue(refusal(429));

    renderHeldInvite();

    await waitFor(() => expect(holdGroupInvite).toHaveBeenCalledTimes(1));
    await settle();
    expect(readCarriedGroupInvite()).toBe('MRT2026X');
  });

  it('sends nothing before a session, for a suspended account, or while impersonating', async () => {
    sessionStorage.setItem(CARRY_KEY, 'MRT2026X');

    auth = { user: null, isAuthenticated: false, isLoading: false, impersonation: { isImpersonating: false } };
    renderHeldInvite().unmount();
    auth = { ...signedIn('active'), isLoading: true };
    renderHeldInvite().unmount();
    auth = signedIn('suspended');
    renderHeldInvite().unmount();
    auth = { ...signedIn('active'), impersonation: { isImpersonating: true } };
    renderHeldInvite().unmount();
    await settle();

    expect(holdGroupInvite).not.toHaveBeenCalled();
    expect(joinHeldInvite).not.toHaveBeenCalled();
    expect(readCarriedGroupInvite()).toBe('MRT2026X');
  });

  it('holds the code once a sign-in completes on the same page', async () => {
    sessionStorage.setItem(CARRY_KEY, 'MRT2026X');
    auth = { user: null, isAuthenticated: false, isLoading: false, impersonation: { isImpersonating: false } };
    holdGroupInvite.mockResolvedValue({ held: true, kind: 'member' });
    const { rerender } = renderHeldInvite();
    await settle();
    expect(holdGroupInvite).not.toHaveBeenCalled();

    auth = signedIn('active');
    rerender();

    await waitFor(() => expect(joinHeldInvite).toHaveBeenCalledTimes(1));
    expect(holdGroupInvite).toHaveBeenCalledWith('MRT2026X');
  });
});

describe('useHeldGroupInvite — the join of what the server holds (carnet#1648)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    sessionStorage.clear();
    client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    holdGroupInvite.mockResolvedValue({ held: true, kind: 'member' });
  });

  it('joins once per page load for an active account, after the carried code is held', async () => {
    sessionStorage.setItem(CARRY_KEY, 'MRT2026X');
    auth = signedIn('active');
    const order: string[] = [];
    holdGroupInvite.mockImplementation(async () => {
      order.push('hold');
      return { held: true, kind: 'member' };
    });
    joinHeldInvite.mockImplementation(async () => {
      order.push('join');
      return { outcome: 'none' };
    });

    // StrictMode runs the effect twice: neither call may go out twice.
    const { rerender } = renderHeldInvite({ strict: true });
    await waitFor(() => expect(joinHeldInvite).toHaveBeenCalledTimes(1));
    rerender();
    await settle();

    expect(order).toEqual(['hold', 'join']);
    expect(holdGroupInvite).toHaveBeenCalledTimes(1);
    expect(joinHeldInvite).toHaveBeenCalledTimes(1);
  });

  it('asks the server with no code carried, and nothing held is no invite', async () => {
    auth = signedIn('active');
    joinHeldInvite.mockResolvedValue({ outcome: 'none' });

    const { result } = renderHeldInvite();

    await waitFor(() => expect(joinHeldInvite).toHaveBeenCalledTimes(1));
    expect(holdGroupInvite).not.toHaveBeenCalled();
    expect(result.current.invitedAsAthlete).toBe(false);
    expect(result.current.joinedRoom).toBeNull();
  });

  it('joins when a pending account is let in while the page is up', async () => {
    auth = signedIn('pending');
    joinHeldInvite.mockResolvedValue({ outcome: 'none' });
    const { rerender } = renderHeldInvite();
    await settle();
    expect(joinHeldInvite).not.toHaveBeenCalled();

    auth = signedIn('active');
    rerender();

    await waitFor(() => expect(joinHeldInvite).toHaveBeenCalledTimes(1));
  });

  it('hands the joined room to the dashboard, marks the page invited, and refreshes the lists', async () => {
    auth = signedIn('active');
    // Held at registration: no code carried, so the join names the kind.
    joinHeldInvite.mockResolvedValue({
      outcome: 'joined',
      kind: 'member',
      group_id: 'group-1',
      group_name: 'Morning Riders',
      conversation_id: 'conv-group-1',
    });
    const invalidate = vi.spyOn(client, 'invalidateQueries');

    const { result } = renderHeldInvite();

    await waitFor(() => expect(result.current.joinedRoom).toEqual({ conversationId: 'conv-group-1' }));
    expect(result.current.invitedAsAthlete).toBe(true);
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['chat-conversations'] });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['groups'] });

    act(() => result.current.onJoinedRoomOpened());
    expect(result.current.joinedRoom).toBeNull();
    expect(result.current.invitedAsAthlete).toBe(true);
  });

  it('opens the Groups list when the server joined but could not file the room', async () => {
    auth = signedIn('active');
    joinHeldInvite.mockResolvedValue({
      outcome: 'joined',
      kind: 'member',
      group_id: 'group-1',
      group_name: 'Morning Riders',
      conversation_id: null,
    });

    const { result } = renderHeldInvite();

    await waitFor(() => expect(result.current.joinedRoom).toEqual({ conversationId: null }));
  });

  it('shows nothing for a group the athlete was already in, but still counts the invite', async () => {
    auth = signedIn('active');
    joinHeldInvite.mockResolvedValue({
      outcome: 'already_member',
      kind: 'member',
      group_id: 'group-1',
      group_name: 'Morning Riders',
    });

    const { result } = renderHeldInvite();

    await waitFor(() => expect(result.current.invitedAsAthlete).toBe(true));
    expect(result.current.joinedRoom).toBeNull();
    expect(addToast).not.toHaveBeenCalled();
  });

  it('opens the room a coach seat attached the account to, and records no athlete', async () => {
    auth = signedIn('active');
    joinHeldInvite.mockResolvedValue({
      outcome: 'joined',
      kind: 'coach',
      group_id: 'group-1',
      group_name: 'Morning Riders',
      conversation_id: 'conv-group-1',
    });

    const { result } = renderHeldInvite();

    await waitFor(() => expect(result.current.joinedRoom).toEqual({ conversationId: 'conv-group-1' }));
    expect(result.current.invitedAsAthlete).toBe(false);
  });

  it("says the server's refusal once, in its own words", async () => {
    auth = signedIn('active');
    joinHeldInvite.mockResolvedValue({ outcome: 'refused', message: 'Ce groupe est complet.' });

    const { result, rerender } = renderHeldInvite();

    await waitFor(() => expect(addToast).toHaveBeenCalledTimes(1));
    rerender();
    await settle();
    expect(addToast).toHaveBeenCalledTimes(1);
    expect(addToast).toHaveBeenCalledWith({ type: 'error', title: 'Ce groupe est complet.' });
    expect(result.current.joinedRoom).toBeNull();
  });

  it('leaves a failed join for the next page load, without retrying in a loop', async () => {
    auth = signedIn('active');
    joinHeldInvite.mockRejectedValue(refusal(500));

    const { result, rerender } = renderHeldInvite();

    await waitFor(() => expect(joinHeldInvite).toHaveBeenCalledTimes(1));
    rerender();
    await settle();
    expect(joinHeldInvite).toHaveBeenCalledTimes(1);
    expect(result.current.joinedRoom).toBeNull();
    expect(addToast).not.toHaveBeenCalled();
  });
});
