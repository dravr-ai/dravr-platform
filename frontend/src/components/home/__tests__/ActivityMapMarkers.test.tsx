// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Pins which recorded activity maps carry start, finish and distance markers on web
// ABOUTME: An on-foot activity with the route_km_markers beta armed gets them; a ride, or the flag off, does not

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ActivityRouteResponse, FeatureFlagMap } from '@pierre/shared-types';
import type { DistanceUnit } from '@pierre/chat-utils';
import { UnitsContext } from '@pierre/ui-logic';
import { ThemeProvider } from '../../../hooks/useTheme';
import { ActivityMap } from '../ActivityMap';
import { activity, routeView } from './homeFixtures';

const api = vi.hoisted(() => ({
  getActivityRoute: vi.fn<(provider: string, id: string) => Promise<ActivityRouteResponse>>(),
  getMyFeatures: vi.fn<() => Promise<{ flags: FeatureFlagMap; known: [] }>>(),
}));

vi.mock('../../../services/api', () => ({
  athleteApi: { getActivityRoute: api.getActivityRoute },
  featureFlagsApi: { getMyFeatures: api.getMyFeatures },
}));

/** Every DOM marker the map pinned, by kind. */
const pinned = vi.hoisted(() => ({ kinds: [] as string[], maps: 0 }));
vi.mock('maplibre-gl', () => ({
  Map: class {
    constructor() {
      pinned.maps += 1;
    }
    addControl() {}
    on() {}
    setStyle() {}
    remove() {}
  },
  Marker: class {
    kind: string;
    constructor(options: { element: HTMLElement }) {
      this.kind = options.element.dataset.routeMarker ?? '';
    }
    setLngLat() {
      return this;
    }
    addTo() {
      pinned.kinds.push(this.kind);
      return this;
    }
    remove() {
      return this;
    }
  },
  AttributionControl: class {},
  NavigationControl: class {},
  setWorkerUrl: () => {},
}));

function renderMap(sport: string, armed: boolean, unit: DistanceUnit = 'metric') {
  api.getMyFeatures.mockResolvedValue({ flags: { route_km_markers: armed }, known: [] });
  const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  render(
    <QueryClientProvider client={queryClient}>
      <ThemeProvider>
        <UnitsContext.Provider value={unit}>
          <ActivityMap activity={activity({ id: 'act-1', sport_type: sport })} />
        </UnitsContext.Provider>
      </ThemeProvider>
    </QueryClientProvider>
  );
  return queryClient;
}

beforeEach(() => {
  vi.clearAllMocks();
  pinned.kinds.length = 0;
  pinned.maps = 0;
  api.getActivityRoute.mockResolvedValue({ route: routeView('strava'), reason: null });
});

describe('ActivityMap markers', () => {
  it('marks the start and the finish of a run when the beta is armed', async () => {
    renderMap('run', true);
    await waitFor(() => expect(pinned.kinds).toContain('start'));
    expect(pinned.kinds).toContain('finish');
    expect(pinned.kinds[pinned.kinds.length - 1]).toBe('start');
  });

  it('counts the distance marks in the athlete units (carnet#835)', async () => {
    // The fixture's track runs 4.5 km: four kilometre marks, two mile marks.
    renderMap('run', true);
    await waitFor(() => expect(pinned.kinds).toContain('finish'));
    expect(pinned.kinds.filter((kind) => kind === 'distance')).toHaveLength(4);

    pinned.kinds.length = 0;
    renderMap('run', true, 'imperial');
    await waitFor(() => expect(pinned.kinds).toContain('finish'));
    expect(pinned.kinds.filter((kind) => kind === 'distance')).toHaveLength(2);
  });

  it('marks a hike and a trail run the same way', async () => {
    renderMap('trail_run', true);
    await waitFor(() => expect(pinned.kinds).toContain('start'));
  });

  it('draws a ride with no marker, the beta armed or not', async () => {
    const client = renderMap('ride', true);
    await screen.findByRole('figure');
    await waitFor(() => expect(client.isFetching()).toBe(0));
    await waitFor(() => expect(pinned.maps).toBeGreaterThan(0));
    expect(pinned.kinds).toEqual([]);
  });

  it('draws a run with no marker when the beta is off', async () => {
    const client = renderMap('run', false);
    await screen.findByRole('figure');
    await waitFor(() => expect(client.isFetching()).toBe(0));
    await waitFor(() => expect(pinned.maps).toBeGreaterThan(0));
    expect(pinned.kinds).toEqual([]);
  });
});
