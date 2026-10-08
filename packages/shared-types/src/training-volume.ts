// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The Home weekly volume wire shape — distance, time, climbing and sessions per sport for each of the last twelve weeks
// ABOUTME: With its parser, so a malformed body is an error rather than a chart of invented zeros

/** What one sport added up to in one week. */
export interface SportVolume {
  /** The sport, as the activity cache spells it (`run`, `ride`, …). */
  sport_type: string;
  /** Workouts counted, one per workout whichever providers hold a copy. */
  activities: number;
  /** Metres, over the workouts that recorded a distance. */
  distance_meters: number;
  /** Elapsed seconds. */
  duration_seconds: number;
  /** Metres climbed, over the workouts that recorded it. */
  elevation_gain_meters: number;
}

/** One Monday-to-Sunday week on the athlete's calendar. */
export interface VolumeWeek {
  /** The Monday it starts on, `YYYY-MM-DD`. */
  week_start: string;
  /** One entry per sport trained; empty for a week without training. */
  sports: SportVolume[];
}

/** `GET /api/me/training-volume`. */
export interface TrainingVolumeResponse {
  /** The athlete's today, `YYYY-MM-DD` in their own timezone. */
  today: string;
  /**
   * The weeks the stored history reaches, oldest first, ending with the week
   * `today` falls in; at most twelve, and empty when nothing is stored. A week
   * before the history begins is absent, never a zero.
   */
  weeks: VolumeWeek[];
}

const CIVIL_DATE = /^\d{4}-\d{2}-\d{2}$/;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isAmount(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0;
}

function parseSportVolume(value: unknown): SportVolume | null {
  if (
    !isRecord(value) ||
    typeof value.sport_type !== 'string' ||
    value.sport_type === '' ||
    !isAmount(value.activities) ||
    !Number.isInteger(value.activities) ||
    !isAmount(value.distance_meters) ||
    !isAmount(value.duration_seconds) ||
    !isAmount(value.elevation_gain_meters)
  ) {
    return null;
  }
  return {
    sport_type: value.sport_type,
    activities: value.activities,
    distance_meters: value.distance_meters,
    duration_seconds: value.duration_seconds,
    elevation_gain_meters: value.elevation_gain_meters,
  };
}

/**
 * Read a `GET /api/me/training-volume` body. Every key must be present and
 * every figure a finite, non-negative number: an omitted figure is rejected
 * rather than read as zero, because the page would draw it as a week without
 * training.
 */
export function parseTrainingVolumeResponse(body: unknown): TrainingVolumeResponse | null {
  if (!isRecord(body) || typeof body.today !== 'string' || !CIVIL_DATE.test(body.today) || !Array.isArray(body.weeks)) {
    return null;
  }
  const weeks: VolumeWeek[] = [];
  for (const entry of body.weeks) {
    if (
      !isRecord(entry) ||
      typeof entry.week_start !== 'string' ||
      !CIVIL_DATE.test(entry.week_start) ||
      !Array.isArray(entry.sports)
    ) {
      return null;
    }
    const sports: SportVolume[] = [];
    for (const sport of entry.sports) {
      const parsed = parseSportVolume(sport);
      if (parsed === null) return null;
      sports.push(parsed);
    }
    weeks.push({ week_start: entry.week_start, sports });
  }
  return { today: body.today, weeks };
}
