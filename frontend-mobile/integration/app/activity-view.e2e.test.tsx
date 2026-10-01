// ABOUTME: e2e — from a Home row to the activity's own screen over one stubbed server, then a question answered under it
// ABOUTME: The real api-client parses the activity's body, so a view built on a broken contract shows its error, never half a card

import React from 'react';
import { act, fireEvent, render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

import { installHttpStub, type HttpStub, type StubRoutes } from './helpers/httpStub';
import { PROSE_OPENING, assistantTurn } from './helpers/chatFixtures';
import {
  LATEST_ROUTE_RESPONSE,
  NO_GPS_ROUTE_RESPONSE,
  PLAN_RESPONSE,
  PROVIDERS_CONNECTED,
  TEMPO_DETAIL_RESPONSE,
  TRAIL_ROUTE_RESPONSE,
  recentResponse,
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

function server(overrides: StubRoutes = {}): StubRoutes {
  return {
    'GET /api/me/training-plan?locale=en': { data: PLAN_RESPONSE },
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
      ['/api/chat/conversations', {}],
      [
        `/api/chat/conversations/${THREAD}/messages`,
        { content: 'Analyze my activity “Tempo Thursday” from Thursday, September 17: how did this effort go?' },
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

  it("says a 404 is not among the athlete's activities", async () => {
    stub = installHttpStub(server({ [DETAIL_URL]: { status: 404, data: { message: 'not found' } } }));
    mockParams = { provider: 'strava', activityId: '9000' };
    const screen = withClient(<ActivityScreen />);

    expect(await screen.findByTestId('activity-not-found')).toBeTruthy();
  });
});
