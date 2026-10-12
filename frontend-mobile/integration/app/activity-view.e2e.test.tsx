// ABOUTME: e2e — from a Home row to the activity's own screen over one stubbed server, then a question answered under it
// ABOUTME: The real api-client parses the activity's body, so a view built on a broken contract shows its error, never half a card

import React from 'react';
import { act, fireEvent, render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

import { installHttpStub, type HttpStub, type StubRoutes } from './helpers/httpStub';
import { PROSE_OPENING, assistantTurn } from './helpers/chatFixtures';
import {
  CALENDAR_URL,
  LATEST_ROUTE_RESPONSE,
  NO_GPS_ROUTE_RESPONSE,
  PLAN_RESPONSE,
  STATUS_RESPONSE,
  VOLUME_RESPONSE,
  PROVIDERS_CONNECTED,
  TEMPO_DETAIL_RESPONSE,
  TRAIL_ROUTE_RESPONSE,
  recentResponse,
  calendarAnswer,
} from './helpers/homeFixtures';

const mockPush = jest.fn();
let mockParams: Record<string, string> = {};
jest.mock('expo-router', () =>
  require('../../jest.expo-router').createExpoRouterMock({
    useRouter: () => ({ push: mockPush, replace: jest.fn(), back: jest.fn(), navigate: jest.fn(), canGoBack: () => true }),
    useLocalSearchParams: () => mockParams,
  }),
);
// No navigator in the harness, so the header the column offsets by is 0 tall.
jest.mock('expo-router/react-navigation', () => ({
  ...jest.requireActual('expo-router/react-navigation'),
  useHeaderHeight: () => 0,
}));
// The composer reads the safe-area insets, which no provider supplies here.
jest.mock('react-native-safe-area-context', () => ({
  ...jest.requireActual('react-native-safe-area-context'),
  useSafeAreaInsets: () => ({ top: 0, bottom: 0, left: 0, right: 0 }),
}));
// The app's auth provider sits above every screen; the thread's voice input reads it.
jest.mock('../../src/contexts/AuthContext', () => ({
  useAuth: () => ({ isAuthenticated: true, isLoading: false, user: { id: 'user-1' } }),
}));
jest.mock('../../src/hooks/useServerStatus', () => ({
  useServerStatus: () => ({ isServerReachable: true, isChecking: false, checkNow: jest.fn() }),
}));

import { HomeScreen } from '../../src/screens/home/HomeScreen';
import { ActivityScreen } from '../../src/screens/activity/ActivityScreen';

const DETAIL_URL = 'GET /api/me/activities/strava/9000';
const THREAD = 'conv-activity-9000';
const GEAR_URL = 'GET /api/me/gear';
const ACTIVITY_GEAR = '/api/me/activities/strava/9000/gear';
const ROAD = '11111111-1111-4111-8111-111111111111';
const TRAIL = '22222222-2222-4222-8222-222222222222';

/** One pair as the gear endpoints answer with it. */
function shoe(id: string, name: string, sport: string, distanceMeters: number) {
  return {
    id,
    kind: 'shoe',
    name,
    sport,
    initial_distance_meters: 0,
    warn_distance_meters: 500_000,
    distance_meters: distanceMeters,
    activity_count: 12,
    warn_reached: false,
    pinned: false,
    is_default: sport === 'run',
    added_at: '2026-09-01T07:00:00Z',
    retired_at: null,
    last_used_at: '2026-09-17T06:30:00Z',
  };
}

function server(overrides: StubRoutes = {}): StubRoutes {
  return {
    'GET /api/me/training-plan?locale=en': { data: PLAN_RESPONSE },
    'GET /api/me/training-status': { data: STATUS_RESPONSE },
    'GET /api/me/training-volume': { data: VOLUME_RESPONSE },
    [CALENDAR_URL]: { data: calendarAnswer('2026-09-21', '2026-09-27') },
    'GET /api/me/activities/recent': { data: recentResponse() },
    'GET /api/me/activities/strava/9001/route': { data: LATEST_ROUTE_RESPONSE },
    'GET /api/me/activities/intervals_icu/i77/route': { data: TRAIL_ROUTE_RESPONSE },
    'GET /api/me/activities/strava/8998/route': { data: NO_GPS_ROUTE_RESPONSE },
    'GET /api/me/activities/strava/9000/route': { data: LATEST_ROUTE_RESPONSE },
    'GET /api/providers': { data: PROVIDERS_CONNECTED },
    [DETAIL_URL]: { data: TEMPO_DETAIL_RESPONSE },
    'POST /api/chat/conversations': {
      data: { id: THREAD, title: 'Sep 30', created_at: '2026-09-30T10:00:00Z', updated_at: '2026-09-30T10:00:00Z' },
    },
    [`POST /api/chat/conversations/${THREAD}/messages`]: { data: assistantTurn() },
    [`GET /api/chat/conversations/${THREAD}/verdicts`]: { data: { verdicts: [] } },
    'PUT /api/me/activities/strava/9000/conversation': { data: { conversation_id: THREAD } },
    // The run's shoes: no pair declared yet.
    [GEAR_URL]: { data: { gear: [] } },
    [`GET ${ACTIVITY_GEAR}`]: { data: { gear_id: null, source: null } },
    ...overrides,
  };
}

function withClient(node: React.ReactElement) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0, refetchOnWindowFocus: false } },
  });
  return render(<QueryClientProvider client={client}>{node}</QueryClientProvider>);
}

describe("an activity's own screen over the wire", () => {
  let stub: HttpStub;

  beforeEach(() => {
    mockPush.mockClear();
    mockParams = {};
  });

  afterEach(() => {
    stub.restore();
  });

  it('opens from its Home row, reads the activity, and answers a question under it', async () => {
    stub = installHttpStub(server());
    const home = withClient(<HomeScreen />);
    fireEvent.press(await home.findByTestId('home-activity-strava-9000'));
    const [[href]] = mockPush.mock.calls;
    expect(href).toEqual({
      pathname: '/(app)/activity/[provider]/[activityId]',
      params: { provider: 'strava', activityId: '9000' },
    });
    home.unmount();

    // The router hands the screen the params the row pushed.
    mockParams = href.params;
    mockPush.mockClear();
    const screen = withClient(<ActivityScreen />);

    expect(await screen.findByTestId('activity-figure-average_heart_rate')).toHaveTextContent('Avg heart rate158 bpm');
    expect(screen.getByTestId('stack-header-title')).toHaveTextContent('Tempo Thursday');
    expect(await screen.findByTestId('route-track')).toBeTruthy();
    const reads = stub.requestsFor('GET').map((request) => request.url);
    expect(reads).toContain('/api/me/activities/strava/9000');
    expect(reads).toContain('/api/me/activities/strava/9000/route');

    await act(async () => {
      fireEvent.press(screen.getByTestId('activity-prompt-analyze'));
    });

    // The reply lands in the activity's own screen, under its figures.
    await waitFor(() => expect(screen.getByText(PROSE_OPENING)).toBeTruthy());
    expect(screen.getByTestId('activity-figures')).toBeTruthy();
    expect(mockPush).not.toHaveBeenCalled();
    const posts = stub.requestsFor('POST').map((request) => [request.url, request.body]);
    expect(posts).toEqual([
      // carnet#829: the thread is created for the question being sent, so the
      // agent bound to it posts no welcome over the answer.
      ['/api/chat/conversations', { athlete_speaks_first: true }],
      [
        `/api/chat/conversations/${THREAD}/messages`,
        {
          content: 'Analyze my activity “Tempo Thursday” from Thursday, September 17: how did this effort go?',
          // carnet#828: a chip's question says it came from a chip, not the keyboard.
          origin: 'chip',
        },
      ],
    ]);
    // The thread is linked to the activity on the server, so any device resumes it.
    await waitFor(() =>
      expect(stub.requestsFor('PUT').map((request) => [request.url, request.body])).toEqual([
        ['/api/me/activities/strava/9000/conversation', { conversation_id: THREAD }],
      ]),
    );
  });

  it('shows the load failure, not a half-drawn view, when the body breaks the contract', async () => {
    stub = installHttpStub(
      server({ [DETAIL_URL]: { data: { ...TEMPO_DETAIL_RESPONSE, splits: [{ index: 'first' }] } } }),
    );
    mockParams = { provider: 'strava', activityId: '9000' };
    const screen = withClient(<ActivityScreen />);

    expect(await screen.findByTestId('activity-failed', {}, { timeout: 6000 })).toBeTruthy();
    expect(screen.queryByTestId('activity-figures')).toBeNull();
    expect(screen.queryByTestId('activity-prompts')).toBeNull();
  }, 10_000);

  it('counts the run on the pair the athlete chooses, over the wire', async () => {
    let attributed: { gear_id: string | null; source: string | null } = { gear_id: ROAD, source: 'default' };
    stub = installHttpStub(
      server({
        [GEAR_URL]: {
          data: { gear: [shoe(ROAD, 'Pegasus 41', 'run', 512_300), shoe(TRAIL, 'Speedgoat 6', 'trail_running', 212_400)] },
        },
        [`GET ${ACTIVITY_GEAR}`]: () => ({ data: attributed }),
        [`PUT ${ACTIVITY_GEAR}`]: (request) => {
          attributed = { gear_id: (request.body as { gear_id: string | null }).gear_id, source: 'declared' };
          return { data: attributed };
        },
      }),
    );
    mockParams = { provider: 'strava', activityId: '9000' };
    const screen = withClient(<ActivityScreen />);

    const shown = () => screen.getByTestId('activity-gear-value').props.children as string;
    await waitFor(() => expect(shown()).toBe('Pegasus 41'));
    expect(screen.getByTestId('activity-gear-default')).toBeTruthy();

    fireEvent.press(screen.getByTestId('activity-gear-select'));
    fireEvent.press(screen.getByTestId(`activity-gear-option-${TRAIL}`));

    await waitFor(() => expect(shown()).toBe('Speedgoat 6'));
    expect(stub.requestsFor('PUT').map((request) => [request.url, request.body])).toEqual([
      [ACTIVITY_GEAR, { gear_id: TRAIL }],
    ]);
    // The ledger is read again for the distance the choice moved.
    await waitFor(() =>
      expect(stub.requestsFor('GET').filter((request) => request.url === '/api/me/gear')).toHaveLength(2),
    );
  });

  it("says a 404 is not among the athlete's activities", async () => {
    stub = installHttpStub(server({ [DETAIL_URL]: { status: 404, data: { message: 'not found' } } }));
    mockParams = { provider: 'strava', activityId: '9000' };
    const screen = withClient(<ActivityScreen />);

    expect(await screen.findByTestId('activity-not-found')).toBeTruthy();
  });
});
