// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves the athlete workspace's plan state both clients draw — one case per state, and which wins when two hold at once
// ABOUTME: Also the agent's name the workspace's tabs and history read, its brand fallback included

import { describe, it, expect, vi } from 'vitest';
import type { CoachRosterAthlete, SubjectPlanResponse, WorkoutPlan } from '@pierre/shared-types';
import {
  workspaceAgentName,
  workspacePlanState,
  type WorkspacePlanRead,
  type WorkspaceThreads,
} from '../src/workspaceHooks';

const PLAN_ID = 'conv-plan-lea';
const IN_A_GROUP: Pick<CoachRosterAthlete, 'groups'> = { groups: [{ id: 'g-lea', name: 'TRAKA' }] };
const NO_GROUP: Pick<CoachRosterAthlete, 'groups'> = { groups: [] };

const PLAN: WorkoutPlan = {
  goal_race: { name: 'The Traka 200', date: '2027-04-30', discipline: 'gravel', priority: 'A' },
  phases: [],
  weeks: [],
  weeks_deferred: 0,
};
const DRAFT: WorkoutPlan = { ...PLAN, goal_race: { ...PLAN.goal_race, name: 'The Traka 360' } };

function subject(live: WorkoutPlan | null, draft: WorkoutPlan | null): SubjectPlanResponse {
  return {
    athlete: { user_id: 'u-lea', display_name: 'Léa Gagnon' },
    today: '2026-11-25',
    live,
    draft,
    draft_state: draft ? { draft_id: 'draft-1', updated_at: '2026-11-25T10:05:00Z', conflicts: [], season_relaid: false } : null,
    proposed_dates: [],
  };
}

type ThreadsSeen = Pick<WorkspaceThreads, 'planThreadId' | 'planError' | 'retryPlanThread' | 'groupsFailed' | 'retryGroups'>;

function threads(over: Partial<ThreadsSeen> = {}): ThreadsSeen {
  return { planThreadId: PLAN_ID, planError: null, retryPlanThread: vi.fn(), groupsFailed: false, retryGroups: vi.fn(), ...over };
}

function read(over: Partial<WorkspacePlanRead> = {}): WorkspacePlanRead {
  return { read: { status: 'ok', plan: subject(PLAN, null) }, isLoading: false, isError: false, refetch: vi.fn(), ...over };
}

describe('workspacePlanState — one case per state', () => {
  it('has no plan thread to work through for an athlete in no group of the coach’s', () => {
    expect(workspacePlanState(NO_GROUP, threads({ planThreadId: null }), read({ read: undefined }))).toEqual({ kind: 'noGroup' });
  });

  it('says the plan thread was refused, with its refusal and its retry', () => {
    const refusal = { response: { status: 403 } };
    const seen = threads({ planThreadId: null, planError: refusal });
    const state = workspacePlanState(IN_A_GROUP, seen, read({ read: undefined }));

    expect(state).toEqual({ kind: 'threadFailed', error: refusal, retry: seen.retryPlanThread });
  });

  it('offers to read the groups again when one failed before the plan thread was known', () => {
    const seen = threads({ planThreadId: null, groupsFailed: true });
    const state = workspacePlanState(IN_A_GROUP, seen, read({ read: undefined }));

    expect(state).toEqual({ kind: 'groupsFailed', retry: seen.retryGroups });
  });

  it('waits while the plan thread is found or opened, and while its plan is read', () => {
    expect(workspacePlanState(IN_A_GROUP, threads({ planThreadId: null }), read({ read: undefined }))).toEqual({ kind: 'loading' });
    expect(workspacePlanState(IN_A_GROUP, threads(), read({ read: undefined, isLoading: true }))).toEqual({ kind: 'loading' });
  });

  it('says the athlete stopped sharing', () => {
    expect(workspacePlanState(IN_A_GROUP, threads(), read({ read: { status: 'consentRevoked' } }))).toEqual({
      kind: 'consentRevoked',
    });
  });

  it('says the plan could not be read, and reads it again on retry', () => {
    const refetch = vi.fn();
    const state = workspacePlanState(IN_A_GROUP, threads(), read({ read: undefined, isError: true, refetch }));

    expect(state.kind).toBe('loadFailed');
    if (state.kind === 'loadFailed') state.retry();
    expect(refetch).toHaveBeenCalledTimes(1);
  });

  it('reads as failed when the read answered nothing at all', () => {
    expect(workspacePlanState(IN_A_GROUP, threads(), read({ read: undefined })).kind).toBe('loadFailed');
  });

  it('says there is no plan yet, live or drafted', () => {
    const empty = subject(null, null);
    expect(workspacePlanState(IN_A_GROUP, threads(), read({ read: { status: 'ok', plan: empty } }))).toEqual({
      kind: 'noPlan',
      planThreadId: PLAN_ID,
      subject: empty,
    });
  });

  it('shows the draft when one is open, else the live plan', () => {
    const drafted = subject(PLAN, DRAFT);
    expect(workspacePlanState(IN_A_GROUP, threads(), read({ read: { status: 'ok', plan: drafted } }))).toEqual({
      kind: 'plan',
      planThreadId: PLAN_ID,
      subject: drafted,
      shown: DRAFT,
    });
    const live = subject(PLAN, null);
    expect(workspacePlanState(IN_A_GROUP, threads(), read({ read: { status: 'ok', plan: live } }))).toEqual({
      kind: 'plan',
      planThreadId: PLAN_ID,
      subject: live,
      shown: PLAN,
    });
    // A draft with no live plan under it is still the plan shown.
    const first = subject(null, DRAFT);
    const state = workspacePlanState(IN_A_GROUP, threads(), read({ read: { status: 'ok', plan: first } }));
    expect(state.kind === 'plan' && state.shown).toBe(DRAFT);
  });
});

describe('workspacePlanState — the order between states that hold at once', () => {
  it('puts no group before a refused plan thread and a failed group read', () => {
    const seen = threads({ planThreadId: null, planError: new Error('refused'), groupsFailed: true });
    expect(workspacePlanState(NO_GROUP, seen, read()).kind).toBe('noGroup');
  });

  it('puts a refused plan thread before a failed group read', () => {
    const seen = threads({ planThreadId: null, planError: new Error('refused'), groupsFailed: true });
    expect(workspacePlanState(IN_A_GROUP, seen, read()).kind).toBe('threadFailed');
  });

  it('lets a known plan thread go on to its plan though a group read failed since', () => {
    expect(workspacePlanState(IN_A_GROUP, threads({ groupsFailed: true }), read()).kind).toBe('plan');
  });

  it('puts a failed group read before the wait for a plan thread', () => {
    expect(workspacePlanState(IN_A_GROUP, threads({ planThreadId: null, groupsFailed: true }), read({ isLoading: true })).kind).toBe(
      'groupsFailed',
    );
  });

  it('waits on a plan still loading before naming anything the last read said', () => {
    const revoked = read({ read: { status: 'consentRevoked' }, isLoading: true, isError: true });
    expect(workspacePlanState(IN_A_GROUP, threads(), revoked).kind).toBe('loading');
  });

  it('puts a revoked sharing before a failed read', () => {
    expect(workspacePlanState(IN_A_GROUP, threads(), read({ read: { status: 'consentRevoked' }, isError: true })).kind).toBe(
      'consentRevoked',
    );
  });

  it('puts a failed read before the plan a previous read left', () => {
    expect(workspacePlanState(IN_A_GROUP, threads(), read({ isError: true })).kind).toBe('loadFailed');
  });
});

describe('workspaceAgentName', () => {
  it('names the plan group’s agent, trimmed', () => {
    expect(workspaceAgentName({ agent_title: '  Coach Max ' }, 'Dravr')).toBe('Coach Max');
  });

  it('falls back to the brand while the group is read or names no agent', () => {
    expect(workspaceAgentName(null, 'Dravr')).toBe('Dravr');
    expect(workspaceAgentName({ agent_title: null }, 'Dravr')).toBe('Dravr');
    expect(workspaceAgentName({ agent_title: '   ' }, 'Dravr')).toBe('Dravr');
  });
});
