// ABOUTME: Unit tests for the route card's pure half — the frame and GeoJSON both maps draw and the words printed under them
// ABOUTME: Red if the minimum span, the lat/lon flip, the inclusive climb slice, the km range or the HC caption drift between clients

import { describe, expect, it } from 'vitest';
import type { RouteBounds, RouteClimb } from '@pierre/scene-types';
import {
  MIN_SPAN_DEGREES,
  alignedSeries,
  climbGeometry,
  climbGradient,
  climbGrade,
  climbRange,
  kilometres,
  metresAt,
  routeFrame,
  routeMarkers,
  trackGeometry,
} from '../src/route';

const climb = (start: number, end: number, category: string | null): RouteClimb => ({
  start_index: start,
  end_index: end,
  avg_gradient: 6.2,
  category,
});

/** A translator that shows which key and params it was handed. */
const t = (key: string, params?: Record<string, string | number>) =>
  `${key}(${JSON.stringify(params ?? {})})`;

const box = (south: number, north: number, west: number, east: number): RouteBounds => ({
  min_latitude: south,
  max_latitude: north,
  min_longitude: west,
  max_longitude: east,
});

describe('routeFrame', () => {
  it('floors the frame at four thousandths of a degree', () => {
    expect(MIN_SPAN_DEGREES).toBe(0.004);
  });

  it('frames a single point on a box exactly the minimum span wide, centred on it', () => {
    const [west, south, east, north] = routeFrame(box(45.5, 45.5, -73.6, -73.6));

    expect(east - west).toBeCloseTo(MIN_SPAN_DEGREES, 12);
    expect(north - south).toBeCloseTo(MIN_SPAN_DEGREES, 12);
    expect(west).toBeCloseTo(-73.602, 12);
    expect(east).toBeCloseTo(-73.598, 12);
    expect(south).toBeCloseTo(45.498, 12);
    expect(north).toBeCloseTo(45.502, 12);
  });

  it('widens a two-fix track narrower than the minimum about its midpoint', () => {
    // Two fixes 0.001° apart north-south and 0.0006° east-west: both axes are
    // under the floor, so both grow to it, each about its own midpoint.
    const [west, south, east, north] = routeFrame(box(45.5, 45.501, -73.6006, -73.6));

    expect(east - west).toBeCloseTo(MIN_SPAN_DEGREES, 12);
    expect(north - south).toBeCloseTo(MIN_SPAN_DEGREES, 12);
    expect((north + south) / 2).toBeCloseTo(45.5005, 12);
    expect((east + west) / 2).toBeCloseTo(-73.6003, 12);
  });

  it('widens only the axis that is under the floor', () => {
    // A straight run due north: long in latitude, a point in longitude.
    const [west, south, east, north] = routeFrame(box(45.5, 45.58, -73.6, -73.6));

    expect([south, north]).toEqual([45.5, 45.58]);
    expect(east - west).toBeCloseTo(MIN_SPAN_DEGREES, 12);
    expect((east + west) / 2).toBeCloseTo(-73.6, 12);
  });

  it('frames a real ride exactly as carried, as west, south, east, north', () => {
    expect(routeFrame(box(45.5, 45.58, -73.68, -73.6))).toEqual([-73.68, 45.5, -73.6, 45.58]);
  });

  it('leaves an axis exactly at the minimum span untouched', () => {
    expect(routeFrame(box(0, MIN_SPAN_DEGREES, 0, MIN_SPAN_DEGREES))).toEqual([
      0,
      0,
      MIN_SPAN_DEGREES,
      MIN_SPAN_DEGREES,
    ]);
  });
});

describe('alignedSeries', () => {
  it('keeps a series as long as the track and drops a ragged or empty one', () => {
    expect(alignedSeries([0, 500, 1000], 3)).toEqual([0, 500, 1000]);
    expect(alignedSeries([0, 500], 3)).toBeNull();
    expect(alignedSeries([], 0)).toBeNull();
    expect(alignedSeries(null, 3)).toBeNull();
  });
});

describe('kilometres and metresAt', () => {
  it('prints metres as one decimal of a kilometre', () => {
    expect(kilometres(42_195, 'en')).toBe('42.2');
    expect(kilometres(0, 'en')).toBe('0.0');
    expect(kilometres(12_400, 'fr')).toBe('12,4');
  });

  it('reads an index the wire supplied, or null when it points nowhere', () => {
    expect(metresAt([0, 500, 1000], 2)).toBe(1000);
    expect(metresAt([0, 500, 1000], 3)).toBeNull();
    expect(metresAt([0, 500, 1000], -1)).toBeNull();
    expect(metresAt([0, 500, 1000], 1.5)).toBeNull();
  });
});

describe('climbRange', () => {
  it('spells the climb as a kilometre range read off the distances', () => {
    expect(climbRange([0, 4000, 6000, 8000], climb(1, 3, '2'), 'en', 'metric')).toBe('km 4.0–8.0');
    expect(climbRange([0, 12_400, 13_000, 15_000], climb(1, 3, '2'), 'fr', 'metric')).toBe('km 12,4–15,0');
  });

  it('prints nothing when the track carried no distances or the climb points past them', () => {
    expect(climbRange(null, climb(1, 3, '2'), 'en', 'metric')).toBeNull();
    expect(climbRange([0, 4000], climb(1, 3, '2'), 'en', 'metric')).toBeNull();
  });
});

describe('climbGradient', () => {
  it("prints the average gradient to one decimal in the athlete's notation", () => {
    expect(climbGradient({ ...climb(0, 1, '3'), avg_gradient: 5.3 }, 'en')).toBe('5.3%');
    expect(climbGradient({ ...climb(0, 1, '3'), avg_gradient: 5.3 }, 'fr')).toBe('5,3%');
    expect(climbGradient({ ...climb(0, 1, '3'), avg_gradient: 12 }, 'de')).toBe('12,0%');
  });
});

describe('climbGrade', () => {
  it('captions hors catégorie as the two letters, never "Cat HC"', () => {
    expect(climbGrade(climb(0, 1, 'HC'), t)).toBe('HC');
  });

  it('puts a numbered grade through the Cat N template', () => {
    expect(climbGrade(climb(0, 1, '3'), t)).toBe('chat.routeClimbCategory({"category":"3"})');
  });

  it('gives an ungraded climb no caption', () => {
    expect(climbGrade(climb(0, 1, null), t)).toBeNull();
  });
});

describe('trackGeometry and climbGeometry', () => {
  const track: Array<[number, number]> = [
    [45.5, -73.6],
    [45.6, -73.5],
    [45.7, -73.4],
    [45.8, -73.3],
  ];

  it('flips latitude-first pairs into GeoJSON longitude-first positions', () => {
    expect(trackGeometry(track)).toEqual({
      type: 'LineString',
      coordinates: [
        [-73.6, 45.5],
        [-73.5, 45.6],
        [-73.4, 45.7],
        [-73.3, 45.8],
      ],
    });
  });

  it('slices each climb inclusively and drops one too short to be a line', () => {
    expect(climbGeometry(track, [climb(1, 2, '4'), climb(3, 3, null)])).toEqual({
      type: 'MultiLineString',
      coordinates: [
        [
          [-73.5, 45.6],
          [-73.4, 45.7],
        ],
      ],
    });
  });
});

describe('routeMarkers', () => {
  /** A straight track north along a meridian, one fix every `spacing` metres. */
  function straight(points: number, spacing: number, offset = 0) {
    const coordinates: Array<[number, number]> = [];
    const distances: number[] = [];
    for (let i = 0; i < points; i += 1) {
      coordinates.push([45 + i * 0.001, -73]);
      distances.push(offset + i * spacing);
    }
    return { coordinates, distances };
  }

  const distanceUnits = (markers: ReturnType<typeof routeMarkers>) =>
    markers.flatMap((marker) => (marker.kind === 'distance' ? [marker.units] : []));

  it('marks every kilometre of a short run between its start and finish', () => {
    const { coordinates, distances } = straight(51, 100); // 5 km
    const markers = routeMarkers(coordinates, distances, 'metric');
    expect(markers[0]).toEqual({ kind: 'start', position: coordinates[0] });
    expect(markers[markers.length - 1]).toEqual({ kind: 'finish', position: coordinates[50] });
    // No mark on the 5 km the finish already sits on.
    expect(distanceUnits(markers)).toEqual([1, 2, 3, 4]);
  });

  it('places a mark between the two fixes it falls between', () => {
    const coordinates: Array<[number, number]> = [
      [45, -73],
      [45.01, -73],
      [45.02, -73],
    ];
    const markers = routeMarkers(coordinates, [0, 800, 1800], 'metric');
    const mark = markers.find((marker) => marker.kind === 'distance');
    // 1000 m is a fifth of the way from 800 m to 1800 m.
    expect(mark?.position[0]).toBeCloseTo(45.012, 6);
    expect(mark?.position[1]).toBe(-73);
  });

  it('thins the marks to every five past fifteen kilometres', () => {
    const { coordinates, distances } = straight(422, 100); // 42.1 km
    expect(distanceUnits(routeMarkers(coordinates, distances, 'metric'))).toEqual([5, 10, 15, 20, 25, 30, 35, 40]);
    const fifteen = straight(151, 100); // exactly 15 km stays dense
    expect(distanceUnits(routeMarkers(fifteen.coordinates, fifteen.distances, 'metric'))).toHaveLength(14);
  });

  it('counts miles for an imperial athlete', () => {
    const { coordinates, distances } = straight(101, 100); // 10 km, 6.2 mi
    expect(distanceUnits(routeMarkers(coordinates, distances, 'imperial'))).toEqual([1, 2, 3, 4, 5, 6]);
  });

  it('reads the activity distance on a privacy-trimmed track', () => {
    // The drawn line starts 1.3 km in: the first mark is km 2, not km 1.
    const { coordinates, distances } = straight(31, 100, 1300);
    expect(distanceUnits(routeMarkers(coordinates, distances, 'metric'))).toEqual([2, 3, 4]);
  });

  it('draws only the start and finish when the distance series is absent or ragged', () => {
    const { coordinates, distances } = straight(30, 100);
    expect(routeMarkers(coordinates, null, 'metric').map((marker) => marker.kind)).toEqual(['start', 'finish']);
    expect(routeMarkers(coordinates, distances.slice(1), 'metric').map((marker) => marker.kind)).toEqual([
      'start',
      'finish',
    ]);
  });

  it('draws nothing for an empty track and a start alone for a single fix', () => {
    expect(routeMarkers([], null, 'metric')).toEqual([]);
    expect(routeMarkers([[45, -73]], [0], 'metric')).toEqual([{ kind: 'start', position: [45, -73] }]);
  });
});
