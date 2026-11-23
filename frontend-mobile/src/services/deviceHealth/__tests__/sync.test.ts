// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The device health sync over a scripted health store (the native module's jest.setup mock) and a stubbed server
// ABOUTME: Anchors move only after a 2xx, pages stay under 2 000 samples, deletions and windows travel, off sends nothing, delete forgets

import AsyncStorage from '@react-native-async-storage/async-storage';
import { DEVICE_SAMPLES_MAX, type DeviceSamples } from '@pierre/shared-types';
import { DravrHealth } from '../../../../modules/dravr-health';
import { deviceHealthApi } from '../../api';
import {
  deleteDeviceHealthData,
  disableDeviceHealth,
  enableDeviceHealth,
  readDeviceHealthState,
  syncDeviceHealth,
} from '..';

jest.mock('../../api', () => ({
  deviceHealthApi: { syncDeviceSamples: jest.fn(), deleteDeviceData: jest.fn() },
}));

const native = DravrHealth as unknown as {
  isAvailable: jest.Mock;
  requestAuthorization: jest.Mock;
  readChanges: jest.Mock;
  readSleep: jest.Mock;
  readQuantities: jest.Mock;
  readDays: jest.Mock;
};
const api = deviceHealthApi as unknown as { syncDeviceSamples: jest.Mock; deleteDeviceData: jest.Mock };

const NOW = Date.parse('2026-09-30T16:00:00.000Z');
const EDT = -4 * 3600;
const COUNTS = { activities: 0, refused: 0, erased: 0, nights: 0, days: 0, self_reports: 0 };
const WORKOUT = 'HKWorkoutTypeIdentifier';
const SLEEP = 'HKCategoryTypeIdentifierSleepAnalysis';
const STEPS = 'HKQuantityTypeIdentifierStepCount';
const RESTING = 'HKQuantityTypeIdentifierRestingHeartRate';
const BODY_MASS = 'HKQuantityTypeIdentifierBodyMass';
const GLUCOSE = 'HKQuantityTypeIdentifierBloodGlucose';
const MOOD = 'HKDataTypeStateOfMind';
const RECORDER = { name: 'Connect', bundle: 'com.garmin.connect.mobile' };

type Page = {
  workouts?: unknown[];
  categories?: unknown[];
  quantities?: unknown[];
  states_of_mind?: unknown[];
  deleted?: string[];
};

/**
 * A store whose types hand over `pages` in order: anchor `null` is page 0,
 * anchor `<type>#n` page n, and each page's anchor is the next one's.
 */
function script(pages: Record<string, Page[]>) {
  native.readChanges.mockImplementation((type: string, anchor: string | null) => {
    const list = pages[type] ?? [];
    const index = anchor === null ? 0 : Number(anchor.split('#')[1]);
    const page = list[index] ?? {};
    return Promise.resolve({
      workouts: page.workouts ?? [],
      categories: page.categories ?? [],
      quantities: page.quantities ?? [],
      states_of_mind: page.states_of_mind ?? [],
      deleted: page.deleted ?? [],
      anchor: `${type}#${Math.min(index + 1, Math.max(list.length, 1))}`,
      more: index + 1 < list.length,
    });
  });
}

function workout(id: string) {
  return {
    id,
    activity_type: 37,
    start: '2026-09-28T10:15:00.000Z',
    end: '2026-09-28T11:00:00.000Z',
    utc_offset_seconds: EDT,
    duration_seconds: 2700,
    distance: { value: 9000, unit: 'm' },
    recorder: RECORDER,
  };
}

function sleep(id: string, start: string, end: string, value = 3) {
  return { id, type: SLEEP, value, start, end, utc_offset_seconds: EDT, recorder: RECORDER };
}

function quantity(id: string, type: string, start: string, value: number, unit: string) {
  return { id, type, value, unit, start, end: start, utc_offset_seconds: EDT, recorder: RECORDER };
}

function bodies(): DeviceSamples[] {
  return api.syncDeviceSamples.mock.calls.map(([body]) => body as DeviceSamples);
}

function size(body: DeviceSamples): number {
  return (
    body.workouts.length +
    body.categories.length +
    body.quantities.length +
    body.days.length +
    body.states_of_mind.length +
    body.sleep_windows.length +
    body.quantity_windows.length +
    body.deleted.workouts.length +
    body.deleted.self_reports.length
  );
}

let user = 0;
/** A fresh account per test: the service keeps one queue per account and store. */
function nextUser(): string {
  user += 1;
  return `athlete-${user}`;
}

async function enabled(): Promise<string> {
  const id = nextUser();
  await expect(enableDeviceHealth(id)).resolves.toBe('enabled');
  return id;
}

beforeEach(async () => {
  jest.clearAllMocks();
  jest.spyOn(Date, 'now').mockReturnValue(NOW);
  await AsyncStorage.clear();
  native.isAvailable.mockResolvedValue(true);
  native.requestAuthorization.mockResolvedValue(true);
  native.readSleep.mockResolvedValue([]);
  native.readQuantities.mockResolvedValue([]);
  native.readDays.mockResolvedValue([]);
  api.syncDeviceSamples.mockResolvedValue(COUNTS);
  api.deleteDeviceData.mockResolvedValue({ activities: 3, nights: 1, days: 2, self_reports: 1 });
  script({});
});

afterEach(() => {
  jest.restoreAllMocks();
});

describe('turning the store on', () => {
  it('asks to read, never to write, and reads back 90 days from no anchor', async () => {
    const id = await enabled();
    const asked = native.requestAuthorization.mock.calls[0][0] as string[];
    expect(asked).toEqual(expect.arrayContaining([WORKOUT, SLEEP, MOOD, 'HKQuantityTypeIdentifierDistanceWalkingRunning']));
    await syncDeviceHealth(id);
    expect(native.readChanges).toHaveBeenCalledWith(WORKOUT, null, '2026-07-02T16:00:00.000Z', 500);
    expect((await readDeviceHealthState(id))?.enabled).toBe(true);
  });

  it('stays off when the prompt did not complete', async () => {
    native.requestAuthorization.mockResolvedValue(false);
    const id = nextUser();
    await expect(enableDeviceHealth(id)).resolves.toBe('declined');
    expect(await readDeviceHealthState(id)).toBeNull();
  });
});

describe('whether anything ever arrived', () => {
  it('stays false while every sync writes nothing, and turns true for good once one writes', async () => {
    const id = await enabled();
    await syncDeviceHealth(id);
    expect((await readDeviceHealthState(id))?.received).toBe(false);

    // The first sync left every type at page 1: the workout arrives there.
    script({ [WORKOUT]: [{}, { workouts: [workout('W1')] }] });
    api.syncDeviceSamples.mockResolvedValueOnce({ ...COUNTS, activities: 1 });
    await syncDeviceHealth(id);
    expect(api.syncDeviceSamples).toHaveBeenCalled();
    expect((await readDeviceHealthState(id))?.received).toBe(true);

    await syncDeviceHealth(id);
    const state = await readDeviceHealthState(id);
    expect(state?.lastCounts?.activities).toBe(0);
    expect(state?.received).toBe(true);
  });

  it('stays false when the server refused every sample it was sent', async () => {
    const id = await enabled();
    await syncDeviceHealth(id);

    script({ [WORKOUT]: [{}, { workouts: [workout('W1')] }] });
    api.syncDeviceSamples.mockResolvedValueOnce({ ...COUNTS, refused: 1 });
    await syncDeviceHealth(id);
    expect(api.syncDeviceSamples).toHaveBeenCalled();
    const state = await readDeviceHealthState(id);
    expect(state?.lastCounts?.refused).toBe(1);
    expect(state?.received).toBe(false);
  });
});

describe('anchors', () => {
  it('are saved only after the server took the page, and a failed page is read again', async () => {
    const id = await enabled();
    script({ [WORKOUT]: [{ workouts: [workout('W1')] }] });
    api.syncDeviceSamples.mockRejectedValueOnce(new Error('HTTP 500'));
    await expect(syncDeviceHealth(id)).rejects.toThrow('HTTP 500');
    expect((await readDeviceHealthState(id))?.anchors).toEqual({});

    await expect(syncDeviceHealth(id)).resolves.toMatchObject({ status: 'synced' });
    const workoutReads = native.readChanges.mock.calls.filter(([type]) => type === WORKOUT);
    expect(workoutReads.map(([, anchor]) => anchor)).toEqual([null, null]);
    expect(bodies()[1].workouts.map((w) => w.id)).toEqual(['W1']);
    expect((await readDeviceHealthState(id))?.anchors[WORKOUT]).toBe(`${WORKOUT}#1`);
  });

  it('keep the last page the server took when a later page fails', async () => {
    const id = await enabled();
    const pages = Array.from({ length: 5 }, (_, page) => ({
      workouts: Array.from({ length: 500 }, (_, n) => workout(`W${page}-${n}`)),
    }));
    script({ [WORKOUT]: pages });
    api.syncDeviceSamples.mockResolvedValueOnce(COUNTS).mockRejectedValueOnce(new Error('HTTP 503'));
    await expect(syncDeviceHealth(id)).rejects.toThrow('HTTP 503');
    expect(bodies().map(size)).toEqual([2000, 500]);
    // Pages 0–3 went in the first request; page 4's anchor waits for its own.
    expect((await readDeviceHealthState(id))?.anchors[WORKOUT]).toBe(`${WORKOUT}#4`);
  });
});

describe('what a request carries', () => {
  it('never more than 2 000 samples, windows and deletions counted', async () => {
    const id = await enabled();
    script({
      [WORKOUT]: [
        { workouts: Array.from({ length: 1500 }, (_, n) => workout(`A${n}`)), deleted: ['D1'] },
        { workouts: Array.from({ length: 1000 }, (_, n) => workout(`B${n}`)) },
      ],
    });
    await syncDeviceHealth(id);
    expect(bodies().every((body) => size(body) <= 2000)).toBe(true);
    expect(bodies().reduce((sum, body) => sum + body.workouts.length, 0)).toBe(2500);
  });

  it('only the fields the wire knows, integers whole', async () => {
    const id = await enabled();
    script({ [WORKOUT]: [{ workouts: [{ ...workout('W1'), activity_type: 37.0000001, extra: 'x' }] }] });
    await syncDeviceHealth(id);
    const [sent] = bodies()[0].workouts;
    expect(sent.activity_type).toBe(37);
    expect(sent).not.toHaveProperty('extra');
    expect(bodies()[0].source).toBe('apple_health');
  });

  it('deleted workouts and moods in their lists, moods as read', async () => {
    const id = await enabled();
    const mood = {
      id: 'M1',
      kind: 2,
      valence: -0.4,
      classification: 3,
      start: '2026-09-29T21:00:00.000Z',
      utc_offset_seconds: EDT,
      recorder: { name: 'Health' },
    };
    script({ [WORKOUT]: [{ deleted: ['W0'] }], [MOOD]: [{ states_of_mind: [mood], deleted: ['M0'] }] });
    await syncDeviceHealth(id);
    const [body] = bodies();
    expect(body.deleted).toEqual({ workouts: ['W0'], self_reports: ['M0'] });
    expect(body.states_of_mind).toEqual([mood]);
  });

  it('a window past one request fails the run and moves no anchor, never dropping it', async () => {
    const id = await enabled();
    script({ [SLEEP]: [{ categories: [sleep('S1', '2026-09-30T03:00:00.000Z', '2026-09-30T10:00:00.000Z')] }] });
    const crowded = Array.from({ length: DEVICE_SAMPLES_MAX + 1 }, (_, i) =>
      sleep(`X${i}`, '2026-09-30T03:00:00.000Z', '2026-09-30T03:00:30.000Z'),
    );
    native.readSleep.mockResolvedValue(crowded);
    await expect(syncDeviceHealth(id)).rejects.toMatchObject({ code: 'window_too_large' });
    expect((await readDeviceHealthState(id))?.anchors[SLEEP]).toBeUndefined();
  });

  it('a changed night as its sleep day, widened to every sample overlapping it', async () => {
    const id = await enabled();
    script({ [SLEEP]: [{ categories: [sleep('S1', '2026-09-30T03:00:00.000Z', '2026-09-30T10:00:00.000Z')] }] });
    // The sleep day is 09-29 12:00 to 09-30 12:00 local (16:00Z to 16:00Z); a
    // nap from 11:00 local crosses its start.
    const nap = sleep('S0', '2026-09-29T15:00:00.000Z', '2026-09-29T17:00:00.000Z', 1);
    const night = sleep('S1', '2026-09-30T03:00:00.000Z', '2026-09-30T10:00:00.000Z');
    native.readSleep.mockResolvedValue([nap, night]);
    await syncDeviceHealth(id);
    const [body] = bodies();
    expect(body.sleep_windows).toEqual([{ from: '2026-09-29T15:00:00.000Z', to: '2026-09-30T16:00:00.000Z' }]);
    expect(body.categories.map((c) => c.id)).toEqual(['S0', 'S1']);
    expect(Object.keys((await readDeviceHealthState(id))?.sleepSpans ?? {}).sort()).toEqual(['S0', 'S1']);
  });

  it('a deleted night as the window it lay in, re-read empty', async () => {
    const id = await enabled();
    script({ [SLEEP]: [{ categories: [sleep('S1', '2026-09-30T03:00:00.000Z', '2026-09-30T10:00:00.000Z')] }] });
    native.readSleep.mockResolvedValue([sleep('S1', '2026-09-30T03:00:00.000Z', '2026-09-30T10:00:00.000Z')]);
    await syncDeviceHealth(id);

    script({ [SLEEP]: [{}, { deleted: ['S1'] }] });
    native.readSleep.mockResolvedValue([]);
    api.syncDeviceSamples.mockClear();
    await syncDeviceHealth(id);
    const [body] = bodies();
    expect(body.sleep_windows).toEqual([{ from: '2026-09-29T16:00:00.000Z', to: '2026-09-30T16:00:00.000Z' }]);
    expect(body.categories).toEqual([]);
    expect((await readDeviceHealthState(id))?.sleepSpans).toEqual({});
  });

  it('a changed step count as the store’s total for its local day', async () => {
    const id = await enabled();
    script({ [STEPS]: [{ quantities: [quantity('Q1', STEPS, '2026-09-30T02:00:00.000Z', 120, 'count')] }] });
    native.readDays.mockResolvedValue([
      { date: '2026-09-29', utc_offset_seconds: EDT, steps: 10432.4, recorder: { name: 'Health' } },
    ]);
    await syncDeviceHealth(id);
    // 02:00Z is 22:00 local on the 29th.
    expect(native.readDays).toHaveBeenCalledWith(['2026-09-29']);
    expect(bodies()[0].days).toEqual([
      { date: '2026-09-29', utc_offset_seconds: EDT, steps: 10432, recorder: { name: 'Health' } },
    ]);
    // The step sample itself never travels: only the day's total does.
    expect(bodies()[0].quantities).toEqual([]);
  });
});

describe('quantity deletions', () => {
  async function sent(id: string, pages: Record<string, Page[]>) {
    script(pages);
    await syncDeviceHealth(id);
    api.syncDeviceSamples.mockClear();
  }

  it('a day-level deletion re-sends that local day for every day-level type', async () => {
    const id = await enabled();
    const resting = quantity('R1', RESTING, '2026-09-29T11:00:00.000Z', 52, 'count/min');
    await sent(id, { [RESTING]: [{ quantities: [resting] }] });
    expect((await readDeviceHealthState(id))?.quantities.R1).toEqual({
      type: RESTING,
      date: '2026-09-29',
      utcOffsetSeconds: EDT,
    });

    const weight = quantity('B1', BODY_MASS, '2026-09-29T12:00:00.000Z', 70.2, 'kg');
    native.readQuantities.mockImplementation((type: string) => Promise.resolve(type === BODY_MASS ? [weight] : []));
    script({ [RESTING]: [{}, { deleted: ['R1'] }] });
    await syncDeviceHealth(id);
    const [body] = bodies();
    const day = { from: '2026-09-29T04:00:00.000Z', to: '2026-09-30T04:00:00.000Z' };
    const window = { ...day, utc_offset_seconds: EDT };
    expect(body.quantity_windows).toEqual([
      { type: RESTING, ...window },
      { type: BODY_MASS, ...window },
    ]);
    expect(body.quantities.map((q) => q.id)).toEqual(['B1']);
    expect(native.readQuantities).toHaveBeenCalledWith(RESTING, day.from, day.to);
    expect(native.readQuantities).toHaveBeenCalledWith(BODY_MASS, day.from, day.to);
    const held = (await readDeviceHealthState(id))?.quantities ?? {};
    expect(held.R1).toBeUndefined();
    expect(held.B1).toBeDefined();
  });

  it('a series deletion re-sends that local day for its own type alone', async () => {
    const id = await enabled();
    await sent(id, { [GLUCOSE]: [{ quantities: [quantity('G1', GLUCOSE, '2026-09-29T13:00:00.000Z', 95, 'mg/dL')] }] });
    const kept = quantity('G2', GLUCOSE, '2026-09-29T18:00:00.000Z', 101, 'mg/dL');
    native.readQuantities.mockResolvedValue([kept]);
    script({ [GLUCOSE]: [{}, { deleted: ['G1'] }] });
    await syncDeviceHealth(id);
    const [body] = bodies();
    expect(body.quantity_windows).toEqual([
      { type: GLUCOSE, from: '2026-09-29T04:00:00.000Z', to: '2026-09-30T04:00:00.000Z', utc_offset_seconds: EDT },
    ]);
    expect(body.quantities).toEqual([kept]);
    expect(native.readQuantities).toHaveBeenCalledTimes(1);
  });

  it('a deleted id the phone never sent sends nothing', async () => {
    const id = await enabled();
    script({ [GLUCOSE]: [{ deleted: ['never-sent'] }] });
    await syncDeviceHealth(id);
    expect(native.readQuantities).not.toHaveBeenCalled();
    expect(api.syncDeviceSamples).not.toHaveBeenCalled();
  });

  it('forgets samples older than the backfill', async () => {
    const id = await enabled();
    await sent(id, {
      [GLUCOSE]: [
        {
          quantities: [
            quantity('OLD', GLUCOSE, '2026-07-01T13:00:00.000Z', 90, 'mg/dL'),
            quantity('NEW', GLUCOSE, '2026-09-29T13:00:00.000Z', 95, 'mg/dL'),
          ],
        },
      ],
    });
    expect(Object.keys((await readDeviceHealthState(id))?.quantities ?? {})).toEqual(['NEW']);
  });
});

describe('turning the store off', () => {
  it('sends nothing and reads nothing while off, and keeps what the server holds', async () => {
    const id = await enabled();
    script({ [WORKOUT]: [{ workouts: [workout('W1')] }] });
    await disableDeviceHealth(id);
    await expect(syncDeviceHealth(id)).resolves.toEqual({ status: 'off' });
    expect(native.readChanges).not.toHaveBeenCalled();
    expect(api.syncDeviceSamples).not.toHaveBeenCalled();
    expect(api.deleteDeviceData).not.toHaveBeenCalled();
  });

  it('stops a run before its next page, keeping none of the page in flight', async () => {
    const id = await enabled();
    const pages = Array.from({ length: 5 }, (_, page) => ({
      workouts: Array.from({ length: 500 }, (_, n) => workout(`W${page}-${n}`)),
    }));
    script({ [WORKOUT]: pages });
    api.syncDeviceSamples.mockImplementationOnce(() => {
      void disableDeviceHealth(id);
      return Promise.resolve(COUNTS);
    });
    await expect(syncDeviceHealth(id)).resolves.toEqual({ status: 'stopped' });
    expect(api.syncDeviceSamples).toHaveBeenCalledTimes(1);
    const state = await readDeviceHealthState(id);
    expect(state?.enabled).toBe(false);
    expect(state?.anchors).toEqual({});
  });

  it('joins a run already going instead of starting a second', async () => {
    const id = await enabled();
    await Promise.all([syncDeviceHealth(id), syncDeviceHealth(id)]);
    expect(native.readChanges.mock.calls.filter(([type]) => type === WORKOUT)).toHaveLength(1);
  });
});

describe('deleting synced data', () => {
  it('deletes on the server, then forgets every anchor and turns the toggle off', async () => {
    const id = await enabled();
    script({ [WORKOUT]: [{ workouts: [workout('W1')] }] });
    await syncDeviceHealth(id);
    expect((await readDeviceHealthState(id))?.anchors[WORKOUT]).toBeDefined();

    await expect(deleteDeviceHealthData(id)).resolves.toMatchObject({ activities: 3, nights: 1 });
    expect(api.deleteDeviceData).toHaveBeenCalledWith('apple_health');
    expect(await readDeviceHealthState(id)).toBeNull();

    // Turning it on again reads the whole backfill from no anchor.
    native.readChanges.mockClear();
    await enableDeviceHealth(id);
    await syncDeviceHealth(id);
    expect(native.readChanges).toHaveBeenCalledWith(WORKOUT, null, expect.any(String), 500);
  });

  it('changes nothing on the phone when the server refuses', async () => {
    const id = await enabled();
    await syncDeviceHealth(id);
    api.deleteDeviceData.mockRejectedValueOnce(new Error('HTTP 500'));
    await expect(deleteDeviceHealthData(id)).rejects.toThrow('HTTP 500');
    const state = await readDeviceHealthState(id);
    expect(state?.enabled).toBe(true);
    expect(state?.anchors[WORKOUT]).toBe(`${WORKOUT}#1`);
  });
});
