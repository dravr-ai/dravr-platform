// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home route sketch — the shared projection drawn as one react-native-svg path, and where its geometry comes from
// ABOUTME: A polyline costs no request, a row without one reads its route, and a recording without GPS or a malformed polyline draws nothing

import React from 'react';
import { act, render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider, defaultScheduler, notifyManager } from '@tanstack/react-query';
import { Path } from 'react-native-svg';
import { decodePolyline, projectRouteToSvgPath } from '@pierre/domain-utils';
import { HOME_ROUTE_UNAVAILABLE_RECHECK_DELAYS_MS } from '@pierre/shared-constants';
import type { ActivityRouteResponse } from '@pierre/shared-types';

import {
  ACTIVITIES,
  NO_GPS_ROUTE_RESPONSE,
  SUMMARY_POLYLINE,
  TRAIL_ROUTE,
  TRAIL_ROUTE_RESPONSE,
} from '../integration/app/helpers/homeFixtures';

const mockGetActivityRoute = jest.fn<Promise<ActivityRouteResponse>, [string, string]>();
jest.mock('../src/services/api', () => ({
  athleteApi: { getActivityRoute: (provider: string, id: string) => mockGetActivityRoute(provider, id) },
}));

import { ActivitySketch, RouteSketch, SKETCH_SIZE } from '../src/screens/home/RouteSketch';

/** The box the sketch projects into; the component's padding is 6. */
const BOX = { width: SKETCH_SIZE, height: SKETCH_SIZE, padding: 6 };

function renderWithClient(ui: React.ReactElement) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  return render(<QueryClientProvider client={client}>{ui}</QueryClientProvider>);
}

beforeEach(() => {
  mockGetActivityRoute.mockReset();
  mockGetActivityRoute.mockResolvedValue(TRAIL_ROUTE_RESPONSE);
});

describe('RouteSketch', () => {
  it('draws the shared projection verbatim, as one path in the route orange', () => {
    const points = decodePolyline(SUMMARY_POLYLINE) ?? [];
    const screen = render(<RouteSketch points={points} testID="sketch" />);

    const paths = screen.UNSAFE_getAllByType(Path);
    expect(paths).toHaveLength(1);
    expect(paths[0].props.d).toBe(projectRouteToSvgPath(points, BOX));
    // Three points, three vertices: north up, so the northernmost fix is on top.
    expect(paths[0].props.d).toMatch(/^M[\d.]+ [\d.]+ L[\d.]+ [\d.]+ L[\d.]+ [\d.]+$/);
    expect(paths[0].props.fill).toBe('none');
    expect(paths[0].props.stroke).toBe('#d9480f');
    expect(screen.getByTestId('sketch').props.accessibilityLabel).toBe('Sketch of the route');
    expect(screen.getByTestId('sketch').props.accessibilityRole).toBe('image');
  });

  it('draws nothing for a track with nothing to draw', () => {
    const screen = render(<RouteSketch points={[[45.5, -73.6]]} testID="sketch" />);

    expect(screen.UNSAFE_queryAllByType(Path)).toHaveLength(0);
    expect(screen.queryByTestId('sketch')).toBeNull();
  });
});

describe('ActivitySketch', () => {
  it('sketches a Strava row from its polyline without asking for a route', () => {
    const screen = renderWithClient(<ActivitySketch activity={ACTIVITIES[1]} />);

    expect(screen.getByTestId('home-activity-sketch-strava-9000')).toBeTruthy();
    expect(screen.UNSAFE_getByType(Path).props.d).toBe(
      projectRouteToSvgPath(decodePolyline(SUMMARY_POLYLINE) ?? [], BOX),
    );
    expect(mockGetActivityRoute).not.toHaveBeenCalled();
  });

  it('asks for the route of a row that carries no polyline, and sketches the answer', async () => {
    expect(ACTIVITIES[2]).toMatchObject({ has_gps: true, summary_polyline: null });
    const screen = renderWithClient(<ActivitySketch activity={ACTIVITIES[2]} />);

    await waitFor(() => expect(screen.getByTestId('home-activity-sketch-intervals_icu-i77')).toBeTruthy());
    expect(mockGetActivityRoute).toHaveBeenCalledWith('intervals_icu', 'i77');
    expect(screen.UNSAFE_getByType(Path).props.d).toBe(projectRouteToSvgPath(TRAIL_ROUTE.coordinates, BOX));
  });

  it('draws no sketch for a row whose route is refused', async () => {
    mockGetActivityRoute.mockResolvedValue({ route: null, reason: 'too_short' });
    const screen = renderWithClient(<ActivitySketch activity={ACTIVITIES[2]} />);

    await waitFor(() => expect(mockGetActivityRoute).toHaveBeenCalledTimes(1));
    expect(screen.queryByTestId('home-activity-sketch-intervals_icu-i77')).toBeNull();
  });

  it('draws no sketch for a row whose route read answers that the recording held no GPS', async () => {
    expect(ACTIVITIES[4]).toMatchObject({ has_gps: true, summary_polyline: null });
    mockGetActivityRoute.mockResolvedValue(NO_GPS_ROUTE_RESPONSE);
    const screen = renderWithClient(<ActivitySketch activity={ACTIVITIES[4]} />);

    await waitFor(() => expect(mockGetActivityRoute).toHaveBeenCalledTimes(1));
    expect(mockGetActivityRoute).toHaveBeenCalledWith('strava', '8998');
    await mockGetActivityRoute.mock.results[0].value;
    expect(screen.queryByTestId('home-activity-sketch-strava-8998')).toBeNull();
    expect(screen.UNSAFE_queryAllByType(Path)).toHaveLength(0);
  });

  it('asks for nothing and draws nothing for a row whose route read held no GPS', () => {
    expect(ACTIVITIES[3].has_gps).toBe(false);
    const screen = renderWithClient(<ActivitySketch activity={ACTIVITIES[3]} />);

    expect(screen.queryByTestId('home-activity-sketch-strava-8999')).toBeNull();
    expect(mockGetActivityRoute).not.toHaveBeenCalled();
  });

  it('draws nothing for a polyline that does not decode, rather than spending a request', () => {
    const screen = renderWithClient(
      <ActivitySketch activity={{ ...ACTIVITIES[1], summary_polyline: '_p~iF~ps|U_' }} />,
    );

    expect(screen.queryByTestId('home-activity-sketch-strava-9000')).toBeNull();
    expect(mockGetActivityRoute).not.toHaveBeenCalled();
  });
});

// The server answers `unavailable` within its bound while the provider read it
// started keeps running, and stores what that read says: a sketch left empty
// by that answer is asked again on a schedule with an end, and a `no_gps` the
// server has since taken back is never held as settled.
describe('ActivitySketch after a route answer the server can take back', () => {
  afterEach(() => {
    notifyManager.setScheduler(defaultScheduler);
    jest.useRealTimers();
  });

  /** Use a fake clock, with React Query's notifications run inline so it only drives the schedule. */
  function fakeClock() {
    jest.useFakeTimers();
    notifyManager.setScheduler((callback) => callback());
  }

  /** Let the clock run, and whatever falls due in that time settle. */
  async function elapse(ms: number) {
    await act(async () => {
      await jest.advanceTimersByTimeAsync(ms);
    });
  }

  it('fills in once the read the server started lands, and asks nothing past the schedule', async () => {
    fakeClock();
    mockGetActivityRoute.mockResolvedValueOnce({ route: null, reason: 'unavailable' });
    const screen = renderWithClient(<ActivitySketch activity={ACTIVITIES[2]} />);

    await elapse(0);
    expect(mockGetActivityRoute).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('home-activity-sketch-intervals_icu-i77')).toBeNull();

    await elapse(HOME_ROUTE_UNAVAILABLE_RECHECK_DELAYS_MS[0]);
    expect(mockGetActivityRoute).toHaveBeenCalledTimes(2);
    expect(screen.getByTestId('home-activity-sketch-intervals_icu-i77')).toBeTruthy();
    expect(screen.UNSAFE_getByType(Path).props.d).toBe(projectRouteToSvgPath(TRAIL_ROUTE.coordinates, BOX));

    // A drawn route never changes: nothing more is asked.
    await elapse(HOME_ROUTE_UNAVAILABLE_RECHECK_DELAYS_MS.reduce((sum, delay) => sum + delay, 0) * 4);
    expect(mockGetActivityRoute).toHaveBeenCalledTimes(2);
  });

  it('stops asking an unavailable route once its schedule has run out', async () => {
    fakeClock();
    mockGetActivityRoute.mockResolvedValue({ route: null, reason: 'unavailable' });
    const screen = renderWithClient(<ActivitySketch activity={ACTIVITIES[2]} />);

    await elapse(0);
    for (const delay of HOME_ROUTE_UNAVAILABLE_RECHECK_DELAYS_MS) {
      await elapse(delay);
    }
    const asked = HOME_ROUTE_UNAVAILABLE_RECHECK_DELAYS_MS.length + 1;
    expect(mockGetActivityRoute).toHaveBeenCalledTimes(asked);
    await elapse(HOME_ROUTE_UNAVAILABLE_RECHECK_DELAYS_MS.reduce((sum, delay) => sum + delay, 0) * 4);
    expect(mockGetActivityRoute).toHaveBeenCalledTimes(asked);
    expect(screen.queryByTestId('home-activity-sketch-intervals_icu-i77')).toBeNull();
  });

  it('asks again for a held no_gps answer when the sketch is shown again, and draws what the server says now', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const sketch = (
      <QueryClientProvider client={client}>
        <ActivitySketch activity={ACTIVITIES[4]} />
      </QueryClientProvider>
    );
    // The incident's answer: a failing scraper had the route stored as no GPS.
    mockGetActivityRoute.mockResolvedValueOnce(NO_GPS_ROUTE_RESPONSE);
    const first = render(sketch);
    await waitFor(() => expect(mockGetActivityRoute).toHaveBeenCalledTimes(1));
    await mockGetActivityRoute.mock.results[0].value;
    expect(first.queryByTestId('home-activity-sketch-strava-8998')).toBeNull();
    first.unmount();

    // The server has since deleted that row and reads the track.
    const second = render(sketch);
    await waitFor(() => expect(second.getByTestId('home-activity-sketch-strava-8998')).toBeTruthy());
    expect(mockGetActivityRoute).toHaveBeenCalledTimes(2);
  });

  it('keeps a drawn route for good: showing the sketch again asks nothing', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    const sketch = (
      <QueryClientProvider client={client}>
        <ActivitySketch activity={ACTIVITIES[2]} />
      </QueryClientProvider>
    );
    const first = render(sketch);
    await waitFor(() => expect(first.getByTestId('home-activity-sketch-intervals_icu-i77')).toBeTruthy());
    first.unmount();

    const second = render(sketch);
    expect(second.getByTestId('home-activity-sketch-intervals_icu-i77')).toBeTruthy();
    expect(mockGetActivityRoute).toHaveBeenCalledTimes(1);
  });
});
