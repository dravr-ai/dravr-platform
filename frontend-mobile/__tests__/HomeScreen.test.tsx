// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home tab over mocked athlete reads — today and tomorrow, the week strip, the map, the sketches, the prompts, every empty state
// ABOUTME: Pins that a rest day and an uncovered day read differently, a day drafts a chat about it, and an activity opens its view

import React from 'react';
import { AccessibilityInfo, Platform } from 'react-native';
import { act, fireEvent, render, waitFor, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { Path } from 'react-native-svg';
import { ROUTE_INK } from '@pierre/shared-constants';
import type { ActivityRouteResponse, RecentActivitiesResponse, TrainingPlanResponse } from '@pierre/shared-types';

import {
  ACTIVITIES,
  LATEST_ROUTE_RESPONSE,
  NO_GPS_ROUTE_RESPONSE,
  NO_PLAN_RESPONSE,
  PLAN_RESPONSE,
  PROVIDERS_CONNECTED,
  PROVIDERS_NONE,
  PROVIDERS_ONLY_FLAGGED,
  PROVIDERS_RECONNECT,
  TRAIL_ROUTE_RESPONSE,
  recentResponse,
} from '../integration/app/helpers/homeFixtures';

const mockPush = jest.fn();
const mockNavigate = jest.fn();
/** The focus callback the screen registered last, so a test can bring the tab back into focus. */
let mockFocusCallback: (() => void) | null = null;
jest.mock('expo-router', () => {
  const React = require('react');
  return require('../jest.expo-router').createExpoRouterMock({
    useRouter: () => ({
      push: mockPush,
      replace: jest.fn(),
      back: jest.fn(),
      navigate: mockNavigate,
      canGoBack: () => true,
    }),
    // The router's contract: run on focus, and again whenever the callback
    // changes while the screen is focused.
    useFocusEffect: (callback: () => void) => {
      mockFocusCallback = callback;
      React.useEffect(() => callback(), [callback]);
    },
  });
});

const mockGetTrainingPlan = jest.fn<Promise<TrainingPlanResponse>, [string?]>();
const mockGetRecentActivities = jest.fn<Promise<RecentActivitiesResponse>, [number?, { retry?: boolean }?]>();
const mockGetActivityRoute = jest.fn<Promise<ActivityRouteResponse>, [string, string, { retry?: boolean }?]>();
const mockGetProvidersStatus = jest.fn();

jest.mock('../src/services/api', () => ({
  athleteApi: {
    getTrainingPlan: (locale?: string) => mockGetTrainingPlan(locale),
    getRecentActivities: (limit?: number, options?: { retry?: boolean }) =>
      options === undefined ? mockGetRecentActivities(limit) : mockGetRecentActivities(limit, options),
    // The query's abort signal is the transport's concern; the retry flag is
    // what the screen decides.
    getActivityRoute: (provider: string, id: string, options?: { retry?: boolean; signal?: AbortSignal }) =>
      options?.retry === undefined
        ? mockGetActivityRoute(provider, id)
        : mockGetActivityRoute(provider, id, { retry: options.retry }),
  },
  oauthApi: { getProvidersStatus: () => mockGetProvidersStatus() },
}));

import { HomeScreen } from '../src/screens/home/HomeScreen';
import { CONNECTIONS_ROUTE, NEW_CONVERSATION_ID } from '../src/navigation/routes';

function renderHome() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={client}>
      <HomeScreen />
    </QueryClientProvider>,
  );
}

/** The navigation a tap that asks the agent about something makes. */
function draftHref(draft: string) {
  return { pathname: '/(app)/chat/[conversationId]', params: { conversationId: NEW_CONVERSATION_ID, draft } };
}

function routeFor(provider: string, id: string): ActivityRouteResponse {
  if (provider === 'strava' && id === '9001') return LATEST_ROUTE_RESPONSE;
  if (provider === 'intervals_icu' && id === 'i77') return TRAIL_ROUTE_RESPONSE;
  if (provider === 'strava' && id === '8998') return NO_GPS_ROUTE_RESPONSE;
  throw new Error(`no route stubbed for ${provider}/${id}`);
}

beforeEach(() => {
  jest.clearAllMocks();
  mockGetTrainingPlan.mockResolvedValue(PLAN_RESPONSE);
  mockGetRecentActivities.mockResolvedValue(recentResponse());
  mockGetActivityRoute.mockImplementation(async (provider, id) => routeFor(provider, id));
  mockGetProvidersStatus.mockResolvedValue(PROVIDERS_CONNECTED);
});

describe('the Home header', () => {
  it('is the Dravr lockup, a button named Home', async () => {
    const screen = renderHome();

    const title = await screen.findByTestId('home-title');
    expect(title.props.accessibilityRole).toBe('button');
    expect(title.props.accessibilityLabel).toBe('Home');
    expect(screen.getByTestId('home-title-wordmark')).toHaveTextContent('DRAVR');
    // Already Home, a press stays: it is never a navigation to a thread.
    fireEvent.press(title);
    expect(mockPush).not.toHaveBeenCalled();
  });
});

describe('today and tomorrow', () => {
  it('enlarges today\'s session with its phase week, and names tomorrow', async () => {
    const screen = renderHome();

    const today = await screen.findByTestId('home-today');
    expect(within(today).getByText('Tempo run')).toBeTruthy();
    // The plan's sport reads in the app language, as an activity row's does.
    expect(within(today).getByText(/Run · 50 min · Z3/)).toBeTruthy();
    // 2026-09-24 is day 17 of a build phase that started 2026-09-07: week 3.
    expect(screen.getByTestId('home-today-phase')).toHaveTextContent('Build · week 3');
    expect(screen.getByTestId('home-tomorrow')).toHaveTextContent(/^Tomorrow/);
    expect(screen.getByTestId('home-tomorrow-value')).toHaveTextContent('Easy run · 40 min');
    // The plan is asked for in the app's language.
    expect(mockGetTrainingPlan).toHaveBeenCalledWith('en');
  });

  it('opens a new chat drafted about today\'s session', async () => {
    const screen = renderHome();

    fireEvent.press(await screen.findByTestId('home-today-day'));

    expect(mockPush).toHaveBeenCalledWith(
      draftHref('Walk me through my session on Thursday, September 24: Tempo run'),
    );
  });

  it('says the plan does not cover a day it never reached, and never calls it rest', async () => {
    mockGetTrainingPlan.mockResolvedValue({ plan: PLAN_RESPONSE.plan, today: '2026-09-27' });
    const screen = renderHome();

    expect(await screen.findByTestId('home-today-uncovered')).toHaveTextContent("Your plan doesn't cover this day.");
    // No enlarged day at all: neither a session nor a rest day stands in for the gap.
    expect(screen.queryByTestId('home-today-day')).toBeNull();
    // Tomorrow is Monday of next week, which the plan keeps as rest.
    expect(screen.getByTestId('home-tomorrow-value')).toHaveTextContent('Rest');
  });

  it('offers to build a plan, as the one filled button, when there is none', async () => {
    mockGetTrainingPlan.mockResolvedValue(NO_PLAN_RESPONSE);
    const screen = renderHome();

    const empty = await screen.findByTestId('home-plan-empty');
    expect(within(empty).getByText('No training plan yet')).toBeTruthy();
    fireEvent.press(screen.getByTestId('home-plan-build'));
    expect(mockPush).toHaveBeenCalledWith(draftHref('Build me a training plan for my goal race.'));
    // No week to show without a plan.
    expect(screen.queryByTestId('home-section-week')).toBeNull();
  });

  it('shows a failed read as a failure with a retry, never as "no plan"', async () => {
    mockGetTrainingPlan.mockRejectedValueOnce(new Error('boom')).mockResolvedValue(PLAN_RESPONSE);
    const screen = renderHome();

    expect(await screen.findByTestId('home-plan-error')).toHaveTextContent(/^Your plan couldn't be loaded\./);
    expect(screen.queryByTestId('home-plan-build')).toBeNull();

    fireEvent.press(screen.getByTestId('home-plan-retry'));
    expect(await screen.findByTestId('home-today')).toBeTruthy();
  });

  it('shows a today the calendar cannot place as a failed read', async () => {
    mockGetTrainingPlan.mockResolvedValue({ plan: PLAN_RESPONSE.plan, today: '2026-02-30' });
    const screen = renderHome();

    expect(await screen.findByTestId('home-plan-error')).toBeTruthy();
    expect(screen.queryByTestId('home-week-strip')).toBeNull();
  });
});

describe('the week strip', () => {
  it('runs Monday to Sunday with today selected, marking sessions, rest and silence apart', async () => {
    const screen = renderHome();

    const strip = await screen.findByTestId('home-week-strip');
    const cells = within(strip).getAllByRole('button');
    expect(cells.map((cell) => cell.props.testID)).toEqual([
      'home-week-day-2026-09-21',
      'home-week-day-2026-09-22',
      'home-week-day-2026-09-23',
      'home-week-day-2026-09-24',
      'home-week-day-2026-09-25',
      'home-week-day-2026-09-26',
      'home-week-day-2026-09-27',
    ]);
    expect(screen.getByTestId('home-week-day-2026-09-24').props.accessibilityState).toEqual({ selected: true });
    expect(screen.getByTestId('home-week-day-2026-09-22').props.accessibilityLabel).toBe('Tuesday 22, Rest');
    expect(screen.getByTestId('home-week-day-2026-09-27').props.accessibilityLabel).toBe(
      "Sunday 27, Your plan doesn't cover this day.",
    );
    expect(within(screen.getByTestId('home-week-day-2026-09-22')).getByTestId('home-week-mark-rest')).toBeTruthy();
    expect(within(screen.getByTestId('home-week-day-2026-09-24')).getByTestId('home-week-mark-session')).toBeTruthy();
    const sunday = screen.getByTestId('home-week-day-2026-09-27');
    expect(within(sunday).queryByTestId('home-week-mark-session')).toBeNull();
    expect(within(sunday).queryByTestId('home-week-mark-rest')).toBeNull();

    // Today's detail is the chat card's day row: steps and fuel included.
    const detail = screen.getByTestId('home-week-detail');
    expect(within(detail).getByText('Steps')).toBeTruthy();
    expect(within(detail).getByText('Fuel')).toBeTruthy();
  });

  it('shows a rest day as rest and drafts the question about it', async () => {
    const screen = renderHome();

    fireEvent.press(await screen.findByTestId('home-week-day-2026-09-22'));
    const detail = screen.getByTestId('home-week-detail');
    expect(within(detail).getByText('Rest')).toBeTruthy();

    fireEvent.press(screen.getByTestId('home-plan-day-2026-09-22'));
    expect(mockPush).toHaveBeenCalledWith(draftHref('Why is Tuesday, September 22 a rest day in my plan?'));
  });

  it('says the plan does not cover Sunday rather than calling it rest', async () => {
    const screen = renderHome();

    fireEvent.press(await screen.findByTestId('home-week-day-2026-09-27'));
    const detail = screen.getByTestId('home-week-detail');
    expect(detail).toHaveTextContent("2026-09-27 Your plan doesn't cover this day.");
    expect(within(detail).queryByText('Rest')).toBeNull();
  });

  it('folds next week under its focus and opens it on a tap', async () => {
    const screen = renderHome();

    const next = await screen.findByTestId('home-next-week');
    expect(next).toHaveTextContent(/Next week.*2026-09-28.*Race-pace long run/);
    expect(next.props.accessibilityState).toEqual({ expanded: false });
    expect(screen.queryByTestId('home-next-week-days')).toBeNull();

    fireEvent.press(next);
    expect(within(screen.getByTestId('home-next-week-days')).getByText('Hill repeats')).toBeTruthy();
    expect(screen.getByTestId('home-next-week').props.accessibilityState).toEqual({ expanded: true });
  });
});

describe('recent activities', () => {
  it('draws the latest on the map and asks for routes only where there is no polyline', async () => {
    const screen = renderHome();

    // The latest activity's route card, lazily loaded, over the mocked MapLibre.
    expect(await screen.findByTestId('route-track')).toBeTruthy();
    const track = screen.getByTestId('route-track').props.data as { coordinates: number[][] };
    expect(track.coordinates[0]).toEqual([-73.6, 45.5]);

    await waitFor(() => expect(mockGetActivityRoute).toHaveBeenCalledTimes(3));
    // The latest (for its map) and the two rows that carry no polyline and
    // may still have a route. The Strava row sketches from its polyline; the
    // row whose route read held no GPS asks for nothing.
    expect(mockGetActivityRoute.mock.calls.sort()).toEqual([
      ['intervals_icu', 'i77'],
      ['strava', '8998'],
      ['strava', '9001'],
    ]);
    expect(mockGetRecentActivities).toHaveBeenCalledWith(undefined);
  });

  it('sketches the older rows that have geometry, and none that do not', async () => {
    const screen = renderHome();

    expect(await screen.findByTestId('home-activity-sketch-strava-9000')).toBeTruthy();
    expect(await screen.findByTestId('home-activity-sketch-intervals_icu-i77')).toBeTruthy();
    expect(screen.getByTestId('home-activity-sketch-strava-9000').props.accessibilityLabel).toBe(
      'Sketch of the route',
    );
    // One row's route read held no GPS; the other's route is read here and
    // answers that it held none.
    await waitFor(() => expect(mockGetActivityRoute).toHaveBeenCalledWith('strava', '8998'));
    await waitFor(() => expect(mockGetActivityRoute.mock.results).toHaveLength(3));
    await Promise.all(mockGetActivityRoute.mock.results.map((read) => read.value));
    expect(screen.queryByTestId('home-activity-sketch-strava-8999')).toBeNull();
    expect(screen.queryByTestId('home-activity-sketch-strava-8998')).toBeNull();
    // Two sketches, and no other route drawing: the map card's climb-less
    // legend draws no swatch. Its full-screen button's icon is an SVG too, so
    // the count is of route paths — the ones stroked in the route orange.
    const routePaths = screen
      .UNSAFE_queryAllByType(Path)
      .filter((path) => path.props.stroke === ROUTE_INK.track);
    expect(routePaths).toHaveLength(2);
  });

  it('prints each row\'s date, sport and the figures the provider reported', async () => {
    const screen = renderHome();

    const latest = await screen.findByTestId('home-activity-strava-9001');
    expect(latest).toHaveTextContent(/^Long ride/);
    expect(latest).toHaveTextContent(/Sun, Sep 20 · Ride · 92\.0 km · 3h 41m 5s · \+850 m/);
    const indoor = screen.getByTestId('home-activity-strava-8999');
    expect(indoor).toHaveTextContent(/Indoor ride · 30\.0 km · 1h$/);
    // A strength session has no distance, and none is invented for it.
    const gym = screen.getByTestId('home-activity-strava-8998');
    expect(gym).toHaveTextContent(/Strength training · 45m$/);
  });

  it("opens the tapped activity's own view, from the latest card and from a row", async () => {
    const screen = renderHome();

    fireEvent.press(await screen.findByTestId('home-activity-strava-9001'));
    expect(mockPush).toHaveBeenCalledWith({
      pathname: '/(app)/activity/[provider]/[activityId]',
      params: { provider: 'strava', activityId: '9001' },
    });

    fireEvent.press(screen.getByTestId('home-activity-intervals_icu-i77'));
    expect(mockPush).toHaveBeenLastCalledWith({
      pathname: '/(app)/activity/[provider]/[activityId]',
      params: { provider: 'intervals_icu', activityId: 'i77' },
    });
    expect(mockPush).toHaveBeenCalledTimes(2);
  });

  it('says a latest activity whose route read held no GPS recorded no track, without asking again', async () => {
    expect(ACTIVITIES[3].has_gps).toBe(false);
    mockGetRecentActivities.mockResolvedValue(recentResponse({ activities: [ACTIVITIES[3]] }));
    const screen = renderHome();

    expect(await screen.findByTestId('home-latest-no-track')).toHaveTextContent(
      'This activity recorded no GPS track.',
    );
    expect(mockGetActivityRoute).not.toHaveBeenCalled();
  });

  it('asks for the route of a latest activity whose row carries no position, and draws the answer', async () => {
    expect(ACTIVITIES[2]).toMatchObject({ has_gps: true, summary_polyline: null });
    mockGetRecentActivities.mockResolvedValue(recentResponse({ activities: [ACTIVITIES[2]] }));
    const screen = renderHome();

    expect(await screen.findByTestId('route-track')).toBeTruthy();
    expect(mockGetActivityRoute.mock.calls).toEqual([['intervals_icu', 'i77']]);
    const track = screen.getByTestId('route-track').props.data as { coordinates: number[][] };
    expect(track.coordinates).toEqual([
      [-74.2, 46.1],
      [-74.18, 46.12],
      [-74.15, 46.13],
    ]);
    expect(screen.queryByTestId('home-latest-no-track')).toBeNull();
  });

  it('says the latest recorded no track once the route read answers that it held no GPS', async () => {
    expect(ACTIVITIES[4]).toMatchObject({ has_gps: true, summary_polyline: null });
    mockGetRecentActivities.mockResolvedValue(recentResponse({ activities: [ACTIVITIES[4]] }));
    const screen = renderHome();

    expect(await screen.findByTestId('home-latest-no-track')).toHaveTextContent(
      'This activity recorded no GPS track.',
    );
    // The row could not say so: the route read did.
    expect(mockGetActivityRoute.mock.calls).toEqual([['strava', '8998']]);
    expect(screen.queryByTestId('route-track')).toBeNull();
  });

  it('says the map is on its way while the route read has not answered', async () => {
    mockGetActivityRoute.mockReturnValue(new Promise<ActivityRouteResponse>(() => undefined));
    mockGetRecentActivities.mockResolvedValue(recentResponse({ activities: [ACTIVITIES[0]] }));
    const screen = renderHome();

    expect(await screen.findByTestId('home-latest-map-loading')).toHaveTextContent('Loading the map…');
    // Not an answer yet, so neither "no track" nor a failure.
    expect(screen.queryByTestId('home-latest-no-track')).toBeNull();
    expect(screen.queryByTestId('home-latest-map-failed')).toBeNull();
    // The words of the row do not wait for the map.
    expect(screen.getByTestId('home-activity-strava-9001')).toHaveTextContent(/^Long ride/);
  });

  it('says a route too short to draw is too short', async () => {
    mockGetActivityRoute.mockResolvedValue({ route: null, reason: 'too_short' });
    mockGetRecentActivities.mockResolvedValue(recentResponse({ activities: [ACTIVITIES[0]] }));
    const screen = renderHome();

    expect(await screen.findByTestId('home-latest-too-short')).toHaveTextContent(
      'This route is too short to draw without showing where it starts.',
    );
  });

  it('says the map failed when the route read fails twice, with a retry', async () => {
    // The route query asks once more before it gives up, never the client's default.
    mockGetActivityRoute
      .mockRejectedValueOnce(new Error('503'))
      .mockRejectedValueOnce(new Error('503'))
      .mockResolvedValue(LATEST_ROUTE_RESPONSE);
    mockGetRecentActivities.mockResolvedValue(recentResponse({ activities: [ACTIVITIES[0]] }));
    const screen = renderHome();

    expect(await screen.findByTestId('home-latest-map-failed', {}, { timeout: 4000 })).toHaveTextContent(
      /^The map couldn't be loaded\./,
    );
    expect(mockGetActivityRoute).toHaveBeenCalledTimes(2);
    fireEvent.press(screen.getByTestId('home-latest-map-retry'));
    expect(await screen.findByTestId('route-track')).toBeTruthy();
  });

  it('says the map failed when the server could not read the route just now, and draws it on a retry past it', async () => {
    mockGetActivityRoute
      .mockResolvedValueOnce({ route: null, reason: 'unavailable' })
      .mockResolvedValue(LATEST_ROUTE_RESPONSE);
    mockGetRecentActivities.mockResolvedValue(recentResponse({ activities: [ACTIVITIES[0]] }));
    const screen = renderHome();

    expect(await screen.findByTestId('home-latest-map-failed')).toHaveTextContent(/^The map couldn't be loaded\./);
    expect(screen.queryByTestId('home-latest-map-loading')).toBeNull();
    expect(screen.queryByTestId('home-latest-no-track')).toBeNull();
    fireEvent.press(screen.getByTestId('home-latest-map-retry'));
    expect(await screen.findByTestId('route-track')).toBeTruthy();
    expect(mockGetActivityRoute).toHaveBeenCalledTimes(2);
    // Only a retry the server reads past its stored `unavailable` can draw
    // the route inside that answer's ten minutes.
    expect(mockGetActivityRoute).toHaveBeenLastCalledWith(ACTIVITIES[0].provider, ACTIVITIES[0].id, { retry: true });
  });

  it('names the provider whose sync failed, dates the list from its last good sync once, and retries the sync', async () => {
    const failure = {
      provider: 'strava',
      provider_name: 'Strava',
      failed_at: '2026-09-29T14:04:00Z',
      last_synced_at: '2026-09-28T21:15:00Z',
    };
    mockGetRecentActivities
      .mockResolvedValueOnce(recentResponse({ as_of: '2026-09-29T08:02:00Z', sync_failure: failure }))
      .mockResolvedValue(recentResponse({ stale: true, sync_failure: failure }));
    const screen = renderHome();

    const failed = await screen.findByTestId('home-activities-sync-failed');
    expect(failed).toHaveTextContent(/Strava · Sync failed/);
    expect(failed).not.toHaveTextContent(/Last synced/);
    expect(screen.getAllByText(/Last synced: /)).toHaveLength(1);
    expect(screen.getByTestId('home-activities-synced-at')).toHaveTextContent(/28 Sep|Sep 28/);
    expect(screen.getByTestId('home-activities-sync-retry').props.className ?? '').toContain('min-h-11');

    fireEvent.press(screen.getByTestId('home-activities-sync-retry'));
    await waitFor(() => expect(mockGetRecentActivities).toHaveBeenLastCalledWith(undefined, { retry: true }));
    // The refresh the retry started is running: the list says it is checking,
    // not that the sync failed.
    expect(await screen.findByTestId('home-activities-refreshing')).toBeTruthy();
    expect(screen.queryByTestId('home-activities-sync-failed')).toBeNull();
  });

  it('announces a failed sync to VoiceOver when it appears, since iOS reads no live region', async () => {
    const originalOS = Platform.OS;
    (Platform as { OS: string }).OS = 'ios';
    const announce = jest.spyOn(AccessibilityInfo, 'announceForAccessibility');
    mockGetRecentActivities.mockResolvedValue(
      recentResponse({
        sync_failure: {
          provider: 'strava',
          provider_name: 'Strava',
          failed_at: '2026-09-29T14:04:00Z',
          last_synced_at: null,
        },
      }),
    );
    const screen = renderHome();

    await screen.findByTestId('home-activities-sync-failed');
    expect(announce).toHaveBeenCalledWith('Strava · Sync failed');
    announce.mockRestore();
    (Platform as { OS: string }).OS = originalOS;
  });

  it('leaves a failed sync to the live region on Android, so TalkBack reads it once', async () => {
    const originalOS = Platform.OS;
    (Platform as { OS: string }).OS = 'android';
    const announce = jest.spyOn(AccessibilityInfo, 'announceForAccessibility');
    mockGetRecentActivities.mockResolvedValue(
      recentResponse({
        sync_failure: {
          provider: 'strava',
          provider_name: 'Strava',
          failed_at: '2026-09-29T14:04:00Z',
          last_synced_at: null,
        },
      }),
    );
    const screen = renderHome();

    const failed = await screen.findByTestId('home-activities-sync-failed');
    expect(failed).toHaveTextContent(/Strava · Sync failed/);
    expect(failed.props.accessibilityLiveRegion).toBe('polite');
    expect(announce).not.toHaveBeenCalled();
    announce.mockRestore();
    (Platform as { OS: string }).OS = originalOS;
  });

  it('says nothing about a failed sync when the last attempt succeeded', async () => {
    const screen = renderHome();

    expect(await screen.findByTestId('home-activities-synced-at')).toBeTruthy();
    expect(screen.queryByTestId('home-activities-sync-failed')).toBeNull();
  });

  it('points an athlete with no provider at Connections', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ activities: [], as_of: null }));
    mockGetProvidersStatus.mockResolvedValue(PROVIDERS_NONE);
    const screen = renderHome();

    expect(await screen.findByTestId('home-activities-no-provider')).toHaveTextContent(
      /Connect a fitness provider to see your recent activities here\./,
    );
    fireEvent.press(screen.getByTestId('home-activities-connect'));
    expect(mockPush).toHaveBeenCalledWith(CONNECTIONS_ROUTE);
  });

  it('tells a connected athlete their activities show up once the provider syncs', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ activities: [] }));
    const screen = renderHome();

    expect(await screen.findByTestId('home-activities-empty')).toHaveTextContent(
      'No activities yet. They show up here once your provider syncs.',
    );
    expect(screen.queryByTestId('home-activities-connect')).toBeNull();
  });

  it('asks the provider status for a list with rows too, and says nothing while every connection is healthy', async () => {
    const screen = renderHome();

    await screen.findByTestId('home-activity-strava-9001');
    await waitFor(() => expect(mockGetProvidersStatus).toHaveBeenCalledTimes(1));
    await mockGetProvidersStatus.mock.results[0].value;
    expect(screen.queryByText('Reconnect needed')).toBeNull();
    expect(screen.queryByTestId('home-activities-no-provider')).toBeNull();
  });

  // The shell's reconnect banner names the connections above every tab; the
  // section repeating it would say the same thing twice on Home.
  it('keeps the rows of a connection to reconnect and leaves naming it to the shell banner', async () => {
    mockGetProvidersStatus.mockResolvedValue(PROVIDERS_RECONNECT);
    const screen = renderHome();

    expect(await screen.findByTestId('home-activity-strava-9001')).toBeTruthy();
    await waitFor(() => expect(mockGetProvidersStatus).toHaveBeenCalledTimes(1));
    await mockGetProvidersStatus.mock.results[0].value;
    expect(screen.getByTestId('home-activity-strava-8998')).toBeTruthy();
    expect(screen.getByTestId('home-activities-synced-at')).toBeTruthy();
    expect(screen.queryByText('Reconnect needed')).toBeNull();
    expect(screen.queryByText(/Reconnect Strava/)).toBeNull();
  });

  // carnet#649: the banner above Home already says to reconnect, and the
  // empty sentence would promise a sync no connection can make.
  it('leaves the section out when there are no rows and every connected provider is flagged', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ activities: [], stale: false }));
    mockGetProvidersStatus.mockResolvedValue(PROVIDERS_ONLY_FLAGGED);
    const screen = renderHome();

    await waitFor(() => expect(mockGetProvidersStatus).toHaveBeenCalledTimes(1));
    await mockGetProvidersStatus.mock.results[0].value;
    await waitFor(() => expect(screen.queryByTestId('home-activities-loading')).toBeNull());
    expect(screen.queryByTestId('home-section-activities')).toBeNull();
    expect(screen.queryByText('Reconnect needed')).toBeNull();
    // Connected, only not usable: never the prompt to connect either.
    expect(screen.queryByTestId('home-activities-no-provider')).toBeNull();
  });

  it('keeps the empty sentence when a connected provider still syncs beside a flagged one', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ activities: [] }));
    mockGetProvidersStatus.mockResolvedValue(PROVIDERS_RECONNECT);
    const screen = renderHome();

    expect(await screen.findByTestId('home-activities-empty')).toBeTruthy();
    expect(screen.queryByText('Reconnect needed')).toBeNull();
  });

  it('shows only the checking line while the server retries a flagged session with no rows', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ activities: [], stale: true }));
    mockGetProvidersStatus.mockResolvedValue(PROVIDERS_ONLY_FLAGGED);
    const screen = renderHome();

    expect(await screen.findByTestId('home-activities-refreshing')).toHaveTextContent(
      'Checking your provider for new activities…',
    );
    await waitFor(() => expect(mockGetProvidersStatus).toHaveBeenCalledTimes(1));
    await mockGetProvidersStatus.mock.results[0].value;
    expect(screen.queryByTestId('home-activities-empty')).toBeNull();
    expect(screen.queryByText('Reconnect needed')).toBeNull();
  });

  it('says nothing about reconnecting while the list has not answered', async () => {
    mockGetRecentActivities.mockReturnValue(new Promise<RecentActivitiesResponse>(() => undefined));
    mockGetProvidersStatus.mockResolvedValue(PROVIDERS_RECONNECT);
    const screen = renderHome();

    await screen.findByTestId('home-activities-loading');
    await waitFor(() => expect(mockGetProvidersStatus).toHaveBeenCalledTimes(1));
    await mockGetProvidersStatus.mock.results[0].value;
    expect(screen.queryByText('Reconnect needed')).toBeNull();
  });

  it('shows a failed list read as a failure with a retry', async () => {
    mockGetRecentActivities.mockRejectedValueOnce(new Error('500')).mockResolvedValue(recentResponse());
    const screen = renderHome();

    expect(await screen.findByTestId('home-activities-error')).toHaveTextContent(
      /^Your recent activities couldn't be loaded\./,
    );
    fireEvent.press(screen.getByTestId('home-activities-retry'));
    expect(await screen.findByTestId('home-activity-strava-9001')).toBeTruthy();
  });

  it('says when it last synced when the cache is fresh', async () => {
    mockGetRecentActivities.mockResolvedValue(
      recentResponse({ stale: false, as_of: '2026-09-22T06:00:00Z' }),
    );
    const screen = renderHome();

    expect(await screen.findByTestId('home-activities-synced-at')).toHaveTextContent(/^Last synced: /);
    expect(screen.queryByTestId('home-activities-refreshing')).toBeNull();
  });

  it('says it is checking the provider while the cache is stale', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ stale: true }));
    const screen = renderHome();

    expect(await screen.findByTestId('home-activities-refreshing')).toHaveTextContent(
      'Checking your provider for new activities…',
    );
  });
});

describe('coming back to Home', () => {
  it('reads each thing once when the list turns out empty', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ activities: [] }));
    const screen = renderHome();

    await screen.findByTestId('home-activities-empty');
    await waitFor(() => expect(mockGetProvidersStatus).toHaveBeenCalledTimes(1));
    // The list turning out empty changes what a refocus reads, never the
    // callback the router holds — so nothing is read a second time.
    expect(mockGetTrainingPlan).toHaveBeenCalledTimes(1);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(1);
  });

  it('reads the plan, the list and the provider status again on a refocus', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ activities: [] }));
    const screen = renderHome();
    await screen.findByTestId('home-activities-empty');
    await waitFor(() => expect(mockGetProvidersStatus).toHaveBeenCalledTimes(1));

    await act(async () => {
      mockFocusCallback?.();
    });

    await waitFor(() => expect(mockGetTrainingPlan).toHaveBeenCalledTimes(2));
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
    expect(mockGetProvidersStatus).toHaveBeenCalledTimes(2);
    // A completed activity's route does not change, so none is asked for.
    expect(mockGetActivityRoute).not.toHaveBeenCalled();
  });

  it('reads the provider status again on a refocus when the list has rows, and no route', async () => {
    const screen = renderHome();
    await screen.findByTestId('home-activity-strava-9001');
    await waitFor(() => expect(mockGetActivityRoute).toHaveBeenCalledTimes(3));
    await waitFor(() => expect(mockGetProvidersStatus).toHaveBeenCalledTimes(1));

    await act(async () => {
      mockFocusCallback?.();
    });

    await waitFor(() => expect(mockGetRecentActivities).toHaveBeenCalledTimes(2));
    expect(mockGetProvidersStatus).toHaveBeenCalledTimes(2);
    expect(mockGetActivityRoute).toHaveBeenCalledTimes(3);
  });

  it('brings the section back once the athlete comes back from reconnecting', async () => {
    mockGetRecentActivities.mockResolvedValue(recentResponse({ activities: [] }));
    mockGetProvidersStatus.mockResolvedValue(PROVIDERS_ONLY_FLAGGED);
    const screen = renderHome();
    await waitFor(() => expect(mockGetProvidersStatus).toHaveBeenCalledTimes(1));
    await mockGetProvidersStatus.mock.results[0].value;
    await waitFor(() => expect(screen.queryByTestId('home-activities-loading')).toBeNull());
    expect(screen.queryByTestId('home-section-activities')).toBeNull();

    // The athlete reconnected while away: from here on every read says so,
    // however many reads the screen made before this point.
    mockGetProvidersStatus.mockResolvedValue(PROVIDERS_CONNECTED);
    const readsBeforeReturn = mockGetProvidersStatus.mock.calls.length;
    await act(async () => {
      mockFocusCallback?.();
    });

    await waitFor(() => expect(mockGetProvidersStatus.mock.calls.length).toBeGreaterThan(readsBeforeReturn));
    expect(await screen.findByTestId('home-activities-empty', {}, { timeout: 5000 })).toBeTruthy();
  });

});

describe('pull to refresh', () => {
  it('reads the plan, the list and the provider status again', async () => {
    const screen = renderHome();
    await screen.findByTestId('home-activity-strava-9001');
    await waitFor(() => expect(mockGetProvidersStatus).toHaveBeenCalledTimes(1));
    expect(mockGetTrainingPlan).toHaveBeenCalledTimes(1);
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(1);

    const scroll = screen.getByTestId('home-scroll');
    await act(async () => {
      scroll.props.refreshControl.props.onRefresh();
    });

    await waitFor(() => expect(mockGetTrainingPlan).toHaveBeenCalledTimes(2));
    expect(mockGetRecentActivities).toHaveBeenCalledTimes(2);
    expect(mockGetProvidersStatus).toHaveBeenCalledTimes(2);
  });
});
