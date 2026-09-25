// ABOUTME: e2e — the Home tab over one stubbed server: the three /api/me reads on the wire, the map, the sketches, the drafts
// ABOUTME: The real api-client parses every body, so a response that breaks the contract shows an error, never a half-drawn card

import React from 'react';
import { act, fireEvent, render, waitFor, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { HOME_STALE_REFETCH_DELAY_MS } from '@pierre/shared-constants';

import { installHttpStub, type HttpStub, type StubRoutes } from './helpers/httpStub';
import {
  ACTIVITIES,
  LATEST_ROUTE_RESPONSE,
  PLAN_RESPONSE,
  PROVIDERS_NONE,
  TRAIL_ROUTE_RESPONSE,
  recentResponse,
} from './helpers/homeFixtures';

const mockPush = jest.fn();
const mockNavigate = jest.fn();
jest.mock('expo-router', () =>
  require('../../jest.expo-router').createExpoRouterMock({
    useRouter: () => ({
      push: mockPush,
      replace: jest.fn(),
      back: jest.fn(),
      navigate: mockNavigate,
      canGoBack: () => true,
    }),
  }),
);
jest.mock('expo-router/unstable-native-tabs', () =>
  require('../../jest.expo-router').createNativeTabsMock(),
);
jest.mock('../../src/hooks/useServerStatus', () => ({
  useServerStatus: () => ({ isServerReachable: true, isChecking: false, checkNow: jest.fn() }),
}));

import { HomeScreen } from '../../src/screens/home/HomeScreen';
import { ConversationsScreen } from '../../src/screens/conversations/ConversationsScreen';
import TabsLayout from '../../app/(app)/(tabs)/_layout';
import { HOME_ROUTE } from '../../src/navigation/routes';

const PLAN_URL = 'GET /api/me/training-plan?locale=en';
const RECENT_URL = 'GET /api/me/activities/recent';
const LATEST_ROUTE_URL = 'GET /api/me/activities/strava/9001/route';
const TRAIL_ROUTE_URL = 'GET /api/me/activities/intervals_icu/i77/route';

/** The server as a Home visit finds it: a plan, five activities, two stored routes. */
function homeServer(overrides: StubRoutes = {}): StubRoutes {
  return {
    [PLAN_URL]: { data: PLAN_RESPONSE },
    [RECENT_URL]: { data: recentResponse() },
    [LATEST_ROUTE_URL]: { data: LATEST_ROUTE_RESPONSE },
    [TRAIL_ROUTE_URL]: { data: TRAIL_ROUTE_RESPONSE },
    ...overrides,
  };
}

function renderHome() {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: 0, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
  return render(
    <QueryClientProvider client={client}>
      <HomeScreen />
    </QueryClientProvider>,
  );
}

describe('the Home tab over the wire', () => {
  let stub: HttpStub;

  beforeEach(() => {
    mockPush.mockClear();
    mockNavigate.mockClear();
  });

  afterEach(() => {
    stub.restore();
    jest.useRealTimers();
  });

  // Turns red if a client stops calling the routes the server serves, adds a
  // `limit` the server would clamp, drops the locale, or asks a provider for a
  // route the row's own polyline already draws.
  it('reads the plan, the list and only the routes it cannot draw from a polyline', async () => {
    stub = installHttpStub(homeServer());
    const screen = renderHome();

    expect(await screen.findByTestId('route-track')).toBeTruthy();
    await screen.findByTestId('home-activity-sketch-intervals_icu-i77');

    const urls = stub.requestsFor('GET').map((request) => request.url).sort();
    expect(urls).toEqual(
      [
        '/api/me/activities/intervals_icu/i77/route',
        '/api/me/activities/recent',
        '/api/me/activities/strava/9001/route',
        '/api/me/training-plan?locale=en',
      ].sort(),
    );
    // All three are reads: none of them carries a body.
    expect(stub.requests.every((request) => request.body === undefined)).toBe(true);
  });

  it('draws today from the server\'s own today and drafts a chat about it', async () => {
    stub = installHttpStub(homeServer());
    const screen = renderHome();

    const today = await screen.findByTestId('home-today');
    expect(within(today).getByText('Tempo run')).toBeTruthy();
    expect(screen.getByTestId('home-today-phase')).toHaveTextContent('Build · week 3');

    fireEvent.press(screen.getByTestId('home-today-day'));
    expect(mockPush).toHaveBeenCalledWith({
      pathname: '/(app)/chat/[conversationId]',
      params: {
        conversationId: 'new',
        draft: 'Walk me through my session on Thursday, September 24: Tempo run',
      },
    });
  });

  it('encodes a provider id that is not URL-safe into the route path', async () => {
    const odd = { ...ACTIVITIES[0], id: 'run/2026 09 20' };
    stub = installHttpStub(
      homeServer({
        [RECENT_URL]: { data: recentResponse({ activities: [odd] }) },
        'GET /api/me/activities/strava/run%2F2026%2009%2020/route': { data: LATEST_ROUTE_RESPONSE },
      }),
    );
    const screen = renderHome();

    expect(await screen.findByTestId('route-track')).toBeTruthy();
  });

  it('shows the list as failed when one row breaks the contract, rather than drawing the others', async () => {
    const broken = { ...ACTIVITIES[1], has_gps: 'yes' };
    stub = installHttpStub(
      homeServer({ [RECENT_URL]: { data: { ...recentResponse(), activities: [ACTIVITIES[0], broken] } } }),
    );
    const screen = renderHome();

    expect(await screen.findByTestId('home-activities-error')).toHaveTextContent(
      /^Your recent activities couldn't be loaded\./,
    );
    expect(screen.queryByTestId('home-activity-strava-9001')).toBeNull();
  });

  it('shows a plan body without its plan key as a failure, never as "no plan"', async () => {
    stub = installHttpStub(homeServer({ [PLAN_URL]: { data: { today: '2026-09-24' } } }));
    const screen = renderHome();

    expect(await screen.findByTestId('home-plan-error')).toBeTruthy();
    expect(screen.queryByTestId('home-plan-build')).toBeNull();
  });

  it('shows a server error on the plan as a failure too', async () => {
    stub = installHttpStub(homeServer({ [PLAN_URL]: { status: 500, data: { error: 'store read failed' } } }));
    const screen = renderHome();

    expect(await screen.findByTestId('home-plan-error')).toBeTruthy();
    expect(screen.queryByTestId('home-plan-build')).toBeNull();
  });

  it('offers to build a plan when the server says there is none', async () => {
    stub = installHttpStub(homeServer({ [PLAN_URL]: { data: { plan: null, today: '2026-09-24' } } }));
    const screen = renderHome();

    fireEvent.press(await screen.findByTestId('home-plan-build'));
    expect(mockPush).toHaveBeenCalledWith({
      pathname: '/(app)/chat/[conversationId]',
      params: { conversationId: 'new', draft: 'Build me a training plan for my goal race.' },
    });
  });

  it('says why the latest has no map when the server refuses its route', async () => {
    stub = installHttpStub(
      homeServer({
        [RECENT_URL]: { data: recentResponse({ activities: [ACTIVITIES[0]] }) },
        [LATEST_ROUTE_URL]: { data: { route: null, reason: 'no_gps' } },
      }),
    );
    const screen = renderHome();

    expect(await screen.findByTestId('home-latest-no-track')).toHaveTextContent(
      'This activity recorded no GPS track.',
    );
  });

  it('says the map failed when the route is not the caller\'s', async () => {
    stub = installHttpStub(
      homeServer({
        [RECENT_URL]: { data: recentResponse({ activities: [ACTIVITIES[0]] }) },
        [LATEST_ROUTE_URL]: { status: 404, data: { error: 'not found' } },
      }),
    );
    const screen = renderHome();

    expect(await screen.findByTestId('home-latest-map-failed')).toBeTruthy();
  });

  it('points an athlete with nothing connected at Connections', async () => {
    stub = installHttpStub(
      homeServer({
        [RECENT_URL]: { data: recentResponse({ activities: [], as_of: null }) },
        'GET /api/providers': { data: PROVIDERS_NONE },
      }),
    );
    const screen = renderHome();

    expect(await screen.findByTestId('home-activities-no-provider')).toBeTruthy();
    fireEvent.press(screen.getByTestId('home-activities-connect'));
    expect(mockPush).toHaveBeenCalledWith('/(app)/(tabs)/(settings)/connections');
  });

  // The one follow-up read: after the delay, once, and the second answer is
  // what the page shows — here the refreshed cache with a sixth activity.
  it('asks once more for a stale list, after the delay, and draws the fresher answer', async () => {
    jest.useFakeTimers();
    const fresher = {
      ...ACTIVITIES[0],
      id: '9002',
      name: 'Recovery spin',
      start_date: '2026-09-23T15:00:00Z',
    };
    let reads = 0;
    stub = installHttpStub(
      homeServer({
        [RECENT_URL]: () => {
          reads += 1;
          return reads === 1
            ? { data: recentResponse({ stale: true, as_of: '2026-09-22T06:00:00Z' }) }
            : { data: recentResponse({ activities: [fresher, ...ACTIVITIES.slice(0, 4)], stale: false }) };
        },
        'GET /api/me/activities/strava/9002/route': { data: LATEST_ROUTE_RESPONSE },
      }),
    );
    const screen = renderHome();

    expect(await screen.findByTestId('home-activities-refreshing')).toBeTruthy();
    expect(reads).toBe(1);

    await act(async () => {
      jest.advanceTimersByTime(HOME_STALE_REFETCH_DELAY_MS);
    });

    expect(await screen.findByTestId('home-activity-strava-9002')).toBeTruthy();
    expect(reads).toBe(2);
    expect(screen.queryByTestId('home-activities-refreshing')).toBeNull();

    await act(async () => {
      jest.advanceTimersByTime(HOME_STALE_REFETCH_DELAY_MS * 4);
    });
    expect(reads).toBe(2);
  });

  it('shows when it last synced once the follow-up is still stale', async () => {
    jest.useFakeTimers();
    stub = installHttpStub(
      homeServer({ [RECENT_URL]: { data: recentResponse({ stale: true, as_of: '2026-09-22T06:00:00Z' }) } }),
    );
    const screen = renderHome();

    await screen.findByTestId('home-activities-refreshing');
    await act(async () => {
      jest.advanceTimersByTime(HOME_STALE_REFETCH_DELAY_MS);
    });

    expect(await screen.findByTestId('home-activities-synced-at')).toHaveTextContent(/^Last synced: /);
    expect(stub.requestsFor('GET').filter((request) => request.url === '/api/me/activities/recent')).toHaveLength(2);
  });
});

describe('the way back Home', () => {
  let stub: HttpStub;

  afterEach(() => {
    stub.restore();
  });

  it('puts Home first in the tab bar, and the chat header lockup leads there', async () => {
    stub = installHttpStub({
      'GET /api/chat/conversations?limit=50&offset=0': {
        data: { conversations: [], total: 0, limit: 50, offset: 0 },
      },
      'GET /api/notifications/unread-count': { data: { unread_count: 0 } },
    });
    const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
    const screen = render(
      <QueryClientProvider client={client}>
        <ConversationsScreen />
        <TabsLayout />
      </QueryClientProvider>,
    );

    const labels = (await screen.findAllByTestId('tab-label')).map((node) => node.props.children);
    expect(labels[0]).toBe('Home');

    const lockup = await screen.findByTestId('conversations-title');
    expect(lockup.props.accessibilityLabel).toBe('Home');
    fireEvent.press(lockup);
    await waitFor(() => expect(mockNavigate).toHaveBeenCalledWith(HOME_ROUTE));
  });
});
