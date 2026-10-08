// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins which recorded activity maps carry start, finish and distance markers on the phone
// ABOUTME: An on-foot activity with the route_km_markers beta armed gets them; a ride, or the flag off, does not

import React from 'react';
import { render, waitFor } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { HomeActivity } from '@pierre/shared-types';

import { ACTIVITIES, LATEST_ROUTE_RESPONSE } from '../integration/app/helpers/homeFixtures';

const mockGetMyFeatures = jest.fn();

jest.mock('../src/services/api', () => {
  const fixtures = jest.requireActual('../integration/app/helpers/homeFixtures');
  return {
    athleteApi: { getActivityRoute: jest.fn(async () => fixtures.LATEST_ROUTE_RESPONSE) },
    featureFlagsApi: { getMyFeatures: (...args: unknown[]) => mockGetMyFeatures(...args) },
  };
});

import { ActivityMap } from '../src/screens/home/ActivityMap';

const RUN = ACTIVITIES.find((activity) => activity.sport_type === 'run') as HomeActivity;
const RIDE = ACTIVITIES.find((activity) => activity.sport_type === 'ride') as HomeActivity;

function renderMap(activity: HomeActivity, armed: boolean) {
  mockGetMyFeatures.mockResolvedValue({ flags: { route_km_markers: armed }, known: [] });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  const screen = render(
    <QueryClientProvider client={client}>
      <ActivityMap activity={activity} testIDPrefix="activity" />
    </QueryClientProvider>,
  );
  return { screen, client };
}

describe('ActivityMap markers', () => {
  it('marks the start and the finish of a run when the beta is armed', async () => {
    expect(LATEST_ROUTE_RESPONSE.route).not.toBeNull();
    const { screen } = renderMap(RUN, true);
    expect(await screen.findByTestId('route-map-marker-start')).toBeTruthy();
    expect(screen.getByTestId('route-map-marker-finish')).toBeTruthy();
  });

  it('draws a ride with no marker, the beta armed', async () => {
    const { screen, client } = renderMap(RIDE, true);
    expect(await screen.findByTestId('route-map')).toBeTruthy();
    await waitFor(() => expect(client.isFetching()).toBe(0));
    expect(screen.queryByTestId('route-map-marker-start')).toBeNull();
  });

  it('draws a run with no marker when the beta is off', async () => {
    const { screen, client } = renderMap(RUN, false);
    expect(await screen.findByTestId('route-map')).toBeTruthy();
    await waitFor(() => expect(client.isFetching()).toBe(0));
    expect(screen.queryByTestId('route-map-marker-start')).toBeNull();
  });
});
