// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Home when requiring the route card yields undefined, as Metro's guarded require does after reporting a throw
// ABOUTME: The loader must reject that empty module so the boundary answers, never hand React.lazy an undefined to trip on

import React from 'react';
import { render, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

// What Metro's `require` returns for a module whose evaluation threw outside
// a module factory: nothing, the throw having gone to `reportFatalError`.
jest.mock('../src/screens/chat/RouteView', () => undefined);

jest.mock('../src/services/api', () => {
  const fixtures = jest.requireActual('../integration/app/helpers/homeFixtures');
  return {
    athleteApi: {
      getTrainingPlan: jest.fn(async () => fixtures.PLAN_RESPONSE),
      getTrainingStatus: jest.fn(async () => fixtures.STATUS_RESPONSE),
      getRecentActivities: jest.fn(async () => fixtures.recentResponse()),
      getActivityRoute: jest.fn(async () => fixtures.LATEST_ROUTE_RESPONSE),
    },
    oauthApi: { getProvidersStatus: jest.fn(async () => fixtures.PROVIDERS_CONNECTED) },
  };
});

import { HomeScreen } from '../src/screens/home/HomeScreen';

describe('Home when the route card module comes back empty', () => {
  let consoleError: jest.SpyInstance;

  beforeEach(() => {
    // React reports the caught load failure; the boundary is what is under test.
    consoleError = jest.spyOn(console, 'error').mockImplementation(() => undefined);
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  it('says the map cannot be drawn inside its container, and React.lazy never sees undefined', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
    const screen = render(
      <QueryClientProvider client={client}>
        <HomeScreen />
      </QueryClientProvider>,
    );

    expect(await screen.findByTestId('home-latest-map-unavailable')).toHaveTextContent(
      "The map couldn't be loaded.",
    );
    expect(
      within(screen.getByTestId('home-latest-map')).getByTestId('home-latest-map-unavailable'),
    ).toBeTruthy();
    expect(screen.queryByText('This activity recorded no GPS track.')).toBeNull();
    // The rejection carried the loader's reason; React's own complaint about
    // an empty dynamic import, and the TypeError after it, never happened.
    const logged = consoleError.mock.calls.map((call) => call.map(String).join(' ')).join('\n');
    expect(logged).not.toContain('Expected the result of a dynamic import() call');
    expect(logged).not.toContain("right operand of 'in' is not an object");
    expect(logged).not.toContain("Cannot use 'in' operator");
    expect(screen.getByTestId('home-today')).toBeTruthy();
  });
});
