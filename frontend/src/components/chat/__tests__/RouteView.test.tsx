// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the web route block — the MapLibre layers, the framed extent, the layer switcher, full screen, control text
// ABOUTME: Red the moment a route renders as a picture of a map instead of a framed MapLibre track

import { describe, it, expect, afterEach, beforeEach, vi } from 'vitest';
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { Map as MapLibreMap } from 'maplibre-gl';
import type { RenderBlock, RouteView as RouteViewData } from '@pierre/scene-types';
import { climbGeometry, trackGeometry } from '@pierre/chat-utils';
import { i18n } from '@pierre/i18n';
import { BASEMAP_STYLE, BOREAL, MAP_LAYERS, ROUTE_INK, mapLayerStyle } from '@pierre/shared-constants';
import { ThemeProvider, useTheme } from '../../../hooks/useTheme';
import RouteView from '../RouteView';
import { SceneView } from '../SceneView';
import { addRouteLayers, CLIMB_SOURCE, TRACK_SOURCE } from '../routeLayers';

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
  /**
   * The published style's label layers as `getStyle` reports them once loaded
   * — OpenFreeMap's place label, English first, and a road shield that reads
   * no name — and every `text-field` the component set on them.
   */
  const placeLabel = [
    'case',
    ['has', 'name:nonlatin'],
    ['concat', ['get', 'name:latin'], '\n', ['get', 'name:nonlatin']],
    ['coalesce', ['get', 'name_en'], ['get', 'name']],
  ];
  const styleLayers = [
    { id: 'background', type: 'background' },
    { id: 'label_city', type: 'symbol', layout: { 'text-field': placeLabel } },
    { id: 'road_shield_us', type: 'symbol', layout: { 'text-field': ['to-string', ['get', 'ref']] } },
  ];
  const relabelled = new Map<string, unknown>();

  const instance = {
    addControl: vi.fn(),
    // A credit swapped for another layer kind is taken off the map first.
    removeControl: vi.fn((control: { compact?: boolean }) => {
      controls.push(['removed', String(control.compact)]);
    }),
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
    getStyle: vi.fn(() => ({ version: 8, sources: {}, layers: styleLayers })),
    setLayoutProperty: vi.fn((id: string, name: string, value: unknown) => {
      if (name === 'text-field') relabelled.set(id, value);
    }),
    remove: vi.fn(),
    resize: vi.fn(),
    fitBounds: vi.fn(),
    cooperativeGestures: { enable: vi.fn(), disable: vi.fn() },
  };

  /** Every setWorkerUrl and map construction, in the order they happened. */
  const calls: string[] = [];

  return {
    instance,
    sources,
    layers,
    handlers,
    constructed,
    controls,
    calls,
    relabelled,
    reset() {
      relabelled.clear();
      sources.clear();
      layers.length = 0;
      handlers.clear();
      constructed.length = 0;
      controls.length = 0;
      calls.length = 0;
    },
  };
});

vi.mock('maplibre-gl', () => ({
  Map: class {
    constructor(options: Record<string, unknown>) {
      harness.calls.push('map');
      harness.constructed.push(options);
      return harness.instance;
    }
  },
  setWorkerUrl: (url: string) => {
    harness.calls.push(`worker:${url}`);
  },
  AttributionControl: class {
    compact?: boolean;
    constructor(options: { compact?: boolean }) {
      this.compact = options.compact;
      harness.controls.push(['attribution', String(options.compact)]);
    }
  },
  NavigationControl: class {
    constructor(options: { showCompass?: boolean }) {
      harness.controls.push(['navigation', String(options.showCompass)]);
    }
  },
}));

// The bundler's `?worker&url` import resolves to the emitted worker's URL.
vi.mock('maplibre-gl/dist/maplibre-gl-worker.mjs?worker&url', () => ({
  default: '/assets/maplibre-gl-worker.js',
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

// The minimum-span frame, the lat/lon transpose and the inclusive climb slice
// are pinned in packages/chat-utils/__tests__/route.test.ts, beside the
// functions both clients draw from.
describe('route ink', () => {
  it('draws the route in the one orange, whatever the scheme', () => {
    addRouteLayers(map, trackGeometry(TRACK), climbGeometry(TRACK, ROUTE.climbs));
    // Not the Boreal accent: a sage line on sage chrome read as more of the app.
    expect(paintFor('route-line')['line-color']).toBe('#d9480f');
    expect(paintFor('route-line')['line-color']).not.toBe(BOREAL.dark.primary);
    expect(paintFor('route-line')['line-color']).not.toBe(BOREAL.light.primary);
    expect(paintFor('route-climb')['line-color']).toBe(ROUTE_INK.climb);
  });
});

describe('route layers', () => {
  it('paints a casing, the track and a dashed climb, and adds nothing twice', () => {
    addRouteLayers(map, trackGeometry(TRACK), climbGeometry(TRACK, ROUTE.climbs));

    expect(harness.sources.has(TRACK_SOURCE)).toBe(true);
    expect(harness.sources.has(CLIMB_SOURCE)).toBe(true);
    expect(harness.layers.map((layer) => layer.id)).toEqual([
      'route-casing',
      'route-line',
      'route-climb',
    ]);

    expect(paintFor('route-casing')['line-color']).toBe('#ffffff');
    // Opaque: over a photograph a translucent casing inherits the pixel under it.
    expect(paintFor('route-casing')['line-opacity']).toBeUndefined();
    expect(paintFor('route-line')['line-color']).toBe(ROUTE_INK.track);

    const climb = paintFor('route-climb');
    expect(climb['line-color']).toBe(ROUTE_INK.climb);
    // The dash is the signal that survives a colourblind reader and a
    // greyscale screenshot — DESIGN.md §8.
    expect(climb['line-dasharray']).toEqual([1.4, 1.1]);
    // Heavier than the track it is laid over, so the accent shows in the gaps.
    expect(Number(climb['line-width'])).toBeGreaterThan(
      Number(paintFor('route-line')['line-width'])
    );

    addRouteLayers(map, trackGeometry(TRACK), climbGeometry(TRACK, ROUTE.climbs));
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
    // West, south, east, north: a real ride is wider than the minimum span on
    // both axes, so it is framed exactly as carried.
    expect(options.bounds).toEqual([-73.68, 45.5, -73.6, 45.58]);
    expect(options.fitBoundsOptions).toEqual({ padding: 24 });
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

    // Nothing is drawn until the basemap style is up, and the figure does not
    // claim a drawn route before it is: a blank map with its controls looks
    // exactly like a map still waiting for its style.
    const figure = screen.getByRole('figure');
    expect(harness.layers).toHaveLength(0);
    expect(figure).not.toHaveAttribute('data-route-drawn');
    harness.handlers.get('style.load')?.();
    expect(harness.layers.map((layer) => layer.id)).toEqual([
      'route-casing',
      'route-line',
      'route-climb',
    ]);
    expect(paintFor('route-line')['line-color']).toBe(ROUTE_INK.track);
    expect(figure).toHaveAttribute('data-route-drawn', 'true');
  });

  it('points MapLibre at the bundled tile worker before it builds the map', async () => {
    render(
      <ThemeProvider>
        <RouteView view={ROUTE} />
      </ThemeProvider>
    );

    await waitFor(() => expect(harness.constructed).toHaveLength(1));
    // Without the URL, MapLibre asks for a worker beside its own module; a
    // bundle has none there, so the worker loads index.html and no tile draws.
    expect(harness.calls).toEqual(['worker:/assets/maplibre-gl-worker.js', 'map']);
  });

  /**
   * An activity that recorded one fix has an extent that is a point. Both
   * clients open it on the shared minimum span, so the same ride is framed
   * from the same distance on the web and on the phone — a zoom cap on the fit
   * would frame it from wherever that cap happens to land instead.
   */
  it('opens a one-fix track on the shared minimum span, with no zoom cap', async () => {
    render(
      <ThemeProvider>
        <RouteView
          view={{
            ...ROUTE,
            coordinates: [[45.5, -73.6]],
            bounds: {
              min_latitude: 45.5,
              max_latitude: 45.5,
              min_longitude: -73.6,
              max_longitude: -73.6,
            },
            elevation_meters: [24],
            distances_meters: [0],
            climbs: [],
          }}
        />
      </ThemeProvider>
    );

    await waitFor(() => expect(harness.constructed).toHaveLength(1));
    const options = harness.constructed[0];
    const [west, south, east, north] = options.bounds as number[];
    expect(east - west).toBeCloseTo(0.004, 10);
    expect(north - south).toBeCloseTo(0.004, 10);
    expect((east + west) / 2).toBeCloseTo(-73.6, 10);
    expect((north + south) / 2).toBeCloseTo(45.5, 10);
    expect(options.fitBoundsOptions).toEqual({ padding: 24 });
  });

  it('repaints on the light basemap in the light scheme, in the same orange', async () => {
    const user = userEvent.setup();
    render(
      <ThemeProvider>
        <SchemeSwitch />
        <RouteView view={ROUTE} />
      </ThemeProvider>
    );

    await waitFor(() => expect(harness.constructed).toHaveLength(1));
    harness.handlers.get('style.load')?.();
    expect(paintFor('route-line')['line-color']).toBe(ROUTE_INK.track);

    await user.click(screen.getByRole('button', { name: 'go light' }));

    await waitFor(() => expect(harness.instance.setStyle).toHaveBeenCalledWith(BASEMAP_STYLE.light));
    // The swap took the layers with it; the reload paints them again, in the
    // ink that does not follow the scheme.
    harness.handlers.get('style.load')?.();
    expect(paintFor('route-line')['line-color']).toBe(ROUTE_INK.track);
    expect(paintFor('route-climb')['line-color']).toBe(ROUTE_INK.climb);
    // One map, restyled — not a second map built over the first.
    expect(harness.constructed).toHaveLength(1);
  });

  it('prints every climb, including one the map cannot draw, and no second distance or source', async () => {
    render(
      <ThemeProvider>
        <RouteView view={ROUTE} />
      </ThemeProvider>
    );

    await waitFor(() => expect(harness.constructed).toHaveLength(1));

    expect(screen.getByText('Mont Royal loop')).toBeInTheDocument();
    // The distance series is measured along the trimmed GPS line, so its last
    // value is not the activity's distance: printed, it read 14.4 km under a
    // 10.40 km activity. The card prints no total at all.
    expect(screen.queryByText('12.5 km')).toBeNull();
    expect(screen.queryByText(/^\d+(\.\d+)? km$/)).toBeNull();

    expect(screen.getByText('Cat 3')).toBeInTheDocument();
    expect(screen.getByText('6.4%')).toBeInTheDocument();
    expect(screen.getByText('km 2.0–8.0')).toBeInTheDocument();

    // The single-fix climb is undrawable but still real, so it is still listed.
    expect(screen.getByText('Cat 4')).toBeInTheDocument();
    expect(screen.getByText('2.1%')).toBeInTheDocument();

    // The dashed line is named in words, so the dash is never the only cue.
    expect(screen.getByText('Climbs')).toBeInTheDocument();
    // Where the track came from is not the athlete's concern: no source line.
    expect(screen.queryByText(/source/i)).toBeNull();
    expect(screen.queryByText(/get_activity_route/)).toBeNull();
    // The canvas carries no text a screen reader can read; the figure does.
    expect(
      screen.getByRole('figure', { name: 'Map of the recorded route: Mont Royal loop' })
    ).toBeInTheDocument();
  });

  it('captions hors catégorie as HC and a numbered grade as Cat N', async () => {
    render(
      <ThemeProvider>
        <RouteView
          view={{
            ...ROUTE,
            climbs: [
              { start_index: 0, end_index: 3, avg_gradient: 8.9, category: 'HC' },
              { start_index: 1, end_index: 3, avg_gradient: 6.4, category: '3' },
            ],
          }}
        />
      </ThemeProvider>
    );

    await waitFor(() => expect(harness.constructed).toHaveLength(1));

    // HC is a name, not a number: no cyclist says "Cat HC".
    expect(screen.getByText('HC')).toBeInTheDocument();
    expect(screen.queryByText('Cat HC')).toBeNull();
    expect(screen.getByText('Cat 3')).toBeInTheDocument();
  });

  it('lists an ungraded climb with its gradient and no invented category', async () => {
    render(
      <ThemeProvider>
        <RouteView
          view={{
            ...ROUTE,
            climbs: [
              { start_index: 0, end_index: 3, avg_gradient: 3.2, category: null },
              { start_index: 1, end_index: 3, avg_gradient: 6.4, category: '3' },
            ],
          }}
        />
      </ThemeProvider>
    );

    await waitFor(() => expect(harness.constructed).toHaveLength(1));

    // Below the category threshold the climb is still a climb — its gradient
    // is listed — but a grade it did not earn is not printed.
    expect(screen.getByText(/3\.2\s*%/)).toBeInTheDocument();
    expect(screen.getByText('Cat 3')).toBeInTheDocument();
    expect(screen.queryByText(/Cat null/)).toBeNull();
    expect(screen.queryByText(/Cat none/)).toBeNull();
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

describe('RouteView layer switcher', () => {
  it('offers map, satellite and terrain with no key configured, and opens on the map', async () => {
    render(
      <ThemeProvider>
        <RouteView view={ROUTE} />
      </ThemeProvider>
    );
    await waitFor(() => expect(harness.constructed).toHaveLength(1));

    const group = screen.getByRole('group', { name: 'Map layer' });
    expect(
      Array.from(group.querySelectorAll('button')).map((button) => button.textContent)
    ).toEqual(['Map', 'Satellite', 'Terrain']);
    expect(screen.getByRole('button', { name: 'Map' })).toHaveAttribute('aria-pressed', 'true');
    expect(harness.constructed[0].style).toBe(BASEMAP_STYLE.dark);
  });

  it('swaps the source to Esri imagery, then Esri topo, then back, repainting the route each time', async () => {
    const user = userEvent.setup();
    render(
      <ThemeProvider>
        <RouteView view={ROUTE} />
      </ThemeProvider>
    );
    await waitFor(() => expect(harness.constructed).toHaveLength(1));
    harness.handlers.get('style.load')?.();

    const standard = screen.getByRole('button', { name: 'Map' });
    const satellite = screen.getByRole('button', { name: 'Satellite' });
    const terrain = screen.getByRole('button', { name: 'Terrain' });
    // Opening on the default layer does not restyle the map it was built on.
    expect(harness.instance.setStyle).not.toHaveBeenCalled();

    await user.click(satellite);

    const esri = MAP_LAYERS.find((layer) => layer.id === 'satellite');
    if (esri === undefined) throw new Error('no satellite layer');
    expect(harness.instance.setStyle).toHaveBeenCalledTimes(1);
    const swapped = harness.instance.setStyle.mock.calls[0] as unknown[];
    expect(swapped[0]).toEqual(mapLayerStyle(esri, 'dark'));
    expect(swapped[0]).toMatchObject({
      sources: {
        satellite: {
          tiles: [
            'https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}',
          ],
          attribution: 'Imagery © Esri, Vantor, Earthstar Geographics',
        },
      },
    });
    // The folded basemap credit is swapped for one that stays open while the
    // imagery is drawn, so the imagery's credit is on the map, not behind (i).
    expect(harness.controls).toEqual([
      ['attribution', 'true'],
      ['navigation', 'false'],
      ['removed', 'true'],
      ['attribution', 'false'],
    ]);
    expect(satellite).toHaveAttribute('aria-pressed', 'true');
    expect(standard).toHaveAttribute('aria-pressed', 'false');
    // The imagery discarded the route with the old style; the load repaints it.
    expect(harness.layers).toHaveLength(0);
    harness.handlers.get('style.load')?.();
    expect(paintFor('route-line')['line-color']).toBe(ROUTE_INK.track);

    await user.click(terrain);
    expect(harness.instance.setStyle).toHaveBeenCalledTimes(2);
    expect(harness.instance.setStyle.mock.calls[1]?.[0]).toMatchObject({
      sources: {
        terrain: {
          tiles: [
            'https://server.arcgisonline.com/ArcGIS/rest/services/World_Topo_Map/MapServer/tile/{z}/{y}/{x}',
          ],
          attribution: '© Esri, USGS, NOAA',
        },
      },
    });
    // Raster to raster: the open credit stays, no control churn.
    expect(harness.controls).toHaveLength(4);
    expect(terrain).toHaveAttribute('aria-pressed', 'true');
    harness.handlers.get('style.load')?.();
    expect(paintFor('route-line')['line-color']).toBe(ROUTE_INK.track);
    expect(harness.constructed).toHaveLength(1);

    await user.click(standard);
    expect(harness.instance.setStyle).toHaveBeenLastCalledWith(BASEMAP_STYLE.dark);
    // Back on the basemap, its credit is the compact one again.
    expect(harness.controls.slice(-2)).toEqual([
      ['removed', 'false'],
      ['attribution', 'true'],
    ]);
  });

  it('is reachable and operable from the keyboard', async () => {
    const user = userEvent.setup();
    render(
      <ThemeProvider>
        <RouteView view={ROUTE} />
      </ThemeProvider>
    );
    await waitFor(() => expect(harness.constructed).toHaveLength(1));

    await user.tab();
    expect(screen.getByRole('button', { name: 'Map' })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole('button', { name: 'Satellite' })).toHaveFocus();
    await user.tab();
    expect(screen.getByRole('button', { name: 'Terrain' })).toHaveFocus();
    await user.keyboard('{Enter}');
    expect(screen.getByRole('button', { name: 'Terrain' })).toHaveAttribute('aria-pressed', 'true');
    await user.tab();
    expect(screen.getByRole('button', { name: 'Full screen' })).toHaveFocus();
  });
});

describe('RouteView layer memory', () => {
  it('opens on the layer this browser last picked, the way Home, the activity view and full screen all do', async () => {
    window.localStorage.setItem('dravr.route_map_layer', 'satellite');
    render(
      <ThemeProvider>
        <RouteView view={ROUTE} />
      </ThemeProvider>
    );
    await waitFor(() => expect(harness.constructed).toHaveLength(1));

    const esri = MAP_LAYERS.find((layer) => layer.id === 'satellite');
    if (esri === undefined) throw new Error('no satellite layer');
    expect(harness.constructed[0].style).toEqual(mapLayerStyle(esri, 'dark'));
    expect(screen.getByRole('button', { name: 'Satellite' })).toHaveAttribute('aria-pressed', 'true');
  });

  it('keeps the pick for the next map, and opens on the map layer for an id no longer registered', async () => {
    const user = userEvent.setup();
    window.localStorage.setItem('dravr.route_map_layer', 'retired-provider');
    const first = render(
      <ThemeProvider>
        <RouteView view={ROUTE} />
      </ThemeProvider>
    );
    await waitFor(() => expect(harness.constructed).toHaveLength(1));
    expect(harness.constructed[0].style).toBe(BASEMAP_STYLE.dark);

    await user.click(screen.getByRole('button', { name: 'Terrain' }));
    expect(window.localStorage.getItem('dravr.route_map_layer')).toBe('terrain');
    first.unmount();

    render(
      <ThemeProvider>
        <RouteView view={ROUTE} />
      </ThemeProvider>
    );
    await waitFor(() => expect(harness.constructed).toHaveLength(2));
    expect(harness.constructed[1].style).toMatchObject({ sources: { terrain: {} } });
  });

  it('still draws the map when storage is blocked', async () => {
    // Only the layer key is blocked: the theme provider reads its own key.
    const realGet = Storage.prototype.getItem;
    const realSet = Storage.prototype.setItem;
    const getItem = vi.spyOn(Storage.prototype, 'getItem').mockImplementation(function (this: Storage, key) {
      if (key === 'dravr.route_map_layer') throw new DOMException('blocked', 'SecurityError');
      return realGet.call(this, key);
    });
    const setItem = vi
      .spyOn(Storage.prototype, 'setItem')
      .mockImplementation(function (this: Storage, key, value) {
        if (key === 'dravr.route_map_layer') throw new DOMException('blocked', 'SecurityError');
        realSet.call(this, key, value);
      });
    try {
      const user = userEvent.setup();
      render(
        <ThemeProvider>
          <RouteView view={ROUTE} />
        </ThemeProvider>
      );
      await waitFor(() => expect(harness.constructed).toHaveLength(1));
      expect(harness.constructed[0].style).toBe(BASEMAP_STYLE.dark);
      await user.click(screen.getByRole('button', { name: 'Satellite' }));
      expect(screen.getByRole('button', { name: 'Satellite' })).toHaveAttribute('aria-pressed', 'true');
    } finally {
      getItem.mockRestore();
      setItem.mockRestore();
    }
  });
});

describe('RouteView basemap labels', () => {
  afterEach(async () => {
    await i18n.changeLanguage('en');
  });

  it("labels the basemap's places in the athlete's language, then the local name, and leaves road shields alone", async () => {
    await i18n.changeLanguage('fr');
    render(
      <ThemeProvider>
        <RouteView view={ROUTE} />
      </ThemeProvider>
    );
    await waitFor(() => expect(harness.constructed).toHaveLength(1));
    harness.handlers.get('style.load')?.();

    // "Island of Montreal" was name_en; the French tile field comes first now.
    const label = harness.relabelled.get('label_city') as unknown[];
    expect(label[0]).toBe('coalesce');
    expect(label[1]).toEqual(['get', 'name:fr']);
    expect(JSON.stringify(label)).not.toContain('name_en');
    expect(harness.relabelled.has('road_shield_us')).toBe(false);
    expect(harness.relabelled.has('background')).toBe(false);
  });
});

describe('RouteView climb figures', () => {
  afterEach(async () => {
    await i18n.changeLanguage('en');
  });

  it("prints a climb's kilometres and gradient in the athlete's decimal notation", async () => {
    await i18n.changeLanguage('fr');
    render(
      <ThemeProvider>
        <RouteView
          view={{
            ...ROUTE,
            distances_meters: [0, 2000, 12_400, 15_000, 15_500],
            climbs: [{ start_index: 2, end_index: 3, avg_gradient: 5.3, category: '3' }],
          }}
        />
      </ThemeProvider>
    );
    await waitFor(() => expect(harness.constructed).toHaveLength(1));

    // A French athlete reads '42,00 km' in the figures above the map; the
    // climb list under it speaks the same notation.
    expect(screen.getByText('km 12,4–15,0')).toBeInTheDocument();
    expect(screen.getByText('5,3%')).toBeInTheDocument();
    expect(screen.queryByText('km 12.4–15.0')).toBeNull();
    expect(screen.queryByText('5.3%')).toBeNull();
  });
});

describe('RouteView control text', () => {
  afterEach(async () => {
    await i18n.changeLanguage('en');
  });

  it("hands MapLibre the athlete's language for its zoom, credit and gesture text", async () => {
    await i18n.changeLanguage('fr');
    render(
      <ThemeProvider>
        <RouteView view={ROUTE} />
      </ThemeProvider>
    );
    await waitFor(() => expect(harness.constructed).toHaveLength(1));

    expect(harness.constructed[0].locale).toEqual({
      'AttributionControl.ToggleAttribution': 'Crédits de la carte',
      'Map.Title': 'Carte du parcours enregistré',
      'NavigationControl.ZoomIn': 'Zoom avant',
      'NavigationControl.ZoomOut': 'Zoom arrière',
      'CooperativeGesturesHandler.MacHelpText': 'Utilise ⌘ + défilement pour zoomer sur la carte',
      'CooperativeGesturesHandler.WindowsHelpText':
        'Utilise Ctrl + défilement pour zoomer sur la carte',
      'CooperativeGesturesHandler.MobileHelpText': 'Utilise deux doigts pour déplacer la carte',
    });
    // The switcher and the full-screen toggle read French too.
    expect(screen.getByRole('group', { name: 'Fond de carte' })).toBeInTheDocument();
    for (const name of ['Plan', 'Satellite', 'Relief', 'Plein écran']) {
      expect(screen.getByRole('button', { name })).toBeInTheDocument();
    }
  });
});

describe('RouteView full screen', () => {
  const original = {
    enabled: Object.getOwnPropertyDescriptor(Document.prototype, 'fullscreenEnabled'),
    element: Object.getOwnPropertyDescriptor(Document.prototype, 'fullscreenElement'),
  };

  afterEach(() => {
    for (const [name, descriptor] of [
      ['fullscreenEnabled', original.enabled],
      ['fullscreenElement', original.element],
    ] as const) {
      if (descriptor) Object.defineProperty(document, name, descriptor);
      else Reflect.deleteProperty(document, name);
    }
    Reflect.deleteProperty(HTMLElement.prototype, 'requestFullscreen');
    Reflect.deleteProperty(document, 'exitFullscreen');
  });

  function stage(): HTMLElement {
    const node = screen.getByRole('figure').querySelector('[data-map-screen]');
    if (!(node instanceof HTMLElement)) throw new Error('no map stage');
    return node;
  }

  it('falls back to an overlay where the Fullscreen API is missing, and Esc closes it', async () => {
    const user = userEvent.setup();
    render(
      <ThemeProvider>
        <RouteView view={ROUTE} />
      </ThemeProvider>
    );
    await waitFor(() => expect(harness.constructed).toHaveLength(1));
    expect(stage()).toHaveAttribute('data-map-screen', 'inline');

    await user.click(screen.getByRole('button', { name: 'Full screen' }));

    expect(stage()).toHaveAttribute('data-map-screen', 'overlay');
    expect(stage()).toHaveClass('fixed', 'inset-0');
    // Full screen, the map is the page and takes the wheel and the drag.
    expect(harness.instance.cooperativeGestures.disable).toHaveBeenCalled();
    expect(harness.instance.resize).toHaveBeenCalled();
    // Reframed for the bigger canvas rather than left at the card's zoom.
    expect(harness.instance.fitBounds).toHaveBeenLastCalledWith([-73.68, 45.5, -73.6, 45.58], {
      padding: 64,
      animate: false,
    });
    expect(screen.getByRole('button', { name: 'Exit full screen' })).toBeInTheDocument();

    await user.keyboard('{Escape}');

    expect(stage()).toHaveAttribute('data-map-screen', 'inline');
    expect(harness.instance.cooperativeGestures.enable).toHaveBeenCalled();
    expect(harness.instance.fitBounds).toHaveBeenLastCalledWith([-73.68, 45.5, -73.6, 45.58], {
      padding: 24,
      animate: false,
    });
    expect(screen.getByRole('button', { name: 'Full screen' })).toBeInTheDocument();
  });

  it('asks the browser for native full screen and follows the browser out of it', async () => {
    const user = userEvent.setup();
    let fullscreen: Element | null = null;
    Object.defineProperty(document, 'fullscreenEnabled', { configurable: true, value: true });
    Object.defineProperty(document, 'fullscreenElement', {
      configurable: true,
      get: () => fullscreen,
    });
    const request = vi.fn(function (this: HTMLElement) {
      fullscreen = stage();
      document.dispatchEvent(new Event('fullscreenchange'));
      return Promise.resolve();
    });
    Object.defineProperty(HTMLElement.prototype, 'requestFullscreen', {
      configurable: true,
      value: request,
    });
    const exit = vi.fn(() => {
      fullscreen = null;
      document.dispatchEvent(new Event('fullscreenchange'));
      return Promise.resolve();
    });
    Object.defineProperty(document, 'exitFullscreen', { configurable: true, value: exit });

    render(
      <ThemeProvider>
        <RouteView view={ROUTE} />
      </ThemeProvider>
    );
    await waitFor(() => expect(harness.constructed).toHaveLength(1));

    await user.click(screen.getByRole('button', { name: 'Full screen' }));

    expect(request).toHaveBeenCalledTimes(1);
    // The element taken full screen is the map's stage, controls included.
    expect(request.mock.contexts[0]).toBe(stage());
    expect(stage()).toHaveAttribute('data-map-screen', 'native');

    await user.click(screen.getByRole('button', { name: 'Exit full screen' }));
    expect(exit).toHaveBeenCalledTimes(1);
    expect(stage()).toHaveAttribute('data-map-screen', 'inline');

    // Esc in native full screen is the browser's: the page only hears the change.
    await user.click(screen.getByRole('button', { name: 'Full screen' }));
    expect(stage()).toHaveAttribute('data-map-screen', 'native');
    act(() => {
      fullscreen = null;
      document.dispatchEvent(new Event('fullscreenchange'));
    });
    expect(stage()).toHaveAttribute('data-map-screen', 'inline');
  });

  it('opens the overlay when the browser refuses the full-screen request', async () => {
    Object.defineProperty(document, 'fullscreenEnabled', { configurable: true, value: true });
    Object.defineProperty(HTMLElement.prototype, 'requestFullscreen', {
      configurable: true,
      value: () => Promise.reject(new TypeError('Permissions check failed')),
    });
    render(
      <ThemeProvider>
        <RouteView view={ROUTE} />
      </ThemeProvider>
    );
    await waitFor(() => expect(harness.constructed).toHaveLength(1));

    fireEvent.click(screen.getByRole('button', { name: 'Full screen' }));

    await waitFor(() => expect(stage()).toHaveAttribute('data-map-screen', 'overlay'));
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
