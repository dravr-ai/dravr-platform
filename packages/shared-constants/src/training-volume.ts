// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Names the Home weekly volume in the reader's language — the heading, the figures, the trend's label and readout
// ABOUTME: The server sends weeks of numbers and sport keys only; both clients render them through these keys and their own t()

/** The corpus key for each line of the weekly volume card. */
export const TRAINING_VOLUME_KEY = {
  heading: 'home.volume.heading',
  thisWeek: 'home.volume.thisWeek',
  filterLabel: 'home.volume.filterLabel',
  allSports: 'home.volume.allSports',
  distance: 'home.volume.distance',
  time: 'home.volume.time',
  elevation: 'home.volume.elevation',
  activitiesOne: 'home.volume.activitiesOne',
  activitiesMany: 'home.volume.activitiesMany',
  chartDistance: 'home.volume.chartDistance',
  chartTime: 'home.volume.chartTime',
  chartAlt: 'home.volume.chartAlt',
  weekReadout: 'home.volume.weekReadout',
  trendTooShort: 'home.volume.trendTooShort',
  empty: 'home.volume.empty',
  loadFailed: 'home.volume.loadFailed',
} as const;

/** The filter's key for every sport at once; a sport's own key is its wire spelling. */
export const ALL_SPORTS = 'all';

/** The shape of `t()` this module needs: a key and its interpolation values. */
type Translate = (key: string, values?: Record<string, string | number>) => string;

/** "3 activities", "1 activity". */
export function activitiesLine(t: Translate, count: number): string {
  return count === 1 ? t(TRAINING_VOLUME_KEY.activitiesOne) : t(TRAINING_VOLUME_KEY.activitiesMany, { count });
}

/** The trend's label: what the bars measure, over how many weeks they cover. */
export function volumeChartLabel(t: Translate, metric: 'distance' | 'duration', weeks: number): string {
  return t(metric === 'distance' ? TRAINING_VOLUME_KEY.chartDistance : TRAINING_VOLUME_KEY.chartTime, {
    count: weeks,
  });
}

/** Round elapsed seconds to whole minutes: a week's time is read in hours and minutes, never seconds. */
export function weekMinutesSeconds(seconds: number): number {
  return Math.round(seconds / 60) * 60;
}
