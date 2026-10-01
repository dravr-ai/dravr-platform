// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile route card — coordinate order, camera framing, climb overlay, layers, full screen
// ABOUTME: A silently transposed track still draws a line, so every assertion here reads real numbers

import React from 'react';
import { Modal } from 'react-native';
import { act, fireEvent, render, screen, within } from '@testing-library/react-native';
import { i18n } from '@pierre/i18n';
import {
  BASEMAP_STYLE,
  DEFAULT_MAP_LAYER,
  MAP_LAYERS,
  ROUTE_INK,
  mapLayerStyle,
} from '@pierre/shared-constants';
import type { RouteView as RouteBlock } from '@pierre/scene-types';

import RouteView from '../RouteView';

// The captions are read through the real English bundle jest.setup.js
// initialises, not a key-echoing stand-in: a climb graded `HC` once rendered
// "Cat HC", and a mock that printed `chat.routeClimbCategory HC` looked fine.

let mockScheme: 'light' | 'dark' = 'dark';

/**
 * The published style the card fetches to relabel: OpenFreeMap's place label,
 * English first, and a road shield that reads no name.
 */
const PUBLISHED_STYLE = {
  version: 8,
  sources: {},
  layers: [
    { id: 'background', type: 'background' },
    {
      id: 'label_city',
      type: 'symbol',
      source: 'openmaptiles',
      layout: { 'text-field': ['coalesce', ['get', 'name_en'], ['get', 'name']] },
    },
    {
      id: 'road_shield_us',
      type: 'symbol',
      source: 'openmaptiles',
      layout: { 'text-field': ['to-string', ['get', 'ref']] },
    },
  ],
};

/**
 * Every style fetch, by URL. A fetch that is never answered leaves the map on
 * the URL, which is what most tests read; one that is answered hands the map
 * the relabelled document. Nothing here reaches the network.
 */
const fetchStyle = jest.fn<Promise<Response>, [string]>(() => new Promise<Response>(() => {}));

jest.mock('../../../constants/theme', () => ({
  useTheme: () => ({
    scheme: mockScheme,
    colors: {
      tokens: { primary: '#a3d0be', onSurface: '#e1e3de' },
    },
  }),
}));

/**
 * Four fixes climbing a hillside, in the `(latitude, longitude)` order the
 * block carries them. Longitudes are negative and latitudes positive, which is
 * what makes a transposition visible rather than merely wrong.
 */
const TRACK: [number, number][] = [
  [45.5, -73.6],
  [45.51, -73.59],
  [45.52, -73.58],
  [45.53, -73.57],
];

function routeBlock(overrides: Partial<RouteBlock> = {}): RouteBlock {
  return {
    coordinates: TRACK,
    bounds: {
      min_latitude: 45.5,
      max_latitude: 45.53,
      min_longitude: -73.6,
      max_longitude: -73.57,
    },
    elevation_meters: [112, 140, 168, 190],
    distances_meters: [0, 1200, 2400, 3600],
    climbs: [],
    title: null,
    source_tool: 'get_activity_route',
    ...overrides,
  };
}

function sourceData(testID: string) {
  return screen.getByTestId(testID).props.data;
}

beforeEach(() => {
  mockScheme = 'dark';
  fetchStyle.mockReset();
  fetchStyle.mockImplementation(() => new Promise<Response>(() => {}));
  global.fetch = fetchStyle as unknown as typeof fetch;
});

describe('RouteView track geometry', () => {
  it('hands MapLibre the track in GeoJSON longitude-latitude order', () => {
    render(<RouteView route={routeBlock()} />);

    const data = sourceData('route-track');
    expect(data.type).toBe('LineString');
    expect(data.coordinates).toEqual([
      [-73.6, 45.5],
      [-73.59, 45.51],
      [-73.58, 45.52],
      [-73.57, 45.53],
    ]);
  });

  it('casts the track in opaque white under a line that is thinner than its casing', () => {
    render(<RouteView route={routeBlock()} />);

    const casing = screen.getByTestId('route-casing').props.paint;
    const line = screen.getByTestId('route-line').props.paint;

    expect(casing['line-color']).toBe('#ffffff');
    // Over a photograph a translucent casing inherits the pixel under it.
    expect(casing['line-opacity']).toBeUndefined();
    expect(casing['line-width']).toBeGreaterThan(line['line-width']);
  });

  it('draws the route in the one orange, not the theme accent, in both schemes', () => {
    render(<RouteView route={routeBlock()} />);
    expect(screen.getByTestId('route-line').props.paint['line-color']).toBe('#d9480f');
    expect(screen.getByTestId('route-line').props.paint['line-color']).not.toBe('#a3d0be');
    screen.unmount();

    mockScheme = 'light';
    render(<RouteView route={routeBlock()} />);
    expect(screen.getByTestId('route-line').props.paint['line-color']).toBe(ROUTE_INK.track);
  });

  it('names no source under the map', () => {
    render(<RouteView route={routeBlock({ source_tool: 'strava' })} />);

    // Where the track came from is not the athlete's concern.
    expect(screen.queryByText(/source/i)).toBeNull();
    expect(screen.queryByText(/strava/i)).toBeNull();
  });

  it('draws no map and says so when the activity recorded no track', () => {
    render(<RouteView route={routeBlock({ coordinates: [] })} />);

    expect(screen.queryByTestId('route-map')).toBeNull();
    expect(screen.getByText('This activity recorded no GPS track.')).toBeTruthy();
  });
});

describe('RouteView camera', () => {
  it('frames the block’s own extent as west, south, east, north', () => {
    render(<RouteView route={routeBlock()} />);

    expect(screen.getByTestId('maplibre-camera').props.bounds).toEqual([
      -73.6, 45.5, -73.57, 45.53,
    ]);
  });

  /**
   * An activity that recorded a single fix has an extent too small to fit: the
   * camera would ask for a zoom past the deepest tile OpenFreeMap serves, which
   * is a map of one tree. The box is widened symmetrically, so the track stays
   * centred on ground that exists.
   */
  it('widens a degenerate extent about its midpoint', () => {
    render(
      <RouteView
        route={routeBlock({
          bounds: {
            min_latitude: 45.5,
            max_latitude: 45.5,
            min_longitude: -73.6,
            max_longitude: -73.6,
          },
        })}
      />,
    );

    const [west, south, east, north] = screen.getByTestId('maplibre-camera').props.bounds;
    expect(north - south).toBeCloseTo(0.004, 10);
    expect(east - west).toBeCloseTo(0.004, 10);
    expect((north + south) / 2).toBeCloseTo(45.5, 10);
    expect((east + west) / 2).toBeCloseTo(-73.6, 10);
  });
});

describe('RouteView climbs', () => {
  const climbing = routeBlock({
    climbs: [
      { start_index: 1, end_index: 3, avg_gradient: 6.4, category: '3' },
      // A climb whose slice is a single fix cannot be a line; it is dropped
      // from the geometry rather than handed to MapLibre as a one-point run.
      { start_index: 2, end_index: 2, avg_gradient: 9.1, category: '2' },
    ],
  });

  it('cuts each drawable climb out of the track it already drew', () => {
    render(<RouteView route={climbing} />);

    const data = sourceData('route-climbs');
    expect(data.type).toBe('MultiLineString');
    expect(data.coordinates).toEqual([
      [
        [-73.59, 45.51],
        [-73.58, 45.52],
        [-73.57, 45.53],
      ],
    ]);
  });

  it('distinguishes a climb by dash and weight as well as by colour', () => {
    render(<RouteView route={climbing} />);

    const climb = screen.getByTestId('route-climb').props;
    expect(climb.paint['line-color']).toBe(ROUTE_INK.climb);
    expect(climb.paint['line-dasharray']).toEqual([1.4, 1.1]);
    expect(climb.paint['line-width']).toBeGreaterThan(
      screen.getByTestId('route-line').props.paint['line-width'],
    );
    // A round cap on a dash draws a lozenge, which blunts the one signal a
    // colourblind reader has.
    expect(climb.layout['line-cap']).toBe('butt');
  });

  /**
   * The legend names every climb the block reported, including the one whose
   * slice was too short to draw: the grade is the finding, and a line too short
   * to see is still a hill the athlete rode up.
   */
  it('spells every climb out in words beneath the map', () => {
    render(<RouteView route={climbing} />);

    expect(screen.getByText('Climbs')).toBeTruthy();
    expect(screen.getByText('Cat 3')).toBeTruthy();
    expect(screen.getByText('6.4%')).toBeTruthy();
    expect(screen.getByText('Cat 2')).toBeTruthy();
    expect(screen.getByText('9.1%')).toBeTruthy();
  });

  it('captions hors catégorie as HC and a numbered grade as Cat N', () => {
    render(
      <RouteView
        route={routeBlock({
          climbs: [
            { start_index: 0, end_index: 3, avg_gradient: 8.9, category: 'HC' },
            { start_index: 1, end_index: 3, avg_gradient: 6.4, category: '3' },
          ],
        })}
      />,
    );

    // HC is a name, not a number: no cyclist says "Cat HC".
    expect(screen.getByText('HC')).toBeTruthy();
    expect(screen.queryByText('Cat HC')).toBeNull();
    expect(screen.getByText('Cat 3')).toBeTruthy();
  });

  it('lists an ungraded climb with its gradient and no invented category', () => {
    render(
      <RouteView
        route={routeBlock({
          climbs: [
            { start_index: 0, end_index: 3, avg_gradient: 3.2, category: null },
            { start_index: 1, end_index: 3, avg_gradient: 6.4, category: '3' },
          ],
        })}
      />,
    );

    // Below the category threshold the climb is still a climb — its gradient
    // is listed — but a grade it did not earn is not printed.
    expect(screen.getByText(/3\.2\s*%/)).toBeTruthy();
    expect(screen.getByText('Cat 3')).toBeTruthy();
    expect(screen.queryByText(/Cat null/)).toBeNull();
    expect(screen.queryByText(/Cat none/)).toBeNull();
  });

  it('reads each climb’s kilometre range off the carried distances', () => {
    render(<RouteView route={climbing} />);

    expect(screen.getByText('km 1.2–3.6')).toBeTruthy();
  });

  it('carries an empty climb geometry and no legend on a route without climbs', () => {
    render(<RouteView route={routeBlock()} />);

    expect(sourceData('route-climbs').coordinates).toEqual([]);
    expect(screen.queryByText('Climbs')).toBeNull();
  });
});

describe('RouteView climb figures', () => {
  afterEach(async () => {
    await i18n.changeLanguage('en');
  });

  it("prints a climb's kilometres and gradient in the athlete's decimal notation", async () => {
    await i18n.changeLanguage('fr');
    render(
      <RouteView
        route={routeBlock({
          distances_meters: [0, 12_400, 15_000, 15_500],
          climbs: [{ start_index: 1, end_index: 2, avg_gradient: 5.3, category: '3' }],
        })}
      />,
    );
    await act(async () => {});

    // A French athlete reads '42,00 km' in the figures above the map; the
    // climb list under it speaks the same notation.
    expect(screen.getByText('km 12,4–15,0')).toBeTruthy();
    expect(screen.getByText('5,3%')).toBeTruthy();
    expect(screen.queryByText('km 12.4–15.0')).toBeNull();
    expect(screen.queryByText('5.3%')).toBeNull();
  });
});

describe('RouteView distance', () => {
  /**
   * The distance series is measured along the drawn GPS line and ends where
   * the privacy trim cuts it, so its last value is not the activity's
   * distance: printed, it read 14.4 km under a 10.40 km activity. The card
   * prints no total, whatever the series carries.
   */
  it('prints no total, even off a full distance series', () => {
    render(<RouteView route={routeBlock()} />);

    expect(screen.queryByText('3.6 km')).toBeNull();
    expect(screen.queryByText(/ km$/)).toBeNull();
  });
});

describe('RouteView chrome', () => {
  it('draws the dark OpenFreeMap sheet on the night canvas', () => {
    render(<RouteView route={routeBlock()} />);

    expect(screen.getByTestId('route-map').props.mapStyle).toBe(
      'https://tiles.openfreemap.org/styles/dark',
    );
  });

  it('draws the quiet pale sheet on paper', () => {
    mockScheme = 'light';
    render(<RouteView route={routeBlock()} />);

    expect(screen.getByTestId('route-map').props.mapStyle).toBe(
      'https://tiles.openfreemap.org/styles/positron',
    );
  });

  /**
   * OpenFreeMap serves OpenStreetMap data and the credit is a condition of
   * using it, so the attribution button is not a preference. The map also must
   * not claim the drag: an athlete reading a thread would be stranded by a card
   * that swallowed the scroll.
   */
  it('keeps the attribution and yields every gesture to the thread', () => {
    render(<RouteView route={routeBlock()} />);

    const map = screen.getByTestId('route-map').props;
    expect(map.attribution).toBe(true);
    expect(map.dragPan).toBe(false);
    expect(map.touchZoom).toBe(false);
  });

  it('reads the block’s caption to a screen reader and above the map', () => {
    render(<RouteView route={routeBlock({ title: 'Mont Royal loop' })} />);

    expect(screen.getByText('Mont Royal loop')).toBeTruthy();
    expect(screen.getByLabelText('Map of the recorded route: Mont Royal loop')).toBeTruthy();
  });

  it('falls back to the untitled label when the block carries no caption', () => {
    render(<RouteView route={routeBlock()} />);

    expect(screen.getByLabelText('Map of the recorded route')).toBeTruthy();
  });
});

describe('RouteView layer switcher', () => {
  function layer(id: string) {
    const found = MAP_LAYERS.find((candidate) => candidate.id === id);
    if (found === undefined) throw new Error(`no ${id} layer`);
    return found;
  }

  it('offers map, satellite and terrain with no key configured, opening on the map', () => {
    render(<RouteView route={routeBlock()} />);

    const switcher = screen.getByLabelText('Map layer');
    expect(within(switcher).getByText('Map')).toBeTruthy();
    expect(within(switcher).getByText('Satellite')).toBeTruthy();
    expect(within(switcher).getByText('Terrain')).toBeTruthy();
    expect(screen.getByTestId('route-layers-map').props.accessibilityState).toEqual({
      selected: true,
      checked: true,
    });
    expect(screen.getByTestId('route-map').props.mapStyle).toBe(
      'https://tiles.openfreemap.org/styles/dark',
    );
  });

  it('swaps the map to Esri imagery, then Esri topo, printing each credit on the map', () => {
    render(<RouteView route={routeBlock()} />);

    fireEvent.press(screen.getByTestId('route-layers-satellite'));
    let style = screen.getByTestId('route-map').props.mapStyle;
    expect(style).toEqual(mapLayerStyle(layer('satellite'), 'dark'));
    expect(style.sources.satellite.tiles[0]).toBe(
      'https://server.arcgisonline.com/ArcGIS/rest/services/World_Imagery/MapServer/tile/{z}/{y}/{x}',
    );
    // The native attribution is a button; Esri's credit is printed on the map
    // itself while the imagery is drawn.
    expect(screen.getByTestId('route-credit')).toHaveTextContent(
      'Imagery © Esri, Vantor, Earthstar Geographics',
    );
    expect(screen.getByTestId('route-layers-satellite').props.accessibilityState.selected).toBe(true);
    // The route is still drawn, in the same ink, over the photograph.
    expect(screen.getByTestId('route-line').props.paint['line-color']).toBe(ROUTE_INK.track);

    fireEvent.press(screen.getByTestId('route-layers-terrain'));
    style = screen.getByTestId('route-map').props.mapStyle;
    expect(style).toEqual(mapLayerStyle(layer('terrain'), 'dark'));
    expect(style.sources.terrain.tiles[0]).toBe(
      'https://server.arcgisonline.com/ArcGIS/rest/services/World_Topo_Map/MapServer/tile/{z}/{y}/{x}',
    );
    expect(screen.getByTestId('route-credit')).toHaveTextContent('© Esri, USGS, NOAA');
    expect(screen.getByTestId('route-layers-terrain').props.accessibilityState.selected).toBe(true);
    expect(screen.getByTestId('route-layers-satellite').props.accessibilityState.selected).toBe(false);

    fireEvent.press(screen.getByTestId('route-layers-map'));
    expect(screen.getByTestId('route-map').props.mapStyle).toBe(
      'https://tiles.openfreemap.org/styles/dark',
    );
    // The basemap's OpenStreetMap credit stays behind MapLibre's own button.
    expect(screen.queryByTestId('route-credit')).toBeNull();
  });
});

describe('RouteView full screen', () => {
  it('opens a full-screen map that takes the gestures, and closes it from its button', () => {
    render(<RouteView route={routeBlock()} />);
    expect(screen.queryByTestId('route-fullscreen-map')).toBeNull();

    fireEvent.press(screen.getByLabelText('Full screen'));

    const full = screen.getByTestId('route-fullscreen-map').props;
    expect(full.dragPan).toBe(true);
    expect(full.touchZoom).toBe(true);
    expect(full.attribution).toBe(true);
    // Same framing as the card it opened from.
    expect(screen.getAllByTestId('maplibre-camera')[1].props.bounds).toEqual([
      -73.6, 45.5, -73.57, 45.53,
    ]);
    // The inline card still yields its gestures to the thread.
    expect(screen.getByTestId('route-map').props.dragPan).toBe(false);

    fireEvent.press(screen.getByLabelText('Exit full screen'));
    expect(screen.queryByTestId('route-fullscreen-map')).toBeNull();
  });

  it('closes on the system back request and keeps the layer picked full screen', () => {
    render(<RouteView route={routeBlock()} />);
    fireEvent.press(screen.getByLabelText('Full screen'));

    fireEvent.press(screen.getByTestId('route-fullscreen-layers-satellite'));
    expect(typeof screen.getByTestId('route-fullscreen-map').props.mapStyle).toBe('object');
    expect(screen.getByTestId('route-fullscreen-credit')).toHaveTextContent(/^Imagery © Esri/);
    fireEvent.press(screen.getByTestId('route-fullscreen-layers-terrain'));
    expect(screen.getByTestId('route-fullscreen-credit')).toHaveTextContent('© Esri, USGS, NOAA');

    // Android's back button and the iOS dismiss gesture arrive as onRequestClose.
    fireEvent(screen.UNSAFE_getByType(Modal), 'requestClose');
    expect(screen.queryByTestId('route-fullscreen-map')).toBeNull();
    // Back on the card, the picked layer stays picked.
    expect(typeof screen.getByTestId('route-map').props.mapStyle).toBe('object');
  });
});

describe('RouteView control layout', () => {
  // The full-screen button used to be docked absolutely over the switcher's
  // corner, so on a narrow card it sat on the last option. Both now share one
  // wrapping row: the switcher reserves the button's width, and where the two
  // do not fit the button takes a row of its own instead of covering a label.
  function expectOneWrappingRow(barId: string, switcherId: string, buttonId: string) {
    const bar = screen.getByTestId(barId);
    expect(bar.props.className).toEqual(expect.stringContaining('absolute'));
    expect(bar.props.className.split(' ')).toEqual(
      expect.arrayContaining(['flex-row', 'flex-wrap', 'gap-2']),
    );
    // A drag between the controls still reaches the map.
    expect(bar.props.pointerEvents).toBe('box-none');

    const switcher = within(bar).getByTestId(switcherId);
    const button = within(bar).getByTestId(buttonId);
    // Laid out by the row, never stacked over each other.
    expect(switcher.props.className.split(' ')).not.toContain('absolute');
    expect(button.props.className.split(' ')).not.toContain('absolute');
    expect(button.props.className.split(' ')).toEqual(
      expect.arrayContaining(['ml-auto', 'h-11', 'w-11']),
    );
    // The switcher shrinks to the row and wraps its options rather than clip one.
    expect(switcher.props.className.split(' ')).toEqual(
      expect.arrayContaining(['max-w-full', 'shrink', 'flex-wrap']),
    );
    for (const layer of MAP_LAYERS) {
      const option = within(switcher).getByTestId(`${switcherId}-${layer.id}`);
      expect(option.props.className.split(' ')).toContain('min-h-11');
    }
  }

  it('keeps the switcher and the full-screen button in one wrapping row on the card', () => {
    render(<RouteView route={routeBlock()} />);

    expectOneWrappingRow('route-controls', 'route-layers', 'route-fullscreen-open');
  });

  it('keeps them in one wrapping row full screen', () => {
    render(<RouteView route={routeBlock()} />);
    fireEvent.press(screen.getByLabelText('Full screen'));

    expectOneWrappingRow('route-fullscreen-controls', 'route-fullscreen-layers', 'route-fullscreen-close');
  });
});

describe('RouteView card width', () => {
  // A short reply above the card once left the web map 174px wide. The card
  // spans the coach's turn whatever its words, and the map keeps one height.
  it('spans the message column, with a map of fixed height across its width', () => {
    render(<RouteView route={routeBlock({ title: 'Morning Trail Run' })} />);

    const card = screen.getByTestId('route-card').props.className.split(' ');
    expect(card).toEqual(expect.arrayContaining(['w-full', 'self-stretch']));
    for (const hug of ['self-start', 'self-center', 'self-end', 'items-start', 'items-center']) {
      expect(card).not.toContain(hug);
    }
    expect(card.filter((name: string) => name.startsWith('max-w-'))).toEqual([]);

    const map = screen.getByTestId('route-card-map').props.className.split(' ');
    expect(map).toEqual(expect.arrayContaining(['w-full', 'h-64']));
  });
});

describe('RouteView default layer', () => {
  // carnet#699: an activity view opened on satellite imagery while Home showed
  // the map, because one pick was remembered for every map opened after it.
  it('opens on the plain map layer, inline and full screen', () => {
    render(<RouteView route={routeBlock()} />);

    expect(DEFAULT_MAP_LAYER).toBe('map');
    expect(screen.getByTestId('route-layers-map').props.accessibilityState).toEqual({
      selected: true,
      checked: true,
    });
    expect(screen.getByTestId('route-map').props.mapStyle).toBe(BASEMAP_STYLE.dark);
    fireEvent.press(screen.getByLabelText('Full screen'));
    expect(screen.getByTestId('route-fullscreen-map').props.mapStyle).toBe(BASEMAP_STYLE.dark);
  });

  it('does not carry a pick to the next map: every map opens on the map layer', () => {
    render(<RouteView route={routeBlock()} />);
    fireEvent.press(screen.getByTestId('route-layers-satellite'));
    expect(screen.getByTestId('route-map').props.mapStyle).toEqual(
      mapLayerStyle(MAP_LAYERS[1], 'dark'),
    );
    screen.unmount();

    render(<RouteView route={routeBlock()} />);
    expect(screen.getByTestId('route-layers-map').props.accessibilityState).toEqual({
      selected: true,
      checked: true,
    });
    expect(screen.getByTestId('route-layers-satellite').props.accessibilityState).toEqual({
      selected: false,
      checked: false,
    });
    expect(screen.getByTestId('route-map').props.mapStyle).toBe(BASEMAP_STYLE.dark);
  });
});

describe('RouteView basemap labels', () => {
  afterEach(async () => {
    await i18n.changeLanguage('en');
  });

  it("hands the map the published style relabelled in the athlete's language", async () => {
    await i18n.changeLanguage('fr');
    mockScheme = 'light';
    fetchStyle.mockImplementation(async () => ({
      ok: true,
      json: async () => PUBLISHED_STYLE,
    }) as unknown as Response);
    render(<RouteView route={routeBlock()} />);

    // "Island of Montreal" was name_en: the French tile field comes first now.
    await act(async () => {});
    expect(fetchStyle).toHaveBeenCalledWith('https://tiles.openfreemap.org/styles/positron');
    const style = screen.getByTestId('route-map').props.mapStyle;
    expect(style.layers[1].layout['text-field']).toEqual([
      'coalesce',
      ['get', 'name:fr'],
      ['get', 'name'],
    ]);
    expect(style.layers[2].layout['text-field']).toEqual(['to-string', ['get', 'ref']]);
  });

  it('keeps drawing the published URL when the style cannot be fetched', async () => {
    fetchStyle.mockImplementation(async () => {
      throw new TypeError('Network request failed');
    });
    render(<RouteView route={routeBlock()} />);
    await act(async () => {});

    expect(screen.getByTestId('route-map').props.mapStyle).toBe(
      'https://tiles.openfreemap.org/styles/dark',
    );
  });
});
