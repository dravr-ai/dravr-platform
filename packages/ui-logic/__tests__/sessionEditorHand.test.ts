// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach's day editor's hand session, its steps and their m:ss codec, the sport-first family pick
// ABOUTME: and the footer and restore lines (carnet#1714). Every figure stays the server's; these only hold and word what the coach wrote

import { describe, expect, it } from 'vitest';
import type { FamilyCatalogueResponse, PlanDay } from '@pierre/shared-types';
import {
  SESSION_EDITOR_KEYS,
  addStep,
  canAddSession,
  canRemoveSession,
  dayIsUncheckedEdit,
  editorDirty,
  familiesForSport,
  familyBuildChanged,
  formatStepDuration,
  handDayRequest,
  handSessionChanged,
  handSessionComplete,
  handSessionOf,
  handSessionMissing,
  handSummary,
  handStatesDuration,
  initialEditorMode,
  isDateFull,
  isSessionMoved,
  moveStep,
  newSessionDay,
  offeredFamilySport,
  pickSport,
  parseStepDuration,
  removeStep,
  replacementLine,
  restoreLine,
  selectBuildSport,
  sessionExpect,
  setHand,
  setSeriesRepeat,
  setStepMeasureOf,
  unresolvedTargetLine,
  updateStep,
} from '../src/sessionEditor';
import { sportChoices } from '../src/sessionSports';

const easyRun: PlanDay = {
  date: '2026-10-20',
  sport: 'run',
  workout: 'Easy 45 min',
  duration_min: 45,
  intensity: 'easy',
  rest: false,
  family_sport: 'run',
};

const longRun: PlanDay = {
  ...easyRun,
  date: '2026-10-25',
  workout: 'Long run 1h40',
  duration_min: 100,
  steps: [
    { label: 'Warm-up', duration_seconds: 600, target_zone: 'easy' },
    { label: 'Steady', duration_seconds: 4800, target_zone: 'Z2' },
  ],
};

const restDay: PlanDay = { date: '2026-10-21', sport: 'rest', workout: 'Rest', intensity: '', rest: true };

const catalogue: FamilyCatalogueResponse = {
  families: [
    {
      slug: 'vo2max_short',
      name: 'VO2max — short intervals',
      purpose: 'vo2max_short',
      purpose_label: 'short-interval VO2max work',
      sports: ['ride', 'run'],
      default_preset: '30_15',
      presets: [
        {
          key: '30_15',
          name: '30/15 (Rønnestad)',
          sports: ['ride', 'run'],
          params: {
            ride: { sets: { min: 2, max: 4, default: 3 } },
            run: { sets: { min: 2, max: 3, default: 2 } },
          },
        },
        {
          key: '40_20',
          name: '40/20',
          sports: ['ride'],
          params: { ride: { sets: { min: 2, max: 4, default: 3 } } },
        },
      ],
    },
    {
      slug: 'endurance',
      name: 'Endurance',
      purpose: 'endurance',
      purpose_label: 'aerobic endurance',
      sports: ['ride', 'run', 'swim'],
      default_preset: 'steady',
      presets: [
        {
          key: 'steady',
          name: 'Steady',
          sports: ['ride', 'run', 'swim'],
          params: {
            ride: { duration_minutes: { min: 30, max: 300, default: 90 } },
            run: { duration_minutes: { min: 30, max: 90, default: 60 } },
            swim: { duration_minutes: { min: 20, max: 90, default: 45 } },
          },
        },
      ],
    },
  ],
};

describe('the hand session', () => {
  it('opens on the day as it stands, steps copied so an edit never reaches the card', () => {
    const session = handSessionOf(longRun);
    expect(session).toEqual({
      rest: false,
      sport: 'run',
      workout: 'Long run 1h40',
      duration_min: 100,
      intensity: 'easy',
      steps: longRun.steps,
    });
    expect(session.steps[0]).not.toBe(longRun.steps?.[0]);
    expect(initialEditorMode(longRun)).toBe('session');
    expect(initialEditorMode({ ...longRun, template_slug: 'endurance' })).toBe('family');
  });

  it('opens a rest day as a rest day, ready to become a session', () => {
    const session = handSessionOf(restDay);
    expect(session.rest).toBe(true);
    expect(handDayRequest(session)).toEqual({ rest: true });
    expect(handSessionChanged(restDay, session)).toBe(false);
    const ride = setHand(session, { rest: false, workout: 'Spin 30 min' });
    expect(handSessionComplete(ride)).toBe(true);
    expect(handDayRequest(ride)).toEqual({ rest: false, sport: 'ride', workout: 'Spin 30 min', intensity: '', steps: [] });
  });

  it('reads as changed only once something the save sends differs', () => {
    const session = handSessionOf(easyRun);
    expect(handSessionChanged(easyRun, session)).toBe(false);
    expect(handSessionChanged(easyRun, setHand(session, { workout: 'Easy 45 min ' }))).toBe(false);
    expect(handSessionChanged(easyRun, setHand(session, { duration_min: 50 }))).toBe(true);
    expect(handSessionChanged(easyRun, setHand(session, { rest: true }))).toBe(true);
  });

  it("states the duration only without steps: with steps the server writes what they total", () => {
    const steady = handSessionOf(easyRun);
    expect(handStatesDuration(steady)).toBe(true);
    expect(handDayRequest(steady)).toMatchObject({ duration_min: 45 });
    const structured = handSessionOf(longRun);
    expect(handStatesDuration(structured)).toBe(false);
    expect(handDayRequest(structured)).not.toHaveProperty('duration_min');
    expect(handStatesDuration(handSessionOf(restDay))).toBe(false);
  });

  it('needs a name, a time and a target on every step, and a distance when it measures by one', () => {
    const session = updateStep(addStep(handSessionOf(easyRun)), 0, { label: 'Strides', duration_seconds: 20 });
    expect(handSessionMissing(session)).toEqual({ key: 'plan.editor.missingStepTarget', values: { number: '1' } });
    const targeted = updateStep(session, 0, { target_zone: 'Z5' });
    expect(handSessionComplete(targeted)).toBe(true);
    const byDistance = setStepMeasureOf(targeted, 0, 'distance');
    expect(handSessionMissing(byDistance)).toEqual({ key: 'plan.editor.missingStepDistance', values: { number: '1' } });
    expect(handSessionComplete(updateStep(byDistance, 0, { distance_meters: 100 }))).toBe(true);
    expect(handSessionMissing(updateStep(targeted, 0, { duration_seconds: 0 }))).toEqual({
      key: 'plan.editor.missingStepDuration',
      values: { number: '1' },
    });
  });

  it('says the sport, then the title, is what the save waits for', () => {
    const blank = handSessionOf(newSessionDay('2026-10-21', []));
    expect(handSessionMissing(blank)).toEqual({ key: 'plan.editor.missingTitle' });
    expect(handSessionMissing(setHand(blank, { sport: '  ' }))).toEqual({ key: 'plan.editor.missingSportName' });
    expect(handSessionMissing(handSessionOf(restDay))).toBeNull();
  });

  it('needs a title and named steps to save, and sends them trimmed, a repeated step as a numbered set', () => {
    const session = addStep(handSessionOf(easyRun));
    expect(handSessionComplete(session)).toBe(false);
    const named = updateStep(session, 0, { label: ' Strides ', duration_seconds: 20, target_zone: ' fast ', repeat: 6 });
    expect(handSessionComplete(named)).toBe(true);
    expect(handSessionComplete(setHand(named, { workout: '  ' }))).toBe(false);
    expect(handDayRequest(named)).toEqual({
      rest: false,
      sport: 'run',
      workout: 'Easy 45 min',
      intensity: 'easy',
      steps: [{ label: 'Strides', duration_seconds: 20, target_zone: 'fast', repeat: 6, repeat_group: 1 }],
    });
    expect(handDayRequest(setSeriesRepeat(named, 0, 1)).steps).toEqual([
      { label: 'Strides', duration_seconds: 20, target_zone: 'fast' },
    ]);
  });

  it("sends the steps' zone span as the intensity, so the card never reads easy over Z4 efforts", () => {
    const session = updateStep(addStep(handSessionOf(easyRun)), 0, { label: 'Tempo', target_zone: 'Z3' });
    expect((handDayRequest(session) as { intensity: string }).intensity).toBe('Z3');
    expect(handSummary(session)).toEqual({ minutes: 5, span: 'Z3' });
    expect(handSummary(handSessionOf(easyRun))).toEqual({ minutes: 45, span: 'easy' });
  });

  it('moves and removes steps, holding a move inside the list', () => {
    const session = handSessionOf(longRun);
    expect(moveStep(session, 1, -1).steps.map((step) => step.label)).toEqual(['Steady', 'Warm-up']);
    expect(moveStep(session, 0, -1).steps).toEqual(session.steps);
    expect(moveStep(session, 1, 1).steps).toEqual(session.steps);
    expect(removeStep(session, 0).steps.map((step) => step.label)).toEqual(['Steady']);
  });

  it("offers the one-tap sports, then the day's own word when it names none of them", () => {
    expect(sportChoices('gravel')).toEqual(['ride', 'run', 'swim', 'strength', 'mobility', 'yoga', 'crossfit', 'gravel']);
    expect(sportChoices('Run')).toEqual(['ride', 'run', 'swim', 'strength', 'mobility', 'yoga', 'crossfit']);
  });
});

describe('the m:ss step duration', () => {
  it('formats seconds as m:ss', () => {
    expect(formatStepDuration(90)).toBe('1:30');
    expect(formatStepDuration(600)).toBe('10:00');
    expect(formatStepDuration(20)).toBe('0:20');
    expect(formatStepDuration(3725)).toBe('62:05');
  });

  it('reads m:ss, :ss and bare minutes, and nothing else', () => {
    expect(parseStepDuration('1:30')).toBe(90);
    expect(parseStepDuration(':20')).toBe(20);
    expect(parseStepDuration(' 10 ')).toBe(600);
    expect(parseStepDuration('1:5')).toBe(65);
    expect(parseStepDuration('1:60')).toBeUndefined();
    expect(parseStepDuration('1:30:00')).toBeUndefined();
    expect(parseStepDuration('abc')).toBeUndefined();
    expect(parseStepDuration('')).toBeUndefined();
  });
});

describe('the sport-first family pick', () => {
  it('lists only the families with a form in the sport, every family when the day reads as none', () => {
    expect(familiesForSport(catalogue, 'swim').map((family) => family.slug)).toEqual(['endurance']);
    expect(familiesForSport(catalogue, undefined)).toHaveLength(2);
  });

  it("keeps the family and format when they have the sport, else moves to one that does, at that sport's defaults", () => {
    const ride = { family: 'vo2max_short', preset: '40_20', sport: 'ride' as const, params: { sets: 4 } };
    expect(selectBuildSport(catalogue, ride, 'run')).toEqual({
      family: 'vo2max_short',
      preset: '30_15',
      sport: 'run',
      params: { sets: 2 },
    });
    expect(selectBuildSport(catalogue, ride, 'swim')).toEqual({
      family: 'endurance',
      preset: 'steady',
      sport: 'swim',
      params: { duration_minutes: 45 },
    });
  });

  it('offers the family mode only for a sport a family is written for, with no note otherwise', () => {
    const rideOnly: FamilyCatalogueResponse = { families: catalogue.families.filter((f) => f.slug === 'vo2max_short') };
    expect(offeredFamilySport(catalogue, 'run', easyRun)).toBe('run');
    expect(offeredFamilySport(rideOnly, 'swim', easyRun)).toBeUndefined();
    expect(offeredFamilySport(catalogue, 'strength', easyRun)).toBeUndefined();
    expect(offeredFamilySport(undefined, 'run', easyRun)).toBeUndefined();
    // The day's own word reads as the family sport the server named for it.
    expect(offeredFamilySport(catalogue, 'Course', { ...easyRun, sport: 'Course' })).toBe('run');
  });

  it('moves both modes with one sport pick, closing the family mode for a sport with no family', () => {
    const hand = handSessionOf(easyRun);
    const build = { family: 'endurance', preset: 'steady', sport: 'run' as const, params: { duration_minutes: 60 } };
    const toStrength = pickSport({ hand, build, mode: 'family' }, 'strength', easyRun, catalogue);
    expect(toStrength.hand.sport).toBe('strength');
    expect(toStrength.mode).toBe('session');
    expect(toStrength.build).toBe(build);
    const toRide = pickSport({ hand, build, mode: 'family' }, 'ride', easyRun, catalogue);
    expect(toRide.mode).toBe('family');
    expect(toRide.build?.sport).toBe('ride');
  });

  it('counts as dirty a changed session, or a family build the coach touched that differs, and nothing else', () => {
    const opening = { family: 'endurance', preset: 'steady', sport: 'run' as const, params: { duration_minutes: 60 } };
    const hand = handSessionOf(easyRun);
    expect(editorDirty(easyRun, hand, opening, null)).toBe(false);
    expect(editorDirty(easyRun, setHand(hand, { duration_min: 50 }), opening, null)).toBe(true);
    expect(editorDirty(easyRun, hand, opening, { ...opening, params: { duration_minutes: 70 } })).toBe(true);
    const familyDay = { ...easyRun, template_slug: 'endurance' };
    expect(editorDirty(familyDay, handSessionOf(familyDay), opening, { ...opening })).toBe(false);
    expect(editorDirty(easyRun, hand, null, opening)).toBe(false);
  });

  it('counts a family build as a change on a day no family built, and on a family day only once it differs', () => {
    const build = { family: 'endurance', preset: 'steady', sport: 'run' as const, params: { duration_minutes: 60 } };
    expect(familyBuildChanged(easyRun, build, build)).toBe(true);
    const familyDay = { ...easyRun, template_slug: 'endurance' };
    expect(familyBuildChanged(familyDay, build, { ...build })).toBe(false);
    expect(familyBuildChanged(familyDay, build, { ...build, params: { duration_minutes: 70 } })).toBe(true);
  });
});

describe('the lines around the save', () => {
  it('names what the save replaces and with what', () => {
    expect(replacementLine(easyRun, { rest: false, name: 'Easy 50 min' })).toEqual({
      key: SESSION_EDITOR_KEYS.replacesWith,
      values: { from: 'Easy 45 min', to: 'Easy 50 min' },
    });
    expect(replacementLine(easyRun, { rest: true })).toEqual({
      key: SESSION_EDITOR_KEYS.replacesWithRest,
      values: { from: 'Easy 45 min' },
    });
    expect(replacementLine(restDay, { rest: false, name: 'Spin' })).toEqual({
      key: SESSION_EDITOR_KEYS.replacesRest,
      values: { to: 'Spin' },
    });
    expect(replacementLine(restDay, { rest: true })).toEqual({ key: SESSION_EDITOR_KEYS.noChanges });
    expect(replacementLine(easyRun, null)).toEqual({ key: SESSION_EDITOR_KEYS.noChanges });
    expect(replacementLine(restDay, { rest: false, name: '  ' })).toBeNull();
    // Only the steps changed: the one session is named once.
    expect(replacementLine(easyRun, { rest: false, name: ' Easy 45 min ' })).toEqual({
      key: SESSION_EDITOR_KEYS.modifies,
      values: { name: 'Easy 45 min' },
    });
  });

  it('says what restoring puts back, only on a day the coach edited', () => {
    expect(restoreLine(easyRun)).toBeNull();
    expect(restoreLine({ ...easyRun, edited: { workout: 'Long run 1h40', rest: false } })).toEqual({
      key: SESSION_EDITOR_KEYS.restoreNote,
      values: { from: 'Long run 1h40' },
    });
    expect(restoreLine({ ...easyRun, edited: { workout: 'Rest', rest: true } })).toEqual({
      key: SESSION_EDITOR_KEYS.restoreNoteRest,
    });
  });

  it('says restoring removes a session the coach added', () => {
    expect(restoreLine({ ...easyRun, added: true })).toEqual({ key: SESSION_EDITOR_KEYS.restoreNoteAdded });
    expect(dayIsUncheckedEdit({ ...easyRun, added: true })).toBe(true);
  });

  it('marks an edited session that names no family as not checked, and nothing else', () => {
    const edited = { workout: 'Easy 45 min', rest: false };
    expect(dayIsUncheckedEdit({ ...easyRun, edited })).toBe(true);
    expect(dayIsUncheckedEdit(easyRun)).toBe(false);
    expect(dayIsUncheckedEdit({ ...easyRun, edited, template_slug: 'endurance' })).toBe(false);
    expect(dayIsUncheckedEdit({ ...restDay, edited })).toBe(false);
  });
});

describe('several sessions on one date (carnet#1716)', () => {
  it('leaves room for a session up to five entries on a date, and removes one only while another stays', () => {
    const entries = (count: number) => Array.from({ length: count }, (_, position) => ({ ...easyRun, position }));
    expect(canAddSession(entries(4))).toBe(true);
    expect(canAddSession(entries(5))).toBe(false);
    expect(canRemoveSession(entries(1))).toBe(false);
    expect(canRemoveSession(entries(2))).toBe(true);
  });

  it("opens a new session in the sport of the date's first session, untitled, after the entries already there", () => {
    expect(newSessionDay('2026-10-20', [restDay, { ...easyRun, position: 1 }])).toEqual({
      date: '2026-10-20',
      sport: 'run',
      family_sport: 'run',
      workout: '',
      intensity: '',
      rest: false,
      position: 2,
    });
    expect(newSessionDay('2026-10-21', [restDay]).sport).toBe('ride');
    expect(newSessionDay('2026-10-21', [{ ...easyRun, sport: 'strength', family_sport: undefined }])).not.toHaveProperty(
      'family_sport',
    );
    expect(handSessionComplete(handSessionOf(newSessionDay('2026-10-21', [])))).toBe(false);
  });

  it('words a refusal over an unreadable step target with its step and target, and nothing else', () => {
    const refusal = (details: Record<string, unknown>) => ({ response: { status: 400, data: { details } } });
    expect(unresolvedTargetLine(refusal({ reason: 'unresolvable_target', step: 3, target: 'seuil' }))).toEqual({
      key: 'plan.editor.targetUnresolved',
      values: { number: '3', target: 'seuil' },
    });
    expect(unresolvedTargetLine(refusal({ reason: 'invalid_session' }))).toBeNull();
    expect(unresolvedTargetLine(refusal({ reason: 'unresolvable_target' }))).toBeNull();
  });

  it('names a session by what the card shows, and reads a moved-session refusal from its reason', () => {
    expect(sessionExpect(easyRun)).toEqual({ sport: 'run', workout: 'Easy 45 min' });
    expect(isSessionMoved({ response: { status: 409, data: { details: { reason: 'session_moved' } } } })).toBe(true);
    expect(isSessionMoved({ response: { status: 409, data: { details: { reason: 'already_linked' } } } })).toBe(false);
    expect(isSessionMoved(new Error('network'))).toBe(false);
    expect(isDateFull({ response: { status: 422, data: { details: { reason: 'date_full' } } } })).toBe(true);
    expect(isDateFull({ response: { status: 422, data: { details: { reason: 'session_moved' } } } })).toBe(false);
  });
});
