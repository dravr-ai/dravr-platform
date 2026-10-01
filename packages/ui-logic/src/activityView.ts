// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What one activity's view prints — its figures, its split and lap rows, pace or speed by sport — and the questions it offers
// ABOUTME: Both clients render these lists as given, so the web and the phone show the same figures in the same order and units

import { formatDecimal } from '@pierre/chat-utils';
import { formatDuration, type DurationTranslate } from '@pierre/domain-utils';
import type { ActivityDetailResponse } from '@pierre/shared-types';

/** How a sport's speed reads: a pace per kilometre on foot, per 100 m in the water, a speed otherwise. */
export type SpeedForm = 'pace_km' | 'pace_100m' | 'speed';

/**
 * Sports whose speed an athlete reads as a pace per kilometre — the canonical
 * keys `sport_type` carries (`activity-sports.json`).
 */
const PACE_PER_KM_SPORTS: ReadonlySet<string> = new Set([
  'run',
  'virtual_run',
  'trail_running',
  'walk',
  'hike',
  'snowshoe',
]);

/** Sports whose speed an athlete reads as a pace per 100 metres. */
const PACE_PER_100M_SPORTS: ReadonlySet<string> = new Set(['swim']);

/** How `sport` reads its speed. A sport outside the vocabulary reads a speed. */
export function speedForm(sport: string): SpeedForm {
  if (PACE_PER_KM_SPORTS.has(sport)) return 'pace_km';
  if (PACE_PER_100M_SPORTS.has(sport)) return 'pace_100m';
  return 'speed';
}

const SECONDS_PER_HOUR = 3600;
const SECONDS_PER_MINUTE = 60;
const METRES_PER_KM = 1000;
const METRES_PER_POOL_UNIT = 100;

/** `m:ss`, or `h:mm:ss` from an hour up. */
export function formatClock(totalSeconds: number): string {
  const seconds = Math.round(totalSeconds);
  const hours = Math.floor(seconds / SECONDS_PER_HOUR);
  const minutes = Math.floor((seconds % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE);
  const secs = seconds % SECONDS_PER_MINUTE;
  const ss = String(secs).padStart(2, '0');
  return hours > 0 ? `${hours}:${String(minutes).padStart(2, '0')}:${ss}` : `${minutes}:${ss}`;
}

/** A whole number in the notation of `language`, rounded to the unit. */
function formatWhole(value: number, language: string): string {
  return formatDecimal(Math.round(value), 0, language);
}

/** `10.00 km` (`10,00 km` in French), or `850 m` under a kilometre. */
export function formatKilometres(meters: number, language: string): string {
  return meters >= METRES_PER_KM
    ? `${formatDecimal(meters / METRES_PER_KM, 2, language)} km`
    : `${formatWhole(meters, language)} m`;
}

/**
 * A speed in metres per second as `form` reads it: `4:16 /km`, `1:52 /100 m`
 * or `28.4 km/h` (`28,4 km/h` in French). Null for a speed of zero, which has
 * no pace.
 */
export function formatSpeed(metersPerSecond: number, form: SpeedForm, language: string): string | null {
  if (!(metersPerSecond > 0)) return null;
  if (form === 'pace_km') return `${formatClock(METRES_PER_KM / metersPerSecond)} /km`;
  if (form === 'pace_100m') return `${formatClock(METRES_PER_POOL_UNIT / metersPerSecond)} /100 m`;
  return `${formatDecimal((metersPerSecond * SECONDS_PER_HOUR) / METRES_PER_KM, 1, language)} km/h`;
}

/** One figure of the view: the catalogue key of its label and its value with its unit. */
export interface ActivityFigure {
  id: string;
  labelKey: string;
  value: string;
}

/** How far a partition's distance may stray from the activity's and still be read as the whole of it. */
const WHOLE_ACTIVITY_DISTANCE_TOLERANCE = 0.01;

/**
 * The whole activity's moving time, when the cache holds it — `null`
 * otherwise, never an estimate.
 *
 * The activity itself carries one time, `duration_seconds`, whose meaning is
 * the provider's: elapsed time on Strava and WHOOP, the timer time (pauses
 * left out) on Garmin, the moving time on a scraped detail page. Its laps
 * partition it, and so do its splits, so their moving times add up to the
 * activity's when every one of them carries one and together they cover the
 * activity's whole distance (a list the provider cut short is not the
 * activity). Laps are read first, as the athlete's own marks; splits
 * otherwise. A sum longer than the activity's duration is not a moving time,
 * and is refused.
 */
export function wholeMovingTimeSeconds(detail: ActivityDetailResponse): number | null {
  const { distance_meters: distance, duration_seconds: duration } = detail.activity;
  if (distance === null || !(distance > 0)) return null;
  const sumOf = (segments: readonly Segment[]): number | null => {
    if (segments.length === 0) return null;
    let moving = 0;
    let covered = 0;
    for (const segment of segments) {
      if (segment.moving_time_seconds === null) return null;
      moving += segment.moving_time_seconds;
      covered += segment.distance_meters;
    }
    const whole = Math.abs(covered - distance) <= distance * WHOLE_ACTIVITY_DISTANCE_TOLERANCE;
    return whole && moving > 0 && moving <= duration ? moving : null;
  };
  return sumOf(detail.laps) ?? sumOf(detail.splits);
}

/**
 * The activity's average speed in metres per second: the provider's own
 * figure when it sent one, otherwise its distance over its moving time when
 * the cache holds it ({@link wholeMovingTimeSeconds}). Never over the
 * activity's duration alone — what that time covers differs by provider, so
 * a pace worked out from it could not say whether it counts the stops.
 */
function averageSpeed(detail: ActivityDetailResponse, moving: number | null): number | null {
  if (detail.average_speed_mps !== null) return detail.average_speed_mps;
  const distance = detail.activity.distance_meters;
  if (distance === null || !(distance > 0) || moving === null) return null;
  return distance / moving;
}

/**
 * The figures an activity's view prints, in order, each only when the server
 * sent it — a figure the cache does not hold is left out, never shown as zero.
 * The moving time is printed when the cache holds it for the whole activity,
 * and the provider's duration always, under a name that claims no more than
 * it is. Pace or speed follows the sport ({@link speedForm}) and is worked
 * out from distance and moving time when the provider sent none
 * ({@link averageSpeed}); a top speed is printed only where the sport reads a
 * speed, since a best pace over one GPS sample says nothing an athlete can use.
 * Every figure is written in the notation of `language`, and the two times in
 * its words (`t`): `45 min 45 s` in French, where a bare `45:45` could be read
 * as hours and minutes.
 */
export function activityFigures(
  t: DurationTranslate,
  detail: ActivityDetailResponse,
  language: string,
): ActivityFigure[] {
  const { activity } = detail;
  const form = speedForm(activity.sport_type);
  const moving = wholeMovingTimeSeconds(detail);
  const average = averageSpeed(detail, moving);
  const figures: ActivityFigure[] = [];
  const push = (id: string, labelKey: string, value: string | null) => {
    if (value !== null) figures.push({ id, labelKey, value });
  };
  push(
    'distance',
    'home.activity.figure.distance',
    activity.distance_meters !== null && activity.distance_meters > 0
      ? formatKilometres(activity.distance_meters, language)
      : null,
  );
  push('moving_time', 'home.activity.figure.movingTime', moving !== null ? formatDuration(t, moving) : null);
  push('duration', 'home.activity.figure.duration', formatDuration(t, activity.duration_seconds));
  push(
    'elevation_gain',
    'home.activity.figure.elevationGain',
    activity.elevation_gain_meters !== null ? `${formatWhole(activity.elevation_gain_meters, language)} m` : null,
  );
  push(
    'average_speed',
    form === 'speed' ? 'home.activity.figure.avgSpeed' : 'home.activity.figure.avgPace',
    average !== null ? formatSpeed(average, form, language) : null,
  );
  push(
    'max_speed',
    'home.activity.figure.maxSpeed',
    form === 'speed' && detail.max_speed_mps !== null ? formatSpeed(detail.max_speed_mps, form, language) : null,
  );
  push(
    'average_heart_rate',
    'home.activity.figure.avgHeartRate',
    detail.average_heart_rate !== null ? `${formatWhole(detail.average_heart_rate, language)} bpm` : null,
  );
  push(
    'max_heart_rate',
    'home.activity.figure.maxHeartRate',
    detail.max_heart_rate !== null ? `${formatWhole(detail.max_heart_rate, language)} bpm` : null,
  );
  push(
    'average_power',
    'home.activity.figure.avgPower',
    detail.average_power !== null ? `${formatWhole(detail.average_power, language)} W` : null,
  );
  push('calories', 'home.activity.figure.calories', detail.calories !== null ? `${formatWhole(detail.calories, language)} kcal` : null);
  return figures;
}

/** One row of the splits or laps table, every cell already formatted; null is an empty cell. */
export interface SegmentRow {
  index: number;
  distance: string;
  time: string;
  speed: string | null;
  heartRate: string | null;
  elevation: string | null;
}

/** The table a list of splits or laps prints, and which of its optional columns any row fills. */
export interface SegmentTable {
  /** The catalogue key of the speed column's heading: pace or speed, by sport. */
  speedLabelKey: string;
  rows: SegmentRow[];
  /** Whether a column has a value in any row — an empty column is not drawn. */
  hasSpeed: boolean;
  hasHeartRate: boolean;
  hasElevation: boolean;
}

interface Segment {
  index: number;
  distance_meters: number;
  elapsed_time_seconds: number;
  moving_time_seconds: number | null;
  average_speed_mps: number | null;
  average_heart_rate: number | null;
}

/**
 * A climb or a descent to the metre with its sign — `+4 m`, `-3 m` — the sign
 * written here rather than by `Intl`, which spells a minus differently by
 * locale; a change that rounds to zero carries none.
 */
function signedMetres(metres: number, language: string): string {
  const rounded = Math.round(metres);
  const sign = rounded > 0 ? '+' : rounded < 0 ? '-' : '';
  return `${sign}${formatWhole(Math.abs(rounded), language)} m`;
}

/**
 * A segment's speed in metres per second: its distance over its moving time
 * when the provider split that time out, so the pace agrees with the time
 * printed beside it — a provider rounds its own speed to the centimetre per
 * second, which moves a kilometre's pace by a second. Without a moving time
 * the row prints the elapsed time, which counts the stops, and the provider's
 * own speed is the one to print.
 */
function segmentSpeed(segment: Segment): number | null {
  const moving = segment.moving_time_seconds;
  if (moving !== null && moving > 0 && segment.distance_meters > 0) return segment.distance_meters / moving;
  return segment.average_speed_mps;
}

function segmentTable<T extends Segment>(
  sport: string,
  segments: readonly T[],
  elevation: (segment: T) => number | null,
  language: string,
): SegmentTable {
  const form = speedForm(sport);
  const rows = segments.map((segment) => {
    const climb = elevation(segment);
    const speed = segmentSpeed(segment);
    return {
      index: segment.index,
      distance: formatKilometres(segment.distance_meters, language),
      // Moving time when the provider split it out, as the provider's own
      // split pace is computed on it; elapsed time otherwise. A clock, as
      // the pace beside it is: a column of `m:ss` reads alike in every
      // language the app speaks and lines up digit for digit.
      time: formatClock(segment.moving_time_seconds ?? segment.elapsed_time_seconds),
      speed: speed !== null ? formatSpeed(speed, form, language) : null,
      heartRate: segment.average_heart_rate !== null ? `${formatWhole(segment.average_heart_rate, language)} bpm` : null,
      elevation: climb !== null ? signedMetres(climb, language) : null,
    };
  });
  return {
    speedLabelKey: form === 'speed' ? 'home.activity.column.speed' : 'home.activity.column.pace',
    rows,
    hasSpeed: rows.some((row) => row.speed !== null),
    hasHeartRate: rows.some((row) => row.heartRate !== null),
    hasElevation: rows.some((row) => row.elevation !== null),
  };
}

/** The splits table, or null when the platform holds no splits for the activity. */
export function splitsTable(detail: ActivityDetailResponse, language: string): SegmentTable | null {
  if (detail.splits.length === 0) return null;
  return segmentTable(
    detail.activity.sport_type,
    detail.splits,
    (split) => split.elevation_difference_meters,
    language,
  );
}

/**
 * The laps table, or null when the platform holds none — and when the only
 * lap is the whole activity, which repeats the figures above it.
 */
export function lapsTable(detail: ActivityDetailResponse, language: string): SegmentTable | null {
  if (detail.laps.length < 2) return null;
  return segmentTable(detail.activity.sport_type, detail.laps, (lap) => lap.elevation_gain_meters, language);
}

/**
 * A question the view offers about the activity: the chip's short label, and
 * the full question it sends, which names the activity by its title and date
 * so the agent reads the right one.
 */
export interface ActivityPrompt {
  id: 'analyze' | 'compare' | 'recovery' | 'adjust';
  labelKey: string;
  textKey: string;
}

/** The questions an activity's view offers under its figures, in order. */
export const ACTIVITY_PROMPTS: readonly ActivityPrompt[] = [
  { id: 'analyze', labelKey: 'home.activity.prompt.analyzeLabel', textKey: 'home.activity.prompt.analyzeText' },
  { id: 'compare', labelKey: 'home.activity.prompt.compareLabel', textKey: 'home.activity.prompt.compareText' },
  { id: 'recovery', labelKey: 'home.activity.prompt.recoveryLabel', textKey: 'home.activity.prompt.recoveryText' },
  { id: 'adjust', labelKey: 'home.activity.prompt.adjustLabel', textKey: 'home.activity.prompt.adjustText' },
];

/**
 * The key of the sentence a question the athlete typed goes out in when it
 * opens the activity's chat: the thread has no other word on which activity
 * it is about, so the first turn names it. Later turns go out as typed.
 */
export const ACTIVITY_ASK_ABOUT_KEY = 'home.activity.askAbout';

/** The translator {@link activityFirstLine} takes — i18next's `t` fits it. */
export type AskAboutTranslate = (
  key: typeof ACTIVITY_ASK_ABOUT_KEY,
  options: { name: string; date: string; question: string }
) => string;

/**
 * The line a question the athlete typed goes out as when it opens the
 * activity's chat: the sentence naming the activity
 * ({@link ACTIVITY_ASK_ABOUT_KEY}), except a slash command, which goes out as
 * typed — wrapped in a sentence, `/plan` is no longer a command.
 */
export function activityFirstLine(
  t: AskAboutTranslate,
  naming: { name: string; date: string },
  question: string
): string {
  return question.startsWith('/') ? question : t(ACTIVITY_ASK_ABOUT_KEY, { ...naming, question });
}
