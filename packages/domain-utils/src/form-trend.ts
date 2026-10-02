// ABOUTME: Geometry for the Home form trend — projects a per-day series of form percentages into an SVG line and its zero line
// ABOUTME: Pure functions shared by the web and mobile charts, so both draw the same days at the same heights

import type { SketchBox } from './route-sketch';

/** One projected day, in the box's own units. */
export interface TrendPoint {
  x: number;
  y: number;
}

/** A form trend ready to draw. */
export interface FormTrendGeometry {
  /**
   * The line, as SVG path data: one sub-path per unbroken run of days that
   * carry a reading. A day without one breaks the line rather than being
   * bridged, because a bridge would draw form on a day nobody measured.
   */
  path: string;
  /** Where zero sits: form level with fitness. Always inside the box. */
  zeroY: number;
  /** One entry per day, in order; null for a day without a reading. */
  points: (TrendPoint | null)[];
}

/** Two decimals of an SVG unit is finer than any pixel the trend is drawn at. */
const PATH_DECIMALS = 2;
/** A line needs two ends. */
const MIN_READINGS = 2;

function fixed(value: number): string {
  return value.toFixed(PATH_DECIMALS);
}

/**
 * Project one value per day into `box`, days evenly spaced left to right.
 *
 * The vertical scale always holds zero as well as every reading, so the zero
 * line is on the chart and a series that stays on one side of it is drawn on
 * that side rather than stretched across the whole height. A flat series is
 * drawn as a level line.
 *
 * Returns null when fewer than two days carry a reading: one point is a
 * number, not a trend, and the caller says so in words.
 */
export function projectFormTrend(
  values: readonly (number | null)[],
  box: SketchBox,
): FormTrendGeometry | null {
  const readings = values.filter((value): value is number => value !== null && Number.isFinite(value));
  if (readings.length < MIN_READINGS) {
    return null;
  }
  const padding = box.padding ?? 0;
  const innerWidth = box.width - 2 * padding;
  const innerHeight = box.height - 2 * padding;
  if (innerWidth <= 0 || innerHeight <= 0) {
    return null;
  }
  const top = Math.max(0, ...readings);
  const bottom = Math.min(0, ...readings);
  const span = top - bottom;
  const steps = values.length - 1;
  // A series of zeros has no span to scale by; it sits on the zero line, mid-height.
  const yOf = (value: number) =>
    span === 0 ? padding + innerHeight / 2 : padding + ((top - value) / span) * innerHeight;
  const xOf = (index: number) => padding + (steps === 0 ? 0 : (index / steps) * innerWidth);

  const points = values.map((value, index) =>
    value !== null && Number.isFinite(value) ? { x: xOf(index), y: yOf(value) } : null,
  );
  let path = '';
  let drawing = false;
  for (const point of points) {
    if (point === null) {
      drawing = false;
      continue;
    }
    path += `${drawing ? 'L' : `${path === '' ? '' : ' '}M`}${fixed(point.x)} ${fixed(point.y)}`;
    drawing = true;
  }
  return { path, zeroY: yOf(0), points };
}

/**
 * The day nearest a horizontal position, among the days that carry a reading —
 * what a pointer over the chart is pointing at. Null when no day has one.
 */
export function nearestTrendIndex(points: readonly (TrendPoint | null)[], x: number): number | null {
  let nearest: number | null = null;
  let distance = Number.POSITIVE_INFINITY;
  points.forEach((point, index) => {
    if (point === null) return;
    const away = Math.abs(point.x - x);
    if (away < distance) {
      distance = away;
      nearest = index;
    }
  });
  return nearest;
}
