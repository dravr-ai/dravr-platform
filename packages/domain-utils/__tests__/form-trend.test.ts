// ABOUTME: Pins the form trend's projection — the zero line, the scale, the break at a day without a reading
// ABOUTME: The web and mobile charts both draw from these functions, so a day sits at the same height on each

import { describe, expect, it } from 'vitest';
import { nearestTrendIndex, projectFormTrend } from '../src/form-trend';

const BOX = { width: 120, height: 60, padding: 10 };

describe('projectFormTrend', () => {
  it('spaces the days evenly and scales the readings between the padded edges', () => {
    const geometry = projectFormTrend([-20, 0, 20], BOX);
    expect(geometry).toEqual({
      path: 'M10.00 50.00L60.00 30.00L110.00 10.00',
      zeroY: 30,
      points: [
        { x: 10, y: 50 },
        { x: 60, y: 30 },
        { x: 110, y: 10 },
      ],
    });
  });

  it('keeps zero on the chart when every reading is on one side of it', () => {
    const below = projectFormTrend([-10, -30], BOX);
    expect(below?.zeroY).toBe(10);
    expect(below?.points[0]?.y).toBeCloseTo(10 + 40 / 3, 9);
    expect(below?.points[1]).toEqual({ x: 110, y: 50 });
    const above = projectFormTrend([5, 10], BOX);
    expect(above?.zeroY).toBe(50);
  });

  it('breaks the line at a day without a reading instead of bridging it', () => {
    const geometry = projectFormTrend([-10, null, -10, 0], BOX);
    expect(geometry?.path).toBe('M10.00 50.00 M76.67 50.00L110.00 10.00');
    expect(geometry?.points[1]).toBeNull();
  });

  it('draws a series of zeros as a level line on the zero line', () => {
    const geometry = projectFormTrend([0, 0, 0], BOX);
    expect(geometry?.zeroY).toBe(30);
    expect(geometry?.path).toBe('M10.00 30.00L60.00 30.00L110.00 30.00');
  });

  it('draws nothing from fewer than two readings, or into a box with no room', () => {
    expect(projectFormTrend([], BOX)).toBeNull();
    expect(projectFormTrend([-12], BOX)).toBeNull();
    expect(projectFormTrend([null, -12, null], BOX)).toBeNull();
    expect(projectFormTrend([-12, Number.NaN], BOX)).toBeNull();
    expect(projectFormTrend([-12, 4], { width: 20, height: 60, padding: 10 })).toBeNull();
  });
});

describe('nearestTrendIndex', () => {
  const points = [{ x: 10, y: 0 }, null, { x: 60, y: 0 }, { x: 110, y: 0 }];

  it('names the day nearest the pointer, skipping a day without a reading', () => {
    expect(nearestTrendIndex(points, 0)).toBe(0);
    expect(nearestTrendIndex(points, 34)).toBe(0);
    expect(nearestTrendIndex(points, 36)).toBe(2);
    expect(nearestTrendIndex(points, 500)).toBe(3);
  });

  it('names no day when none carries a reading', () => {
    expect(nearestTrendIndex([null, null], 10)).toBeNull();
    expect(nearestTrendIndex([], 10)).toBeNull();
  });
});
