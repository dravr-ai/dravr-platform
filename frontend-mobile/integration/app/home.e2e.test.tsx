// ABOUTME: e2e — the Home tab over one stubbed server: the /api/me reads and the provider status on the wire, the map, the sketches, the drafts
// ABOUTME: The real api-client parses every body, so a response that breaks the contract shows an error, never a half-drawn card

import React from 'react';
import { act, fireEvent, render, waitFor, within } from '@testing-library/react-native';
import type { AxiosAdapter } from 'axios';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { HOME_STALE_REFETCH_DELAYS_MS, QUERY_KEYS } from '@pierre/shared-constants';

import { installHttpStub, type HttpStub, type StubRoutes } from './helpers/httpStub';
import {
  ACTIVITIES,
  LATEST_ROUTE_RESPONSE,
  NO_GPS_ROUTE_RESPONSE,
  PLAN_RESPONSE,
  STATUS_RESPONSE,
  PROVIDERS_CONNECTED,
  PROVIDERS_NONE,
  PROVIDERS_ONLY_FLAGGED,
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
import { apiClient } from '../../src/services/api';
import { ConversationsScreen } from '../../src/screens/conversations/ConversationsScreen';
import TabsLayout from '../../app/(app)/(tabs)/_layout';
import { HOME_ROUTE } from '../../src/navigation/routes';

const PLAN_URL = 'GET /api/me/training-plan?locale=en';
const STATUS_URL = 'GET /api/me/training-status';
const RECENT_URL = 'GET /api/me/activities/recent';
// Home's route reads are one burst: each carries the `burst` flag the server
// reads to hand its provider turn to the newest activity first.
const LATEST_ROUTE_URL = 'GET /api/me/activities/strava/9001/route?burst=true';
const TRAIL_ROUTE_URL = 'GET /api/me/activities/intervals_icu/i77/route?burst=true';
const GYM_ROUTE_URL = 'GET /api/me/activities/strava/8998/route?burst=true';
/** The athlete's retry is read at once, outside any burst. */
const LATEST_ROUTE_RETRY_URL = 'GET /api/me/activities/strava/9001/route?retry=true';
const PROVIDERS_URL = 'GET /api/providers';
const RECENT_PATH = '/api/me/activities/recent';

/**
 * How long the first map a test file draws may take to appear.
 *
 * The route card is loaded on demand behind a Suspense boundary
 * (`LazyRouteView`), and each test file has its own module registry, so the
 * first map in a file suspends once: its module graph is required — and
 * transformed, on a runner whose transform cache is cold — and React then
 * holds the revealed card back for its Suspense fallback throttle
 * (`FALLBACK_THROTTLE_MS`, 300 ms in React 19) after the "Loading the map"
 * fallback committed. The client itself adds no wait: the route's first ask
 * is immediate (`HOME_ROUTE_PENDING_BACKOFF_MS[0]` is 0) and the stub answers
 * it at once. Testing Library's 1 s default covers the render around it; the
 * second second is the card's one-time load. Every later map in the file is
 * drawn in tens of milliseconds.
 */
const FIRST_MAP_TIMEOUT_MS = 2_000;

/** The whole follow-up schedule, first answer to last ask. */
const SCHEDULE_MS = HOME_STALE_REFETCH_DELAYS_MS.reduce((sum, delay) => sum + delay, 0);

/**
 * The server as a Home visit finds it: a plan, five activities, the answer to
 * each route read the page makes, and one healthy connection.
 */
function homeServer(overrides: StubRoutes = {}): StubRoutes {
  return {
    [PLAN_URL]: { data: PLAN_RESPONSE },
    [STATUS_URL]: { data: STATUS_RESPONSE },
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
  // row's own polyline already draws, asks again for one the row says held
  // no GPS, or stops fetching the basemap style it relabels.
  it('reads the plan, the status, the list, the provider status and only the routes it has to ask for', async () => {
    stub = installHttpStub(homeServer());
    const screen = renderHome();

    expect(await screen.findByTestId('route-track', {}, { timeout: FIRST_MAP_TIMEOUT_MS })).toBeTruthy();
    await screen.findByTestId('home-activity-sketch-intervals_icu-i77');
    await waitFor(() => expect(stub.requestsFor('GET')).toHaveLength(8));

    const urls = stub.requestsFor('GET').map((request) => request.url).sort();
    expect(urls).toEqual(
      [
        // The map's published basemap style, fetched once so its place names
        // can be relabelled in the athlete's language.
        'https://tiles.openfreemap.org/styles/dark',
        '/api/me/activities/intervals_icu/i77/route?burst=true',
        '/api/me/activities/recent',
        '/api/me/activities/strava/8998/route?burst=true',
        '/api/me/activities/strava/9001/route?burst=true',
        '/api/me/training-plan?locale=en',
        '/api/me/training-status',
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
        'GET /api/me/activities/strava/run%2F2026%2009%2020/route?burst=true': { data: LATEST_ROUTE_RESPONSE },
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
      '/api/me/activities/intervals_icu/i77/route?burst=true',
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
      '/api/me/activities/strava/8998/route?burst=true',
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
    expect(stub.requestsFor('GET').filter((request) => request.url.includes('/route'))).toEqual([]);
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

  // The server keeps an `unavailable` answer for ten minutes: a plain re-ask
  // gets it again. Only the retry, which the server reads past, can draw it.
  it('says the map failed when the server could not read the route just now, and draws it on a retry past it', async () => {
    stub = installHttpStub(
      homeServer({
        [RECENT_URL]: { data: recentResponse({ activities: [ACTIVITIES[0]] }) },
        [LATEST_ROUTE_URL]: { data: { route: null, reason: 'unavailable' } },
        [LATEST_ROUTE_RETRY_URL]: { data: LATEST_ROUTE_RESPONSE },
      }),
    );
    const screen = renderHome();

    expect(await screen.findByTestId('home-latest-map-failed')).toHaveTextContent(/^The map couldn't be loaded\./);
    expect(screen.queryByTestId('home-latest-no-track')).toBeNull();
    fireEvent.press(screen.getByTestId('home-latest-map-retry'));
    expect(await screen.findByTestId('route-track')).toBeTruthy();
    const routeReads = stub.requestsFor('GET').filter((request) => request.url.includes('/strava/9001/route'));
    expect(routeReads.map((request) => request.url)).toEqual([
      '/api/me/activities/strava/9001/route?burst=true',
      '/api/me/activities/strava/9001/route?retry=true',
    ]);
    // A stalled connection gives up at the route's own bound, never the five
    // minutes the phone's transport gives a chat turn.
    expect(routeReads.every((request) => request.timeout === 30_000)).toBe(true);
  });

  it('keeps saying the map could not be loaded when the retried read fails too, never that it has no GPS', async () => {
    stub = installHttpStub(
      homeServer({
        [RECENT_URL]: { data: recentResponse({ activities: [ACTIVITIES[0]] }) },
        [LATEST_ROUTE_URL]: { data: { route: null, reason: 'unavailable' } },
        [LATEST_ROUTE_RETRY_URL]: { data: { route: null, reason: 'unavailable' } },
      }),
    );
    const screen = renderHome();

    await screen.findByTestId('home-latest-map-failed');
    fireEvent.press(screen.getByTestId('home-latest-map-retry'));
    await waitFor(() =>
      expect(stub.requestsFor('GET').some((request) => request.url.endsWith('/route?retry=true'))).toBe(true),
    );
    expect(await screen.findByTestId('home-latest-map-failed')).toHaveTextContent(/^The map couldn't be loaded\./);
    expect(screen.queryByTestId('home-latest-no-track')).toBeNull();
  });

  // The server's contract after a failed sync: while the failing provider is
  // paused its answer is not stale and carries the failure; the retry's own
  // answer is stale — the refresh it started is running — and still carries
  // it; the first fresh answer on the schedule is the one that clears it.
  it('says which sync failed, keeps the rows, and a retry clears it on the first fresh answer', async () => {
    jest.useFakeTimers();
    const failure = {
      provider: 'strava',
      provider_name: 'Strava',
      failed_at: '2026-09-29T14:04:00Z',
      last_synced_at: '2026-09-29T05:15:00Z',
    };
    let followUps = 0;
    stub = installHttpStub(
      homeServer({
        [RECENT_URL]: () => {
          followUps += 1;
          return followUps === 1
            ? { data: recentResponse({ sync_failure: failure }) }
            : { data: recentResponse({ as_of: '2026-09-29T14:30:00Z', sync_failure: null }) };
        },
        [`${RECENT_URL}?retry=true`]: { data: recentResponse({ stale: true, sync_failure: failure }) },
      }),
    );
    const screen = renderHome();

    const failed = await screen.findByTestId('home-activities-sync-failed');
    expect(failed).toHaveTextContent(/^Strava · Sync failed/);
    expect(screen.getByTestId('home-activities-synced-at')).toHaveTextContent(/^Last synced: /);
    expect(screen.getByTestId(`home-activity-${ACTIVITIES[0].provider}-${ACTIVITIES[0].id}`)).toBeTruthy();

    fireEvent.press(screen.getByTestId('home-activities-sync-retry'));
    await elapse(0);
    expect(screen.getByTestId('home-activities-fetching')).toBeTruthy();
    expect(screen.queryByTestId('home-activities-sync-failed')).toBeNull();

    // The schedule restarts from the retry: its first follow-up lands the fresh answer.
    await elapse(HOME_STALE_REFETCH_DELAYS_MS[0]);
    expect(followUps).toBe(2);
    expect(screen.queryByTestId('home-activities-sync-failed')).toBeNull();
    expect(screen.queryByTestId('home-activities-fetching')).toBeNull();
    expect(stub.requestsFor('GET').filter((request) => request.url === `${RECENT_PATH}?retry=true`)).toHaveLength(1);
  });

  it('keeps saying the sync failed through the stale schedule while the retried refresh keeps failing', async () => {
    jest.useFakeTimers();
    const failure = {
      provider: 'strava',
      provider_name: 'Strava',
      failed_at: '2026-09-29T14:04:00Z',
      last_synced_at: '2026-09-29T05:15:00Z',
    };
    stub = installHttpStub(
      homeServer({
        // Paused between attempts: not stale, and the failure stands.
        [RECENT_URL]: { data: recentResponse({ sync_failure: { ...failure, failed_at: '2026-09-29T14:06:00Z' } }) },
        [`${RECENT_URL}?retry=true`]: { data: recentResponse({ stale: true, sync_failure: failure }) },
      }),
    );
    const screen = renderHome();

    await screen.findByTestId('home-activities-sync-failed');
    fireEvent.press(screen.getByTestId('home-activities-sync-retry'));
    await elapse(0);
    expect(screen.getByTestId('home-activities-fetching')).toBeTruthy();

    await elapse(HOME_STALE_REFETCH_DELAYS_MS[0]);
    expect(await screen.findByTestId('home-activities-sync-failed')).toHaveTextContent(/^Strava · Sync failed/);
    expect(screen.queryByTestId('home-activities-fetching')).toBeNull();
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

  // The shell's banner names the connections; Home keeps its rows and says
  // nothing more about them.
  it('keeps the rows of a connection to reconnect without naming it a second time', async () => {
    stub = installHttpStub(homeServer({ [PROVIDERS_URL]: { data: PROVIDERS_RECONNECT } }));
    const screen = renderHome();

    expect(await screen.findByTestId('home-activity-strava-9001')).toBeTruthy();
    await waitFor(() => expect(stub.requests.some((request) => request.url === '/api/providers')).toBe(true));
    expect(screen.queryByText('Reconnect needed')).toBeNull();
    expect(screen.queryByText(/Reconnect Strava/)).toBeNull();
    expect(screen.queryByTestId('home-activities-no-provider')).toBeNull();
  });

  // A healthy connection still syncs, so the empty sentence is true.
  it('keeps the empty sentence when one connected provider still syncs beside the flagged ones', async () => {
    stub = installHttpStub(
      homeServer({
        [RECENT_URL]: { data: recentResponse({ activities: [] }) },
        [PROVIDERS_URL]: { data: PROVIDERS_RECONNECT },
      }),
    );
    const screen = renderHome();

    expect(await screen.findByTestId('home-activities-empty')).toBeTruthy();
    expect(screen.queryByText('Reconnect needed')).toBeNull();
  });

  // carnet#649: with every connection flagged the empty sentence would
  // promise a sync nobody can make, and "reconnect" is the banner's to say.
  it('leaves the section out for an empty cache when every connected provider is flagged', async () => {
    stub = installHttpStub(
      homeServer({
        [RECENT_URL]: { data: recentResponse({ activities: [], stale: false }) },
        [PROVIDERS_URL]: { data: PROVIDERS_ONLY_FLAGGED },
      }),
    );
    const screen = renderHome();

    await screen.findByTestId('home-today');
    await waitFor(() => expect(stub.requests.some((request) => request.url === '/api/providers')).toBe(true));
    await waitFor(() => expect(screen.queryByTestId('home-activities-loading')).toBeNull());
    expect(screen.queryByTestId('home-section-activities')).toBeNull();
    expect(screen.queryByText('Reconnect needed')).toBeNull();
    expect(screen.queryByTestId('home-activities-empty')).toBeNull();
    expect(screen.queryByTestId('home-activities-fetching')).toBeNull();
  });

  // The server says stale only when it started a refresh — for a flagged
  // scrape session, its throttled retry — so then, and only then, the
  // section says it is checking, and nothing else.
  it('says it is checking, and nothing more, while the server retries a flagged session', async () => {
    stub = installHttpStub(
      homeServer({
        [RECENT_URL]: { data: recentResponse({ activities: [], stale: true }) },
        [PROVIDERS_URL]: { data: PROVIDERS_ONLY_FLAGGED },
      }),
    );
    const screen = renderHome();

    const section = await screen.findByTestId('home-section-activities');
    expect(await within(section).findByTestId('home-activities-fetching')).toHaveTextContent(
      'Fetching your latest activities…',
    );
    await waitFor(() => expect(stub.requests.some((request) => request.url === '/api/providers')).toBe(true));
    expect(within(section).queryByText('Reconnect needed')).toBeNull();
    expect(within(section).queryByTestId('home-activities-empty')).toBeNull();
  });

  it('never says it is checking on a list the server answered fresh, flagged connection or not', async () => {
    stub = installHttpStub(
      homeServer({
        [RECENT_URL]: { data: recentResponse({ stale: false }) },
        [PROVIDERS_URL]: { data: PROVIDERS_RECONNECT },
      }),
    );
    const screen = renderHome();

    expect(await screen.findByTestId('home-activities-synced-at')).toBeTruthy();
    expect(screen.queryByTestId('home-activities-fetching')).toBeNull();
  });

  // A stale answer held from an earlier visit reports a refresh that may
  // have ended hours ago: the page waits for this visit's own read.
  it('does not say it is checking off a held stale answer until the server says so again', async () => {
    let answer: (() => void) | null = null;
    stub = installHttpStub(
      homeServer({
        [RECENT_URL]: () => ({ data: recentResponse({ stale: false }) }),
      }),
    );
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false, gcTime: Infinity, refetchOnWindowFocus: false } },
    });
    client.setQueryData(QUERY_KEYS.home.recentActivities(), recentResponse({ stale: true }));
    const gate = new Promise<void>((resolve) => {
      answer = resolve;
    });
    // Hold this visit's list read until the held answer has been drawn.
    const adapter = apiClient.defaults.adapter as AxiosAdapter;
    apiClient.defaults.adapter = async (config) => {
      if (config.url === RECENT_PATH) await gate;
      return adapter(config);
    };
    const screen = render(
      <QueryClientProvider client={client}>
        <HomeScreen />
      </QueryClientProvider>,
    );

    // The held rows draw at once, without a claim that a refresh is running.
    expect(await screen.findByTestId('home-activity-strava-9001')).toBeTruthy();
    expect(screen.queryByTestId('home-activities-fetching')).toBeNull();

    // This visit's read answers fresh: the sync time, still no spinner.
    await act(async () => {
      answer?.();
    });
    expect(await screen.findByTestId('home-activities-synced-at')).toBeTruthy();
    expect(screen.queryByTestId('home-activities-fetching')).toBeNull();
    apiClient.defaults.adapter = adapter;
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
        'GET /api/me/activities/strava/9002/route?burst=true': { data: LATEST_ROUTE_RESPONSE },
      }),
    );
    const screen = renderHome();

    // The list is headed by the row that says new activities are on their
    // way, a polite status, above the rows the cache already holds.
    const fetching = await screen.findByTestId('home-activities-fetching');
    expect(fetching).toHaveTextContent('Fetching your latest activities…');
    expect(fetching.props.role).toBe('status');
    expect(fetching.props.accessibilityLiveRegion).toBe('polite');
    expect(
      screen.getAllByTestId(/^home-activit(ies-fetching|y-latest)$/).map((node) => node.props.testID),
    ).toEqual(['home-activities-fetching', 'home-activity-latest']);
    expect(reads).toBe(1);

    // The first follow-up finds the server still refreshing.
    await elapse(HOME_STALE_REFETCH_DELAYS_MS[0]);
    expect(reads).toBe(2);
    expect(screen.getByTestId('home-activities-fetching')).toBeTruthy();
    expect(screen.queryByTestId('home-activity-strava-9002')).toBeNull();

    // The second finds the new ride.
    await elapse(HOME_STALE_REFETCH_DELAYS_MS[1]);
    expect(await screen.findByTestId('home-activity-strava-9002')).toBeTruthy();
    expect(reads).toBe(3);
    expect(screen.queryByTestId('home-activities-fetching')).toBeNull();

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

    await screen.findByTestId('home-activities-fetching');
    for (const [index, delay] of HOME_STALE_REFETCH_DELAYS_MS.entries()) {
      // A follow-up is still owed, and the page says it is checking.
      expect(screen.getByTestId('home-activities-fetching')).toBeTruthy();
      await elapse(delay);
      expect(recentReads()).toHaveLength(index + 2);
    }

    await waitFor(() => expect(screen.queryByTestId('home-activities-fetching')).toBeNull());
    expect(screen.getByTestId('home-activities-synced-at')).toHaveTextContent(/^Last synced: /);

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

    await screen.findByTestId('home-activities-fetching');
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
    expect(await screen.findByTestId('home-activities-fetching')).toBeTruthy();

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
      [PROVIDERS_URL]: { data: PROVIDERS_CONNECTED },
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

describe('the shell reconnect banner', () => {
  let stub: HttpStub;

  afterEach(() => {
    stub.restore();
  });

  function renderShell() {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
    return render(
      <QueryClientProvider client={client}>
        <TabsLayout />
        <HomeScreen />
      </QueryClientProvider>,
    );
  }

  function shellServer(providers: typeof PROVIDERS_CONNECTED): StubRoutes {
    return homeServer({
      'GET /api/chat/conversations?limit=50&offset=0': {
        data: { conversations: [], total: 0, limit: 50, offset: 0 },
      },
      'GET /api/notifications/unread-count': { data: { unread_count: 0 } },
      [PROVIDERS_URL]: { data: providers },
    });
  }

  beforeEach(() => {
    mockPush.mockClear();
  });

  // Turns red if the banner leaves the shell, stops deduplicating names across
  // backends, names a disconnected provider, or leads anywhere but Connections.
  it('names every connection to reconnect above the tabs, once, and leads to Connections', async () => {
    stub = installHttpStub(shellServer(PROVIDERS_RECONNECT));
    const screen = renderShell();

    const banner = await screen.findByTestId('reconnect-banner');
    expect(within(banner).getByTestId('reconnect-banner-providers')).toHaveTextContent(
      'Reconnect Strava, Garmin to see your new activities.',
    );
    expect(within(banner).getByTestId('reconnect-banner-message').props.accessibilityRole).toBe('alert');
    expect(within(banner).getByText('Reconnect needed')).toBeTruthy();
    // Home and the banner share one provider-status read.
    await screen.findByTestId('home-activity-strava-9001');
    expect(stub.requests.filter((request) => request.url === '/api/providers')).toHaveLength(1);

    fireEvent.press(within(banner).getByTestId('reconnect-banner-action'));
    expect(mockPush).toHaveBeenCalledWith('/(app)/(tabs)/(settings)/connections');
  });

  it('draws nothing while every connection is healthy', async () => {
    stub = installHttpStub(shellServer(PROVIDERS_CONNECTED));
    const screen = renderShell();

    await screen.findByTestId('home-activity-strava-9001');
    await waitFor(() => expect(stub.requests.some((request) => request.url === '/api/providers')).toBe(true));
    expect(screen.queryByTestId('reconnect-banner')).toBeNull();
  });
});
