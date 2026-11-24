// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: What the device health sync keeps on the phone, per account and store — the toggle, the read anchors, the sleep spans and quantities sent, the last sync
// ABOUTME: In AsyncStorage like the app's other non-secret preferences; anchors are written only after the server took the page they cover

import AsyncStorage from '@react-native-async-storage/async-storage';
import type { DeviceHealthCounts, DeviceHealthSource } from '@pierre/shared-types';

/** A sleep sample the server holds: its span, so a deletion that names only its id finds its night. */
export interface SentSleepSpan {
  start: string;
  end: string;
  utcOffsetSeconds: number;
}

/** A quantity sample the server holds: its type and local day, so a deletion that names only its id finds its window. */
export interface SentQuantity {
  type: string;
  /** Local `YYYY-MM-DD`. */
  date: string;
  utcOffsetSeconds: number;
}

export interface DeviceHealthState {
  /** The athlete's toggle, on only after a completed permission prompt. */
  enabled: boolean;
  /** Where the first read began (RFC 3339): every read stays after it. */
  since: string;
  /** Each type's anchor or changes token, opaque, as of the last page the server took. */
  anchors: Record<string, string>;
  /** The sleep samples sent, by sample id. */
  sleepSpans: Record<string, SentSleepSpan>;
  /** The windowed quantity samples sent, by sample id. */
  quantities: Record<string, SentQuantity>;
  /** When a sync last finished (RFC 3339). */
  lastSyncAt: string | null;
  /** What that sync wrote. */
  lastCounts: DeviceHealthCounts | null;
  /** Whether any sync has written anything: HealthKit hides a refused read, so none ever is the only sign. */
  received: boolean;
}

function storageKey(userId: string, source: DeviceHealthSource): string {
  return ['deviceHealth', source, userId].join(':');
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function stringMap(value: unknown): Record<string, string> {
  if (!isRecord(value)) return {};
  return Object.fromEntries(
    Object.entries(value).filter((entry): entry is [string, string] => typeof entry[1] === 'string'),
  );
}

function spanMap(value: unknown): Record<string, SentSleepSpan> {
  if (!isRecord(value)) return {};
  const spans: Record<string, SentSleepSpan> = {};
  for (const [id, span] of Object.entries(value)) {
    if (
      isRecord(span) &&
      typeof span.start === 'string' &&
      typeof span.end === 'string' &&
      typeof span.utcOffsetSeconds === 'number'
    ) {
      spans[id] = { start: span.start, end: span.end, utcOffsetSeconds: span.utcOffsetSeconds };
    }
  }
  return spans;
}

function quantityMap(value: unknown): Record<string, SentQuantity> {
  if (!isRecord(value)) return {};
  const sent: Record<string, SentQuantity> = {};
  for (const [id, entry] of Object.entries(value)) {
    if (
      isRecord(entry) &&
      typeof entry.type === 'string' &&
      typeof entry.date === 'string' &&
      typeof entry.utcOffsetSeconds === 'number'
    ) {
      sent[id] = { type: entry.type, date: entry.date, utcOffsetSeconds: entry.utcOffsetSeconds };
    }
  }
  return sent;
}

function counts(value: unknown): DeviceHealthCounts | null {
  if (!isRecord(value)) return null;
  const keys = ['activities', 'refused', 'erased', 'nights', 'days', 'self_reports'] as const;
  if (!keys.every((key) => typeof value[key] === 'number')) return null;
  return {
    activities: Number(value.activities),
    refused: Number(value.refused),
    erased: Number(value.erased),
    nights: Number(value.nights),
    days: Number(value.days),
    self_reports: Number(value.self_reports),
  };
}

/** The stored state, or `null` when this account never turned the store on (or deleted its data). */
export async function loadState(userId: string, source: DeviceHealthSource): Promise<DeviceHealthState | null> {
  let raw: string | null;
  try {
    raw = await AsyncStorage.getItem(storageKey(userId, source));
  } catch (error) {
    console.warn('Device health state could not be read:', error);
    return null;
  }
  if (raw === null) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    return null;
  }
  if (!isRecord(parsed) || typeof parsed.enabled !== 'boolean' || typeof parsed.since !== 'string') return null;
  return {
    enabled: parsed.enabled,
    since: parsed.since,
    anchors: stringMap(parsed.anchors),
    sleepSpans: spanMap(parsed.sleepSpans),
    quantities: quantityMap(parsed.quantities),
    lastSyncAt: typeof parsed.lastSyncAt === 'string' ? parsed.lastSyncAt : null,
    lastCounts: counts(parsed.lastCounts),
    received: parsed.received === true,
  };
}

export async function saveState(userId: string, source: DeviceHealthSource, state: DeviceHealthState): Promise<void> {
  await AsyncStorage.setItem(storageKey(userId, source), JSON.stringify(state));
}

/** Forget everything: the next turn-on prompts again and reads the whole backfill. */
export async function clearState(userId: string, source: DeviceHealthSource): Promise<void> {
  await AsyncStorage.removeItem(storageKey(userId, source));
}
