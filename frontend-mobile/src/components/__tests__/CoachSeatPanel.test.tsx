// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The phone's onboarding seat panel on the shared seat probe (carnet#1529) — a link made on the screen still reads the grant back
// ABOUTME: Pins that the session is read once a coaching platform connects, the provider cards are asked for again, and the step says the coach coaches

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { QUERY_KEYS, type CoachSeatProvider } from '@pierre/shared-constants';

// One session user for every reader — the panel and the shared probe — so a patch re-renders the panel.
const mockAuth = { managesRoster: false, listeners: new Set<() => void>() };
const mockUpdateUser = jest.fn();
jest.mock('../../contexts/AuthContext', () => {
  const { useSyncExternalStore } = require('react');
  const subscribe = (listener: () => void) => {
    mockAuth.listeners.add(listener);
    return () => mockAuth.listeners.delete(listener);
  };
  return {
    useAuth: () => {
      const managesRoster = useSyncExternalStore(subscribe, () => mockAuth.managesRoster);
      return {
        user: { id: 'u1', manages_roster: managesRoster },
        updateUser: async (patch: { manages_roster?: boolean }) => {
          mockUpdateUser(patch);
          if (patch.manages_roster !== undefined) {
            mockAuth.managesRoster = patch.manages_roster;
            mockAuth.listeners.forEach((listener) => listener());
          }
        },
      };
    },
  };
});

const mockGetSession = jest.fn();
const mockGetCoachAccessRequest = jest.fn();
jest.mock('../../services/api', () => ({
  userApi: {
    getCoachAccessRequest: () => mockGetCoachAccessRequest(),
    requestCoachAccess: jest.fn(),
  },
  authApi: { getSession: () => mockGetSession() },
  groupsApi: { getGroup: jest.fn() },
}));

import { CoachSeatPanel } from '../CoachSeatPanel';

const card = (connected: boolean, account_role?: 'athlete' | 'coach'): CoachSeatProvider => ({
  provider: 'sciotte_trainingpeaks',
  display_name: 'TrainingPeaks',
  connected,
  ...(account_role ? { account_role } : {}),
});

describe('CoachSeatPanel', () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockAuth.managesRoster = false;
    mockGetCoachAccessRequest.mockResolvedValue({ request: null });
    mockGetSession.mockResolvedValue({ user: { manages_roster: false } });
  });

  it('a TrainingPeaks link made on the screen still reads the grant back, and asks for the cards again', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
    const invalidate = jest.spyOn(client, 'invalidateQueries');
    const tree = (providers: CoachSeatProvider[]) => (
      <QueryClientProvider client={client}>
        <CoachSeatPanel providers={providers} onDone={jest.fn()} />
      </QueryClientProvider>
    );
    const view = render(tree([card(false)]));
    expect(await view.findByTestId('coach-seat-ask')).toBeTruthy();
    expect(mockGetSession).not.toHaveBeenCalled();

    // The link answers: a coach account on the verified email earns coach access.
    mockGetSession.mockResolvedValue({ user: { manages_roster: true } });
    view.rerender(tree([card(true, 'coach')]));

    await waitFor(() => expect(mockUpdateUser).toHaveBeenCalledWith({ manages_roster: true }));
    expect(invalidate).toHaveBeenCalledWith({ queryKey: QUERY_KEYS.providers.status() });
    expect(await view.findByTestId('coach-seat-status-coach')).toBeTruthy();
  });
});
