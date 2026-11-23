// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The device health sync — turn the phone's store on, read each type from its anchor, send pages of at most 2 000 samples, save anchors after a 2xx
// ABOUTME: One run at a time per account and store; turning off stops sending at the next page, and deleting erases the server's copy and every anchor

import {
  DEVICE_SAMPLES_MAX,
  deviceSamplesCount,
  type DeviceCategorySample,
  type DeviceHealthCounts,
  type DeviceHealthSource,
  type DeviceSamples,
} from '@pierre/shared-types';
import { DravrHealth, type DravrHealthNative } from '../../../modules/dravr-health';
import { deviceHealthApi } from '../api';
import {
  BACKFILL_DAYS,
  dayLevelTypes,
  deviceHealthSource,
  readPermissionTypes,
  syncedTypes,
  type DeviceTypeSpec,
} from './catalogue';
import { categorySample, daySample, nativeChunk, pickAll, quantitySample, type NativeChunk } from './samples';
import {
  clearState,
  loadState,
  saveState,
  type DeviceHealthState,
  type SentQuantity,
  type SentSleepSpan,
} from './state';
import {
  DAY_MS,
  datesBetween,
  isoOf,
  localDates,
  mergeOverlapping,
  sleepDayWindows,
  type Window,
} from './windows';
import { DeviceHealthError } from './errors';

/** How many samples one native read hands back at most. */
const READ_LIMIT = 500;

/** How many times a sleep window is widened to the samples overlapping it before it is sent as it stands. */
const MAX_WIDENINGS = 4;

export type EnableOutcome = 'enabled' | 'unavailable' | 'declined';

export type SyncOutcome =
  | { status: 'synced'; counts: DeviceHealthCounts }
  | { status: 'off' }
  | { status: 'unavailable' }
  | { status: 'stopped' };

/** The part of a request one unit of work adds: a sample, a deletion, or a sleep window with its samples. */
type Atom = Omit<DeviceSamples, 'source'>;

/** What the phone learns from one page of one type, beside its anchor. */
interface Learned {
  addSpans: Record<string, SentSleepSpan>;
  removeSpans: string[];
  addQuantities: Record<string, SentQuantity>;
  removeQuantities: string[];
}

/** What a page the server took lets the phone remember. */
interface Commit extends Learned {
  type: string;
  anchor: string | null;
}

function nothingLearned(): Learned {
  return { addSpans: {}, removeSpans: [], addQuantities: {}, removeQuantities: [] };
}

/** The indexes as this run knows them, ahead of what the server took. */
interface Working {
  spans: Record<string, SentSleepSpan>;
  quantities: Record<string, SentQuantity>;
}

/**
 * The sleep spans worth keeping: those ending within the backfill. A
 * deletion of an older sample finds no span and re-reads nothing, as the
 * first read never sent it either.
 */
function recentSpans(spans: Record<string, SentSleepSpan>): Record<string, SentSleepSpan> {
  const horizon = Date.now() - BACKFILL_DAYS * DAY_MS;
  return Object.fromEntries(Object.entries(spans).filter(([, span]) => Date.parse(span.end) >= horizon));
}

/** The quantity samples worth keeping: those of a local day within the backfill. */
function recentQuantities(sent: Record<string, SentQuantity>): Record<string, SentQuantity> {
  const horizon = Date.now() - BACKFILL_DAYS * DAY_MS;
  return Object.fromEntries(
    Object.entries(sent).filter(([, entry]) => dayStart(entry.date, entry.utcOffsetSeconds) + DAY_MS >= horizon),
  );
}

/** The instant a local `YYYY-MM-DD` day begins under `utcOffsetSeconds`. */
function dayStart(date: string, utcOffsetSeconds: number): number {
  return Date.parse(`${date}T00:00:00Z`) - utcOffsetSeconds * 1000;
}

/** The athlete turned the store off, or deleted its data, while a run was sending. */
class SyncStopped extends Error {}

function zeroCounts(): DeviceHealthCounts {
  return { activities: 0, refused: 0, erased: 0, nights: 0, days: 0, self_reports: 0 };
}

function emptyAtom(): Atom {
  return {
    workouts: [],
    categories: [],
    quantities: [],
    days: [],
    states_of_mind: [],
    sleep_windows: [],
    quantity_windows: [],
    deleted: { workouts: [], self_reports: [] },
  };
}

function atomCount(atom: Atom): number {
  return deviceSamplesCount({ source: 'apple_health', ...atom });
}

function into(target: Atom, atom: Atom): void {
  target.workouts.push(...atom.workouts);
  target.categories.push(...atom.categories);
  target.quantities.push(...atom.quantities);
  target.days.push(...atom.days);
  target.states_of_mind.push(...atom.states_of_mind);
  target.sleep_windows.push(...atom.sleep_windows);
  target.quantity_windows.push(...atom.quantity_windows);
  target.deleted.workouts.push(...atom.deleted.workouts);
  target.deleted.self_reports.push(...atom.deleted.self_reports);
}

// ---------------------------------------------------------------------------
// One run at a time, per account and store

const queues = new Map<string, Promise<unknown>>();
const running = new Map<string, Promise<SyncOutcome>>();
const generations = new Map<string, number>();
const listeners = new Set<() => void>();

function keyOf(userId: string, source: DeviceHealthSource): string {
  return `${source}:${userId}`;
}

function generation(key: string): number {
  return generations.get(key) ?? 0;
}

/** Run `task` after every earlier task of `key`, whatever they ended in. */
function serial<T>(key: string, task: () => Promise<T>): Promise<T> {
  const previous = queues.get(key) ?? Promise.resolve();
  const next = previous.then(task, task);
  queues.set(
    key,
    next.catch(() => undefined),
  );
  return next;
}

function notify(): void {
  for (const listener of listeners) listener();
}

/** Called whenever the stored state or a run's progress changes. */
export function subscribeDeviceHealth(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Whether a run is going for this account on this phone's store. */
export function isDeviceHealthSyncing(userId: string): boolean {
  const source = deviceHealthSource();
  return source !== null && running.has(keyOf(userId, source));
}

/** The stored state for this account on this phone's store. */
export function readDeviceHealthState(userId: string): Promise<DeviceHealthState | null> {
  const source = deviceHealthSource();
  return source === null ? Promise.resolve(null) : loadState(userId, source);
}

/** Whether this phone has a store the sync can read. */
export async function deviceHealthAvailable(): Promise<boolean> {
  if (deviceHealthSource() === null || DravrHealth === null) return false;
  try {
    return await DravrHealth.isAvailable();
  } catch (error) {
    console.warn('Device health availability could not be read:', error);
    return false;
  }
}

// ---------------------------------------------------------------------------
// The athlete's three actions

/**
 * Ask to read the store, then turn it on. "On" is the athlete's toggle after
 * a prompt that completed: HealthKit never says what was declined. A store
 * turned off earlier keeps its anchors and carries on from them.
 */
export async function enableDeviceHealth(userId: string): Promise<EnableOutcome> {
  const source = deviceHealthSource();
  const native = DravrHealth;
  if (source === null || native === null || !(await native.isAvailable())) return 'unavailable';
  const completed = await native.requestAuthorization(readPermissionTypes(source));
  if (!completed) return 'declined';
  await serial(keyOf(userId, source), async () => {
    const held = await loadState(userId, source);
    const state: DeviceHealthState = held
      ? { ...held, enabled: true }
      : {
          enabled: true,
          since: isoOf(Date.now() - BACKFILL_DAYS * DAY_MS),
          anchors: {},
          sleepSpans: {},
          quantities: {},
          lastSyncAt: null,
          lastCounts: null,
          received: false,
        };
    await saveState(userId, source, state);
  });
  notify();
  return 'enabled';
}

/** Stop syncing; what the server holds stays. A run sending now stops before its next page. */
export async function disableDeviceHealth(userId: string): Promise<void> {
  const source = deviceHealthSource();
  if (source === null) return;
  const key = keyOf(userId, source);
  generations.set(key, generation(key) + 1);
  await serial(key, async () => {
    const held = await loadState(userId, source);
    if (held) await saveState(userId, source, { ...held, enabled: false });
  });
  notify();
}

/**
 * Delete everything synced from this store on the server, then forget the
 * anchors and turn the toggle off: turning it on again reads the whole
 * backfill. Nothing changes on the phone when the server refuses.
 */
export async function deleteDeviceHealthData(userId: string): Promise<DeviceHealthCounts> {
  const source = deviceHealthSource();
  if (source === null) throw new DeviceHealthError('no_store');
  const key = keyOf(userId, source);
  generations.set(key, generation(key) + 1);
  const counts = await serial(key, async () => {
    const removed = await deviceHealthApi.deleteDeviceData(source);
    await clearState(userId, source);
    return removed;
  });
  notify();
  return counts;
}

/**
 * Send what changed since the last run. Joins the run already going rather
 * than starting a second one; does nothing while the toggle is off.
 */
export function syncDeviceHealth(userId: string): Promise<SyncOutcome> {
  const source = deviceHealthSource();
  if (source === null || DravrHealth === null) return Promise.resolve({ status: 'unavailable' });
  const native = DravrHealth;
  const key = keyOf(userId, source);
  const current = running.get(key);
  if (current) return current;
  const run = serial(key, () => syncRun(native, userId, source, key))
    .catch((error: unknown) => {
      if (error instanceof SyncStopped) return { status: 'stopped' } as const;
      throw error;
    })
    .finally(() => {
      running.delete(key);
      notify();
    });
  running.set(key, run);
  notify();
  return run;
}

// ---------------------------------------------------------------------------
// One run

async function syncRun(
  native: DravrHealthNative,
  userId: string,
  source: DeviceHealthSource,
  key: string,
): Promise<SyncOutcome> {
  const started = generation(key);
  const held = await loadState(userId, source);
  if (!held?.enabled) return { status: 'off' };

  const committed: DeviceHealthState = {
    ...held,
    anchors: { ...held.anchors },
    sleepSpans: { ...held.sleepSpans },
    quantities: { ...held.quantities },
  };
  const working: Working = { spans: { ...held.sleepSpans }, quantities: { ...held.quantities } };
  const totals = zeroCounts();
  let batch = emptyAtom();
  let commits: Commit[] = [];

  const stopIfTurnedOff = () => {
    if (generation(key) !== started) throw new SyncStopped();
  };

  const flush = async () => {
    stopIfTurnedOff();
    if (atomCount(batch) > 0) {
      const answer = await deviceHealthApi.syncDeviceSamples({ source, ...batch });
      for (const name of Object.keys(totals) as (keyof DeviceHealthCounts)[]) totals[name] += answer[name];
    }
    // A turn-off or delete while the page was in flight wins: its anchors are not kept.
    stopIfTurnedOff();
    for (const commit of commits) {
      if (commit.anchor !== null) committed.anchors[commit.type] = commit.anchor;
      Object.assign(committed.sleepSpans, commit.addSpans);
      for (const id of commit.removeSpans) delete committed.sleepSpans[id];
      Object.assign(committed.quantities, commit.addQuantities);
      for (const id of commit.removeQuantities) delete committed.quantities[id];
    }
    await saveState(userId, source, committed);
    batch = emptyAtom();
    commits = [];
  };

  const add = async (atom: Atom) => {
    const size = atomCount(atom);
    if (size > DEVICE_SAMPLES_MAX) {
      // One re-read window holding more samples than a request may carry. A
      // window replaces what it covers, so it cannot travel in parts: the run
      // fails before this window's anchor moves, rather than dropping it.
      throw new DeviceHealthError('window_too_large');
    }
    if (atomCount(batch) + size > DEVICE_SAMPLES_MAX) await flush();
    into(batch, atom);
  };

  for (const spec of syncedTypes(source)) {
    let anchor: string | null = committed.anchors[spec.type] ?? null;
    for (;;) {
      stopIfTurnedOff();
      const chunk = nativeChunk(await native.readChanges(spec.type, anchor, committed.since, READ_LIMIT));
      if (chunk.dropped > 0) console.warn(`Device health: ${chunk.dropped} unreadable ${spec.type} samples skipped`);
      const { atoms, learned } = await route(native, source, spec, chunk, working, committed.since);
      for (const atom of atoms) await add(atom);
      commits.push({ type: spec.type, anchor: chunk.anchor, ...learned });
      anchor = chunk.anchor;
      if (!chunk.more || chunk.anchor === null) break;
    }
  }
  await flush();

  stopIfTurnedOff();
  committed.sleepSpans = recentSpans(committed.sleepSpans);
  committed.quantities = recentQuantities(committed.quantities);
  committed.lastSyncAt = isoOf(Date.now());
  committed.lastCounts = totals;
  // A refused sample landed nothing: only what was written or erased says the store answered.
  const landed = totals.activities + totals.erased + totals.nights + totals.days + totals.self_reports;
  committed.received = committed.received || landed > 0;
  await saveState(userId, source, committed);
  return { status: 'synced', counts: totals };
}

interface Routed {
  atoms: Atom[];
  learned: Learned;
}

async function route(
  native: DravrHealthNative,
  source: DeviceHealthSource,
  spec: DeviceTypeSpec,
  chunk: NativeChunk,
  working: Working,
  since: string,
): Promise<Routed> {
  if (spec.role === 'sleep') return sleepAtoms(native, chunk, working.spans, since);
  if (spec.role === 'days') return { atoms: await dayAtoms(native, chunk, since), learned: nothingLearned() };
  const atoms = directAtoms(spec, chunk);
  if (spec.deletions !== 'window') return { atoms, learned: nothingLearned() };
  const windowed = await quantityWindowAtoms(native, source, spec, chunk, working.quantities);
  return { atoms: [...atoms, ...windowed.atoms], learned: windowed.learned };
}

function sentQuantity(sample: { type: string; start: string; utc_offset_seconds: number }): SentQuantity | null {
  const [date] = localDates({ start: sample.start, end: sample.start, utcOffsetSeconds: sample.utc_offset_seconds });
  return date === undefined ? null : { type: sample.type, date, utcOffsetSeconds: sample.utc_offset_seconds };
}

/**
 * A quantity type whose samples the server files by id-less day or instant:
 * every sample sent is remembered by id with its local day, and a deletion
 * that hits one re-reads that day whole as a quantity window. A day-level
 * type re-sends the day for every day-level type of the store, since the
 * server replaces the day's recovery and body records whole; a series type
 * re-sends the day for itself alone. A deletion of a sample never sent (or
 * older than the backfill) sends nothing.
 */
async function quantityWindowAtoms(
  native: DravrHealthNative,
  source: DeviceHealthSource,
  spec: DeviceTypeSpec,
  chunk: NativeChunk,
  working: Record<string, SentQuantity>,
): Promise<Routed> {
  const learned = nothingLearned();
  const remember = (id: string, entry: SentQuantity | null) => {
    if (entry === null) return;
    learned.addQuantities[id] = entry;
    working[id] = entry;
  };
  for (const sample of chunk.quantities) remember(sample.id, sentQuantity(sample));

  // One window per (types, local day), however many deletions name it.
  const days = new Map<string, { types: string[]; date: string; utcOffsetSeconds: number }>();
  for (const id of chunk.deleted) {
    const held = working[id];
    if (!held) continue;
    learned.removeQuantities.push(id);
    delete working[id];
    const types = spec.window === 'day' ? dayLevelTypes(source) : [held.type];
    days.set(`${types.join(',')}|${held.date}`, { types, date: held.date, utcOffsetSeconds: held.utcOffsetSeconds });
  }

  const atoms: Atom[] = [];
  for (const { types, date, utcOffsetSeconds } of days.values()) {
    const from = isoOf(dayStart(date, utcOffsetSeconds));
    const to = isoOf(dayStart(date, utcOffsetSeconds) + DAY_MS);
    const atom = emptyAtom();
    for (const type of types) {
      const { items } = pickAll(await native.readQuantities(type, from, to), quantitySample);
      const samples = items.filter((sample) => sample.type === type);
      atom.quantity_windows.push({ type, from, to, utc_offset_seconds: utcOffsetSeconds });
      atom.quantities.push(...samples);
      for (const sample of samples) remember(sample.id, sentQuantity(sample));
    }
    atoms.push(atom);
  }
  return { atoms, learned };
}

/** Each sample and each kept deletion, one atom apiece, so pages fill exactly. */
function directAtoms(spec: DeviceTypeSpec, chunk: NativeChunk): Atom[] {
  const atoms: Atom[] = [];
  const one = (fill: (atom: Atom) => void) => {
    const atom = emptyAtom();
    fill(atom);
    atoms.push(atom);
  };
  for (const sample of chunk.workouts) one((atom) => atom.workouts.push(sample));
  for (const sample of chunk.categories) one((atom) => atom.categories.push(sample));
  for (const sample of chunk.quantities) one((atom) => atom.quantities.push(sample));
  for (const sample of chunk.statesOfMind) one((atom) => atom.states_of_mind.push(sample));
  const list = spec.deletions;
  if (list === 'workouts' || list === 'self_reports') {
    for (const id of chunk.deleted) one((atom) => atom.deleted[list].push(id));
  }
  return atoms;
}

/**
 * The local days a change touched, each sent as the store's own total. A
 * deletion names no day, so every day of the last `BACKFILL_DAYS` is sent
 * again.
 */
async function dayAtoms(native: DravrHealthNative, chunk: NativeChunk, since: string): Promise<Atom[]> {
  const dates = new Set<string>();
  if (chunk.deleted.length > 0) {
    const from = Math.max(Date.parse(since), Date.now() - BACKFILL_DAYS * DAY_MS);
    for (const date of datesBetween(isoOf(from), new Date(Date.now()))) dates.add(date);
  }
  for (const sample of chunk.quantities) {
    for (const date of localDates({ start: sample.start, end: sample.end, utcOffsetSeconds: sample.utc_offset_seconds })) {
      dates.add(date);
    }
  }
  if (dates.size === 0) return [];
  const { items } = pickAll(await native.readDays([...dates].sort()), daySample);
  return items.map((day) => ({ ...emptyAtom(), days: [day] }));
}

function spanOf(sample: DeviceCategorySample): SentSleepSpan {
  return { start: sample.start, end: sample.end, utcOffsetSeconds: sample.utc_offset_seconds };
}

/** The sleep samples overlapping `window`, read whole from the store. */
async function readWindow(native: DravrHealthNative, window: Window): Promise<DeviceCategorySample[]> {
  return pickAll(await native.readSleep(isoOf(window.from), isoOf(window.to)), categorySample).items;
}

/**
 * The sleep days a change or a deletion touched, each widened to every
 * sleep sample overlapping it and re-read whole: one atom per window, the
 * window with all its samples, so the server replaces exactly the nights
 * the window holds and none is ever cut.
 */
async function sleepAtoms(
  native: DravrHealthNative,
  chunk: NativeChunk,
  working: Record<string, SentSleepSpan>,
  since: string,
): Promise<Routed> {
  const learned = nothingLearned();
  const { removeSpans, addSpans } = learned;
  const touched: Window[] = [];
  for (const sample of chunk.categories) touched.push(...sleepDayWindows(spanOf(sample)));
  for (const id of chunk.deleted) {
    // A Health Connect session's stages are sent as `<session id>-<n>`.
    for (const held of Object.keys(working).filter((key) => key === id || key.startsWith(`${id}-`))) {
      touched.push(...sleepDayWindows(working[held]));
      removeSpans.push(held);
      delete working[held];
    }
  }
  if (touched.length === 0) return { atoms: [], learned };

  let windows = mergeOverlapping(touched);
  let reads = await Promise.all(windows.map((window) => readWindow(native, window)));
  for (let round = 0; round < MAX_WIDENINGS; round += 1) {
    const widened = mergeOverlapping(
      windows.map((window, index) => {
        let { from, to } = window;
        for (const sample of reads[index]) {
          from = Math.min(from, Date.parse(sample.start));
          to = Math.max(to, Date.parse(sample.end));
        }
        return { from, to };
      }),
    );
    const settled =
      widened.length === windows.length &&
      widened.every((window, index) => window.from === windows[index].from && window.to === windows[index].to);
    if (settled) break;
    windows = widened;
    reads = await Promise.all(windows.map((window) => readWindow(native, window)));
  }

  const sinceMs = Date.parse(since);
  const atoms = windows.map((window, index): Atom => {
    const samples = reads[index].filter(
      (sample) => Date.parse(sample.start) >= window.from && Date.parse(sample.end) <= window.to,
    );
    for (const sample of samples) {
      if (Date.parse(sample.end) >= sinceMs) {
        addSpans[sample.id] = spanOf(sample);
        working[sample.id] = spanOf(sample);
      }
    }
    return { ...emptyAtom(), categories: samples, sleep_windows: [{ from: isoOf(window.from), to: isoOf(window.to) }] };
  });
  return { atoms, learned };
}
