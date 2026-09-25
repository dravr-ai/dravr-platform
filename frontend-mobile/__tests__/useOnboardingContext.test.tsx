// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the one onboarding-context assembly the root gate and the progress hairline both read
// ABOUTME: Drives the real per-step hooks over a mocked status endpoint and the in-memory AsyncStorage

import React from 'react';
import { act, renderHook, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import AsyncStorage from '@react-native-async-storage/async-storage';
import { currentOnboardingStep } from '@pierre/shared-constants';

const mockAuth: { isAuthenticated: boolean; user: { id: string; role: string; user_status: string } | null } = {
  isAuthenticated: true,
  user: { id: 'u1', role: 'user', user_status: 'active' },
};
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => mockAuth,
}));

const mockGetOnboardingStatus = jest.fn();
const mockGetAvailableChannels = jest.fn();
jest.mock('../src/services/api', () => ({
  userApi: {
    getOnboardingStatus: (...args: unknown[]) => mockGetOnboardingStatus(...args),
    setOnboardingStep: jest.fn().mockResolvedValue(undefined),
  },
  messagingApi: {
    getAvailableChannels: (...args: unknown[]) => mockGetAvailableChannels(...args),
  },
}));

import { useOnboardingContext } from '../src/hooks/useOnboardingContext';
import { useOnboardingProgress } from '../src/hooks/useOnboardingProgress';

function status(needsProviderConnection: boolean) {
  return { needs_provider_connection: needsProviderConnection, steps: [], chosen_channel: null };
}

function setup() {
  // No garbage-collection timer, so nothing outlives the test that made it.
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: Infinity } } });
  const wrapper = ({ children }: { children: React.ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return { client, wrapper };
}

// The in-memory storage's own read, kept so a test that stalls reads can put it back.
const readItem = (AsyncStorage.getItem as jest.Mock).getMockImplementation();

afterEach(() => {
  (AsyncStorage.getItem as jest.Mock).mockImplementation(readItem);
});

async function markDone(...prefixes: string[]) {
  await AsyncStorage.multiSet(prefixes.map((prefix) => [`${prefix}u1`, '1'] as [string, string]));
}

describe('useOnboardingContext', () => {
  beforeEach(async () => {
    jest.clearAllMocks();
    await AsyncStorage.clear();
    mockAuth.isAuthenticated = true;
    mockAuth.user = { id: 'u1', role: 'user', user_status: 'active' };
    mockGetAvailableChannels.mockResolvedValue([]);
  });

  it('assembles a pre-connect record: the chosen profile type is done, about-you is current', async () => {
    mockGetOnboardingStatus.mockResolvedValue(status(true));
    await markDone('dravr.profile_type_chosen.');
    const { wrapper } = setup();

    const { result } = renderHook(() => useOnboardingContext(), { wrapper });

    await waitFor(() => {
      expect(result.current.context.needsProviderConnection).toBe(true);
      expect(result.current.context.profileTypeChosen).toBe(true);
      expect(result.current.settled).toBe(true);
    });
    expect(result.current.context).toEqual({
      onboardingActive: true,
      needsProviderConnection: true,
      skippedProvider: false,
      justOnboarded: false,
      profileTypeChosen: true,
      aboutYouDone: false,
      parqDone: false,
      coachProposalDone: false,
      messagingAvailableCount: 0,
      messagingChannelChosen: false,
      messagingChannelDone: false,
      messagingConfigureDone: false,
    });
    expect(currentOnboardingStep(result.current.context)?.id).toBe('about_you');
    // Messaging is post-connect; nothing asks for the channel list before it.
    expect(mockGetAvailableChannels).not.toHaveBeenCalled();
  });

  it('holds while a pre-connect flag is in flight, failing open meanwhile', async () => {
    mockGetOnboardingStatus.mockResolvedValue(status(true));
    (AsyncStorage.getItem as jest.Mock).mockImplementation(() => new Promise(() => undefined));
    const { wrapper } = setup();

    const { result } = renderHook(() => useOnboardingContext(), { wrapper });

    await waitFor(() => expect(result.current.context.needsProviderConnection).toBe(true));
    expect(result.current.settled).toBe(false);
    expect(result.current.context.profileTypeChosen).toBe(true);
    expect(result.current.context.aboutYouDone).toBe(true);
    expect(result.current.context.parqDone).toBe(true);
  });

  it('observes the first connect as justOnboarded and makes the coach proposal current', async () => {
    mockGetOnboardingStatus.mockResolvedValue(status(true));
    await markDone('dravr.profile_type_chosen.', 'dravr.about_you_done.', 'dravr.parq_done.');
    const { client, wrapper } = setup();

    const { result } = renderHook(() => useOnboardingContext(), { wrapper });
    await waitFor(() => expect(currentOnboardingStep(result.current.context)?.id).toBe('connect_provider'));
    expect(result.current.context.justOnboarded).toBe(false);

    // The OAuth round trip lands: the next read of the record says connected.
    mockGetOnboardingStatus.mockResolvedValue(status(false));
    await act(async () => {
      await client.invalidateQueries({ queryKey: ['user-onboarding-status'] });
    });

    await waitFor(() => {
      expect(result.current.context.justOnboarded).toBe(true);
      expect(result.current.settled).toBe(true);
    });
    expect(result.current.context.needsProviderConnection).toBe(false);
    expect(currentOnboardingStep(result.current.context)?.id).toBe('coach_proposal');
  });

  it('never intercepts a user whose record was already connected when the session began', async () => {
    mockGetOnboardingStatus.mockResolvedValue(status(false));
    const { wrapper } = setup();

    const { result } = renderHook(() => useOnboardingContext(), { wrapper });

    await waitFor(() => {
      expect(result.current.context.needsProviderConnection).toBe(false);
      expect(result.current.settled).toBe(true);
    });
    expect(result.current.context.justOnboarded).toBe(false);
    expect(currentOnboardingStep(result.current.context)).toBeNull();
  });

  it('exempts an admin: no status request, onboarding inactive, nothing to hold for', async () => {
    mockAuth.user = { id: 'u1', role: 'admin', user_status: 'active' };
    const { wrapper } = setup();

    const { result } = renderHook(() => useOnboardingContext(), { wrapper });

    await waitFor(() => expect(result.current.context.coachProposalDone).toBe(false));
    expect(result.current.context.onboardingActive).toBe(false);
    expect(result.current.context.needsProviderConnection).toBeUndefined();
    expect(result.current.settled).toBe(true);
    expect(mockGetOnboardingStatus).not.toHaveBeenCalled();
  });
});

describe('useOnboardingProgress', () => {
  beforeEach(async () => {
    jest.clearAllMocks();
    await AsyncStorage.clear();
    mockAuth.isAuthenticated = true;
    mockAuth.user = { id: 'u1', role: 'user', user_status: 'active' };
    mockGetAvailableChannels.mockResolvedValue([]);
  });

  it('reads the shared context: every pre-connect step behind, connect current on the connect screen', async () => {
    mockGetOnboardingStatus.mockResolvedValue(status(true));
    await markDone('dravr.profile_type_chosen.', 'dravr.about_you_done.', 'dravr.parq_done.');
    const { wrapper } = setup();

    const { result } = renderHook(() => useOnboardingProgress('connect_provider'), { wrapper });

    await waitFor(() =>
      expect(result.current.map(({ id, status: s }) => [id, s])).toEqual([
        ['profile_type', 'done'],
        ['about_you', 'done'],
        ['parq', 'done'],
        ['connect_provider', 'current'],
      ]),
    );
  });

  // The coach-proposal screen mounts after the first-connect transition, so
  // its own mount never sees it; the bar still shows the step as current.
  it('puts the coach proposal on the bar as the current step from its own screen', async () => {
    mockGetOnboardingStatus.mockResolvedValue(status(false));
    await markDone('dravr.profile_type_chosen.', 'dravr.about_you_done.', 'dravr.parq_done.');
    const { wrapper } = setup();

    const { result } = renderHook(() => useOnboardingProgress('coach_proposal'), { wrapper });

    await waitFor(() =>
      expect(result.current.map(({ id, status: s }) => [id, s])).toEqual([
        ['profile_type', 'done'],
        ['about_you', 'done'],
        ['parq', 'done'],
        ['connect_provider', 'done'],
        ['coach_proposal', 'current'],
      ]),
    );
  });
});
