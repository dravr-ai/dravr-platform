// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What the Home tab opens — an athlete's Home as ever, or for a coach who manages a roster the "Mes athlètes / Mon entraînement" switch
// ABOUTME: Pins that a coach lands on their athletes, that the switch reaches their own Home and back, and that an athlete never asks the roster

import React from 'react';
import { fireEvent, render } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { ROSTER } from '../integration/app/helpers/coachRosterFixtures';

jest.mock('expo-router', () =>
  require('../jest.expo-router').createExpoRouterMock({
    useRouter: () => ({ push: jest.fn(), replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true }),
  }),
);

let mockManagesRoster: boolean | undefined = true;
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ user: { id: 'u1', manages_roster: mockManagesRoster }, isAuthenticated: true }),
}));

const mockGetRoster = jest.fn();
jest.mock('../src/services/api', () => ({
  coachRosterApi: { getRoster: (q?: unknown) => mockGetRoster(q), dismissAttention: jest.fn(), restoreAttention: jest.fn() },
  chatApi: { getConversations: jest.fn().mockResolvedValue({ conversations: [], total: 0 }), openAthleteThread: jest.fn() },
}));

// The athlete's Home is its own suite's business; here it is a marker that shows the switch it was handed.
jest.mock('../src/screens/home/HomeScreen', () => {
  const { View, Text: T } = require('react-native');
  return {
    HomeScreen: ({ scopeSwitch }: { scopeSwitch?: React.ReactNode }) => (
      <View testID="home-screen">
        {scopeSwitch}
        <T>athlete home</T>
      </View>
    ),
  };
});

import { HomeRoot } from '../src/screens/home/HomeRoot';

function renderRoot() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  return render(
    <QueryClientProvider client={client}>
      <HomeRoot />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  jest.clearAllMocks();
  mockManagesRoster = true;
  mockGetRoster.mockResolvedValue(ROSTER);
});

describe('HomeRoot', () => {
  it('lands a coach who manages a roster on their athletes, "Mes athlètes" pressed', async () => {
    const screen = renderRoot();
    await screen.findByTestId('coach-queue');
    expect(screen.getByTestId('coach-home')).toBeTruthy();
    expect(screen.getByTestId('coach-home-switch-athletes').props.accessibilityState.selected).toBe(true);
    expect(screen.getByTestId('coach-home-switch-me').props.accessibilityState.selected).toBe(false);
    expect(screen.queryByTestId('home-screen')).toBeNull();
  });

  it('switches to their own training, the same Home an athlete has, and back', async () => {
    const screen = renderRoot();
    await screen.findByTestId('coach-queue');
    fireEvent.press(screen.getByTestId('coach-home-switch-me'));
    expect(screen.getByTestId('home-screen')).toBeTruthy();
    expect(screen.queryByTestId('coach-home')).toBeNull();
    // The switch stays at the top of their own Home, with the other side pressed.
    expect(screen.getByTestId('coach-home-switch-me').props.accessibilityState.selected).toBe(true);
    fireEvent.press(screen.getByTestId('coach-home-switch-athletes'));
    await screen.findByTestId('coach-queue');
    expect(screen.queryByTestId('home-screen')).toBeNull();
  });

  it('shows the same words on the switch as the web: Mes athlètes, Mon entraînement', async () => {
    const screen = renderRoot();
    await screen.findByTestId('coach-queue');
    expect(screen.getByTestId('coach-home-switch-athletes')).toHaveTextContent(/athletes/i);
    expect(screen.getByTestId('coach-home-switch-me')).toHaveTextContent(/training/i);
  });

  it.each([[false], [undefined]])('opens the athlete Home alone, with no switch and no roster read, for manages_roster %s', (flag) => {
    mockManagesRoster = flag;
    const screen = renderRoot();
    expect(screen.getByTestId('home-screen')).toBeTruthy();
    expect(screen.queryByTestId('coach-home-switch')).toBeNull();
    expect(mockGetRoster).not.toHaveBeenCalled();
  });
});
