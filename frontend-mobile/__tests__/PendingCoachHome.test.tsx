// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The phone's pending coach Home (carnet#1529) and where the Home tab routes a coach still waiting for coach access
// ABOUTME: Pins the server's rows, the seat request, the group made the onboarding way, the platforms, and only a training coach's switch

import React from 'react';
import { fireEvent, render, waitFor, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { CoachSetupResponse } from '@pierre/shared-types';

const mockPush = jest.fn();
jest.mock('expo-router', () =>
  require('../jest.expo-router').createExpoRouterMock({
    useRouter: () => ({ push: mockPush, replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true }),
  }),
);

const mockSession = { user: { id: 'coach-1', role: 'user', manages_roster: false } as Record<string, unknown> };
jest.mock('../src/contexts/AuthContext', () => ({
  useAuth: () => ({ user: mockSession.user, updateUser: jest.fn() }),
}));

const mockGetCoachSetup = jest.fn();
const mockGetCoachAccessRequest = jest.fn();
const mockRequestCoachAccess = jest.fn();
const mockGetOnboardingStatus = jest.fn();
const mockCreateGroup = jest.fn();
const mockCreateConversation = jest.fn();
const mockCreateInvite = jest.fn();
const mockListInvites = jest.fn();
const mockOpenGroupRoom = jest.fn();
jest.mock('../src/services/api', () => ({
  userApi: {
    getCoachSetup: () => mockGetCoachSetup(),
    getCoachAccessRequest: () => mockGetCoachAccessRequest(),
    requestCoachAccess: (groupId?: string) => mockRequestCoachAccess(groupId),
    cancelCoachAccessRequest: jest.fn(),
    getOnboardingStatus: () => mockGetOnboardingStatus(),
  },
  authApi: { getSession: jest.fn() },
  oauthApi: { getProvidersStatus: jest.fn().mockResolvedValue({ providers: [] }) },
  coachesApi: {
    list: jest.fn().mockResolvedValue({
      agents: [
        { id: 'agent-coach', title: 'Coach desk', tags: ['coach-tool'] },
        { id: 'agent-endurance', title: 'Endurance', tags: ['endurance'] },
      ],
    }),
  },
  groupsApi: {
    createGroup: (request: unknown) => mockCreateGroup(request),
    createInvite: (groupId: string, request: unknown) => mockCreateInvite(groupId, request),
    listInvites: (groupId: string) => mockListInvites(groupId),
    getGroup: jest.fn(),
  },
  chatApi: {
    createConversation: (options: unknown) => mockCreateConversation(options),
    openGroupRoom: (groupId: string) => mockOpenGroupRoom(groupId),
    openAthleteThread: jest.fn(),
  },
}));

// The screens behind the other branches of the Home tab, tested on their own.
jest.mock('../src/screens/home/HomeScreen', () => {
  const { Text } = require('react-native');
  return { HomeScreen: ({ scopeSwitch }: { scopeSwitch?: unknown }) => <Text testID="athlete-home">{scopeSwitch ? 'switch' : 'none'}</Text> };
});
jest.mock('../src/screens/coach-home/CoachHomeScreen', () => {
  const { Text } = require('react-native');
  return {
    CoachHomeScreen: ({ scopeSwitch }: { scopeSwitch?: unknown }) => (
      <Text testID="coach-home">{scopeSwitch ? 'switch' : 'none'}</Text>
    ),
  };
});

import { PendingCoachHomeScreen } from '../src/screens/coach-home/PendingCoachHomeScreen';
import { HomeRoot } from '../src/screens/home/HomeRoot';

// The human coach's own access and Home, in English.
const SEAT_TITLE = 'Get your coach access'; // humanCoach.seat.panelTitle
const SCOPE_PENDING = "Coach access pending · your coach Home opens as soon as it's on"; // humanCoach.pendingHome.scopePending

const WAIVED = [
  { step_id: 'about_you', status: 'not_applicable' },
  { step_id: 'parq', status: 'not_applicable' },
];

function setup(overrides: Partial<CoachSetupResponse> = {}): CoachSetupResponse {
  return {
    rows: [
      { id: 'seat', order: 1, done: false, current: true },
      { id: 'invite', order: 2, done: false, current: false },
      { id: 'platform', order: 3, done: false, current: false },
    ],
    done_count: 0,
    total: 3,
    groups: [],
    athlete: null,
    platform: null,
    ...overrides,
  };
}

function wrap(node: React.ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
}

beforeEach(() => {
  jest.clearAllMocks();
  mockSession.user = { id: 'coach-1', role: 'user', manages_roster: false };
  mockGetCoachSetup.mockResolvedValue(setup());
  mockGetCoachAccessRequest.mockResolvedValue({ request: null });
  mockGetOnboardingStatus.mockResolvedValue({ needs_provider_connection: true, steps: WAIVED, coaches_others: true });
  mockListInvites.mockResolvedValue({ invites: [] });
});

describe('PendingCoachHomeScreen', () => {
  it('lists the three steps with the seat current and the one filled request', async () => {
    const { findByTestId, getByTestId } = wrap(<PendingCoachHomeScreen />);

    expect(await findByTestId('coach-setup-count')).toHaveTextContent('0 of 3');
    expect(getByTestId('coach-setup-row-seat-current')).toHaveTextContent(SEAT_TITLE);
    expect(await findByTestId('coach-access-request')).toBeTruthy();
    expect(getByTestId('coach-setup-row-invite-open')).toHaveTextContent('Invite your athlete');
    // TrainingPeaks and Intervals.icu, the same rows Group info shows (D3).
    expect(getByTestId('coach-seat-route-sciotte_trainingpeaks')).toBeTruthy();
    expect(getByTestId('coach-seat-route-intervals_icu')).toBeTruthy();
    expect(getByTestId('home-title')).toBeTruthy();
  });

  it('asks in one tap, then dates the request and offers to withdraw it', async () => {
    mockRequestCoachAccess.mockResolvedValue({
      request: {
        id: 'req-1',
        user_id: 'coach-1',
        group_id: null,
        group_tenant_id: null,
        status: 'pending',
        created_at: '2026-11-12T19:32:00Z',
        decided_at: null,
        decided_by: null,
      },
    });
    const { findByTestId, getByTestId } = wrap(<PendingCoachHomeScreen />);

    fireEvent.press(await findByTestId('coach-access-request'));
    await waitFor(() => expect(mockRequestCoachAccess).toHaveBeenCalledWith(undefined));
    expect(await findByTestId('coach-seat-row-pending')).toBeTruthy();
    expect(getByTestId('coach-access-pending')).toHaveTextContent(/^Request sent .*\. We'll let you know here and by email\.$/);
    expect(getByTestId('coach-access-cancel')).toHaveTextContent('Cancel request');
    await waitFor(() =>
      expect(getByTestId('pending-coach-scope')).toHaveTextContent(SCOPE_PENDING),
    );
  });

  it('makes the first group the onboarding way, named after the athlete, on the endurance agent', async () => {
    mockCreateGroup.mockResolvedValue({ id: 'group-1', agent_id: 'agent-endurance', name: 'Léa' });
    mockCreateConversation.mockResolvedValue({ id: 'conv-1' });
    mockCreateInvite.mockResolvedValue({ code: 'JOIN-1' });
    const { findByTestId, getByTestId } = wrap(<PendingCoachHomeScreen />);

    fireEvent.changeText(await findByTestId('coach-setup-athlete'), 'Léa');
    await waitFor(() => expect(getByTestId('coach-setup-create')).not.toBeDisabled());
    fireEvent.press(getByTestId('coach-setup-create'));

    await waitFor(() =>
      expect(mockCreateGroup).toHaveBeenCalledWith({ name: 'Léa', agent_id: 'agent-endurance', coach_is_me: true }),
    );
    await waitFor(() => expect(mockGetCoachSetup).toHaveBeenCalledTimes(2));
  });

  it('ticks what the server says is done, and opens a group’s room', async () => {
    mockGetCoachSetup.mockResolvedValue(
      setup({
        rows: [
          { id: 'seat', order: 1, done: false, current: true },
          { id: 'invite', order: 2, done: true, current: false },
          { id: 'platform', order: 3, done: true, current: false },
        ],
        done_count: 2,
        groups: [{ id: 'group-1', name: 'Léa · Marathon' }],
        athlete: { name: 'Léa Gagnon', group_id: 'group-1', joined_at: '2026-11-10T09:00:00Z' },
        platform: 'intervals_icu',
      }),
    );
    mockOpenGroupRoom.mockResolvedValue({ id: 'room-1' });
    const { findByTestId, getByTestId } = wrap(<PendingCoachHomeScreen />);

    expect(await findByTestId('coach-setup-athlete-joined')).toHaveTextContent('Léa Gagnon joined your group on Nov 10.');
    expect(getByTestId('coach-setup-row-invite-done')).toHaveTextContent(/· done$/);
    expect(getByTestId('coach-setup-platform-linked')).toHaveTextContent('Intervals.icu is linked.');
    fireEvent.press(within(getByTestId('coach-setup-rooms')).getByTestId('coach-setup-room-group-1'));
    await waitFor(() => expect(mockOpenGroupRoom).toHaveBeenCalledWith('group-1'));
    await waitFor(() => expect(mockPush).toHaveBeenCalled());
  });
});

describe('HomeRoot', () => {
  it('lands a coach who does not train on the pending coach Home, with no switch', async () => {
    const { findByTestId, queryByTestId } = wrap(<HomeRoot />);
    expect(await findByTestId('pending-coach-home')).toBeTruthy();
    expect(queryByTestId('coach-home-switch')).toBeNull();
    expect(queryByTestId('athlete-home')).toBeNull();
  });

  it('gives a pending coach who trains the switch to their own Home', async () => {
    mockGetOnboardingStatus.mockResolvedValue({ needs_provider_connection: true, steps: [], coaches_others: true });
    const { findByTestId } = wrap(<HomeRoot />);
    expect(await findByTestId('pending-coach-home')).toBeTruthy();
    fireEvent.press(await findByTestId('coach-home-switch-me'));
    expect(await findByTestId('athlete-home')).toHaveTextContent('switch');
  });

  it('opens the coach Home once the session holds coach access', async () => {
    mockSession.user = { id: 'coach-1', role: 'user', manages_roster: true };
    const { findByTestId } = wrap(<HomeRoot />);
    expect(await findByTestId('coach-home')).toBeTruthy();
  });

  it('gives a coach with access who does not train no « Mon entraînement »', async () => {
    mockSession.user = { id: 'coach-1', role: 'user', manages_roster: true };
    const { findByTestId, getByTestId, queryByTestId } = wrap(<HomeRoot />);
    await findByTestId('coach-home');
    await waitFor(() => expect(getByTestId('coach-home')).toHaveTextContent('none'));
    expect(queryByTestId('athlete-home')).toBeNull();
  });

  it('keeps the switch for a coach with access who trains, and their own Home behind it', async () => {
    mockSession.user = { id: 'coach-1', role: 'user', manages_roster: true };
    mockGetOnboardingStatus.mockResolvedValue({ needs_provider_connection: false, steps: [], coaches_others: true });
    const { findByTestId } = wrap(<HomeRoot />);
    expect(await findByTestId('coach-home')).toHaveTextContent('switch');
  });

  it('leaves an athlete on the athlete Home', async () => {
    mockGetOnboardingStatus.mockResolvedValue({ needs_provider_connection: false, steps: [], coaches_others: false });
    const { findByTestId } = wrap(<HomeRoot />);
    expect(await findByTestId('athlete-home')).toHaveTextContent('none');
  });
});
