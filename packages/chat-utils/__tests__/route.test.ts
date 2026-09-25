// ABOUTME: Unit tests for the route card's pure half — the GeoJSON both maps draw and the words printed under them
// ABOUTME: Red if the lat/lon flip, the inclusive climb slice, the km range or the HC caption drift from what both clients show

import { describe, expect, it } from 'vitest';
import type { RouteClimb } from '@pierre/scene-types';
import {
  alignedSeries,
  climbGeometry,
  climbGrade,
  climbRange,
  kilometres,
  metresAt,
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
