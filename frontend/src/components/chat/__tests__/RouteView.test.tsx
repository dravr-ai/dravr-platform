// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the route block — the lat/lon transpose, the climb slices, and the two-scheme basemap
// ABOUTME: Red the moment a route renders as a picture of a map instead of a framed MapLibre track

import { describe, it, expect, beforeEach, vi } from 'vitest';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { Map as MapLibreMap } from 'maplibre-gl';
import type { RenderBlock, RouteView as RouteViewData } from '@pierre/scene-types';
import { BOREAL } from '@pierre/shared-constants';
import { ThemeProvider, useTheme } from '../../../hooks/useTheme';
import RouteView from '../RouteView';
import { SceneView } from '../SceneView';
import {
  addRouteLayers,
  BASEMAP_STYLE,
  climbGeometry,
  CLIMB_SOURCE,
  routeInk,
  TRACK_SOURCE,
  trackGeometry,
  viewportBounds,
} from '../routeLayers';

interface LayerSpec {
  id: string;
  source: string;
  paint: Record<string, unknown>;
  layout: Record<string, unknown>;
}

/**
 * A MapLibre stand-in. jsdom has no WebGL, so a real `Map` cannot be
 * constructed at all — but every fact this component is responsible for is a
 * fact about the calls it makes, so the stub records them and the assertions
 * read them back.
 */
const harness = vi.hoisted(() => {
  const sources = new Map<string, unknown>();
  const layers: Array<{ id: string; source: string; paint: Record<string, unknown> }> = [];
  const handlers = new Map<string, () => void>();
  const constructed: Array<Record<string, unknown>> = [];
  const controls: Array<[string, string]> = [];

  const instance = {
    addControl: vi.fn(),
    addSource: vi.fn((id: string, spec: unknown) => {
      sources.set(id, spec);
    }),
    addLayer: vi.fn((spec: LayerSpec) => {
      layers.push({ id: spec.id, source: spec.source, paint: spec.paint });
    }),
    getSource: vi.fn((id: string) => sources.get(id)),
    on: vi.fn((event: string, handler: () => void) => {
      handlers.set(event, handler);
    }),
    // A style swap discards every source and layer with it — the reason
    // `addRouteLayers` has to be both idempotent and re-runnable.
    setStyle: vi.fn(() => {
      sources.clear();
      layers.length = 0;
    }),
    remove: vi.fn(),
  };

  return {
    instance,
    sources,
    layers,
    handlers,
    constructed,
    controls,
    reset() {
      sources.clear();
      layers.length = 0;
      handlers.clear();
      constructed.length = 0;
      controls.length = 0;
    },
  };
});

vi.mock('maplibre-gl', () => ({
  Map: class {
    constructor(options: Record<string, unknown>) {
      harness.constructed.push(options);
      return harness.instance;
    }
  },
  AttributionControl: class {
    constructor(options: { compact?: boolean }) {
      harness.controls.push(['attribution', String(options.compact)]);
    }
  },
  NavigationControl: class {
    constructor(options: { showCompass?: boolean }) {
      harness.controls.push(['navigation', String(options.showCompass)]);
    }
  },
}));

/** A Mont Royal loop: five fixes, one three-point climb and one that is a point. */
const TRACK: Array<[number, number]> = [
  [45.5, -73.6],
  [45.52, -73.62],
  [45.54, -73.64],
  [45.56, -73.66],
  [45.58, -73.68],
];

const ROUTE: RouteViewData = {
  coordinates: TRACK,
  bounds: {
    min_latitude: 45.5,
    max_latitude: 45.58,
    min_longitude: -73.68,
    max_longitude: -73.6,
  },
  elevation_meters: [24, 61, 118, 152, 141],
  distances_meters: [0, 2000, 4000, 8000, 12500],
  climbs: [
    { start_index: 1, end_index: 3, avg_gradient: 6.4, category: '3' },
    { start_index: 4, end_index: 4, avg_gradient: 2.1, category: '4' },
  ],
  title: 'Mont Royal loop',
  source_tool: 'get_activity_route',
};

const map = harness.instance as unknown as MapLibreMap;

function paintFor(id: string): Record<string, unknown> {
  const layer = harness.layers.find((candidate) => candidate.id === id);
  if (!layer) throw new Error(`no layer ${id}: ${harness.layers.map((l) => l.id).join(', ')}`);
  return layer.paint;
}

/** A control that flips the athlete's scheme, so both themes are exercised. */
function SchemeSwitch() {
  const { setPreference } = useTheme();
  return (
    <button type="button" onClick={() => setPreference('light')}>
      go light
    </button>
  );
}

beforeEach(() => {
  harness.reset();
  vi.clearAllMocks();
  window.localStorage.clear();
});

describe('route geometry', () => {
  it('transposes the carried (latitude, longitude) into GeoJSON [longitude, latitude]', () => {
    const line = trackGeometry([
      [45.5, -73.6],
      [45.58, -73.68],
    ]);

    expect(line.type).toBe('LineString');
    expect(line.coordinates).toEqual([
      [-73.6, 45.5],
      [-73.68, 45.58],
    ]);
  });

  it('slices a climb inclusively and drops one that cannot be a line', () => {
    const climbs = climbGeometry(TRACK, ROUTE.climbs);

    expect(climbs.type).toBe('MultiLineString');
    // The second climb spans a single fix, so it is not a line at all.
    expect(climbs.coordinates).toEqual([
      [
        [-73.62, 45.52],
        [-73.64, 45.54],
        [-73.66, 45.56],
      ],
    ]);
  });

  it('frames the carried extent as [[west, south], [east, north]]', () => {
    expect(viewportBounds(ROUTE.bounds)).toEqual([
      [-73.68, 45.5],
      [-73.6, 45.58],
    ]);
  });

  it('inks the track in the accent and the climbs in the body ink, per scheme', () => {
    expect(routeInk('light').track).toBe(BOREAL.light.primary);
    expect(routeInk('light').climb).toBe(BOREAL.light.onSurface);
    expect(routeInk('dark').track).toBe(BOREAL.dark.primary);
    expect(routeInk('dark').climb).toBe(BOREAL.dark.onSurface);
    // The two lines must never resolve to one colour, in either scheme.
    expect(routeInk('light').track).not.toBe(routeInk('light').climb);
    expect(routeInk('dark').track).not.toBe(routeInk('dark').climb);
  });
});

describe('route layers', () => {
  it('paints a casing, the track and a dashed climb, and adds nothing twice', () => {
    const ink = routeInk('dark');
    addRouteLayers(map, trackGeometry(TRACK), climbGeometry(TRACK, ROUTE.climbs), ink);

    expect(harness.sources.has(TRACK_SOURCE)).toBe(true);
    expect(harness.sources.has(CLIMB_SOURCE)).toBe(true);
    expect(harness.layers.map((layer) => layer.id)).toEqual([
      'route-casing',
      'route-line',
      'route-climb',
    ]);

    expect(paintFor('route-casing')['line-color']).toBe('#ffffff');
    expect(paintFor('route-line')['line-color']).toBe(BOREAL.dark.primary);

    const climb = paintFor('route-climb');
    expect(climb['line-color']).toBe(BOREAL.dark.onSurface);
    // The dash is the signal that survives a colourblind reader and a
    // greyscale screenshot — DESIGN.md §8.
    expect(climb['line-dasharray']).toEqual([1.4, 1.1]);
    // Heavier than the track it is laid over, so the accent shows in the gaps.
    expect(Number(climb['line-width'])).toBeGreaterThan(
      Number(paintFor('route-line')['line-width'])
    );

    addRouteLayers(map, trackGeometry(TRACK), climbGeometry(TRACK, ROUTE.climbs), ink);
    expect(harness.layers).toHaveLength(3);
  });
});

describe('RouteView', () => {
  it('builds a map framed on the carried bounds and paints the track when the style loads', async () => {
    render(
      <ThemeProvider>
        <RouteView view={ROUTE} />
      </ThemeProvider>
    );

    await waitFor(() => expect(harness.constructed).toHaveLength(1));
    const options = harness.constructed[0];
    expect(options.bounds).toEqual([
      [-73.68, 45.5],
      [-73.6, 45.58],
    ]);
    // Dravr is dark-first, so an athlete with no stored preference gets the
    // dark basemap rather than a paper-white lamp on the near-black canvas.
    expect(options.style).toBe(BASEMAP_STYLE.dark);
    // A wheel or a one-finger drag has to scroll the thread, not pan the map.
    expect(options.cooperativeGestures).toBe(true);
    // MapLibre's own attribution lands under the zoom stack; the card docks a
    // compact one opposite instead.
    expect(options.attributionControl).toBe(false);
    expect(harness.controls).toEqual([
      ['attribution', 'true'],
      ['navigation', 'false'],
    ]);

    // Nothing is drawn until the basemap style is up.
    expect(harness.layers).toHaveLength(0);
    harness.handlers.get('style.load')?.();
    expect(harness.layers.map((layer) => layer.id)).toEqual([
      'route-casing',
      'route-line',
      'route-climb',
    ]);
    expect(paintFor('route-line')['line-color']).toBe(BOREAL.dark.primary);
  });

  it('repaints on the light basemap in the light scheme, in the light inks', async () => {
    const user = userEvent.setup();
    render(
      <ThemeProvider>
        <SchemeSwitch />
        <RouteView view={ROUTE} />
      </ThemeProvider>
    );

    await waitFor(() => expect(harness.constructed).toHaveLength(1));
    harness.handlers.get('style.load')?.();
    expect(paintFor('route-line')['line-color']).toBe(BOREAL.dark.primary);

    await user.click(screen.getByRole('button', { name: 'go light' }));

    await waitFor(() => expect(harness.instance.setStyle).toHaveBeenCalledWith(BASEMAP_STYLE.light));
    // The swap took the layers with it; the reload paints them in light ink.
    harness.handlers.get('style.load')?.();
    expect(paintFor('route-line')['line-color']).toBe(BOREAL.light.primary);
    expect(paintFor('route-climb')['line-color']).toBe(BOREAL.light.onSurface);
    // One map, restyled — not a second map built over the first.
    expect(harness.constructed).toHaveLength(1);
  });

  it('prints the distance and every climb, including one the map cannot draw', async () => {
    render(
      <ThemeProvider>
        <RouteView view={ROUTE} />
      </ThemeProvider>
    );

    await waitFor(() => expect(harness.constructed).toHaveLength(1));

    expect(screen.getByText('Mont Royal loop')).toBeInTheDocument();
    // The last cumulative distance IS the total; nothing is summed here.
    expect(screen.getByText('12.5 km')).toBeInTheDocument();

    expect(screen.getByText('Cat 3')).toBeInTheDocument();
    expect(screen.getByText('6.4%')).toBeInTheDocument();
    expect(screen.getByText('km 2.0–8.0')).toBeInTheDocument();

    // The single-fix climb is undrawable but still real, so it is still listed.
    expect(screen.getByText('Cat 4')).toBeInTheDocument();
    expect(screen.getByText('2.1%')).toBeInTheDocument();

    // The dashed line is named in words, so the dash is never the only cue.
    expect(screen.getByText('Climbs')).toBeInTheDocument();
    expect(screen.getByText('source: get_activity_route')).toBeInTheDocument();
    // The canvas carries no text a screen reader can read; the figure does.
    expect(
      screen.getByRole('figure', { name: 'Map of the recorded route: Mont Royal loop' })
    ).toBeInTheDocument();
  });

  it('drops the kilometre marks rather than inventing them when the series is ragged', async () => {
    render(
      <ThemeProvider>
        <RouteView view={{ ...ROUTE, distances_meters: [0, 2000] }} />
      </ThemeProvider>
    );

    await waitFor(() => expect(harness.constructed).toHaveLength(1));

    expect(screen.queryByText('12.5 km')).toBeNull();
    expect(screen.queryByText(/^km /)).toBeNull();
    // The climbs themselves survive — the gradient never depended on distance.
    expect(screen.getByText('Cat 3')).toBeInTheDocument();
    expect(screen.getByText('6.4%')).toBeInTheDocument();
  });

  it('says why there is no map, and builds none, for a track with no positions', () => {
    render(
      <ThemeProvider>
        <RouteView view={{ ...ROUTE, coordinates: [], climbs: [], distances_meters: null }} />
      </ThemeProvider>
    );

    expect(screen.getByText('This activity recorded no GPS track.')).toBeInTheDocument();
    expect(harness.constructed).toHaveLength(0);
  });
});

describe('SceneView block switch', () => {
  it('sends a route block to the map card', async () => {
    const block: RenderBlock = { kind: 'route', ...ROUTE };
    render(
      <ThemeProvider>
        <SceneView block={block} />
      </ThemeProvider>
    );

    await waitFor(() => expect(harness.constructed).toHaveLength(1));
    expect(screen.getByText('Mont Royal loop')).toBeInTheDocument();
  });
});
