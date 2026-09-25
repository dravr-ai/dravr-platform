// ABOUTME: Unit tests for the route sketch geometry — polyline decoding and SVG path projection
// ABOUTME: Pins the published polyline vectors, the malformed inputs that must decode to nothing, and the projection's framing

import { describe, it, expect } from 'vitest';
import { decodePolyline, projectRouteToSvgPath, type SketchBox } from '../src/route-sketch';

/** Every `M`/`L` vertex of a path, as numbers. */
function vertices(path: string): Array<[number, number]> {
  return path.split(' L').map((segment) => {
    const [x, y] = segment.replace(/^M/, '').split(' ').map(Number);
    return [x, y];
  });
}

function spans(drawn: Array<[number, number]>): { minX: number; maxX: number; minY: number; maxY: number } {
  const xs = drawn.map(([x]) => x);
  const ys = drawn.map(([, y]) => y);
  return { minX: Math.min(...xs), maxX: Math.max(...xs), minY: Math.min(...ys), maxY: Math.max(...ys) };
}

describe('decodePolyline', () => {
  it('decodes the reference polyline from the encoding specification', () => {
    const points = decodePolyline('_p~iF~ps|U_ulLnnqC_mqNvxq`@');
    expect(points).toHaveLength(3);
    expect(points?.[0][0]).toBeCloseTo(38.5, 5);
    expect(points?.[0][1]).toBeCloseTo(-120.2, 5);
    expect(points?.[1][0]).toBeCloseTo(40.7, 5);
    expect(points?.[1][1]).toBeCloseTo(-120.95, 5);
    expect(points?.[2][0]).toBeCloseTo(43.252, 5);
    expect(points?.[2][1]).toBeCloseTo(-126.453, 5);
  });

  it('decodes the origin, and an empty string as a polyline of no points', () => {
    expect(decodePolyline('??')).toEqual([[0, 0]]);
    expect(decodePolyline('')).toEqual([]);
  });

  it('decodes at precision 6, the OSRM and Valhalla default', () => {
    const points = decodePolyline('nncx`@oefsrIPO', 6);
    expect(points).toHaveLength(2);
    expect(points?.[0][0]).toBeCloseTo(-17.7134, 6);
    expect(points?.[0][1]).toBeCloseTo(178.065, 6);
    expect(points?.[1][0]).toBeCloseTo(-17.713409, 6);
    expect(points?.[1][1]).toBeCloseTo(178.065008, 6);
  });

  it('decodes a precision-7 polyline near the antimeridian without 32-bit overflow', () => {
    // 178.065° at 7 decimals is 1,780,650,000, which zigzags to 3,561,300,000:
    // past the signed 32-bit range a shift-based decoder wraps at.
    const points = decodePolyline('~ykzpI_`giciBfEgE', 7);
    expect(points).toHaveLength(2);
    expect(points?.[0][0]).toBeCloseTo(-17.7134, 7);
    expect(points?.[0][1]).toBeCloseTo(178.065, 7);
    expect(points?.[1][0]).toBeCloseTo(-17.71341, 7);
    expect(points?.[1][1]).toBeCloseTo(178.06501, 7);
  });

  it.each([
    { label: 'a value cut off mid-chunk', encoded: '_p~iF~ps|U_ulLnnqC_mqNvxq`' },
    { label: 'a latitude without its longitude', encoded: '_p~iF' },
    { label: 'a character below the encoding range', encoded: '_p~iF ps|U' },
    { label: 'a character above the encoding range', encoded: `_p~iF${String.fromCharCode(0xff)}ps|U` },
    { label: 'a latitude off the globe', encoded: '_uybQ?' },
  ])('refuses $label', ({ encoded }) => {
    expect(decodePolyline(encoded)).toBeNull();
  });

  it('rejects a precision that is not an integer from 0 to 10', () => {
    expect(() => decodePolyline('??', 11)).toThrow(RangeError);
    expect(() => decodePolyline('??', 2.5)).toThrow(RangeError);
    expect(() => decodePolyline('??', -1)).toThrow(RangeError);
  });
});

describe('projectRouteToSvgPath', () => {
  // A loop near Montréal: 0.02° of latitude by 0.04° of longitude, which at
  // 45.51° N is about 1.4 times wider than it is tall.
  const loop: Array<[number, number]> = [
    [45.5, -73.6],
    [45.52, -73.6],
    [45.52, -73.56],
    [45.5, -73.56],
  ];

  it('draws one vertex per point, starting with a move, inside the padded box', () => {
    const path = projectRouteToSvgPath(loop, { width: 200, height: 80, padding: 4 });
    expect(path).not.toBeNull();
    expect(path?.startsWith('M')).toBe(true);
    const drawn = vertices(path as string);
    expect(drawn).toHaveLength(4);
    const { minX, maxX, minY, maxY } = spans(drawn);
    expect(minX).toBeGreaterThanOrEqual(4);
    expect(maxX).toBeLessThanOrEqual(196);
    expect(minY).toBeGreaterThanOrEqual(4);
    expect(maxY).toBeLessThanOrEqual(76);
  });

  it('fills the limiting axis edge to edge and centres the other', () => {
    // The box is far wider than the loop's 1.4:1, so height limits the scale.
    const { minX, maxX, minY, maxY } = spans(
      vertices(projectRouteToSvgPath(loop, { width: 200, height: 80, padding: 4 }) as string),
    );
    expect(minY).toBeCloseTo(4, 1);
    expect(maxY).toBeCloseTo(76, 1);
    expect((minX + maxX) / 2).toBeCloseTo(100, 1);
  });

  it('puts north at the top', () => {
    const drawn = vertices(projectRouteToSvgPath(loop, { width: 100, height: 100 }) as string);
    // The second point is 0.02° north of the first, so it sits higher on screen.
    expect(drawn[1][1]).toBeLessThan(drawn[0][1]);
  });

  it('scales longitude by the cosine of latitude, so the shape keeps its real proportions', () => {
    const { minX, maxX, minY, maxY } = spans(
      vertices(projectRouteToSvgPath(loop, { width: 1000, height: 1000 }) as string),
    );
    const expected = (0.04 * Math.cos((45.51 * Math.PI) / 180)) / 0.02;
    expect((maxX - minX) / (maxY - minY)).toBeCloseTo(expected, 2);
  });

  it('draws a due-north track down the middle of the box', () => {
    const drawn = vertices(
      projectRouteToSvgPath(
        [
          [45.5, -73.6],
          [45.6, -73.6],
        ],
        { width: 200, height: 100 },
      ) as string,
    );
    expect(drawn).toEqual([
      [100, 100],
      [100, 0],
    ]);
  });

  it('draws a track crossing the antimeridian as one short line, not one spanning the globe', () => {
    const drawn = vertices(
      projectRouteToSvgPath(
        [
          [-16.5, 179.9],
          [-16.6, -179.9],
        ],
        { width: 100, height: 100 },
      ) as string,
    );
    // Unwrapped, the step is 0.2° east and 0.1° south, so the drop fills about
    // half the box; read as a 359.8° span it would flatten to a sliver.
    expect(Math.abs(drawn[1][1] - drawn[0][1])).toBeGreaterThan(40);
  });

  it('collapses points that land on the same spot at the drawn resolution', () => {
    const path = projectRouteToSvgPath(
      [
        [45.5, -73.6],
        [45.5, -73.6],
        [45.6, -73.5],
      ],
      { width: 50, height: 50 },
    );
    expect(vertices(path as string)).toHaveLength(2);
  });

  it('draws a decoded polyline end to end', () => {
    const decoded = decodePolyline('_p~iF~ps|U_ulLnnqC_mqNvxq`@');
    expect(decoded).not.toBeNull();
    const path = projectRouteToSvgPath(decoded ?? [], { width: 64, height: 40, padding: 2 });
    expect(path).toMatch(/^M[\d.]+ [\d.]+ L[\d.]+ [\d.]+ L[\d.]+ [\d.]+$/);
  });

  const square = { width: 100, height: 100 };
  it.each<{ label: string; points: Array<[number, number]>; box: SketchBox }>([
    { label: 'fewer than two points', points: [[45.5, -73.6]], box: square },
    { label: 'every point in one place', points: [[45.5, -73.6], [45.5, -73.6]], box: square },
    { label: 'a point off the globe', points: [[45.5, -73.6], [91, -73.6]], box: square },
    { label: 'a point that is not a number', points: [[45.5, -73.6], [Number.NaN, -73.6]], box: square },
    { label: 'a box with no room inside its padding', points: loop, box: { width: 20, height: 20, padding: 10 } },
    { label: 'a negative padding', points: loop, box: { width: 20, height: 20, padding: -1 } },
  ])('returns null for $label', ({ points, box }) => {
    expect(projectRouteToSvgPath(points, box)).toBeNull();
  });
});
