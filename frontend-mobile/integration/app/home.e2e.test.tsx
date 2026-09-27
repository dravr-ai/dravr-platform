// ABOUTME: e2e — the Home tab over one stubbed server: the /api/me reads and the provider status on the wire, the map, the sketches, the drafts
// ABOUTME: The real api-client parses every body, so a response that breaks the contract shows an error, never a half-drawn card

import React from 'react';
import { act, fireEvent, render, waitFor, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { HOME_STALE_REFETCH_DELAYS_MS } from '@pierre/shared-constants';

import { installHttpStub, type HttpStub, type StubRoutes } from './helpers/httpStub';
import {
  ACTIVITIES,
  LATEST_ROUTE_RESPONSE,
  NO_GPS_ROUTE_RESPONSE,
  PLAN_RESPONSE,
  PROVIDERS_CONNECTED,
  PROVIDERS_NONE,
  PROVIDERS_RECONNECT,
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
const GYM_ROUTE_URL = 'GET /api/me/activities/strava/8998/route';
const PROVIDERS_URL = 'GET /api/providers';
const RECENT_PATH = '/api/me/activities/recent';

/** The whole follow-up schedule, first answer to last ask. */
const SCHEDULE_MS = HOME_STALE_REFETCH_DELAYS_MS.reduce((sum, delay) => sum + delay, 0);

/**
 * The server as a Home visit finds it: a plan, five activities, the answer to
 * each route read the page makes, and one healthy connection.
 */
function homeServer(overrides: StubRoutes = {}): StubRoutes {
  return {
    [PLAN_URL]: { data: PLAN_RESPONSE },
    [RECENT_URL]: { data: recentResponse() },
    [LATEST_ROUTE_URL]: { data: LATEST_ROUTE_RESPONSE },
    [TRAIL_ROUTE_URL]: { data: TRAIL_ROUTE_RESPONSE },
    [GYM_ROUTE_URL]: { data: NO_GPS_ROUTE_RESPONSE },
    [PROVIDERS_URL]: { data: PROVIDERS_CONNECTED },
    ...overrides,
  };
}

/**
 * Let the fake clock run, then what it started settle: the read, its answer,
 * and the render after it, which React Query puts on a zero-delay timeout of
 * its own — a millisecond away on a fake clock.
 */
async function elapse(ms: number) {
  await act(async () => {
    await jest.advanceTimersByTimeAsync(ms);
  });
  await act(async () => {
    await jest.advanceTimersByTimeAsync(1);
  });
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
  // `limit` the server would clamp, drops the locale, asks for a route the
  // row's own polyline already draws, or asks again for one the row says
  // held no GPS.
  it('reads the plan, the list, the provider status and only the routes it has to ask for', async () => {
    stub = installHttpStub(homeServer());
    const screen = renderHome();

    expect(await screen.findByTestId('route-track')).toBeTruthy();
    await screen.findByTestId('home-activity-sketch-intervals_icu-i77');
    await waitFor(() => expect(stub.requestsFor('GET')).toHaveLength(6));

    const urls = stub.requestsFor('GET').map((request) => request.url).sort();
    expect(urls).toEqual(
      [
        '/api/me/activities/intervals_icu/i77/route',
        '/api/me/activities/recent',
        '/api/me/activities/strava/8998/route',
        '/api/me/activities/strava/9001/route',
        '/api/me/training-plan?locale=en',
        '/api/providers',
      ].sort(),
    );
    // Every one is a read: none of them carries a body.
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

  it('asks for the route of a latest activity whose row carries no position, and draws it', async () => {
    expect(ACTIVITIES[2]).toMatchObject({ has_gps: true, summary_polyline: null });
    stub = installHttpStub(homeServer({ [RECENT_URL]: { data: recentResponse({ activities: [ACTIVITIES[2]] }) } }));
    const screen = renderHome();

    const track = (await screen.findByTestId('route-track')).props.data as { coordinates: number[][] };
    expect(track.coordinates[0]).toEqual([-74.2, 46.1]);
    expect(screen.queryByTestId('home-latest-no-track')).toBeNull();
    expect(stub.requestsFor('GET').map((request) => request.url)).toContain(
      '/api/me/activities/intervals_icu/i77/route',
    );
  });

  it('says a never-read latest activity recorded no track once its route read says so', async () => {
    expect(ACTIVITIES[4]).toMatchObject({ has_gps: true, summary_polyline: null });
    stub = installHttpStub(homeServer({ [RECENT_URL]: { data: recentResponse({ activities: [ACTIVITIES[4]] }) } }));
    const screen = renderHome();

    expect(await screen.findByTestId('home-latest-no-track')).toHaveTextContent(
      'This activity recorded no GPS track.',
    );
    expect(stub.requestsFor('GET').map((request) => request.url)).toContain(
      '/api/me/activities/strava/8998/route',
    );
  });

  it('puts no route read on the wire for a row that says its route held no GPS', async () => {
    expect(ACTIVITIES[3].has_gps).toBe(false);
    stub = installHttpStub(homeServer({ [RECENT_URL]: { data: recentResponse({ activities: [ACTIVITIES[3]] }) } }));
    const screen = renderHome();

    expect(await screen.findByTestId('home-latest-no-track')).toHaveTextContent(
      'This activity recorded no GPS track.',
    );
    await waitFor(() =>
      expect(stub.requestsFor('GET').map((request) => request.url)).toContain('/api/providers'),
    );
    expect(stub.requestsFor('GET').filter((request) => request.url.endsWith('/route'))).toEqual([]);
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

  it('names the connections to reconnect above the rows it still shows, and leads to Connections', async () => {
    stub = installHttpStub(homeServer({ [PROVIDERS_URL]: { data: PROVIDERS_RECONNECT } }));
    const screen = renderHome();

    expect(await screen.findByTestId('home-reconnect-provider')).toHaveTextContent(
      'Reconnect Strava, Garmin to see your new activities.Reconnect',
    );
    expect(await screen.findByTestId('home-activity-strava-9001')).toBeTruthy();
    expect(screen.queryByTestId('home-activities-no-provider')).toBeNull();

    fireEvent.press(screen.getByTestId('home-activities-reconnect'));
    expect(mockPush).toHaveBeenCalledWith('/(app)/(tabs)/(settings)/connections');
  });

  it('names them in place of the empty sentence when the cache holds no rows', async () => {
    stub = installHttpStub(
      homeServer({
        [RECENT_URL]: { data: recentResponse({ activities: [] }) },
        [PROVIDERS_URL]: { data: PROVIDERS_RECONNECT },
      }),
    );
    const screen = renderHome();

    expect(await screen.findByTestId('home-reconnect-provider')).toBeTruthy();
    expect(screen.queryByTestId('home-activities-empty')).toBeNull();
    expect(screen.queryByTestId('home-activities-connect')).toBeNull();
  });

  // The follow-up schedule: an ask after each delay while the answer is still
  // stale, and the first answer that is not is what the page shows — here the
  // refreshed cache with a sixth activity, on the second follow-up.
  it('asks again on the schedule until the list has caught up, and draws the fresher answer', async () => {
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
          return reads < 3
            ? { data: recentResponse({ stale: true, as_of: '2026-09-22T06:00:00Z' }) }
            : { data: recentResponse({ activities: [fresher, ...ACTIVITIES.slice(0, 4)], stale: false }) };
        },
        'GET /api/me/activities/strava/9002/route': { data: LATEST_ROUTE_RESPONSE },
      }),
    );
    const screen = renderHome();

    expect(await screen.findByTestId('home-activities-refreshing')).toBeTruthy();
    expect(reads).toBe(1);

    // The first follow-up finds the server still refreshing.
    await elapse(HOME_STALE_REFETCH_DELAYS_MS[0]);
    expect(reads).toBe(2);
    expect(screen.getByTestId('home-activities-refreshing')).toBeTruthy();
    expect(screen.queryByTestId('home-activity-strava-9002')).toBeNull();

    // The second finds the new ride.
    await elapse(HOME_STALE_REFETCH_DELAYS_MS[1]);
    expect(await screen.findByTestId('home-activity-strava-9002')).toBeTruthy();
    expect(reads).toBe(3);
    expect(screen.queryByTestId('home-activities-refreshing')).toBeNull();

    await elapse(SCHEDULE_MS * 2);
    expect(reads).toBe(3);
  });

  it('stops after the last delay, and shows when it last synced while the list is still stale', async () => {
    jest.useFakeTimers();
    stub = installHttpStub(
      homeServer({ [RECENT_URL]: { data: recentResponse({ stale: true, as_of: '2026-09-22T06:00:00Z' }) } }),
    );
    const screen = renderHome();
    const recentReads = () => stub.requestsFor('GET').filter((request) => request.url === RECENT_PATH);

    await screen.findByTestId('home-activities-refreshing');
    for (const [index, delay] of HOME_STALE_REFETCH_DELAYS_MS.entries()) {
      // A follow-up is still owed, and the page says it is checking.
      expect(screen.getByTestId('home-activities-refreshing')).toBeTruthy();
      await elapse(delay);
      expect(recentReads()).toHaveLength(index + 2);
    }

    expect(await screen.findByTestId('home-activities-synced-at')).toHaveTextContent(/^Last synced: /);
    expect(screen.queryByTestId('home-activities-refreshing')).toBeNull();

    await elapse(SCHEDULE_MS * 2);
    expect(recentReads()).toHaveLength(1 + HOME_STALE_REFETCH_DELAYS_MS.length);
  });

  it('follows up a pull to refresh that finds the list still stale after the schedule ended', async () => {
    jest.useFakeTimers();
    stub = installHttpStub(
      homeServer({ [RECENT_URL]: { data: recentResponse({ stale: true, as_of: '2026-09-22T06:00:00Z' }) } }),
    );
    const screen = renderHome();
    const recentReads = () => stub.requestsFor('GET').filter((request) => request.url === RECENT_PATH);

    await screen.findByTestId('home-activities-refreshing');
    for (const delay of HOME_STALE_REFETCH_DELAYS_MS) {
      await elapse(delay);
    }
    await screen.findByTestId('home-activities-synced-at');
    const afterFirstSchedule = 1 + HOME_STALE_REFETCH_DELAYS_MS.length;
    expect(recentReads()).toHaveLength(afterFirstSchedule);

    await elapse(SCHEDULE_MS);
    await act(async () => {
      screen.getByTestId('home-scroll').props.refreshControl.props.onRefresh();
    });
    await elapse(0);

    // The pull's own read, and the page is checking again.
    expect(recentReads()).toHaveLength(afterFirstSchedule + 1);
    expect(await screen.findByTestId('home-activities-refreshing')).toBeTruthy();

    await elapse(HOME_STALE_REFETCH_DELAYS_MS[0]);
    expect(recentReads()).toHaveLength(afterFirstSchedule + 2);
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
