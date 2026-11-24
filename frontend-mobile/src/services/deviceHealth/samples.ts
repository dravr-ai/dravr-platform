// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Reads what the native health module answers into barrage's DeviceSamples fields, picking each field by name
// ABOUTME: The server refuses any unknown field and any fractional integer, so nothing is passed through and integers are rounded

import type {
  DeviceCategorySample,
  DeviceDaySample,
  DeviceMeasure,
  DeviceQuantitySample,
  DeviceRecorder,
  DeviceStateOfMindSample,
  DeviceWorkoutSample,
} from '@pierre/shared-types';
import { DeviceHealthError } from './errors';

/** One page of one type's changes, checked. */
export interface NativeChunk {
  workouts: DeviceWorkoutSample[];
  categories: DeviceCategorySample[];
  quantities: DeviceQuantitySample[];
  statesOfMind: DeviceStateOfMindSample[];
  deleted: string[];
  /** The anchor after this page; `null` when the store gave none. */
  anchor: string | null;
  more: boolean;
  /** Samples dropped because a field the server needs was missing or unreadable. */
  dropped: number;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function isText(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0;
}

function isFinite(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value);
}

/** A whole number, as the wire's integer fields need it; `null` when it is none. */
function integer(value: unknown): number | null {
  return isFinite(value) ? Math.round(value) : null;
}

function recorder(value: unknown): DeviceRecorder | null {
  if (!isRecord(value) || !isText(value.name)) return null;
  const picked: DeviceRecorder = { name: value.name };
  if (isText(value.bundle)) picked.bundle = value.bundle;
  if (isText(value.device_model)) picked.device_model = value.device_model;
  return picked;
}

function measure(value: unknown): DeviceMeasure | null {
  if (!isRecord(value) || !isFinite(value.value) || !isText(value.unit)) return null;
  return { value: value.value, unit: value.unit };
}

/** The fields every span-carrying sample shares, or `null`. */
function span(value: Record<string, unknown>) {
  const offset = integer(value.utc_offset_seconds);
  const by = recorder(value.recorder);
  if (!isText(value.id) || !isText(value.start) || !isText(value.end) || offset === null || by === null) {
    return null;
  }
  return { id: value.id, start: value.start, end: value.end, utc_offset_seconds: offset, recorder: by };
}

export function workoutSample(value: unknown): DeviceWorkoutSample | null {
  if (!isRecord(value)) return null;
  const base = span(value);
  const activityType = integer(value.activity_type);
  if (base === null || activityType === null || activityType < 0) return null;
  const sample: DeviceWorkoutSample = { ...base, activity_type: activityType };
  if (isFinite(value.duration_seconds)) sample.duration_seconds = value.duration_seconds;
  const distance = measure(value.distance);
  if (distance) sample.distance = distance;
  const energy = measure(value.energy);
  if (energy) sample.energy = energy;
  if (typeof value.indoor === 'boolean') sample.indoor = value.indoor;
  return sample;
}

export function categorySample(value: unknown): DeviceCategorySample | null {
  if (!isRecord(value)) return null;
  const base = span(value);
  const raw = integer(value.value);
  if (base === null || raw === null || !isText(value.type)) return null;
  const sample: DeviceCategorySample = { ...base, type: value.type, value: raw };
  if (typeof value.cycle_start === 'boolean') sample.cycle_start = value.cycle_start;
  if (typeof value.protection_used === 'boolean') sample.protection_used = value.protection_used;
  return sample;
}

export function quantitySample(value: unknown): DeviceQuantitySample | null {
  if (!isRecord(value)) return null;
  const base = span(value);
  if (base === null || !isText(value.type) || !isFinite(value.value) || !isText(value.unit)) return null;
  return { ...base, type: value.type, value: value.value, unit: value.unit };
}

export function stateOfMindSample(value: unknown): DeviceStateOfMindSample | null {
  if (!isRecord(value)) return null;
  const kind = integer(value.kind);
  const classification = integer(value.classification);
  const offset = integer(value.utc_offset_seconds);
  const by = recorder(value.recorder);
  if (
    !isText(value.id) ||
    !isText(value.start) ||
    !isFinite(value.valence) ||
    kind === null ||
    classification === null ||
    offset === null ||
    by === null
  ) {
    return null;
  }
  return {
    id: value.id,
    kind,
    valence: value.valence,
    classification,
    start: value.start,
    utc_offset_seconds: offset,
    recorder: by,
  };
}

export function daySample(value: unknown): DeviceDaySample | null {
  if (!isRecord(value) || !isText(value.date) || !/^\d{4}-\d{2}-\d{2}$/.test(value.date)) return null;
  const offset = integer(value.utc_offset_seconds);
  const by = recorder(value.recorder);
  const steps = integer(value.steps);
  const kcal = isFinite(value.active_kcal) ? value.active_kcal : null;
  if (offset === null || by === null || (steps === null && kcal === null)) return null;
  const day: DeviceDaySample = { date: value.date, utc_offset_seconds: offset, recorder: by };
  if (steps !== null) day.steps = Math.max(0, steps);
  if (kcal !== null) day.active_kcal = kcal;
  return day;
}

/** Every element `read` accepts, and how many it dropped. */
export function pickAll<T>(value: unknown, read: (item: unknown) => T | null): { items: T[]; dropped: number } {
  if (!Array.isArray(value)) return { items: [], dropped: 0 };
  const items: T[] = [];
  for (const item of value) {
    const picked = read(item);
    if (picked !== null) items.push(picked);
  }
  return { items, dropped: value.length - items.length };
}

/**
 * Read one page of `readChanges`. A body that is not a page at all (no
 * `more` flag) throws: the sync stops before any anchor moves.
 */
export function nativeChunk(value: unknown): NativeChunk {
  if (!isRecord(value) || typeof value.more !== 'boolean') {
    throw new DeviceHealthError('not_a_page');
  }
  const workouts = pickAll(value.workouts, workoutSample);
  const categories = pickAll(value.categories, categorySample);
  const quantities = pickAll(value.quantities, quantitySample);
  const moods = pickAll(value.states_of_mind, stateOfMindSample);
  const deleted = Array.isArray(value.deleted) ? value.deleted.filter(isText) : [];
  return {
    workouts: workouts.items,
    categories: categories.items,
    quantities: quantities.items,
    statesOfMind: moods.items,
    deleted,
    anchor: isText(value.anchor) ? value.anchor : null,
    more: value.more,
    dropped: workouts.dropped + categories.dropped + quantities.dropped + moods.dropped,
  };
}
