// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Unit tests for a hand-written session's structure — sets under one ×N, moves that never split a set, time or distance, chip targets (carnet#1727)
// ABOUTME: And the sport chips: the one-tap sports, the day's own word, and which sport words open the family mode

import { describe, expect, it } from 'vitest';
import type { PlanStep } from '@pierre/shared-types';
import {
  appendSeries,
  appendStep,
  appendToSeries,
  duplicateStep,
  firstStepGap,
  moveBlockAt,
  moveStepAt,
  numberedSteps,
  parseTarget,
  percentTarget,
  profileBlocks,
  removeStepAt,
  rpeTarget,
  setSeriesRepeat,
  setStepMeasure,
  stepBlocks,
  stepMoveTarget,
  stepsForSave,
  stepsTotalSeconds,
  targetLevel,
  targetSpan,
  zoneTarget,
} from '../src/sessionSteps';
import { chosenSport, familySportFor, quickSportOf, sameSport, sportChoices } from '../src/sessionSports';

const step = (label: string, seconds: number, target: string, extra: Partial<PlanStep> = {}): PlanStep => ({
  label,
  duration_seconds: seconds,
  target_zone: target,
  ...extra,
});

/** 15' warm-up, 5 × (3' Z4 + 2' Z1), 10' cool-down — the providers' own grouping, no set numbers yet. */
const fartlek: PlanStep[] = [
  step('Warm-up', 900, 'Z1'),
  step('Effort', 180, 'Z4', { repeat: 5 }),
  step('Recovery', 120, 'Z1', { repeat: 5 }),
  step('Cool-down', 600, 'Z1'),
];

const labels = (steps: PlanStep[]) => steps.map((s) => s.label);

const defaults = {
  work: { label: 'Effort', target: 'Z4' },
  recovery: { label: 'Recovery', target: 'Z1' },
};

describe('sets of steps', () => {
  it('reads adjacent steps repeating the same number of times as one set, as the calendar push does', () => {
    expect(stepBlocks(fartlek)).toEqual([
      { kind: 'step', index: 0 },
      { kind: 'series', start: 1, end: 3, repeat: 5 },
      { kind: 'step', index: 3 },
    ]);
  });

  it('keeps apart two adjacent sets with the same repeat once they are numbered', () => {
    const two = [
      step('A', 60, 'Z5', { repeat: 3, repeat_group: 1 }),
      step('B', 60, 'Z5', { repeat: 3, repeat_group: 2 }),
    ];
    expect(stepBlocks(two)).toHaveLength(2);
    expect(stepBlocks(two.map(({ repeat_group: _, ...rest }) => rest))).toHaveLength(1);
  });

  it('numbers every set in order and strips the repeat off lone steps', () => {
    const numbered = numberedSteps([step('Lone', 60, 'Z2', { repeat: 1, repeat_group: 7 }), ...fartlek.slice(1, 3)]);
    expect(numbered[0]).toEqual(step('Lone', 60, 'Z2'));
    expect(numbered.slice(1).map((s) => [s.repeat, s.repeat_group])).toEqual([
      [5, 1],
      [5, 1],
    ]);
  });

  it('adds a set of an effort and a recovery, five times, in a set of its own', () => {
    const added = appendSeries(fartlek, defaults);
    expect(labels(added).slice(-2)).toEqual(['Effort', 'Recovery']);
    expect(added.slice(-2).map((s) => [s.repeat, s.repeat_group, s.target_zone])).toEqual([
      [5, 2, 'Z4'],
      [5, 2, 'Z1'],
    ]);
    expect(stepBlocks(added)).toHaveLength(4);
  });

  it('adds a step at the end of a set, repeating with it, and a lone step at the end of the session', () => {
    const inSet = appendToSeries(fartlek, 1);
    expect(labels(inSet)).toEqual(['Warm-up', 'Effort', 'Recovery', '', 'Cool-down']);
    expect(stepBlocks(inSet)[1]).toEqual({ kind: 'series', start: 1, end: 4, repeat: 5 });
    expect(appendStep(fartlek).at(-1)).toEqual({ label: '', duration_seconds: 300, target_zone: '' });
    expect(appendToSeries(fartlek, 0)).toEqual(numberedSteps(fartlek));
  });

  it('changes a set’s count for all its steps, bounded, and at one leaves its steps alone', () => {
    expect(setSeriesRepeat(fartlek, 1, 8).slice(1, 3).map((s) => s.repeat)).toEqual([8, 8]);
    expect(setSeriesRepeat(fartlek, 1, 500)[1]?.repeat).toBe(50);
    const dissolved = setSeriesRepeat(fartlek, 1, 1);
    expect(dissolved[1]).toEqual(step('Effort', 180, 'Z4'));
    expect(stepBlocks(dissolved).every((block) => block.kind === 'step')).toBe(true);
  });

  it('duplicates a step into its own set, and removes one', () => {
    const copied = duplicateStep(fartlek, 1);
    expect(labels(copied)).toEqual(['Warm-up', 'Effort', 'Effort', 'Recovery', 'Cool-down']);
    expect(stepBlocks(copied)[1]).toMatchObject({ kind: 'series', start: 1, end: 4 });
    expect(labels(removeStepAt(fartlek, 2))).toEqual(['Warm-up', 'Effort', 'Cool-down']);
  });

  it('moves a step inside its set, and a lone step past a whole set, never splitting one', () => {
    expect(labels(moveStepAt(fartlek, 2, -1))).toEqual(['Warm-up', 'Recovery', 'Effort', 'Cool-down']);
    expect(labels(moveStepAt(fartlek, 1, -1))).toEqual(labels(fartlek));
    expect(labels(moveStepAt(fartlek, 3, -1))).toEqual(['Warm-up', 'Cool-down', 'Effort', 'Recovery']);
    expect(labels(moveStepAt(fartlek, 0, 1))).toEqual(['Effort', 'Recovery', 'Warm-up', 'Cool-down']);
    expect(labels(moveBlockAt(fartlek, 1, 1))).toEqual(['Warm-up', 'Cool-down', 'Effort', 'Recovery']);
    expect(labels(moveBlockAt(fartlek, 0, -1))).toEqual(labels(fartlek));
  });

  it('says where a move lands the step, and null where it would change nothing, as the move itself does', () => {
    expect(stepMoveTarget(fartlek, 2, -1)).toBe(1);
    expect(stepMoveTarget(fartlek, 1, -1)).toBeNull();
    expect(stepMoveTarget(fartlek, 3, -1)).toBe(1);
    expect(stepMoveTarget(fartlek, 0, 1)).toBe(2);
    expect(stepMoveTarget(fartlek, 3, 1)).toBeNull();
    for (const index of fartlek.keys()) {
      for (const delta of [-1, 1] as const) {
        const moved = labels(moveStepAt(fartlek, index, delta));
        const to = stepMoveTarget(fartlek, index, delta);
        if (to === null) expect(moved).toEqual(labels(fartlek));
        else expect(moved[to]).toBe(fartlek[index]?.label);
      }
    }
  });

  it('totals every step’s time times its repeats, the length the server writes', () => {
    expect(stepsTotalSeconds(fartlek)).toBe(900 + 5 * 300 + 600);
  });
});

describe('time or distance', () => {
  it('keeps a time beside a distance, and drops the distance going back to time', () => {
    const byDistance = setStepMeasure(fartlek, 1, 'distance');
    expect(byDistance[1]).toMatchObject({ duration_seconds: 180, distance_meters: 0 });
    expect(setStepMeasure(byDistance, 1, 'time')[1]).not.toHaveProperty('distance_meters');
  });

  it('names the first step that lacks a name, a time, its distance or a target', () => {
    expect(firstStepGap(fartlek)).toBeNull();
    expect(firstStepGap([step('', 60, 'Z1')])).toEqual({ kind: 'label', number: 1 });
    expect(firstStepGap([step('A', 60, 'Z1'), step('B', 0, 'Z1')])).toEqual({ kind: 'duration', number: 2 });
    expect(firstStepGap([step('A', 60, 'Z1', { distance_meters: 0 })])).toEqual({ kind: 'distance', number: 1 });
    expect(firstStepGap([step('A', 60, ' ')])).toEqual({ kind: 'target', number: 1 });
  });

  it('sends names and targets trimmed and sets numbered', () => {
    expect(stepsForSave([step(' A ', 60, ' Z2 ', { repeat: 3 })])).toEqual([
      step('A', 60, 'Z2', { repeat: 3, repeat_group: 1 }),
    ]);
  });
});

describe('targets the chips write', () => {
  it('writes only spellings the server grammar reads', () => {
    expect(zoneTarget(4)).toBe('Z4');
    expect(zoneTarget(9)).toBe('Z7');
    expect(percentTarget(93, 88, 'ftp')).toBe('88-93% FTP');
    expect(percentTarget(75, 75, 'threshold_hr')).toBe('75% threshold HR');
    expect(percentTarget(90, 95, 'threshold_pace')).toBe('90-95% threshold pace');
    expect(percentTarget(75, 75, null)).toBe('75%');
    expect(rpeTarget(7)).toBe('RPE 7');
    expect(rpeTarget(7, 6)).toBe('RPE 6-7');
    expect(rpeTarget(0, 12)).toBe('RPE 1-10');
  });

  it('reads back its own spellings as chips and leaves every other target as text', () => {
    expect(parseTarget('z3')).toEqual({ kind: 'zone', zone: 3 });
    expect(parseTarget('88-93% FTP')).toEqual({ kind: 'percent', low: 88, high: 93, anchor: 'ftp' });
    expect(parseTarget('75%')).toEqual({ kind: 'percent', low: 75, high: 75, anchor: null });
    expect(parseTarget('RPE 6-7')).toEqual({ kind: 'rpe', low: 6, high: 7 });
    expect(parseTarget('sweet spot')).toEqual({ kind: 'typed', text: 'sweet spot' });
    expect(parseTarget('  ')).toEqual({ kind: 'none' });
  });

  it('spans the targets for the header, only when they are all zones or all RPE', () => {
    expect(targetSpan(fartlek)).toBe('Z1–Z4');
    expect(targetSpan([step('A', 60, 'RPE 3'), step('B', 60, 'RPE 6-8')])).toBe('RPE 3–8');
    expect(targetSpan([step('A', 60, 'Z2'), step('B', 60, 'RPE 6')])).toBeNull();
    expect(targetSpan([step('A', 60, 'tempo')])).toBeNull();
    expect(targetSpan([])).toBeNull();
  });

  it('draws a bar as high as its target, every repetition of a set in turn, the later ones quieter', () => {
    expect(targetLevel('Z7')).toBe(1);
    // A named target is drawn at one neutral height: which zone it reads as is the kernel's to say.
    expect(targetLevel('threshold')).toBe(targetLevel('tempo'));
    expect(targetLevel('threshold')).toBeLessThan(targetLevel('Z4'));
    const blocks = profileBlocks(fartlek);
    expect(blocks).toHaveLength(3);
    const set = blocks[1];
    expect(set?.kind).toBe('series');
    if (set?.kind === 'series') {
      expect(set.bars).toHaveLength(10);
      expect(set.bars.filter((bar) => !bar.repeated).map((bar) => bar.index)).toEqual([1, 2]);
    }
  });
});

describe('the sport chips', () => {
  it('names the same sport by its shared label or its word', () => {
    expect(sameSport('Run', 'run')).toBe(true);
    expect(sameSport('weight_training', 'strength_training')).toBe(true);
    expect(sameSport('', '')).toBe(false);
    expect(quickSportOf('YOGA')).toBe('yoga');
    expect(quickSportOf('Escalade')).toBeNull();
  });

  it("selects the chip a word names, and none for a sport the coach named", () => {
    const choices = sportChoices('run');
    expect(chosenSport('Run', choices)).toBe('run');
    expect(chosenSport('Escalade', choices)).toBeNull();
    expect(chosenSport('Escalade', sportChoices('Escalade'))).toBe('Escalade');
  });

  it('opens the family mode only for a family sport: the chip’s word, or the day’s word as the server read it', () => {
    const day = { sport: 'Course', family_sport: 'run' as const };
    expect(familySportFor('ride', day)).toBe('ride');
    expect(familySportFor('Course', day)).toBe('run');
    expect(familySportFor('strength', day)).toBeUndefined();
    expect(familySportFor('Escalade', { sport: 'Escalade' })).toBeUndefined();
  });
});
