// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for the mobile route card — coordinate order, camera framing, climb overlay
// ABOUTME: A silently transposed track still draws a line, so every assertion here reads real numbers

import React from 'react';
import { render, screen } from '@testing-library/react-native';
import type { RouteView as RouteBlock } from '@pierre/scene-types';

import RouteView from '../RouteView';

jest.mock('@pierre/i18n', () => ({
  useTranslation: () => ({
    t: (key: string, opts?: Record<string, unknown>) =>
      opts ? [key, ...Object.values(opts)].join(' ') : key,
  }),
}));

let mockScheme: 'light' | 'dark' = 'dark';

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

  it('casts the track in white under a line that is thinner than its casing', () => {
    render(<RouteView route={routeBlock()} />);

    const casing = screen.getByTestId('route-casing').props.paint;
    const line = screen.getByTestId('route-line').props.paint;

    expect(casing['line-color']).toBe('#ffffff');
    expect(line['line-color']).toBe('#a3d0be');
    expect(casing['line-width']).toBeGreaterThan(line['line-width']);
  });

  it('names the tool the track came from', () => {
    render(<RouteView route={routeBlock()} />);

    expect(screen.getByText(/get_activity_route/)).toBeTruthy();
  });

  it('draws no map and says so when the activity recorded no track', () => {
    render(<RouteView route={routeBlock({ coordinates: [] })} />);

    expect(screen.queryByTestId('maplibre-map')).toBeNull();
    expect(screen.getByText('chat.routeNoTrack')).toBeTruthy();
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
    expect(climb.paint['line-color']).toBe('#e1e3de');
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

    expect(screen.getByText('chat.routeClimbs')).toBeTruthy();
    expect(screen.getByText('chat.routeClimbCategory 3')).toBeTruthy();
    expect(screen.getByText('6.4%')).toBeTruthy();
    expect(screen.getByText('chat.routeClimbCategory 2')).toBeTruthy();
    expect(screen.getByText('9.1%')).toBeTruthy();
  });

  it('reads each climb’s kilometre range off the carried distances', () => {
    render(<RouteView route={climbing} />);

    expect(screen.getByText('km 1.2–3.6')).toBeTruthy();
  });

  it('carries an empty climb geometry and no legend on a route without climbs', () => {
    render(<RouteView route={routeBlock()} />);

    expect(sourceData('route-climbs').coordinates).toEqual([]);
    expect(screen.queryByText('chat.routeClimbs')).toBeNull();
  });
});

describe('RouteView distance', () => {
  it('prints the total off the last carried distance', () => {
    render(<RouteView route={routeBlock()} />);

    expect(screen.getByText('3.6 km')).toBeTruthy();
  });

  /**
   * The photograveur contract says a parallel series is index-aligned or
   * absent, never padded. A ragged one would print a distance read off the end
   * of the array, so the card drops the kilometre marks rather than inventing
   * them.
   */
  it('prints no distance when the series does not align with the track', () => {
    render(<RouteView route={routeBlock({ distances_meters: [0, 1200] })} />);

    expect(screen.queryByText(/ km$/)).toBeNull();
  });

  it('prints no distance when the activity recorded none', () => {
    render(<RouteView route={routeBlock({ distances_meters: null })} />);

    expect(screen.queryByText(/ km$/)).toBeNull();
  });
});

describe('RouteView chrome', () => {
  it('draws the dark OpenFreeMap sheet on the night canvas', () => {
    render(<RouteView route={routeBlock()} />);

    expect(screen.getByTestId('maplibre-map').props.mapStyle).toBe(
      'https://tiles.openfreemap.org/styles/dark',
    );
  });

  it('draws the quiet pale sheet on paper', () => {
    mockScheme = 'light';
    render(<RouteView route={routeBlock()} />);

    expect(screen.getByTestId('maplibre-map').props.mapStyle).toBe(
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

    const map = screen.getByTestId('maplibre-map').props;
    expect(map.attribution).toBe(true);
    expect(map.dragPan).toBe(false);
    expect(map.touchZoom).toBe(false);
  });

  it('reads the block’s caption to a screen reader and above the map', () => {
    render(<RouteView route={routeBlock({ title: 'Mont Royal loop' })} />);

    expect(screen.getByText('Mont Royal loop')).toBeTruthy();
    expect(screen.getByLabelText('chat.routeAltTitled Mont Royal loop')).toBeTruthy();
  });

  it('falls back to the untitled label when the block carries no caption', () => {
    render(<RouteView route={routeBlock()} />);

    expect(screen.getByLabelText('chat.routeAlt')).toBeTruthy();
  });
});
