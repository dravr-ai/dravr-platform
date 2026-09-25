// ABOUTME: Unit tests for the route card's pure half — the frame and GeoJSON both maps draw and the words printed under them
// ABOUTME: Red if the minimum span, the lat/lon flip, the inclusive climb slice, the km range or the HC caption drift between clients

import { describe, expect, it } from 'vitest';
import type { RouteBounds, RouteClimb } from '@pierre/scene-types';
import {
  MIN_SPAN_DEGREES,
  alignedSeries,
  climbGeometry,
  climbGrade,
  climbRange,
  kilometres,
  metresAt,
  routeFrame,
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
    expect(kilometres(42_195)).toBe('42.2');
    expect(kilometres(0)).toBe('0.0');
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
    expect(climbRange([0, 4000, 6000, 8000], climb(1, 3, '2'))).toBe('km 4.0–8.0');
  });

  it('prints nothing when the track carried no distances or the climb points past them', () => {
    expect(climbRange(null, climb(1, 3, '2'))).toBeNull();
    expect(climbRange([0, 4000], climb(1, 3, '2'))).toBeNull();
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
