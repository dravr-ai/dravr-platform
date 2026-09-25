// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Home in a runtime without MapLibre linked in (Expo Go) — the latest activity's map becomes a sentence, the page stays
// ABOUTME: Loading the native module throws there; the route boundary must catch it rather than unmount the whole tab

import React from 'react';
import { render } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

import { LATEST_ROUTE_RESPONSE, PLAN_RESPONSE, recentResponse } from '../integration/app/helpers/homeFixtures';

// What requiring the package does where its native side is missing.
jest.mock('@maplibre/maplibre-react-native', () => {
  throw new Error("Invariant Violation: Native module 'MLRNModule' is not available in Expo Go");
});

jest.mock('../src/services/api', () => {
  const fixtures = jest.requireActual('../integration/app/helpers/homeFixtures');
  return {
    athleteApi: {
      getTrainingPlan: jest.fn(async () => fixtures.PLAN_RESPONSE),
      getRecentActivities: jest.fn(async () => fixtures.recentResponse()),
      getActivityRoute: jest.fn(async () => fixtures.LATEST_ROUTE_RESPONSE),
    },
    oauthApi: { getProvidersStatus: jest.fn() },
  };
});

import { HomeScreen } from '../src/screens/home/HomeScreen';

describe('Home without a native map', () => {
  beforeEach(() => {
    // React reports the caught load failure; the boundary is what is under test.
    jest.spyOn(console, 'error').mockImplementation(() => undefined);
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it('says the map cannot be drawn and keeps every other section', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
    const screen = render(
      <QueryClientProvider client={client}>
        <HomeScreen />
      </QueryClientProvider>,
    );

    // The route came back; only the drawing of it failed.
    expect(LATEST_ROUTE_RESPONSE.route).not.toBeNull();
    // The activity has a track, so the sentence says the map failed — not
    // that there was nothing to map.
    expect(await screen.findByTestId('home-latest-map-unavailable')).toHaveTextContent(
      "The map couldn't be loaded.",
    );
    expect(screen.queryByText('This activity recorded no GPS track.')).toBeNull();
    expect(screen.queryByTestId('route-track')).toBeNull();
    // The rest of Home is still there.
    expect(screen.getByTestId('home-today')).toBeTruthy();
    expect(screen.getByTestId('home-activity-strava-9001')).toBeTruthy();
    expect(screen.getByTestId('home-activity-sketch-strava-9000')).toBeTruthy();
    expect(PLAN_RESPONSE.plan).not.toBeNull();
    expect(recentResponse().activities).toHaveLength(5);
  });
});
