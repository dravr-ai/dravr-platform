// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Names the Home training status in the reader's language — each form band, the form line, the load ratio, the recovery days
// ABOUTME: The server sends bands and numbers only; both clients render them through these keys and their own t()

import type { FormBand, FormReading, TrainingLoadRatio } from '@pierre/shared-types';
import { signedWhole } from './groups';

/**
 * The name of each form band. The words follow the engine's own reading of
 * the band (`FormBand::label` in dravr-cageux): they say how deep fatigue runs
 * against the athlete's own fitness, and never grade it as safe or harmful.
 */
export const FORM_BAND_LABEL_KEY: Record<FormBand, string> = {
  insufficient_history: 'home.status.band.insufficientHistory',
  deep_fatigue: 'home.status.band.deepFatigue',
  heavy_block: 'home.status.band.heavyBlock',
  productive: 'home.status.band.productive',
  balanced: 'home.status.band.balanced',
  fresh: 'home.status.band.fresh',
  detraining: 'home.status.band.detraining',
};

/** The one sentence under each band's name. */
export const FORM_BAND_DETAIL_KEY: Record<FormBand, string> = {
  insufficient_history: 'home.status.bandDetail.insufficientHistory',
  deep_fatigue: 'home.status.bandDetail.deepFatigue',
  heavy_block: 'home.status.bandDetail.heavyBlock',
  productive: 'home.status.bandDetail.productive',
  balanced: 'home.status.bandDetail.balanced',
  fresh: 'home.status.bandDetail.fresh',
  detraining: 'home.status.bandDetail.detraining',
};

/** The corpus key for each other line of the status block. */
export const TRAINING_STATUS_KEY = {
  heading: 'home.status.heading',
  form: 'home.status.form',
  trendLabel: 'home.status.trendLabel',
  trendHint: 'home.status.trendHint',
  trendAlt: 'home.status.trendAlt',
  trendDay: 'home.status.trendDay',
  trendTooShort: 'home.status.trendTooShort',
  loadRatio: 'home.status.loadRatio',
  recoveryNone: 'home.status.recoveryNone',
  recoveryOne: 'home.status.recoveryOne',
  recoveryMany: 'home.status.recoveryMany',
  empty: 'home.status.empty',
  loadFailed: 'home.status.loadFailed',
} as const;

/** The shape of `t()` this module needs: a key and its interpolation values. */
type Translate = (key: string, values?: Record<string, string | number>) => string;

/**
 * "Form -12% of your fitness", or null when there is no chronic base to
 * scale form against — the band then says so and no figure is printed.
 */
export function formLine(t: Translate, reading: FormReading): string | null {
  return reading.pct_of_fitness === null
    ? null
    : t(TRAINING_STATUS_KEY.form, { pct: signedWhole(reading.pct_of_fitness) });
}

/**
 * The recent load as a multiple of the athlete's own average. `ratio` is the
 * figure already written in the reader's notation, since the notation is the
 * client's to choose. States what the number is, and nothing it predicts.
 */
export function loadRatioLine(t: Translate, load: TrainingLoadRatio, ratio: string): string {
  return t(TRAINING_STATUS_KEY.loadRatio, {
    acute: load.acute_days,
    chronic: load.chronic_days,
    ratio,
  });
}

/** How many lighter days today's form calls for, in words. */
export function recoveryLine(t: Translate, days: number): string {
  if (days === 0) return t(TRAINING_STATUS_KEY.recoveryNone);
  if (days === 1) return t(TRAINING_STATUS_KEY.recoveryOne);
  return t(TRAINING_STATUS_KEY.recoveryMany, { count: days });
}

const CIVIL_DATE = /^(\d{4})-(\d{2})-(\d{2})$/;
const MS_PER_DAY = 86_400_000;

/** Midnight UTC of a `YYYY-MM-DD` date, or null for anything else. */
function civilDayMs(date: string): number | null {
  const match = CIVIL_DATE.exec(date);
  if (match === null) return null;
  const ms = Date.UTC(Number(match[1]), Number(match[2]) - 1, Number(match[3]));
  return Number.isNaN(ms) ? null : ms;
}

/**
 * The days a served trend covers: from its first day to its last. Read off
 * the series itself, never off the window the server aims for, because a thin
 * history answers fewer days than that window and the label must not claim
 * the ones it did not get. Null when the series has no two dated ends.
 */
export function trendSpanDays(trend: readonly { date: string }[]): number | null {
  if (trend.length < 2) return null;
  const first = civilDayMs(trend[0].date);
  const last = civilDayMs(trend[trend.length - 1].date);
  if (first === null || last === null || last <= first) return null;
  return Math.round((last - first) / MS_PER_DAY);
}

/** "Your form over the last 8 days" — the span the line drawn under it covers; null without one. */
export function trendLabelLine(t: Translate, trend: readonly { date: string }[]): string | null {
  const days = trendSpanDays(trend);
  return days === null ? null : t(TRAINING_STATUS_KEY.trendLabel, { days });
}

/** One day of the trend, as the chart's readout and a screen reader say it. */
export function trendDayLine(t: Translate, reading: FormReading, date: string): string {
  const band = t(FORM_BAND_LABEL_KEY[reading.band]);
  return reading.pct_of_fitness === null
    ? `${date} · ${band}`
    : t(TRAINING_STATUS_KEY.trendDay, { date, pct: signedWhole(reading.pct_of_fitness), band });
}
