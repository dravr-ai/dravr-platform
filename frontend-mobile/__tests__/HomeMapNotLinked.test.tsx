// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Home in Expo Go, where MapLibre's native modules are not registered — the map says it cannot be drawn, in its own container
// ABOUTME: Nothing is required or thrown there: under Metro either one puts a full-screen error overlay over the whole tab

import React from 'react';
import { NativeModules } from 'react-native';
import { render, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';

import { MAP_NATIVE_MODULE } from '../src/screens/chat/routeCardLoader';

// Records whether anything evaluated the package. Under Metro, evaluating it
// where the native side is missing is what put the fatal error screen up.
const mockPackageEvaluated = jest.fn();
jest.mock('@maplibre/maplibre-react-native', () => {
  mockPackageEvaluated();
  throw new Error("TurboModuleRegistry.getEnforcing(...): 'MLRNCameraModule' could not be found");
});

jest.mock('../src/services/api', () => {
  const fixtures = jest.requireActual('../integration/app/helpers/homeFixtures');
  return {
    athleteApi: {
      getTrainingPlan: jest.fn(async () => fixtures.PLAN_RESPONSE),
      getRecentActivities: jest.fn(async () => fixtures.recentResponse()),
      getActivityRoute: jest.fn(async () => fixtures.LATEST_ROUTE_RESPONSE),
    },
    oauthApi: { getProvidersStatus: jest.fn(async () => fixtures.PROVIDERS_CONNECTED) },
  };
});

import { HomeScreen } from '../src/screens/home/HomeScreen';

describe('Home where MapLibre is not linked in', () => {
  const linked = NativeModules[MAP_NATIVE_MODULE];

  let consoleError: jest.SpyInstance;

  beforeEach(() => {
    delete NativeModules[MAP_NATIVE_MODULE];
    // Spied, not silenced for convenience: nothing may be reported here.
    consoleError = jest.spyOn(console, 'error').mockImplementation(() => undefined);
  });

  afterEach(() => {
    NativeModules[MAP_NATIVE_MODULE] = linked;
    jest.restoreAllMocks();
  });

  it('says the map cannot be drawn inside its container, without loading the package', async () => {
    const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
    const screen = render(
      <QueryClientProvider client={client}>
        <HomeScreen />
      </QueryClientProvider>,
    );

    const sentence = await screen.findByTestId('home-latest-map-unavailable');
    expect(sentence).toHaveTextContent("The map couldn't be loaded.");
    expect(
      within(screen.getByTestId('home-latest-map')).getByTestId('home-latest-map-unavailable'),
    ).toBeTruthy();
    expect(screen.queryByText('This activity recorded no GPS track.')).toBeNull();
    expect(mockPackageEvaluated).not.toHaveBeenCalled();
    // A missing native side is not an error: React reports a caught render
    // error through console.error, which a dev build shows as a full-screen
    // overlay, so none may reach it.
    const logged = consoleError.mock.calls.map((call) => call.map(String).join(' ')).join('\n');
    expect(logged).not.toContain('native_module_not_linked');
    expect(logged).not.toContain('RouteCardUnavailableError');
    expect(logged).not.toContain('MLRNCameraModule');
    // The rest of Home is still there.
    expect(screen.getByTestId('home-today')).toBeTruthy();
    expect(screen.getByTestId('home-activity-strava-9001')).toBeTruthy();
  });
});
