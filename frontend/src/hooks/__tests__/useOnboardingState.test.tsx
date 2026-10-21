// ABOUTME: Tests the web onboarding hook's coach-proposal gate against the server status, as a fresh tab sees it
// ABOUTME: Pins carnet#1042 (fresh-tab gate) and carnet#1101 (the step's write carries the athlete's lead agent)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import type { ReactNode } from 'react';
import { describe, it, expect, beforeEach, vi } from 'vitest';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { currentOnboardingStep } from '../../onboarding/steps';

const getOnboardingStatus = vi.fn();
const setOnboardingStep = vi.fn();

vi.mock('../useAuth', () => ({
  useAuth: () => ({
    user: { user_id: 'u1', role: 'user', user_status: 'active' },
    isAuthenticated: true,
  }),
}));
vi.mock('../useAvailableChannels', () => ({
  useAvailableChannels: () => ({ channels: [] }),
}));
vi.mock('../../services/api', () => ({
  userApi: {
    getOnboardingStatus: (...args: unknown[]) => getOnboardingStatus(...args),
    setOnboardingStep: (...args: unknown[]) => setOnboardingStep(...args),
  },
}));

import { useOnboardingState } from '../useOnboardingState';

/** A connected athlete past the pre-connect questions, with the given server step rows. */
function connectedStatus(steps: { step_id: string; status: string }[]) {
  return {
    needs_provider_connection: false,
    coaches_others: false,
    chosen_channel: null,
    steps: [
      { step_id: 'profile_type', status: 'complete' },
      { step_id: 'about_you', status: 'complete' },
      { step_id: 'parq', status: 'complete' },
      { step_id: 'check_in', status: 'complete' },
      ...steps,
    ],
  };
}

const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });

function wrapper({ children }: { children: ReactNode }) {
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

describe('useOnboardingState — coach proposal in a fresh tab (carnet#1042)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setOnboardingStep.mockResolvedValue(undefined);
    localStorage.clear();
    client.clear();
  });

  it('provider already connected, coach step unrecorded → the proposal is shown', async () => {
    getOnboardingStatus.mockResolvedValue(connectedStatus([]));

    const { result } = renderHook(() => useOnboardingState(), { wrapper });

    await waitFor(() => expect(result.current.ctx.needsProviderConnection).toBe(false));
    // This tab mounted after the OAuth return, so it never saw the flip.
    expect(result.current.ctx.justOnboarded).toBe(false);
    expect(currentOnboardingStep(result.current.ctx)?.id).toBe('coach_proposal');
  });

  it.each(['complete', 'skipped'])(
    'coach step recorded %s on the server → the proposal is not shown',
    async (status) => {
      getOnboardingStatus.mockResolvedValue(
        connectedStatus([{ step_id: 'coach_proposal', status }]),
      );

      const { result } = renderHook(() => useOnboardingState(), { wrapper });

      await waitFor(() => expect(result.current.ctx.needsProviderConnection).toBe(false));
      expect(result.current.ctx.coachProposalDone).toBe(true);
      expect(currentOnboardingStep(result.current.ctx)).toBeNull();
    },
  );

  it('coach step done in this browser only → the proposal is not shown, not even for a frame', async () => {
    localStorage.setItem('dravr.coach_proposal_done.u1', '1');
    getOnboardingStatus.mockResolvedValue(connectedStatus([]));

    const { result } = renderHook(() => useOnboardingState(), { wrapper });

    expect(result.current.ctx.coachProposalDone).toBe(true);
    await waitFor(() => expect(result.current.ctx.needsProviderConnection).toBe(false));
    expect(currentOnboardingStep(result.current.ctx)).toBeNull();
  });
});

describe('useOnboardingState — the coach step records the lead agent (carnet#1101)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setOnboardingStep.mockResolvedValue(undefined);
    localStorage.clear();
    client.clear();
    getOnboardingStatus.mockResolvedValue(connectedStatus([]));
  });

  it('completing with a lead sends its id, which the server makes the selected agent', async () => {
    const { result } = renderHook(() => useOnboardingState(), { wrapper });
    await waitFor(() => expect(result.current.ctx.needsProviderConnection).toBe(false));

    act(() => result.current.completeCoachProposal('coach-trail'));

    expect(setOnboardingStep).toHaveBeenCalledTimes(1);
    expect(setOnboardingStep).toHaveBeenCalledWith('coach_proposal', 'complete', {
      agentId: 'coach-trail',
    });
    expect(result.current.ctx.coachProposalDone).toBe(true);
  });

  it('completing without a lead sends no agent id, so the server changes no selection', async () => {
    const { result } = renderHook(() => useOnboardingState(), { wrapper });
    await waitFor(() => expect(result.current.ctx.needsProviderConnection).toBe(false));

    act(() => result.current.completeCoachProposal());

    expect(setOnboardingStep).toHaveBeenCalledTimes(1);
    const [stepId, status, options] = setOnboardingStep.mock.calls[0];
    expect([stepId, status]).toEqual(['coach_proposal', 'complete']);
    expect(options?.agentId).toBeUndefined();
  });
});
