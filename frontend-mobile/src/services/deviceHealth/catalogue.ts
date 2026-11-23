// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Which OS store this phone has, the types the device health sync reads from it, and how each type's changes travel
// ABOUTME: Identifiers are the OS's own (HealthKit type identifiers, Health Connect record class names); routing only, no meaning

import { Platform } from 'react-native';
import type { DeviceHealthSource } from '@pierre/shared-types';

/**
 * How one type's changes reach the server:
 * - `direct`: its samples are sent as read;
 * - `sleep`: a change names the sleep days to re-read whole, sent as windows;
 * - `days`: a change names the local days whose OS totals are sent.
 */
export type DeviceTypeRole = 'direct' | 'sleep' | 'days';

/**
 * Where a type's deleted sample ids go: barrage's `deleted` lists; `window`,
 * a quantity window re-reading the span the deleted sample lay in (the OS
 * names a deletion by id alone); or nowhere.
 */
export type DeviceDeletionList = 'workouts' | 'self_reports' | 'window' | 'none';

/**
 * How a quantity type's window spans: `day`, a whole local day, re-sent with
 * every day-level type of the store, since the server files them on one
 * record per day; `series`, that type alone over the day.
 */
export type QuantityWindowKind = 'day' | 'series';

export interface DeviceTypeSpec {
  /** The identifier the native module reads it by. */
  type: string;
  role: DeviceTypeRole;
  deletions: DeviceDeletionList;
  /** Set for every type whose deletions are `window`. */
  window?: QuantityWindowKind;
}

/** How far back the first read goes; older history comes from the export import. */
export const BACKFILL_DAYS = 90;

const APPLE_SELF_REPORT_CATEGORIES = [
  'MindfulSession',
  'MenstrualFlow',
  'IntermenstrualBleeding',
  'OvulationTestResult',
  'CervicalMucusQuality',
  'SexualActivity',
  'Contraceptive',
  'PregnancyTestResult',
  'ProgesteroneTestResult',
  'Pregnancy',
  'Lactation',
];

/**
 * HealthKit, in the order a sync reads it. Step and energy deletions re-send
 * days, sleep deletions re-send nights, quantity deletions re-send a window.
 */
const APPLE_TYPES: readonly DeviceTypeSpec[] = [
  { type: 'HKWorkoutTypeIdentifier', role: 'direct', deletions: 'workouts' },
  { type: 'HKCategoryTypeIdentifierSleepAnalysis', role: 'sleep', deletions: 'none' },
  { type: 'HKQuantityTypeIdentifierStepCount', role: 'days', deletions: 'none' },
  { type: 'HKQuantityTypeIdentifierActiveEnergyBurned', role: 'days', deletions: 'none' },
  { type: 'HKQuantityTypeIdentifierRestingHeartRate', role: 'direct', deletions: 'window', window: 'day' },
  { type: 'HKQuantityTypeIdentifierHeartRateVariabilitySDNN', role: 'direct', deletions: 'window', window: 'series' },
  { type: 'HKQuantityTypeIdentifierBodyMass', role: 'direct', deletions: 'window', window: 'day' },
  { type: 'HKQuantityTypeIdentifierBloodGlucose', role: 'direct', deletions: 'window', window: 'series' },
  { type: 'HKQuantityTypeIdentifierBasalBodyTemperature', role: 'direct', deletions: 'self_reports' },
  ...APPLE_SELF_REPORT_CATEGORIES.map(
    (name): DeviceTypeSpec => ({ type: `HKCategoryTypeIdentifier${name}`, role: 'direct', deletions: 'self_reports' }),
  ),
  // State of Mind (iOS 18+): the native side reads nothing on an older iOS.
  { type: 'HKDataTypeStateOfMind', role: 'direct', deletions: 'self_reports' },
];

/** Read only so a workout's distance can be summed; never read as samples of their own. */
const APPLE_READ_ONLY = [
  'HKQuantityTypeIdentifierDistanceWalkingRunning',
  'HKQuantityTypeIdentifierDistanceCycling',
  'HKQuantityTypeIdentifierDistanceSwimming',
  'HKQuantityTypeIdentifierDistanceWheelchair',
  'HKQuantityTypeIdentifierDistanceDownhillSnowSports',
];

/** Health Connect, in the order a sync reads it. It has no mood record type. */
const HEALTH_CONNECT_TYPES: readonly DeviceTypeSpec[] = [
  { type: 'ExerciseSessionRecord', role: 'direct', deletions: 'workouts' },
  { type: 'SleepSessionRecord', role: 'sleep', deletions: 'none' },
  { type: 'StepsRecord', role: 'days', deletions: 'none' },
  { type: 'ActiveCaloriesBurnedRecord', role: 'days', deletions: 'none' },
  { type: 'RestingHeartRateRecord', role: 'direct', deletions: 'window', window: 'day' },
  { type: 'HeartRateVariabilityRmssdRecord', role: 'direct', deletions: 'window', window: 'day' },
  { type: 'WeightRecord', role: 'direct', deletions: 'window', window: 'day' },
];

/** Read only so a session's distance can be summed. */
const HEALTH_CONNECT_READ_ONLY = ['DistanceRecord'];

/** The OS store on this phone, or `null` where there is none (web). */
export function deviceHealthSource(): DeviceHealthSource | null {
  if (Platform.OS === 'ios') return 'apple_health';
  if (Platform.OS === 'android') return 'health_connect';
  return null;
}

/** The types a sync reads from `source`, in order. */
export function syncedTypes(source: DeviceHealthSource): readonly DeviceTypeSpec[] {
  return source === 'apple_health' ? APPLE_TYPES : HEALTH_CONNECT_TYPES;
}

/** The store's day-level quantity types: re-sent together for a day one of them lost a sample on. */
export function dayLevelTypes(source: DeviceHealthSource): string[] {
  return syncedTypes(source)
    .filter((spec) => spec.window === 'day')
    .map((spec) => spec.type);
}

/** Every type the permission prompt asks to read. Nothing is ever asked for writing. */
export function readPermissionTypes(source: DeviceHealthSource): string[] {
  const readOnly = source === 'apple_health' ? APPLE_READ_ONLY : HEALTH_CONNECT_READ_ONLY;
  return [...syncedTypes(source).map((spec) => spec.type), ...readOnly];
}
