// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The on-device health wire shapes — the raw HealthKit / Health Connect samples the phone sends (barrage's DeviceSamples) and the counts the server answers
// ABOUTME: Field for field barrage's serde shape, which refuses unknown fields; the counts parser reads any other body as null

/** The OS store a request is read from: barrage's `LiveSource` key. */
export type DeviceHealthSource = 'apple_health' | 'health_connect';

export const DEVICE_HEALTH_SOURCES: readonly DeviceHealthSource[] = ['apple_health', 'health_connect'];

/** The most samples one request carries, every list counted, deletions and windows included (barrage `MAX_SAMPLES`). */
export const DEVICE_SAMPLES_MAX = 2000;

/**
 * The app and device that wrote a sample into the OS store: HealthKit's
 * `sourceRevision.source.name`, `.bundleIdentifier` and product type; Health
 * Connect's `dataOrigin.packageName` and `metadata.device.model`.
 */
export interface DeviceRecorder {
  name: string;
  bundle?: string | null;
  device_model?: string | null;
}

/** A value in the unit the OS spelt it in. */
export interface DeviceMeasure {
  value: number;
  unit: string;
}

/** A HealthKit `HKWorkout` or a Health Connect `ExerciseSessionRecord`. Instants are RFC 3339. */
export interface DeviceWorkoutSample {
  id: string;
  /** `HKWorkoutActivityType` raw value, or Health Connect's `exerciseType` constant: an integer. */
  activity_type: number;
  start: string;
  end: string;
  /** An integer number of seconds. */
  utc_offset_seconds: number;
  duration_seconds?: number | null;
  distance?: DeviceMeasure | null;
  energy?: DeviceMeasure | null;
  indoor?: boolean | null;
  recorder: DeviceRecorder;
}

/** A HealthKit `HKCategorySample`, or one stage of a Health Connect sleep session. */
export interface DeviceCategorySample {
  id: string;
  /** `HKCategoryTypeIdentifier…`, or `SleepSessionRecord`. */
  type: string;
  /** The raw category value or stage constant: an integer. */
  value: number;
  start: string;
  end: string;
  utc_offset_seconds: number;
  cycle_start?: boolean | null;
  protection_used?: boolean | null;
  recorder: DeviceRecorder;
}

/** A quantity sample: resting heart rate, HRV, body mass, glucose, basal body temperature. */
export interface DeviceQuantitySample {
  id: string;
  /** `HKQuantityTypeIdentifier…`, or a Health Connect record class name. */
  type: string;
  value: number;
  unit: string;
  start: string;
  end: string;
  utc_offset_seconds: number;
  recorder: DeviceRecorder;
}

/** One local day's totals as the OS's own statistics query merges them. */
export interface DeviceDaySample {
  /** `YYYY-MM-DD`, the athlete's local calendar date. */
  date: string;
  utc_offset_seconds: number;
  /** An integer. */
  steps?: number | null;
  active_kcal?: number | null;
  recorder: DeviceRecorder;
}

/** A HealthKit `HKStateOfMind` sample (iOS 18+). */
export interface DeviceStateOfMindSample {
  id: string;
  /** `HKStateOfMindKind`: 1 a momentary emotion, 2 a daily mood. */
  kind: number;
  /** In [-1, 1]. */
  valence: number;
  /** `HKStateOfMindValenceClassification`, 1 to 7. */
  classification: number;
  start: string;
  utc_offset_seconds: number;
  recorder: DeviceRecorder;
}

/** A span whose sleep samples were all re-read and sent in the same request. */
export interface DeviceSleepWindow {
  from: string;
  to: string;
}

/**
 * A span every sample of one quantity type in `[from, to)` was re-read in and
 * sent in the same request: the server replaces that type's readings there
 * with them, none when there are none. A day-level type (resting heart rate,
 * rMSSD, body mass) spans whole local days, midnight to midnight.
 */
export interface DeviceQuantityWindow {
  /** The quantity type, as its samples name it. */
  type: string;
  from: string;
  to: string;
  /** The offset of the local day it covers, in whole seconds; `from` and `to` are local midnights at it for a day-level type. */
  utc_offset_seconds: number;
}

/** Ids of samples the OS store deleted since the last read. */
export interface DeviceDeletedSamples {
  workouts: string[];
  self_reports: string[];
}

/** `POST /api/device-health/{source}/sync`'s body: barrage's `DeviceSamples`. */
export interface DeviceSamples {
  source: DeviceHealthSource;
  workouts: DeviceWorkoutSample[];
  categories: DeviceCategorySample[];
  quantities: DeviceQuantitySample[];
  days: DeviceDaySample[];
  states_of_mind: DeviceStateOfMindSample[];
  sleep_windows: DeviceSleepWindow[];
  quantity_windows: DeviceQuantityWindow[];
  deleted: DeviceDeletedSamples;
}

/** How many samples a request carries, as barrage counts them against `DEVICE_SAMPLES_MAX`. */
export function deviceSamplesCount(samples: DeviceSamples): number {
  return (
    samples.workouts.length +
    samples.categories.length +
    samples.quantities.length +
    samples.days.length +
    samples.states_of_mind.length +
    samples.sleep_windows.length +
    samples.quantity_windows.length +
    samples.deleted.workouts.length +
    samples.deleted.self_reports.length
  );
}

/** What the sync and the delete answer: how many rows of each kind they wrote or removed. */
export interface DeviceHealthCounts {
  activities: number;
  refused: number;
  erased: number;
  nights: number;
  days: number;
  self_reports: number;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isCount(value: unknown): value is number {
  return typeof value === 'number' && Number.isInteger(value) && value >= 0;
}

/**
 * Read the counts. `activities`, `nights`, `days` and `self_reports` are
 * required; `refused` and `erased`, which only a sync has to say, read as 0
 * when absent. Any count that is present must be a non-negative integer, or
 * the whole body reads as `null`.
 */
export function parseDeviceHealthCounts(body: unknown): DeviceHealthCounts | null {
  if (!isRecord(body)) return null;
  const optional = (value: unknown): number | null => (value === undefined ? 0 : isCount(value) ? value : null);
  const refused = optional(body.refused);
  const erased = optional(body.erased);
  if (
    !isCount(body.activities) ||
    !isCount(body.nights) ||
    !isCount(body.days) ||
    !isCount(body.self_reports) ||
    refused === null ||
    erased === null
  ) {
    return null;
  }
  return {
    activities: body.activities,
    refused,
    erased,
    nights: body.nights,
    days: body.days,
    self_reports: body.self_reports,
  };
}
