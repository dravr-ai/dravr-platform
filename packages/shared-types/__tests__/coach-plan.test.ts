// ABOUTME: Unit tests for the coach plan-draft wire parsers — the draft state, the subject-plan read, publish and its conflict
// ABOUTME: Feeds each parser the bodies the contract fixes, then the malformed ones it must refuse rather than half-render

import { describe, it, expect } from 'vitest';
import {
  PLAN_PUBLISH_CONFLICT,
  parseAthletePlanResponse,
  parseDraftState,
  parsePlanPublishConflict,
  parsePlanSubject,
  parsePublishedSubjectPlan,
  parseSubjectPlanResponse,
} from '../src/coach-plan';

/** A draft state as the contract serialises it, with one week the athlete changed. */
const draftState = {
  draft_id: 'draft-7',
  updated_at: '2026-10-16T14:05:00Z',
  conflicts: [{ week_start: '2026-10-19', reason: 'athlete_changed_week' }],
  season_relaid: false,
};

/** The smallest plan card the shared check accepts. */
const plan = {
  goal_race: { name: 'Harricana 65K', date: '2027-09-11', priority: 'A' },
  phases: [],
  weeks: [],
  weeks_deferred: 0,
};

describe('parseDraftState', () => {
  it('reads a draft state with its conflicts', () => {
    expect(parseDraftState(draftState)).toEqual(draftState);
  });

  it('reads a stamp to the microsecond, or with an offset, and keeps it as the server wrote it', () => {
    const micro = { ...draftState, updated_at: '2026-10-16T14:05:00.482913Z' };
    expect(parseDraftState(micro)).toEqual(micro);
    const offset = { ...draftState, updated_at: '2026-10-16T10:05:00.482913-04:00' };
    expect(parseDraftState(offset)).toEqual(offset);
  });

  it('refuses a conflict reason the contract does not name', () => {
    expect(
      parseDraftState({ ...draftState, conflicts: [{ week_start: '2026-10-19', reason: 'coach_changed_week' }] }),
    ).toBeNull();
  });

  it('refuses a week that is not a civil date, a bad stamp, or a missing flag', () => {
    expect(parseDraftState({ ...draftState, conflicts: [{ week_start: '19/10/2026', reason: 'athlete_changed_week' }] })).toBeNull();
    expect(parseDraftState({ ...draftState, updated_at: 'yesterday' })).toBeNull();
    expect(parseDraftState({ ...draftState, updated_at: '2026-10-16T14:05:00' })).toBeNull();
    expect(parseDraftState({ ...draftState, season_relaid: 'no' })).toBeNull();
    expect(parseDraftState({ ...draftState, draft_id: '' })).toBeNull();
    expect(parseDraftState(null)).toBeNull();
  });
});

describe('parsePlanSubject', () => {
  it('reads the athlete a draft is for', () => {
    expect(parsePlanSubject({ user_id: 'user-alice', display_name: 'Alice Runner' })).toEqual({
      user_id: 'user-alice',
      display_name: 'Alice Runner',
    });
  });

  it('refuses a subject without an id or a name', () => {
    expect(parsePlanSubject({ user_id: '', display_name: 'Alice' })).toBeNull();
    expect(parsePlanSubject({ user_id: 'user-alice' })).toBeNull();
    expect(parsePlanSubject('user-alice')).toBeNull();
  });
});

describe('parseAthletePlanResponse', () => {
  const body = {
    athlete: { user_id: 'user-alice', display_name: 'Alice Runner' },
    today: '2026-10-28',
    live: plan,
  };

  it("reads a coached athlete's live plan, and an athlete with none", () => {
    expect(parseAthletePlanResponse(body)?.live?.goal_race.name).toBe('Harricana 65K');
    expect(parseAthletePlanResponse({ ...body, live: null })).toEqual({
      athlete: body.athlete,
      today: '2026-10-28',
      live: null,
    });
  });

  it('refuses a body whose plan, day or athlete breaks its shape', () => {
    expect(parseAthletePlanResponse({ ...body, live: { weeks: [] } })).toBeNull();
    expect(parseAthletePlanResponse({ ...body, today: 'today' })).toBeNull();
    expect(parseAthletePlanResponse({ ...body, athlete: null })).toBeNull();
    expect(parseAthletePlanResponse(null)).toBeNull();
  });
});

describe('parseSubjectPlanResponse', () => {
  const body = {
    athlete: { user_id: 'user-alice', display_name: 'Alice Runner' },
    today: '2026-10-16',
    live: plan,
    draft: plan,
    draft_state: draftState,
  };

  it('reads the live plan, the draft and where the draft stands', () => {
    const read = parseSubjectPlanResponse(body);
    expect(read?.athlete.display_name).toBe('Alice Runner');
    expect(read?.live?.goal_race.name).toBe('Harricana 65K');
    expect(read?.draft_state?.conflicts).toHaveLength(1);
  });

  it('reads an athlete with no plan and no draft open', () => {
    expect(parseSubjectPlanResponse({ ...body, live: null, draft: null, draft_state: null })).toEqual({
      athlete: body.athlete,
      today: '2026-10-16',
      live: null,
      draft: null,
      draft_state: null,
    });
  });

  it('refuses a body whose plan or draft state breaks its shape', () => {
    expect(parseSubjectPlanResponse({ ...body, live: { weeks: [] } })).toBeNull();
    expect(parseSubjectPlanResponse({ ...body, draft_state: { draft_id: 'x' } })).toBeNull();
    expect(parseSubjectPlanResponse({ ...body, today: 'today' })).toBeNull();
    expect(parseSubjectPlanResponse({ ...body, athlete: null })).toBeNull();
  });
});

describe('parsePublishedSubjectPlan', () => {
  it('reads a publish that replaced a season', () => {
    expect(
      parsePublishedSubjectPlan({ plan_id: 'plan-2', superseded_plan_id: 'plan-1', athlete_notified: true }),
    ).toEqual({ plan_id: 'plan-2', superseded_plan_id: 'plan-1', athlete_notified: true });
  });

  it('reads a first season, which replaced nothing', () => {
    expect(
      parsePublishedSubjectPlan({ plan_id: 'plan-1', superseded_plan_id: null, athlete_notified: false })
        ?.superseded_plan_id,
    ).toBeNull();
  });

  it('refuses a body without its notice flag', () => {
    expect(parsePublishedSubjectPlan({ plan_id: 'plan-1', superseded_plan_id: null })).toBeNull();
  });

  // The publish says whether the room was told which days changed (carnet#1249).
  it('reads whether the publish was said in the room, either way', () => {
    const published = { plan_id: 'plan-2', superseded_plan_id: 'plan-1', athlete_notified: true };
    expect(parsePublishedSubjectPlan({ ...published, posted_in_room: true })).toEqual({
      ...published,
      posted_in_room: true,
    });
    // A room that is not the coach's and the athlete's alone hears nothing.
    expect(parsePublishedSubjectPlan({ ...published, posted_in_room: false })?.posted_in_room).toBe(false);
  });

  it('invents no room line for a body that names none, and refuses one that is not a flag', () => {
    const published = parsePublishedSubjectPlan({ plan_id: 'plan-2', superseded_plan_id: null, athlete_notified: true });
    expect(published).not.toBeNull();
    expect(published !== null && 'posted_in_room' in published).toBe(false);
    expect(
      parsePublishedSubjectPlan({ plan_id: 'plan-2', superseded_plan_id: null, athlete_notified: true, posted_in_room: 'yes' }),
    ).toBeNull();
  });
});

describe('parsePlanPublishConflict', () => {
  it('reads the weeks and the re-laid season a refused publish names', () => {
    expect(
      parsePlanPublishConflict({
        error_code: PLAN_PUBLISH_CONFLICT,
        conflicts: draftState.conflicts,
        season_relaid: true,
      }),
    ).toEqual({ error_code: 'plan_publish_conflict', conflicts: draftState.conflicts, season_relaid: true });
  });

  it('is null for a 409 body that is not the publish conflict', () => {
    expect(parsePlanPublishConflict({ error_code: 'several_tenants', conflicts: [], season_relaid: false })).toBeNull();
    expect(parsePlanPublishConflict({ code: 'Conflict', message: 'no' })).toBeNull();
  });
});
