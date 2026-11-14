// ABOUTME: Tests the web onboarding hook's coach-proposal gate against the server status, as a fresh tab sees it
// ABOUTME: Pins carnet#1042 (fresh-tab gate), carnet#1101 (the step's write carries the lead agent), carnet#1366 (the analytics question)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import type { ReactNode } from 'react';
import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { currentOnboardingStep, onboardingProgress } from '../../onboarding/steps';

const getOnboardingStatus = vi.fn();
const setOnboardingStep = vi.fn();
const setCoachingRole = vi.fn();
const updateAnalyticsConsent = vi.fn();
const updateUser = vi.fn();

const SESSION_USER = { user_id: 'u1', role: 'user', user_status: 'active' };
let sessionUser: Record<string, unknown> = SESSION_USER;

vi.mock('../useAuth', () => ({
  useAuth: () => ({
    user: sessionUser,
    isAuthenticated: true,
    updateUser,
  }),
}));
vi.mock('../useAvailableChannels', () => ({
  useAvailableChannels: () => ({ channels: [] }),
}));
vi.mock('../../services/api', () => ({
  userApi: {
    getOnboardingStatus: (...args: unknown[]) => getOnboardingStatus(...args),
    setOnboardingStep: (...args: unknown[]) => setOnboardingStep(...args),
    setCoachingRole: (...args: unknown[]) => setCoachingRole(...args),
    updateAnalyticsConsent: (...args: unknown[]) => updateAnalyticsConsent(...args),
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
      { step_id: 'parq', status: 'complete' },
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

describe('useOnboardingState — the team step writes the team and waits for it (carnet#1101, carnet#1095)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setOnboardingStep.mockResolvedValue(undefined);
    localStorage.clear();
    client.clear();
    getOnboardingStatus.mockResolvedValue(connectedStatus([]));
  });

  it('Start sends the lead and every agent joining, and advances nothing by itself', async () => {
    const { result } = renderHook(() => useOnboardingState(), { wrapper });
    await waitFor(() => expect(result.current.ctx.needsProviderConnection).toBe(false));

    await act(() =>
      result.current.persistCoachProposal({
        status: 'complete',
        leadAgentId: 'agent-endurance',
        teamAgentIds: ['agent-endurance', 'agent-fuel'],
      }),
    );

    expect(setOnboardingStep).toHaveBeenCalledTimes(1);
    expect(setOnboardingStep).toHaveBeenCalledWith('coach_proposal', 'complete', {
      agentId: 'agent-endurance',
      teamAgentIds: ['agent-endurance', 'agent-fuel'],
    });
    // The step opens the lead's topic before Home mounts, so advancing is its call.
    expect(result.current.ctx.coachProposalDone).toBe(false);

    act(() => result.current.completeCoachProposal());
    expect(result.current.ctx.coachProposalDone).toBe(true);
    expect(setOnboardingStep).toHaveBeenCalledTimes(1);
  });

  it('Skip is recorded as skipped with the lead alone', async () => {
    const { result } = renderHook(() => useOnboardingState(), { wrapper });
    await waitFor(() => expect(result.current.ctx.needsProviderConnection).toBe(false));

    await act(() =>
      result.current.persistCoachProposal({
        status: 'skipped',
        leadAgentId: 'agent-endurance',
        teamAgentIds: ['agent-endurance'],
      }),
    );

    expect(setOnboardingStep).toHaveBeenCalledWith('coach_proposal', 'skipped', {
      agentId: 'agent-endurance',
      teamAgentIds: ['agent-endurance'],
    });
  });

  it('a failed write rejects, so the step can keep the athlete on it', async () => {
    setOnboardingStep.mockRejectedValueOnce(new Error('503'));
    const { result } = renderHook(() => useOnboardingState(), { wrapper });
    await waitFor(() => expect(result.current.ctx.needsProviderConnection).toBe(false));

    await expect(
      result.current.persistCoachProposal({ status: 'complete', leadAgentId: 'agent-endurance' }),
    ).rejects.toThrow('503');
    expect(result.current.ctx.coachProposalDone).toBe(false);
  });

  it('a step written with no proposal sends no agent id, so the server changes no selection', async () => {
    const { result } = renderHook(() => useOnboardingState(), { wrapper });
    await waitFor(() => expect(result.current.ctx.needsProviderConnection).toBe(false));

    await act(() => result.current.persistCoachProposal({ status: 'complete' }));

    expect(setOnboardingStep).toHaveBeenCalledTimes(1);
    const [stepId, status, options] = setOnboardingStep.mock.calls[0];
    expect([stepId, status]).toEqual(['coach_proposal', 'complete']);
    expect(options?.agentId).toBeUndefined();
    expect(options?.teamAgentIds).toBeUndefined();
  });
});

describe('useOnboardingState — a step put off reads skipped on the bar at once', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setOnboardingStep.mockResolvedValue(undefined);
    localStorage.clear();
    client.clear();
  });

  it('« Continuer sans connecter » is saved: the PAR-Q is next and Connect draws skipped', async () => {
    localStorage.setItem('dravr.profile_type_chosen.u1', '1');
    getOnboardingStatus.mockResolvedValue({
      needs_provider_connection: true,
      coaches_others: false,
      chosen_channel: null,
      steps: [{ step_id: 'profile_type', status: 'complete' }],
    });
    const { result } = renderHook(() => useOnboardingState(), { wrapper });
    await waitFor(() => expect(currentOnboardingStep(result.current.ctx)?.id).toBe('connect_provider'));

    act(() => result.current.skipProvider());

    expect(currentOnboardingStep(result.current.ctx)?.id).toBe('parq');
    expect(setOnboardingStep).toHaveBeenCalledWith('connect_provider', 'skipped');
    await waitFor(() => expect(result.current.ctx.skippedSteps).toEqual(['connect_provider']));
    expect(onboardingProgress(result.current.ctx).map((s) => s.status)).toEqual([
      'done',
      'skipped',
      'current',
      'upcoming',
    ]);
  });

  it('a skip saved in an earlier session keeps Connect away at the next sign-in', async () => {
    localStorage.setItem('dravr.profile_type_chosen.u1', '1');
    localStorage.setItem('dravr.parq_done.u1', '1');
    localStorage.setItem('dravr.coach_proposal_done.u1', '1');
    getOnboardingStatus.mockResolvedValue({
      needs_provider_connection: true,
      coaches_others: false,
      chosen_channel: null,
      steps: [
        { step_id: 'profile_type', status: 'complete' },
        { step_id: 'connect_provider', status: 'skipped' },
        { step_id: 'parq', status: 'complete' },
        { step_id: 'coach_proposal', status: 'complete' },
      ],
    });
    const { result } = renderHook(() => useOnboardingState(), { wrapper });
    await waitFor(() => expect(result.current.ctx.needsProviderConnection).toBe(true));

    expect(result.current.ctx.skippedProvider).toBe(true);
    expect(currentOnboardingStep(result.current.ctx)).toBeNull();
  });

  it('a skipped team is cached once its write lands', async () => {
    getOnboardingStatus.mockResolvedValue(connectedStatus([]));
    const { result } = renderHook(() => useOnboardingState(), { wrapper });
    await waitFor(() => expect(result.current.ctx.needsProviderConnection).toBe(false));

    await act(() => result.current.persistCoachProposal({ status: 'skipped', leadAgentId: 'agent-endurance' }));

    // The cache notifies its readers on the next tick.
    await waitFor(() => expect(result.current.ctx.skippedSteps).toEqual(['coach_proposal']));
  });
});

describe('useOnboardingState — an invite link records the athlete role', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setOnboardingStep.mockResolvedValue(undefined);
    setCoachingRole.mockResolvedValue({ coaches_others: false });
    localStorage.clear();
    client.clear();
    sessionUser = SESSION_USER;
  });

  const NEW_ACCOUNT = {
    needs_provider_connection: true,
    coaches_others: false,
    chosen_channel: null,
    steps: [],
  };

  it('opens on Tes données with Profil done, because the role is recorded', async () => {
    getOnboardingStatus.mockResolvedValue(NEW_ACCOUNT);
    const { result } = renderHook(() => useOnboardingState({ invitedAsAthlete: true }), { wrapper });

    await waitFor(() => expect(currentOnboardingStep(result.current.ctx)?.id).toBe('connect_provider'));
    expect(setCoachingRole).toHaveBeenCalledWith(false);
    expect(setOnboardingStep).toHaveBeenCalledWith('profile_type', 'complete');
    expect(onboardingProgress(result.current.ctx).map((s) => [s.id, s.status])).toEqual([
      ['profile_type', 'done'],
      ['connect_provider', 'current'],
      ['parq', 'upcoming'],
      ['coach_proposal', 'upcoming'],
    ]);
  });

  it('records nothing without an invite: the role question is asked', async () => {
    getOnboardingStatus.mockResolvedValue(NEW_ACCOUNT);
    const { result } = renderHook(() => useOnboardingState(), { wrapper });

    await waitFor(() => expect(currentOnboardingStep(result.current.ctx)?.id).toBe('profile_type'));
    expect(setCoachingRole).not.toHaveBeenCalled();
  });

  it('never overrides a role already given', async () => {
    getOnboardingStatus.mockResolvedValue({ ...NEW_ACCOUNT, coaches_others: true });
    const { result } = renderHook(() => useOnboardingState({ invitedAsAthlete: true }), { wrapper });

    await waitFor(() => expect(result.current.ctx.coachesOthers).toBe(true));
    expect(setCoachingRole).not.toHaveBeenCalled();
  });
});

describe('useOnboardingState — the usage-analytics question (carnet#1366)', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    setOnboardingStep.mockResolvedValue(undefined);
    updateAnalyticsConsent.mockResolvedValue({ message: 'ok', enabled: true });
    localStorage.clear();
    client.clear();
    getOnboardingStatus.mockResolvedValue(
      connectedStatus([{ step_id: 'coach_proposal', status: 'complete' }]),
    );
  });

  afterEach(() => {
    sessionUser = SESSION_USER;
  });

  it('an onboarded account the server says was never asked meets the question', async () => {
    sessionUser = { ...SESSION_USER, analytics_consent_answered: false };

    const { result } = renderHook(() => useOnboardingState(), { wrapper });

    await waitFor(() => expect(result.current.ctx.needsProviderConnection).toBe(false));
    expect(result.current.ctx.analyticsConsentAnswered).toBe(false);
    expect(currentOnboardingStep(result.current.ctx)?.id).toBe('analytics_consent');
  });

  it('a session user that does not say counts as answered, so nothing is asked', async () => {
    const { result } = renderHook(() => useOnboardingState(), { wrapper });

    await waitFor(() => expect(result.current.ctx.needsProviderConnection).toBe(false));
    expect(result.current.ctx.analyticsConsentAnswered).toBe(true);
    expect(currentOnboardingStep(result.current.ctx)).toBeNull();
  });

  it.each([true, false])('an answer of %s is stored, then marks the session user answered', async (enabled) => {
    const { result } = renderHook(() => useOnboardingState(), { wrapper });

    await act(() => result.current.answerAnalyticsConsent(enabled));

    expect(updateAnalyticsConsent).toHaveBeenCalledWith(enabled);
    expect(updateUser).toHaveBeenCalledWith({
      analytics_consent: enabled,
      analytics_consent_answered: true,
    });
  });

  it('a failed write rejects and leaves the session user unanswered', async () => {
    updateAnalyticsConsent.mockRejectedValueOnce(new Error('503'));
    const { result } = renderHook(() => useOnboardingState(), { wrapper });

    await expect(result.current.answerAnalyticsConsent(false)).rejects.toThrow('503');
    expect(updateUser).not.toHaveBeenCalled();
  });
});

describe('useOnboardingState — the coach path: seat, group, invite', () => {
  /** A coach who does not train, past profile_type, with nothing connected. */
  const coachStatus = {
    needs_provider_connection: true,
    coaches_others: true,
    chosen_channel: null,
    steps: [
      { step_id: 'profile_type', status: 'complete' },
      { step_id: 'about_you', status: 'not_applicable' },
      { step_id: 'parq', status: 'not_applicable' },
    ],
  };

  beforeEach(() => {
    vi.clearAllMocks();
    sessionUser = SESSION_USER;
    setOnboardingStep.mockResolvedValue(undefined);
    getOnboardingStatus.mockResolvedValue(coachStatus);
    localStorage.clear();
    client.clear();
  });

  it('opens on the seat, moves to the group once past it, and remembers it on this device', async () => {
    const { result } = renderHook(() => useOnboardingState(), { wrapper });
    await waitFor(() => expect(currentOnboardingStep(result.current.ctx)?.id).toBe('coach_seat'));

    act(() => result.current.completeCoachSeat());

    expect(currentOnboardingStep(result.current.ctx)?.id).toBe('coach_group');
    expect(localStorage.getItem('dravr.coach_seat_done.u1')).toBe('1');
    // The seat itself is server state: no step row is written for it.
    expect(setOnboardingStep).not.toHaveBeenCalled();
  });

  it('a seat passed on this device is not shown again, not even for a frame', async () => {
    localStorage.setItem('dravr.coach_seat_done.u1', '1');
    const { result } = renderHook(() => useOnboardingState(), { wrapper });
    expect(result.current.ctx.coachSeatDone).toBe(true);
    await waitFor(() => expect(currentOnboardingStep(result.current.ctx)?.id).toBe('coach_group'));
  });

  it('the group made moves to the invite; the step is recorded only once the invite is behind', async () => {
    localStorage.setItem('dravr.coach_seat_done.u1', '1');
    const { result } = renderHook(() => useOnboardingState(), { wrapper });
    await waitFor(() => expect(currentOnboardingStep(result.current.ctx)?.id).toBe('coach_group'));

    act(() => result.current.markCoachGroupCreated());
    expect(currentOnboardingStep(result.current.ctx)?.id).toBe('coach_invite');
    expect(setOnboardingStep).not.toHaveBeenCalled();

    act(() => result.current.completeCoachGroup('complete'));
    expect(currentOnboardingStep(result.current.ctx)).toBeNull();
    expect(setOnboardingStep).toHaveBeenCalledWith('coach_group', 'complete');
  });

  it('a coach whose invitation settled the role and the seat starts on the group, both settled on the bar', async () => {
    getOnboardingStatus.mockResolvedValue({
      ...coachStatus,
      settled_by_invite: true,
      steps: [...coachStatus.steps, { step_id: 'coach_seat', status: 'complete' }],
    });
    const { result } = renderHook(() => useOnboardingState(), { wrapper });

    await waitFor(() => expect(currentOnboardingStep(result.current.ctx)?.id).toBe('coach_group'));
    expect(result.current.settledByInvite).toBe(true);
    const bar = onboardingProgress(result.current.ctx);
    expect(bar.map((s) => [s.id, s.status])).toEqual([
      ['profile_type', 'done'],
      ['coach_seat', 'done'],
      ['coach_group', 'current'],
      ['coach_invite', 'upcoming'],
    ]);
    // Nothing was written from this device: the server recorded both.
    expect(localStorage.getItem('dravr.coach_seat_done.u1')).toBeNull();
    expect(setOnboardingStep).not.toHaveBeenCalled();
  });

  it('a coach the server says no invitation settled is not told otherwise', async () => {
    const { result } = renderHook(() => useOnboardingState(), { wrapper });
    await waitFor(() => expect(currentOnboardingStep(result.current.ctx)?.id).toBe('coach_seat'));
    expect(result.current.settledByInvite).toBe(false);
  });
});
