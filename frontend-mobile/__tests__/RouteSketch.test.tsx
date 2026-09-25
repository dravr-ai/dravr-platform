// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home route sketch — the shared projection drawn as one react-native-svg path, and where its geometry comes from
// ABOUTME: A polyline costs no request, a GPS row without one reads its stored route, and an indoor or malformed row draws nothing

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { Path } from 'react-native-svg';
import { decodePolyline, projectRouteToSvgPath } from '@pierre/domain-utils';
import type { ActivityRouteResponse } from '@pierre/shared-types';

import { ACTIVITIES, SUMMARY_POLYLINE, TRAIL_ROUTE, TRAIL_ROUTE_RESPONSE } from '../integration/app/helpers/homeFixtures';

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
  it('draws the shared projection verbatim, as one path in the primary ink', () => {
    const points = decodePolyline(SUMMARY_POLYLINE) ?? [];
    const screen = render(<RouteSketch points={points} testID="sketch" />);

    const paths = screen.UNSAFE_getAllByType(Path);
    expect(paths).toHaveLength(1);
    expect(paths[0].props.d).toBe(projectRouteToSvgPath(points, BOX));
    // Three points, three vertices: north up, so the northernmost fix is on top.
    expect(paths[0].props.d).toMatch(/^M[\d.]+ [\d.]+ L[\d.]+ [\d.]+ L[\d.]+ [\d.]+$/);
    expect(paths[0].props.fill).toBe('none');
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

  it('sketches a GPS row without a polyline from its stored route', async () => {
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

  it('asks for nothing and draws nothing for an indoor activity', () => {
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
