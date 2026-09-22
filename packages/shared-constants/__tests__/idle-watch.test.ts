// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The idle contract both clients obey, driven with fake timers and no platform at all
// ABOUTME: Hidden pauses polls at once; only the idle deadline drops a stream; a return runs queued work

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { IDLE_STOP_AFTER_MS, IdleWatch } from '../src/query-policy';

/** Every callback the watch made, in the order it made them. */
type Event = 'idle' | 'suspend' | 'active';

function watchWithLog(): { watch: IdleWatch; events: Event[] } {
  const events: Event[] = [];
  const watch = new IdleWatch({
    onIdle: () => events.push('idle'),
    onSuspend: () => events.push('suspend'),
    onActive: () => events.push('active'),
  });
  return { watch, events };
}

describe('IdleWatch', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it('goes idle after the threshold without an interaction, and back on the next one', () => {
    const { watch, events } = watchWithLog();

    vi.advanceTimersByTime(IDLE_STOP_AFTER_MS - 1);
    expect(events).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(events).toEqual(['idle']);
    expect(watch.isIdle).toBe(true);

    watch.noteInteraction();
    watch.noteInteraction();
    expect(events).toEqual(['idle', 'active']);
    expect(watch.isIdle).toBe(false);
    watch.stop();
  });

  it('pauses the moment it is hidden, and a return inside the threshold never goes idle', () => {
    const { watch, events } = watchWithLog();

    watch.suspend();
    expect(events).toEqual(['suspend']);
    expect(watch.isHidden).toBe(true);
    expect(watch.isIdle).toBe(false);

    // A minute on the Strava tab, then back.
    vi.advanceTimersByTime(60_000);
    watch.resume();
    expect(events).toEqual(['suspend', 'active']);
    expect(watch.isHidden).toBe(false);

    // The return is an interaction: a full threshold again from here.
    vi.advanceTimersByTime(IDLE_STOP_AFTER_MS - 1);
    expect(events).toEqual(['suspend', 'active']);
    vi.advanceTimersByTime(1);
    expect(events).toEqual(['suspend', 'active', 'idle']);
    watch.stop();
  });

  it('goes idle once hidden past the deadline its last interaction set', () => {
    const { watch, events } = watchWithLog();

    vi.advanceTimersByTime(60_000);
    watch.suspend();
    // Hiding does not restart the clock an interaction started.
    vi.advanceTimersByTime(IDLE_STOP_AFTER_MS - 60_000 - 1);
    expect(events).toEqual(['suspend']);
    vi.advanceTimersByTime(1);
    expect(events).toEqual(['suspend', 'idle']);

    watch.resume();
    expect(events).toEqual(['suspend', 'idle', 'active']);
    expect(watch.isIdle).toBe(false);
    watch.stop();
  });

  it('holds a visible client active for as long as a turn runs', () => {
    const { watch, events } = watchWithLog();
    const release = watch.holdWhileBusy();

    vi.advanceTimersByTime(IDLE_STOP_AFTER_MS * 3);
    expect(events).toEqual([]);

    // The threshold measures the quiet after the turn, not during it.
    release();
    vi.advanceTimersByTime(IDLE_STOP_AFTER_MS - 1);
    expect(events).toEqual([]);
    vi.advanceTimersByTime(1);
    expect(events).toEqual(['idle']);
    watch.stop();
  });

  it('lets a held turn outlive a hidden stretch, and drops it only a full threshold after hiding', () => {
    const { watch, events } = watchWithLog();
    watch.holdWhileBusy();

    // The athlete watched the turn for ten minutes, then looked away.
    vi.advanceTimersByTime(IDLE_STOP_AFTER_MS * 2);
    watch.suspend();
    expect(events).toEqual(['suspend']);

    vi.advanceTimersByTime(IDLE_STOP_AFTER_MS - 1);
    expect(events).toEqual(['suspend']);
    vi.advanceTimersByTime(1);
    expect(events).toEqual(['suspend', 'idle']);
    watch.stop();
  });

  it('ignores interactions while hidden — only resume ends a hidden stretch', () => {
    const { watch, events } = watchWithLog();
    watch.suspend();

    // A reply scrolling itself into view in a background tab is not a human.
    watch.noteInteraction();
    expect(watch.isHidden).toBe(true);
    expect(events).toEqual(['suspend']);

    vi.advanceTimersByTime(IDLE_STOP_AFTER_MS);
    expect(events).toEqual(['suspend', 'idle']);
    watch.noteInteraction();
    expect(events).toEqual(['suspend', 'idle']);
    watch.stop();
  });

  it('counts each departure once: hidden, or idle while visible', () => {
    const { watch } = watchWithLog();
    expect(watch.absences).toBe(0);

    watch.suspend();
    expect(watch.absences).toBe(1);
    // Idle while already hidden is the same absence, not a second one.
    vi.advanceTimersByTime(IDLE_STOP_AFTER_MS);
    expect(watch.absences).toBe(1);

    watch.resume();
    expect(watch.absences).toBe(1);
    vi.advanceTimersByTime(IDLE_STOP_AFTER_MS);
    expect(watch.absences).toBe(2);
    watch.stop();
  });

  it('runs present work at once and queues absent work for the return, after onActive', () => {
    const order: string[] = [];
    const watch = new IdleWatch({
      onIdle: () => order.push('idle'),
      onSuspend: () => order.push('suspend'),
      onActive: () => order.push('active'),
    });

    watch.whenPresent(() => order.push('now'));
    expect(order).toEqual(['now']);

    watch.suspend();
    watch.whenPresent(() => order.push('re-read'));
    expect(order).toEqual(['now', 'suspend']);

    watch.resume();
    expect(order).toEqual(['now', 'suspend', 'active', 're-read']);

    // Queued work runs once, on the first return only.
    watch.suspend();
    watch.resume();
    expect(order).toEqual(['now', 'suspend', 'active', 're-read', 'suspend', 'active']);

    // An idle client queues too, and an interaction is the return.
    vi.advanceTimersByTime(IDLE_STOP_AFTER_MS);
    watch.whenPresent(() => order.push('after idle'));
    watch.noteInteraction();
    expect(order.slice(-3)).toEqual(['idle', 'active', 'after idle']);
    watch.stop();
  });

  it('drops queued work when torn down, and reports nothing further', () => {
    const { watch, events } = watchWithLog();
    const work = vi.fn();
    watch.suspend();
    watch.whenPresent(work);

    watch.stop();
    watch.resume();
    vi.advanceTimersByTime(IDLE_STOP_AFTER_MS * 2);
    expect(work).not.toHaveBeenCalled();
    expect(events).toEqual(['suspend']);
  });
});
