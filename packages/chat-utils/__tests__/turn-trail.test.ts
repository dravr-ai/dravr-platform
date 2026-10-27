// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the turn trail both chat clients draw while a long turn runs and fold when it ends
// ABOUTME: Pins the tool → step mapping, the between-tools phase, the thread-level trails, the timer, the fold and the awaited-reply settle

import { describe, it, expect } from 'vitest';
import type { MessageRole, TurnProgress } from '@pierre/shared-types';
import {
  EMPTY_TURN_TRAILS,
  TURN_ELAPSED_AFTER_MS,
  TURN_SAFE_TO_LEAVE_AFTER_MS,
  TURN_STEP_KEYS,
  TURN_TRAIL_KEYS,
  awaitedSince,
  foldTurnTrail,
  formatTurnDuration,
  reduceTurnTrail,
  reduceTurnTrails,
  runningStep,
  startTurnTrail,
  toolStepLabel,
  turnCurrentLine,
  turnElapsedMs,
  turnIndicator,
  type TurnTrail,
  type TurnTrails,
} from '../src/turn-trail';

const T0 = 1_000_000;

function tool(id: string, title: string, status: string): TurnProgress {
  return { kind: 'tool', id, title, status };
}

function stage(title: string, status = 'started'): TurnProgress {
  return { kind: 'stage', id: title, title, status };
}

function feed(trail: TurnTrail, ...frames: TurnProgress[]): TurnTrail {
  return frames.reduce((acc, progress) => reduceTurnTrail(acc, { type: 'progress', progress }), trail);
}

describe('toolStepLabel', () => {
  it('names the plan-building tools in the coach’s words, never by their own name', () => {
    expect(toolStepLabel('get_group_member_activities')).toEqual({ key: TURN_STEP_KEYS.readMemberActivities });
    expect(toolStepLabel('recommend_plan_flavour')).toEqual({ key: TURN_STEP_KEYS.choosePlanApproach });
    expect(toolStepLabel('save_training_plan')).toEqual({ key: TURN_STEP_KEYS.savePlan });
    expect(toolStepLabel('get_training_plan')).toEqual({ key: TURN_STEP_KEYS.readPlan });
    expect(toolStepLabel('list_workout_templates')).toEqual({ key: TURN_STEP_KEYS.pickWorkouts });
  });

  it('finds a known tool inside a runner-qualified title, the most specific one first', () => {
    expect(toolStepLabel('pierre__get_group_member_activities')).toEqual({
      key: TURN_STEP_KEYS.readMemberActivities,
    });
    expect(toolStepLabel('mcp/get_activities')).toEqual({ key: TURN_STEP_KEYS.readActivities });
  });

  it('falls back to a generic step for a tool it does not know', () => {
    expect(toolStepLabel('frobnicate_widgets')).toEqual({ key: TURN_STEP_KEYS.lookingUp });
    expect(toolStepLabel('')).toEqual({ key: TURN_STEP_KEYS.lookingUp });
  });
});

describe('reduceTurnTrail', () => {
  it('starts thinking with no steps', () => {
    const trail = startTurnTrail(T0);
    expect(trail).toEqual({ startedAt: T0, steps: [], phase: 'thinking', finishedAt: null });
    expect(turnCurrentLine(trail)).toEqual({ key: TURN_STEP_KEYS.thinking });
  });

  it('reads the question during prompt assembly, then thinks once dispatched', () => {
    const reading = feed(startTurnTrail(T0), stage('prompt_assembly'));
    expect(turnCurrentLine(reading)).toEqual({ key: TURN_STEP_KEYS.readingQuestion });
    const dispatched = feed(reading, stage('prompt_assembly', 'finished'), stage('dispatch'));
    expect(turnCurrentLine(dispatched)).toEqual({ key: TURN_STEP_KEYS.thinking });
  });

  it('accumulates one step per call id and checks it off whatever way it ended', () => {
    const trail = feed(
      startTurnTrail(T0),
      tool('c1', 'get_training_plan', 'Pending'),
      tool('c1', 'get_training_plan', 'InProgress'),
    );
    expect(trail.steps).toEqual([{ id: 'c1', label: { key: TURN_STEP_KEYS.readPlan }, done: false }]);
    expect(turnCurrentLine(trail)).toEqual({ key: TURN_STEP_KEYS.readPlan });

    const failed = feed(trail, tool('c1', 'get_training_plan', 'Failed'));
    expect(failed.steps[0].done).toBe(true);
    expect(runningStep(failed)).toBeNull();
  });

  it('takes a call that never ran off the trail instead of checking it off', () => {
    const trail = feed(
      startTurnTrail(T0),
      tool('c1', 'get_training_plan', 'InProgress'),
      tool('c1', 'get_training_plan', 'Completed'),
      tool('c2', 'save_training_plan', 'InProgress'),
    );
    expect(turnCurrentLine(trail)).toEqual({ key: TURN_STEP_KEYS.savePlan });

    const withdrawn = feed(trail, tool('c2', 'save_training_plan', 'NotRun'));
    expect(withdrawn.steps).toEqual([{ id: 'c1', label: { key: TURN_STEP_KEYS.readPlan }, done: true }]);
    expect(runningStep(withdrawn)).toBeNull();

    const folded = foldTurnTrail(
      reduceTurnTrail(withdrawn, { type: 'finished', at: T0 + 2_000, toolsCalled: ['get_training_plan'] }),
    );
    expect(folded?.steps.map(step => step.id)).toEqual(['c1']);
  });

  it('ignores a never-ran frame for a call it never saw start', () => {
    const trail = feed(startTurnTrail(T0), tool('c1', 'get_stats', 'Completed'));
    expect(feed(trail, tool('c9', 'save_training_plan', 'NotRun'))).toBe(trail);
  });

  it('reads as analysing between tools, never back to the first line', () => {
    const trail = feed(
      startTurnTrail(T0),
      stage('prompt_assembly'),
      tool('c1', 'get_group_member_activities', 'InProgress'),
      tool('c1', 'get_group_member_activities', 'Completed'),
    );
    expect(turnCurrentLine(trail)).toEqual({ key: TURN_STEP_KEYS.analysing });

    const next = feed(trail, tool('c2', 'recommend_plan_flavour', 'InProgress'));
    expect(next.steps.map(s => s.done)).toEqual([true, false]);
    expect(turnCurrentLine(next)).toEqual({ key: TURN_STEP_KEYS.choosePlanApproach });
  });

  it('says it is writing once the reply’s words arrive', () => {
    const trail = reduceTurnTrail(feed(startTurnTrail(T0), tool('c1', 'get_stats', 'Completed')), {
      type: 'prose',
    });
    expect(turnCurrentLine(trail)).toEqual({ key: TURN_STEP_KEYS.writing });
  });

  it('thinks again when the words it was writing are taken back', () => {
    const writing = reduceTurnTrail(feed(startTurnTrail(T0), tool('c1', 'get_stats', 'Completed')), {
      type: 'prose',
    });
    const retracted = reduceTurnTrail(writing, { type: 'retract' });
    expect(retracted.phase).toBe('thinking');
    expect(retracted.steps).toEqual(writing.steps);
    expect(turnCurrentLine(retracted)).not.toEqual({ key: TURN_STEP_KEYS.writing });
    // The words that follow mark it writing again.
    expect(turnCurrentLine(reduceTurnTrail(retracted, { type: 'prose' }))).toEqual({
      key: TURN_STEP_KEYS.writing,
    });
  });

  it('ignores a retraction while it is not writing', () => {
    const thinking = feed(startTurnTrail(T0), tool('c1', 'get_stats', 'Completed'));
    expect(reduceTurnTrail(thinking, { type: 'retract' })).toBe(thinking);
  });

  it('closes every step when the turn ends and adds the calls no frame announced', () => {
    const running = feed(startTurnTrail(T0), tool('c1', 'get_training_plan', 'InProgress'));
    const done = reduceTurnTrail(running, {
      type: 'finished',
      at: T0 + 5_000,
      toolsCalled: ['get_training_plan', 'save_training_plan'],
    });
    expect(done.finishedAt).toBe(T0 + 5_000);
    expect(done.steps).toEqual([
      { id: 'c1', label: { key: TURN_STEP_KEYS.readPlan }, done: true },
      { id: 'called-1', label: { key: TURN_STEP_KEYS.savePlan }, done: true },
    ]);
    // A finished trail takes no more events.
    expect(feed(done, tool('c9', 'get_stats', 'InProgress'))).toBe(done);
  });

  it('returns the same trail for a frame that changes nothing', () => {
    const trail = feed(startTurnTrail(T0), tool('c1', 'get_stats', 'InProgress'));
    expect(feed(trail, tool('c1', 'get_stats', 'Pending'))).toBe(trail);
    expect(feed(trail, stage('dispatch', 'finished'))).toBe(trail);
  });
});

describe('the indicator clock', () => {
  it('formats seconds, minutes and hours as catalogue keys', () => {
    expect(formatTurnDuration(14_900)).toEqual({ key: TURN_TRAIL_KEYS.elapsedSeconds, params: { seconds: 14 } });
    expect(formatTurnDuration(134_000)).toEqual({
      key: TURN_TRAIL_KEYS.elapsedMinutes,
      params: { minutes: 2, seconds: 14 },
    });
    expect(formatTurnDuration(3_900_000)).toEqual({
      key: TURN_TRAIL_KEYS.elapsedHours,
      params: { hours: 1, minutes: 5 },
    });
    expect(formatTurnDuration(-5)).toEqual({ key: TURN_TRAIL_KEYS.elapsedSeconds, params: { seconds: 0 } });
  });

  it('shows the timer after ten seconds and the safe-to-leave note after forty-five', () => {
    expect(turnIndicator(TURN_ELAPSED_AFTER_MS - 1)).toEqual({ elapsed: null, safeToLeave: null });
    expect(turnIndicator(TURN_ELAPSED_AFTER_MS).elapsed).toEqual({
      key: TURN_TRAIL_KEYS.elapsedSeconds,
      params: { seconds: 10 },
    });
    expect(turnIndicator(TURN_SAFE_TO_LEAVE_AFTER_MS - 1).safeToLeave).toBeNull();
    expect(turnIndicator(TURN_SAFE_TO_LEAVE_AFTER_MS).safeToLeave).toEqual({ key: TURN_TRAIL_KEYS.safeToLeave });
  });

  it('never runs backwards when the server clock is ahead', () => {
    expect(turnElapsedMs(T0 + 3_000, T0)).toBe(0);
    expect(turnElapsedMs(T0, T0 + 3_000)).toBe(3_000);
  });
});

describe('foldTurnTrail', () => {
  function finished(steps: number, tookMs: number): TurnTrail {
    let trail = startTurnTrail(T0);
    for (let i = 0; i < steps; i += 1) trail = feed(trail, tool(`c${i}`, 'get_stats', 'Completed'));
    return reduceTurnTrail(trail, { type: 'finished', at: T0 + tookMs });
  }

  it('folds nothing while the turn runs, or for a quick answer with no tool', () => {
    expect(foldTurnTrail(startTurnTrail(T0))).toBeNull();
    expect(foldTurnTrail(finished(0, 3_000))).toBeNull();
  });

  it('folds a long turn into its duration and step count', () => {
    expect(foldTurnTrail(finished(6, 420_000))).toMatchObject({
      line: { key: TURN_TRAIL_KEYS.foldedN, params: { count: 6 } },
      duration: { key: TURN_TRAIL_KEYS.elapsedMinutes, params: { minutes: 7, seconds: 0 } },
    });
    expect(foldTurnTrail(finished(1, 2_000))?.line).toEqual({
      key: TURN_TRAIL_KEYS.foldedOne,
      params: { count: 1 },
    });
    expect(foldTurnTrail(finished(0, 30_000))?.line.key).toBe(TURN_TRAIL_KEYS.foldedNoSteps);
  });
});

describe('awaitedSince', () => {
  const row = (id: string, role: MessageRole) => ({ id, role });
  const AWAITING = { question_id: 'q', asked_at: '2026-10-26T10:00:00Z' };

  it('is nothing when the server awaits nothing', () => {
    expect(awaitedSince(null, [])).toBeNull();
    expect(awaitedSince(undefined, [])).toBeNull();
  });

  it('starts the clock at the question while its reply has not landed', () => {
    expect(awaitedSince(AWAITING, [row('a', 'assistant'), row('q', 'user')])).toBe(
      Date.parse('2026-10-26T10:00:00Z'),
    );
  });

  it('is settled by a reply after the question even while the flag is stale', () => {
    expect(awaitedSince(AWAITING, [row('q', 'user'), row('r', 'assistant')])).toBeNull();
  });

  it('ignores a malformed timestamp', () => {
    expect(awaitedSince({ question_id: 'q', asked_at: 'yesterday' }, [row('q', 'user')])).toBeNull();
  });
});

describe('reduceTurnTrails', () => {
  const progress = (frame: TurnProgress) => ({ type: 'event' as const, event: { type: 'progress' as const, progress: frame } });

  function running(): TurnTrails {
    const begun = reduceTurnTrails(EMPTY_TURN_TRAILS, { type: 'begin', at: T0 });
    return reduceTurnTrails(begun, progress(tool('call-1', 'get_activities', 'InProgress')));
  }

  it('starts a live trail on begin, and feeds it the turn\'s frames', () => {
    const state = running();
    expect(state.live?.startedAt).toBe(T0);
    expect(state.live?.steps.map(step => [step.label.key, step.done])).toEqual([
      [TURN_STEP_KEYS.readActivities, false],
    ]);
    expect(state.finished.size).toBe(0);
  });

  it('changes nothing — the same state — for a frame or a finish with no turn running', () => {
    expect(reduceTurnTrails(EMPTY_TURN_TRAILS, progress(tool('c', 'get_activities', 'InProgress')))).toBe(
      EMPTY_TURN_TRAILS,
    );
    expect(reduceTurnTrails(EMPTY_TURN_TRAILS, { type: 'finish', at: T0, replyId: 'reply-1' })).toBe(
      EMPTY_TURN_TRAILS,
    );
  });

  it('returns the same state for a frame the live trail ignores', () => {
    const state = running();
    expect(reduceTurnTrails(state, progress(stage('dispatch', 'finished')))).toBe(state);
  });

  it('folds the finished trail under its reply, steps closed and unannounced tools added', () => {
    const state = reduceTurnTrails(running(), {
      type: 'finish',
      at: T0 + 5_000,
      replyId: 'reply-1',
      toolsCalled: ['get_activities', 'save_training_plan'],
    });
    expect(state.live).toBeNull();
    const folded = state.finished.get('reply-1');
    expect(folded?.finishedAt).toBe(T0 + 5_000);
    expect(folded?.steps.map(step => [step.label.key, step.done])).toEqual([
      [TURN_STEP_KEYS.readActivities, true],
      [TURN_STEP_KEYS.savePlan, true],
    ]);
  });

  it('keeps the trails of earlier replies when a later one folds', () => {
    const first = reduceTurnTrails(running(), { type: 'finish', at: T0 + 1, replyId: 'reply-1' });
    const second = reduceTurnTrails(
      reduceTurnTrails(first, { type: 'begin', at: T0 + 2 }),
      { type: 'finish', at: T0 + 3, replyId: 'reply-2' },
    );
    expect([...second.finished.keys()]).toEqual(['reply-1', 'reply-2']);
    // The earlier map is not mutated.
    expect([...first.finished.keys()]).toEqual(['reply-1']);
  });

  it('drops the live trail when the turn ends without a reply', () => {
    const state = reduceTurnTrails(running(), { type: 'finish', at: T0 + 1, replyId: null });
    expect(state.live).toBeNull();
    expect(state.finished.size).toBe(0);
  });

  it('forgets every trail on reset, and is a no-op on a thread that holds none', () => {
    const folded = reduceTurnTrails(running(), { type: 'finish', at: T0 + 1, replyId: 'reply-1' });
    const live = reduceTurnTrails(folded, { type: 'begin', at: T0 + 2 });
    expect(reduceTurnTrails(live, { type: 'reset' })).toBe(EMPTY_TURN_TRAILS);
    expect(reduceTurnTrails(EMPTY_TURN_TRAILS, { type: 'reset' })).toBe(EMPTY_TURN_TRAILS);
  });
});
