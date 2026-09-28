// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: e2e — the reconnect banner over one stubbed server: in an open chat thread, once above the tabs, never on Home's section
// ABOUTME: The thread mounts the banner itself, and its header names only providers still syncing; all read one /api/providers answer

import React from 'react';
import { fireEvent, render, waitFor, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactTestInstance } from 'react-test-renderer';

import { installHttpStub, type HttpStub, type StubRoutes } from './helpers/httpStub';
import {
  LATEST_ROUTE_RESPONSE,
  NO_GPS_ROUTE_RESPONSE,
  PLAN_RESPONSE,
  PROVIDERS_CONNECTED,
  PROVIDERS_ONLY_FLAGGED,
  PROVIDERS_RECONNECT,
  TRAIL_ROUTE_RESPONSE,
  recentResponse,
} from './helpers/homeFixtures';

const mockPush = jest.fn();
jest.mock('expo-router', () =>
  require('../../jest.expo-router').createExpoRouterMock({
    useRouter: () => ({
      push: mockPush,
      replace: jest.fn(),
      back: jest.fn(),
      navigate: jest.fn(),
      canGoBack: () => true,
    }),
    useLocalSearchParams: () => ({ conversationId: 'new' }),
    usePathname: () => '/chat/new',
    useIsFocused: () => true,
  }),
);
jest.mock('expo-router/unstable-native-tabs', () =>
  require('../../jest.expo-router').createNativeTabsMock(),
);
// No navigator under a test, so the header the thread's column offsets by is 0 tall.
jest.mock('expo-router/react-navigation', () => ({
  ...jest.requireActual('expo-router/react-navigation'),
  useHeaderHeight: () => 0,
}));
jest.mock('../../src/hooks/useServerStatus', () => ({
  useServerStatus: () => ({ isServerReachable: true, isChecking: false, checkNow: jest.fn() }),
}));
jest.mock('../../src/contexts/AuthContext', () => ({
  useAuth: () => ({ isAuthenticated: true, isLoading: false, user: { id: 'user-1' } }),
}));

import { ChatScreen } from '../../src/screens/chat/ChatScreen';
import { HomeScreen } from '../../src/screens/home/HomeScreen';
import TabsLayout from '../../app/(app)/(tabs)/_layout';

const PROVIDERS_PATH = '/api/providers';

/** What an empty thread and the tab shell read, with `providers` as the status. */
function server(providers: typeof PROVIDERS_CONNECTED): StubRoutes {
  return {
    'GET /api/providers': { data: providers },
    'GET /api/chat/conversations?limit=50&offset=0': {
      data: { conversations: [], total: 0, limit: 50, offset: 0 },
    },
    'GET /api/notifications/unread-count': { data: { unread_count: 0 } },
    'GET /api/me/training-plan?locale=en': { data: PLAN_RESPONSE },
    'GET /api/me/activities/recent': { data: recentResponse() },
    'GET /api/me/activities/strava/9001/route': { data: LATEST_ROUTE_RESPONSE },
    'GET /api/me/activities/intervals_icu/i77/route': { data: TRAIL_ROUTE_RESPONSE },
    'GET /api/me/activities/strava/8998/route': { data: NO_GPS_ROUTE_RESPONSE },
  };
}

function renderWith(children: React.ReactNode) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 }, mutations: { retry: false } },
  });
  return render(<QueryClientProvider client={client}>{children}</QueryClientProvider>);
}

describe('the reconnect banner across the app', () => {
  let stub: HttpStub;

  beforeEach(() => {
    mockPush.mockClear();
  });

  afterEach(() => {
    stub.restore();
  });

  // Turns red if the thread stops mounting the banner: the tab shell's copy
  // sits under the pushed thread, where nobody sees it.
  it('names every connection to reconnect inside an open thread, and leads to Connections', async () => {
    stub = installHttpStub(server(PROVIDERS_RECONNECT));
    const screen = renderWith(<ChatScreen />);

    const banner = await screen.findByTestId('reconnect-banner');
    expect(within(banner).getByTestId('reconnect-banner-providers')).toHaveTextContent(
      'Reconnect Strava, Garmin to see your new activities.',
    );
    expect(screen.getAllByTestId('reconnect-banner')).toHaveLength(1);
    // The thread is under the native header, which already clears the status
    // bar: the banner adds only its own padding.
    const strip = banner.children[0];
    expect(typeof strip).not.toBe('string');
    expect((strip as ReactTestInstance).props.style).toEqual({ paddingTop: 12 });

    fireEvent.press(within(banner).getByTestId('reconnect-banner-action'));
    expect(mockPush).toHaveBeenCalledWith('/(app)/(tabs)/(settings)/connections');
  });

  it('draws nothing in a thread while every connection is healthy', async () => {
    stub = installHttpStub(server(PROVIDERS_CONNECTED));
    const screen = renderWith(<ChatScreen />);

    await screen.findByTestId('chat-screen');
    await waitFor(() =>
      expect(stub.requests.some((request) => request.url === PROVIDERS_PATH)).toBe(true),
    );
    expect(screen.queryByTestId('reconnect-banner')).toBeNull();
  });

  // The header under the thread's title must not call a flagged connection
  // connected while the banner right below it says to reconnect it.
  it('names only the providers still syncing in the thread header, never a flagged one', async () => {
    stub = installHttpStub(server(PROVIDERS_RECONNECT));
    const screen = renderWith(<ChatScreen />);

    expect(await screen.findByTestId('chat-header-provider-status')).toHaveTextContent(
      'Intervals.icu connected',
    );
    expect(screen.queryByText(/Strava.*connected|Garmin.*connected/)).toBeNull();
  });

  it('says a reconnect is needed in the thread header when every connected provider is flagged', async () => {
    stub = installHttpStub(server(PROVIDERS_ONLY_FLAGGED));
    const screen = renderWith(<ChatScreen />);

    expect(await screen.findByTestId('chat-header-provider-status')).toHaveTextContent('Reconnect needed');
    expect(await screen.findByTestId('reconnect-banner')).toBeTruthy();
  });

  it('keeps the connected wording in the thread header while every connection is healthy', async () => {
    stub = installHttpStub(server(PROVIDERS_CONNECTED));
    const screen = renderWith(<ChatScreen />);

    expect(await screen.findByTestId('chat-header-provider-status')).toHaveTextContent('Strava connected');
  });

  // Home under the shell: the shell's banner is the only place the reconnect
  // is said — the section neither repeats it nor draws a second banner.
  it('shows one banner above the tabs on Home, and Home does not say it again', async () => {
    stub = installHttpStub(server(PROVIDERS_RECONNECT));
    const screen = renderWith(
      <>
        <TabsLayout />
        <HomeScreen />
      </>,
    );

    await screen.findByTestId('home-activity-strava-9001');
    await screen.findByTestId('reconnect-banner');
    expect(screen.getAllByTestId('reconnect-banner')).toHaveLength(1);
    expect(screen.getAllByText('Reconnect needed')).toHaveLength(1);
    expect(screen.getAllByText(/Reconnect Strava, Garmin/)).toHaveLength(1);
    // One status read feeds the banner and Home.
    expect(stub.requests.filter((request) => request.url === PROVIDERS_PATH)).toHaveLength(1);
  });
});
