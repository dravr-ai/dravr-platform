// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home weekly volume, read for the page — the sports trained, a week's totals for one sport or all, the bars of the trend
// ABOUTME: Pure functions shared by the web and mobile cards, so both sum the same weeks and draw the same heights

import type { SketchBox } from './route-sketch';

/** One sport's figures in one week, as the server sends them. */
export interface SportVolumeInput {
  sport_type: string;
  activities: number;
  distance_meters: number;
  duration_seconds: number;
  elevation_gain_meters: number;
}

/** One week, as the server sends it. */
export interface VolumeWeekInput {
  week_start: string;
  sports: readonly SportVolumeInput[];
}

/** A week's figures summed over the sports a filter keeps. */
export interface VolumeTotals {
  activities: number;
  distance_meters: number;
  duration_seconds: number;
  elevation_gain_meters: number;
}

/** What the trend's bars measure. */
export type VolumeMetric = 'distance' | 'duration';

/** One bar, in the box's own units. */
export interface VolumeBar {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** The share of each week's slot left empty between bars. */
const BAR_GAP_SHARE = 0.3;

/**
 * The sports trained over `weeks`, most time first, ties by name — the order
 * the filter offers them in.
 */
export function volumeSports(weeks: readonly VolumeWeekInput[]): string[] {
  const seconds = new Map<string, number>();
  for (const week of weeks) {
    for (const sport of week.sports) {
      seconds.set(sport.sport_type, (seconds.get(sport.sport_type) ?? 0) + sport.duration_seconds);
    }
  }
  return [...seconds.entries()]
    .sort(([a, aSeconds], [b, bSeconds]) => bSeconds - aSeconds || a.localeCompare(b))
    .map(([sport]) => sport);
}

/** `week` summed over every sport, or over `sport` alone. */
export function weekTotals(week: VolumeWeekInput, sport: string | null): VolumeTotals {
  const totals: VolumeTotals = { activities: 0, distance_meters: 0, duration_seconds: 0, elevation_gain_meters: 0 };
  for (const entry of week.sports) {
    if (sport !== null && entry.sport_type !== sport) continue;
    totals.activities += entry.activities;
    totals.distance_meters += entry.distance_meters;
    totals.duration_seconds += entry.duration_seconds;
    totals.elevation_gain_meters += entry.elevation_gain_meters;
  }
  return totals;
}

/**
 * Distance when any of the weeks recorded one for the selection, time
 * otherwise: strength work or a pool session logged without a distance still
 * has a trend, and it is never drawn as a row of empty bars.
 */
export function volumeMetric(totals: readonly VolumeTotals[]): VolumeMetric {
  return totals.some((week) => week.distance_meters > 0) ? 'distance' : 'duration';
}

/** The figure a bar stands for. */
export function metricValue(totals: VolumeTotals, metric: VolumeMetric): number {
  return metric === 'distance' ? totals.distance_meters : totals.duration_seconds;
}

/**
 * One bar per value, left to right, standing on the bottom edge of `box`
 * inside its padding. The tallest bar fills the height; a zero is a bar of no
 * height, which the chart's baseline shows. Empty when there is nothing to
 * draw.
 */
export function projectVolumeBars(values: readonly number[], box: SketchBox): VolumeBar[] {
  if (values.length === 0) return [];
  const padding = box.padding ?? 0;
  const innerWidth = Math.max(0, box.width - 2 * padding);
  const innerHeight = Math.max(0, box.height - 2 * padding);
  const slot = innerWidth / values.length;
  const width = slot * (1 - BAR_GAP_SHARE);
  const peak = Math.max(...values.map((value) => (Number.isFinite(value) && value > 0 ? value : 0)));
  const floor = box.height - padding;
  return values.map((value, index) => {
    const height = peak > 0 && Number.isFinite(value) && value > 0 ? (value / peak) * innerHeight : 0;
    return {
      x: padding + index * slot + (slot - width) / 2,
      y: floor - height,
      width,
      height,
    };
  });
}

/** The index of the bar whose slot holds `x`, in the box's units; clamped to the ends. */
export function volumeBarAt(count: number, x: number, box: SketchBox): number | null {
  if (count <= 0) return null;
  const padding = box.padding ?? 0;
  const innerWidth = box.width - 2 * padding;
  if (innerWidth <= 0) return null;
  const index = Math.floor(((x - padding) / innerWidth) * count);
  return Math.min(count - 1, Math.max(0, index));
}
