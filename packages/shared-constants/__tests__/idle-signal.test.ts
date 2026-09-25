// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The send path's side of the idle contract, driven through a real registered IdleWatch and fake timers
// ABOUTME: The turn signal aborts after the idle window, a busy hold defers it, and queued work runs on return

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { IDLE_STOP_AFTER_MS, IdleWatch } from '../src/query-policy';
import {
  holdIdleWhileBusy,
  idleAbort,
  idleSignal,
  registerIdleWatch,
  resetIdleAbort,
  trackAbsence,
  whenAthleteReturns,
} from '../src/idle-signal';

describe('idle signal with a registered watch', () => {
  let watch: IdleWatch;

  beforeEach(() => {
    vi.useFakeTimers();
    resetIdleAbort();
    // Bound exactly as both clients bind it at their root.
    watch = new IdleWatch({ onIdle: idleAbort, onSuspend: () => {}, onActive: resetIdleAbort });
    registerIdleWatch(watch);
  });

  afterEach(() => {
    registerIdleWatch(null);
    watch.stop();
    resetIdleAbort();
    vi.useRealTimers();
  });

  it('aborts the open turn once the idle window passes, and a return starts an unaborted stretch', () => {
    const inFlight = idleSignal();

    vi.advanceTimersByTime(IDLE_STOP_AFTER_MS - 1);
    expect(inFlight.aborted).toBe(false);
    vi.advanceTimersByTime(1);
    expect(inFlight.aborted).toBe(true);
    // Still the same stretch: a turn sent now would be born aborted.
    expect(idleSignal()).toBe(inFlight);

    watch.noteInteraction();
    const fresh = idleSignal();
    expect(fresh).not.toBe(inFlight);
    expect(fresh.aborted).toBe(false);
    expect(inFlight.aborted).toBe(true);
  });

  it('holds a visible client active for a slow turn, and aborts a window after the release', () => {
    const inFlight = idleSignal();
    const release = holdIdleWhileBusy();

    vi.advanceTimersByTime(IDLE_STOP_AFTER_MS * 3);
    expect(inFlight.aborted).toBe(false);

    release();
    vi.advanceTimersByTime(IDLE_STOP_AFTER_MS - 1);
    expect(inFlight.aborted).toBe(false);
    vi.advanceTimersByTime(1);
    expect(inFlight.aborted).toBe(true);
  });

  it('resolves whenAthleteReturns at once while the athlete is here', () => {
    let ran = 0;
    whenAthleteReturns(() => {
      ran += 1;
    });
    expect(ran).toBe(1);
  });

  it('resolves whenAthleteReturns on the return from idle, once, and not before', async () => {
    vi.advanceTimersByTime(IDLE_STOP_AFTER_MS);
    expect(watch.isIdle).toBe(true);

    let settled = false;
    const back = new Promise<void>((resolve) => whenAthleteReturns(resolve)).then(() => {
      settled = true;
    });
    await Promise.resolve();
    expect(settled).toBe(false);

    watch.noteInteraction();
    await back;
    expect(settled).toBe(true);
    // The re-read sees a fresh stretch: the return ran onActive first.
    expect(idleSignal().aborted).toBe(false);
  });

  it('resolves whenAthleteReturns when a hidden client is shown again', async () => {
    const work = vi.fn();
    watch.suspend();
    whenAthleteReturns(work);
    vi.advanceTimersByTime(60_000);
    expect(work).not.toHaveBeenCalled();

    watch.resume();
    expect(work).toHaveBeenCalledTimes(1);
    watch.suspend();
    watch.resume();
    expect(work).toHaveBeenCalledTimes(1);
  });

  it('answers trackAbsence with whether the athlete left while the turn ran', () => {
    const asked = trackAbsence();
    expect(asked()).toBe(false);

    watch.suspend();
    watch.resume();
    expect(asked()).toBe(true);
    // A turn sent after the return starts with the athlete present.
    expect(trackAbsence()()).toBe(false);
  });
});

describe('idle signal with no watch registered', () => {
  beforeEach(() => {
    registerIdleWatch(null);
    resetIdleAbort();
  });

  it('holds nothing, reports nobody away, and runs return work at once', () => {
    const release = holdIdleWhileBusy();
    release();
    expect(trackAbsence()()).toBe(false);

    const work = vi.fn();
    whenAthleteReturns(work);
    expect(work).toHaveBeenCalledTimes(1);
  });

  it('aborts only the stretch it was asked to, and resets to a live signal', () => {
    const first = idleSignal();
    idleAbort();
    expect(first.aborted).toBe(true);
    resetIdleAbort();
    const second = idleSignal();
    expect(second).not.toBe(first);
    expect(second.aborted).toBe(false);
    // Resetting a live stretch keeps it: turns already open are not orphaned.
    resetIdleAbort();
    expect(idleSignal()).toBe(second);
  });
});
