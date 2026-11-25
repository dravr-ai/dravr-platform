// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach's day editor on the phone — a session edited by hand or built from a family, or a new one added to a date, saved into the draft
// ABOUTME: Writes name the session by position and as the card showed it; a moved one rereads; a date's extra session removes; the card marks edits

import React from 'react';
import { AccessibilityInfo, Alert, Modal } from 'react-native';
import { act, fireEvent, render, screen, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider, defaultScheduler, notifyManager } from '@tanstack/react-query';
import Toast from 'react-native-toast-message';
import { i18n } from '@pierre/i18n';
import type {
  FamilyCatalogueResponse,
  PlanDay,
  SessionFinding,
  SessionPreview,
  SubjectPlanResponse,
  WorkoutPlan,
} from '@pierre/shared-types';

const CONVERSATION_ID = 'conv-plan-chloe';

// The chat API, in place of the server: the families, the preview, the two
// session saves, the restore, the remove and the add the editor's shared
// hooks call.
const mockGetFamilies = jest.fn();
const mockPreview = jest.fn();
const mockSaveDay = jest.fn();
const mockSaveHandDay = jest.fn();
const mockRestore = jest.fn();
const mockRemove = jest.fn();
const mockAdd = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    getSubjectFamilies: (...a: unknown[]) => mockGetFamilies(...a),
    previewSubjectSession: (...a: unknown[]) => mockPreview(...a),
    saveSubjectDraftDay: (...a: unknown[]) => mockSaveDay(...a),
    saveSubjectDraftHandDay: (...a: unknown[]) => mockSaveHandDay(...a),
    restoreSubjectDraftDay: (...a: unknown[]) => mockRestore(...a),
    removeSubjectDraftDay: (...a: unknown[]) => mockRemove(...a),
    addSubjectDraftSession: (...a: unknown[]) => mockAdd(...a),
  },
}));

jest.mock('../src/screens/chat/presentMessageMenu', () => ({ presentMessageMenu: jest.fn() }));

import { QUERY_KEYS } from '@pierre/shared-constants';
import SessionEditorSheet from '../src/screens/chat/SessionEditorSheet';
import WorkoutPlanCard, { type SessionEditorTarget } from '../src/screens/chat/WorkoutPlanCard';
import { MessageList } from '../src/screens/chat/MessageList';
import type { Message } from '../src/types';

/** Two families as the catalogue sends them: one with a single value, one with five over two presets. */
const CATALOGUE: FamilyCatalogueResponse = {
  families: [
    {
      slug: 'endurance_steady',
      name: 'Steady endurance',
      purpose: 'endurance',
      sports: ['ride', 'run'],
      default_preset: 'steady',
      presets: [
        {
          key: 'steady',
          name: 'Steady',
          sports: ['ride', 'run'],
          params: { ride: { duration_minutes: { min: 45, max: 240, default: 90 } }, run: { duration_minutes: { min: 45, max: 240, default: 90 } } },
        },
      ],
    },
    {
      slug: 'vo2max_short',
      name: 'Short VO2max intervals',
      purpose: 'vo2max_short',
      purpose_label: 'VO2max, short intervals',
      sports: ['ride', 'run'],
      default_preset: 'ronnestad_30_15',
      presets: [
        {
          key: 'ronnestad_30_15',
          name: '30/15 (Rønnestad)',
          summary: 'Short work, shorter rest, three sets.',
          sports: ['ride', 'run'],
          params: {
            ride: {
              sets: { min: 2, max: 4, default: 3 },
              reps: { min: 10, max: 15, default: 13 },
              work_seconds: { min: 30, max: 30, default: 30 },
              rest_seconds: { min: 15, max: 15, default: 15 },
              set_rest_seconds: { min: 120, max: 300, default: 180 },
            },
            run: {
              sets: { min: 2, max: 4, default: 3 },
              reps: { min: 10, max: 15, default: 13 },
              work_seconds: { min: 30, max: 30, default: 30 },
              rest_seconds: { min: 15, max: 15, default: 15 },
              set_rest_seconds: { min: 120, max: 300, default: 180 },
            },
          },
        },
        {
          key: 'billat_30_30',
          name: '30/30 (Billat)',
          sports: ['run'],
          params: { run: { reps: { min: 10, max: 20, default: 15 } } },
        },
      ],
    },
  ],
};

/** The catalogue with a family in one sport each beside the two above, so the sport filter has something to leave out. */
const SPORT_CATALOGUE: FamilyCatalogueResponse = {
  families: [
    ...CATALOGUE.families,
    {
      slug: 'sweet_spot',
      name: 'Sweet spot',
      purpose: 'sweet_spot',
      sports: ['ride'],
      default_preset: 'two_by_twenty',
      presets: [
        {
          key: 'two_by_twenty',
          name: '2 × 20',
          sports: ['ride'],
          params: { ride: { sets: { min: 2, max: 3, default: 2 } } },
        },
      ],
    },
    {
      slug: 'swim_css',
      name: 'CSS swim',
      purpose: 'threshold',
      sports: ['swim'],
      default_preset: 'css_100s',
      presets: [
        {
          key: 'css_100s',
          name: '100s at CSS',
          sports: ['swim'],
          params: { swim: { reps: { min: 8, max: 20, default: 10 } } },
        },
      ],
    },
  ],
};

/** A day the agent built from the 30/15 preset, with two values stated. */
const FAMILY_DAY: PlanDay = {
  date: '2026-11-24',
  sport: 'run',
  workout: '3 × 12 × 30/15',
  duration_min: 58,
  intensity: 'vo2max',
  rest: false,
  template_slug: 'vo2max_short',
  template_preset: 'ronnestad_30_15',
  template_params: { sets: 3, reps: 12 },
  family_sport: 'run',
};

const REST_DAY: PlanDay = { date: '2026-11-25', sport: 'rest', workout: 'Rest', intensity: 'rest', rest: true };

/** A session the agent wrote without a family: what most days of a plan are. */
const AGENT_DAY: PlanDay = {
  date: '2026-11-26',
  sport: 'run',
  workout: 'Easy run',
  duration_min: 45,
  intensity: 'easy',
  rest: false,
  family_sport: 'run',
  steps: [
    { label: 'Warm-up', duration_seconds: 600, target_zone: 'Z1' },
    { label: 'Steady', duration_seconds: 1800, target_zone: 'Z2' },
    { label: 'Cool-down', duration_seconds: 300, target_zone: 'Z1' },
  ],
};

/** The agent's day after the coach rewrote it by hand in their draft. */
const EDITED_DAY: PlanDay = {
  ...AGENT_DAY,
  workout: 'Easy 50 min',
  duration_min: 50,
  edited: { workout: 'Easy run', rest: false },
};

/** A second session the coach added to the agent's day, after its run (carnet#1716). */
const ADDED_DAY: PlanDay = {
  date: AGENT_DAY.date,
  position: 1,
  sport: 'strength',
  workout: 'Core circuit',
  duration_min: 20,
  intensity: 'easy',
  rest: false,
  added: true,
};

/** What each write sends as the session it is about: the card's sport and title. */
const AGENT_EXPECT = { sport: 'run', workout: 'Easy run' };
const FAMILY_EXPECT = { sport: 'run', workout: '3 × 12 × 30/15' };

/** A write the server refused because the session at that position is no longer the one the card showed. */
const SESSION_MOVED = Object.assign(new Error('Request failed with status code 409'), {
  isAxiosError: true,
  response: {
    status: 409,
    data: { error: { code: 'conflict', message: 'session moved' }, details: { reason: 'session_moved' } },
  },
});

/** The session the kernel measures for a 30/15 build, with the findings the envelope check fired. */
function previewOf(sets: number, findings: SessionFinding[] = []): SessionPreview {
  return {
    family: 'vo2max_short',
    preset: 'ronnestad_30_15',
    sport: 'run',
    duration_min: 58,
    steps: [
      { label: 'Warm-up', duration_seconds: 900, target_zone: 'Z2' },
      { label: '30/15', duration_seconds: 30, target_zone: '4:05/km', repeat: 12 },
    ],
    derived: {
      sets,
      reps: 12,
      work_seconds: 30,
      rest_seconds: 15,
      ratio: 2,
      work_pct: 100,
      rest_pct: 50,
      on_time_minutes: sets * 6,
      total_minutes: 58.5,
      aerobic_max: 'vVO2max',
    },
    findings,
    ...(findings.length > 0 ? { outside_evidence: 'Outside the evidence' } : {}),
    anchor: 'Anchor: vVO2max 4:05/km, your own.',
  };
}

const ABOVE_RANGE: SessionFinding = {
  code: 'param_out_of_range',
  text: 'Six sets is past what the 30/15 trials used.',
  param: 'sets',
};

/** What a write answers: the subject plan, read after it. */
const SAVED: SubjectPlanResponse = {
  athlete: { user_id: 'user-chloe', display_name: 'Chloé Tremblay' },
  today: '2026-11-21',
  live: null,
  draft: null,
  draft_state: null,
} as unknown as SubjectPlanResponse;

function newClient(): QueryClient {
  // No garbage-collection timers: a cache entry's five-minute timer would
  // otherwise hold the run open after the last assertion.
  return new QueryClient({
    defaultOptions: {
      queries: { retry: false, gcTime: Infinity },
      mutations: { retry: false, gcTime: Infinity },
    },
  });
}

/**
 * The sheet open on `day`: editing it, alone on its date unless `entries`
 * says what else the date holds, or adding a session to its date.
 */
function renderEditor(
  day: PlanDay = FAMILY_DAY,
  { entries = [day], mode = 'edit' }: { entries?: PlanDay[]; mode?: SessionEditorTarget['mode'] } = {},
) {
  const onClose = jest.fn();
  const client = newClient();
  render(
    <QueryClientProvider client={client}>
      <SessionEditorSheet
        conversationId={CONVERSATION_ID}
        athleteName="Chloé Tremblay"
        target={{ day, entries, mode }}
        onClose={onClose}
      />
    </QueryClientProvider>,
  );
  return { onClose, client };
}

/** Let the clock run, and whatever falls due in that time settle. */
async function elapse(ms: number) {
  await act(async () => {
    await jest.advanceTimersByTimeAsync(ms);
  });
}

/** The build the last preview measured. */
function lastPreviewed(): Record<string, unknown> {
  const calls = mockPreview.mock.calls;
  return calls[calls.length - 1][1] as Record<string, unknown>;
}

/** The footer's sentence: what a save would replace, and with what. */
function footer() {
  return screen.getByTestId('session-editor-replacement');
}

/** Press the destructive button of the last confirm the sheet asked for, and let the write settle. */
async function confirmLast(alert: jest.SpyInstance) {
  const [, , buttons] = alert.mock.calls[alert.mock.calls.length - 1] as [
    string,
    string,
    { style?: string; onPress?: () => void }[],
  ];
  await act(async () => {
    buttons.find((b) => b.style === 'destructive')?.onPress?.();
    await jest.advanceTimersByTimeAsync(0);
  });
}

/** Open a step's editor from its collapsed row. */
function openStep(index: number) {
  fireEvent.press(screen.getByTestId(`session-editor-step-${index}`));
}

/** Type into a step's m:ss field the way a thumb does: focus, type, leave. */
function typeStepDuration(index: number, text: string) {
  const field = screen.getByTestId(`session-editor-step-${index}-duration`);
  fireEvent(field, 'focus');
  fireEvent.changeText(field, text);
  fireEvent(field, 'blur');
}

beforeEach(() => {
  jest.clearAllMocks();
  // A fake clock for the preview's 300 ms settle, with React Query's
  // notifications run inline so the clock only drives the debounce.
  jest.useFakeTimers();
  notifyManager.setScheduler((callback) => callback());
  mockGetFamilies.mockResolvedValue(CATALOGUE);
  mockPreview.mockImplementation((_id: string, request: { params?: { sets?: number } }) =>
    Promise.resolve(previewOf(request.params?.sets ?? 3)),
  );
  mockSaveDay.mockResolvedValue(SAVED);
  mockSaveHandDay.mockResolvedValue(SAVED);
  mockRestore.mockResolvedValue(SAVED);
  mockRemove.mockResolvedValue(SAVED);
  mockAdd.mockResolvedValue(SAVED);
});

afterEach(() => {
  notifyManager.setScheduler(defaultScheduler);
  jest.useRealTimers();
});

describe('the day editor on a session the agent wrote', () => {
  it("opens on the day's own session, under the day's date, with nothing to save yet", async () => {
    renderEditor(AGENT_DAY);
    await elapse(300);

    expect(screen.getByTestId('session-editor-title')).toHaveTextContent('Edit Thu, Nov 26');
    // A run has families in the catalogue: "Write" is on, the family mode one tap away.
    expect(screen.getByTestId('session-editor-mode-session')).toHaveProp('accessibilityState', { checked: true });
    expect(screen.getByTestId('session-editor-mode-session')).toHaveTextContent('Write');
    expect(screen.getByTestId('session-editor-mode-family')).toHaveTextContent('From an evidence-backed family');
    expect(screen.getByTestId('session-editor-mode-family')).toHaveProp('accessibilityState', { checked: false });
    expect(screen.getByTestId('session-editor-rest')).toHaveProp('value', false);
    // The title is the header's headline field, named "Title", over the summary the steps total.
    const title = screen.getByTestId('session-editor-workout');
    expect(title).toHaveProp('value', 'Easy run');
    expect(title).toHaveProp('accessibilityLabel', 'Title');
    expect(title).toHaveProp('placeholder', 'Session title');
    expect(screen.getByTestId('session-editor-summary')).toHaveTextContent('Run · 45 min · Z1–Z2');
    // One chip per one-tap sport, in the card's words, then "Other…"; the day's own is picked.
    expect(screen.getByTestId('session-editor-sport-run')).toHaveProp('accessibilityState', { selected: true });
    expect(screen.getByTestId('session-editor-sport-run')).toHaveProp('accessibilityRole', 'button');
    expect(screen.getByTestId('session-editor-sport-run')).toHaveTextContent('Run');
    expect(screen.getByTestId('session-editor-sport-ride')).toHaveProp('accessibilityState', { selected: false });
    expect(screen.getByTestId('session-editor-sport-strength')).toHaveTextContent('Strength training');
    expect(screen.getByTestId('session-editor-sport-other')).toHaveTextContent('Other…');
    expect(screen.getByTestId('session-editor-sport-run').props.className).toMatch(/\bmin-h-11\b/);
    // With steps the server writes the duration they total, and the span is the intensity.
    expect(screen.queryByTestId('session-editor-duration')).toBeNull();
    expect(screen.queryByTestId('session-editor-intensity')).toBeNull();
    // Each step at rest is one row: its number, name and tags, the whole row a button.
    const row = screen.getByTestId('session-editor-step-0');
    expect(row).toHaveProp('accessibilityLabel', 'Edit step 1: Warm-up');
    expect(row).toHaveTextContent('1Warm-up10:00Z1');
    expect(screen.getByTestId('session-editor-step-1')).toHaveTextContent('2Steady30:00Z2');
    expect(screen.queryByTestId('session-editor-step-0-label')).toBeNull();
    expect(screen.getByTestId('session-editor-profile')).toHaveProp('accessibilityLabel', 'Session profile');
    expect(screen.getByTestId('session-editor-profile-step-1')).toHaveProp(
      'accessibilityLabel',
      'Step 2: Steady, 30:00, Z2',
    );
    expect(screen.getByTestId('session-editor-unchecked')).toHaveTextContent("Not checked against a family's evidence.");
    // Nothing changed: the footer says so, and Save waits.
    expect(footer()).toHaveTextContent('No changes yet.');
    expect(screen.queryByTestId('session-editor-missing')).toBeNull();
    expect(screen.getByTestId('session-editor-save')).toBeDisabled();
    // A hand edit has no preview, and the catalogue's states stay in the family mode.
    expect(mockPreview).not.toHaveBeenCalled();
    expect(screen.queryByTestId('session-editor-loading')).toBeNull();
    expect(screen.queryByTestId('session-editor-edited')).toBeNull();
  });

  it('opens one step at a time, from its row or its bar in the profile', async () => {
    renderEditor(AGENT_DAY);
    await elapse(0);

    openStep(1);
    expect(screen.getByTestId('session-editor-step-1-open')).toBeTruthy();
    expect(screen.getByTestId('session-editor-step-1-label')).toHaveProp('value', 'Steady');
    expect(screen.getByTestId('session-editor-profile-step-1')).toHaveProp('accessibilityState', { selected: true });

    fireEvent.press(screen.getByTestId('session-editor-profile-step-2'));
    expect(screen.queryByTestId('session-editor-step-1-open')).toBeNull();
    expect(screen.getByTestId('session-editor-step-1')).toBeTruthy();
    expect(screen.getByTestId('session-editor-step-2-label')).toHaveProp('value', 'Cool-down');
  });

  it('names every step control by its number, each a 44pt target, a move that changes nothing dimmed', async () => {
    renderEditor(AGENT_DAY);
    await elapse(0);

    openStep(1);
    const up = screen.getByTestId('session-editor-step-1-up');
    expect(up).toHaveProp('accessibilityLabel', 'Move step 2 up');
    expect(screen.getByTestId('session-editor-step-1-down')).toHaveProp('accessibilityLabel', 'Move step 2 down');
    expect(screen.getByTestId('session-editor-step-1-duplicate')).toHaveProp('accessibilityLabel', 'Duplicate step 2');
    expect(screen.getByTestId('session-editor-step-1-remove')).toHaveProp('accessibilityLabel', 'Remove step 2');
    expect(up.props.className).toMatch(/\bh-11\b/);
    expect(up.props.className).toMatch(/\bw-11\b/);
    expect(up).toHaveProp('accessibilityState', { disabled: false });

    openStep(0);
    expect(screen.getByTestId('session-editor-step-0-up')).toHaveProp('accessibilityState', { disabled: true });
    openStep(2);
    expect(screen.getByTestId('session-editor-step-2-down')).toHaveProp('accessibilityState', { disabled: true });
  });

  it("keeps a step's typed duration while it has focus, and shows m:ss once it loses it", async () => {
    renderEditor(AGENT_DAY);
    await elapse(0);

    openStep(0);
    const field = screen.getByTestId('session-editor-step-0-duration');
    // A text keyboard: a number pad has no colon.
    expect(field).toHaveProp('keyboardType', 'numbers-and-punctuation');
    fireEvent(field, 'focus');
    // Seconds past 59 are not a duration: the text stays, the step does not move.
    fireEvent.changeText(field, '1:75');
    expect(screen.getByTestId('session-editor-step-0-duration')).toHaveProp('value', '1:75');
    fireEvent(field, 'blur');
    expect(screen.getByTestId('session-editor-step-0-duration')).toHaveProp('value', '10:00');
    expect(footer()).toHaveTextContent('No changes yet.');

    typeStepDuration(0, '1:30');
    expect(screen.getByTestId('session-editor-step-0-duration')).toHaveProp('value', '1:30');
    // Bare minutes read as minutes.
    openStep(1);
    typeStepDuration(1, '25');
    expect(screen.getByTestId('session-editor-step-1-duration')).toHaveProp('value', '25:00');
  });

  it('moves a step by 15 s with the buttons beside its time, never under zero', async () => {
    renderEditor({ ...AGENT_DAY, steps: [{ label: 'Strides', duration_seconds: 15, target_zone: 'Z5' }] });
    await elapse(0);

    openStep(0);
    const more = screen.getByTestId('session-editor-step-0-duration-more');
    expect(more).toHaveProp('accessibilityLabel', '15 seconds more');
    expect(more.props.className).toMatch(/\bh-11\b/);
    fireEvent.press(more);
    expect(screen.getByTestId('session-editor-step-0-duration')).toHaveProp('value', '0:30');
    fireEvent.press(screen.getByTestId('session-editor-step-0-duration-less'));
    fireEvent.press(screen.getByTestId('session-editor-step-0-duration-less'));
    expect(screen.getByTestId('session-editor-step-0-duration')).toHaveProp('value', '0:00');
    expect(screen.getByTestId('session-editor-step-0-duration-less')).toHaveProp('accessibilityState', {
      disabled: true,
    });
    expect(screen.getByTestId('session-editor-missing')).toHaveTextContent('Give step 1 a time to save.');
  });

  it('saves a hand edit through the hand mutation with the exact body, closes, and says so', async () => {
    const { onClose } = renderEditor(AGENT_DAY);
    await elapse(0);

    fireEvent.changeText(screen.getByTestId('session-editor-workout'), 'Easy 50 min');
    openStep(0);
    typeStepDuration(0, '12');
    openStep(2);
    fireEvent.press(screen.getByTestId('session-editor-step-2-remove'));
    expect(screen.queryByTestId('session-editor-step-2')).toBeNull();
    fireEvent.press(screen.getByTestId('session-editor-add-step'));
    // A step just added opens, with the caret in its name.
    expect(screen.getByTestId('session-editor-step-2-label')).toHaveProp('autoFocus', true);
    fireEvent.changeText(screen.getByTestId('session-editor-step-2-label'), 'Strides');
    typeStepDuration(2, ':20');
    fireEvent.press(screen.getByTestId('session-editor-step-2-target-z5'));
    fireEvent.press(screen.getByTestId('session-editor-step-2-up'));

    // The open card follows the step it moved.
    expect(screen.getByTestId('session-editor-step-1-label')).toHaveProp('value', 'Strides');
    expect(footer()).toHaveTextContent('Replaces Easy run with Easy 50 min.');
    const save = screen.getByTestId('session-editor-save');
    expect(save).not.toBeDisabled();
    fireEvent.press(save);
    await elapse(0);

    // Session 0 of the date, as the card showed it: a position that now holds
    // another session is refused rather than overwritten. The steps' span is
    // the intensity the card prints.
    expect(mockSaveHandDay).toHaveBeenCalledWith(
      CONVERSATION_ID,
      '2026-11-26',
      0,
      {
        rest: false,
        sport: 'run',
        workout: 'Easy 50 min',
        intensity: 'Z1–Z5',
        steps: [
          { label: 'Warm-up', duration_seconds: 720, target_zone: 'Z1' },
          { label: 'Strides', duration_seconds: 20, target_zone: 'Z5' },
          { label: 'Steady', duration_seconds: 1800, target_zone: 'Z2' },
        ],
      },
      AGENT_EXPECT,
      'en',
    );
    expect(mockSaveDay).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(Toast.show).toHaveBeenCalledWith({
      type: 'success',
      text1: 'Saved to your draft. Chloé Tremblay sees it only once you publish.',
    });
  });

  it('states the duration only for a session without steps, and sends it', async () => {
    renderEditor({ ...AGENT_DAY, steps: undefined });
    await elapse(0);

    expect(screen.getByTestId('session-editor-duration')).toHaveProp('value', '45');
    expect(screen.getByTestId('session-editor-intensity')).toHaveProp('value', 'easy');
    expect(screen.getByTestId('session-editor-summary')).toHaveTextContent('Run · 45 min · easy');
    expect(screen.queryByTestId('session-editor-profile')).toBeNull();
    fireEvent.changeText(screen.getByTestId('session-editor-duration'), '50');
    fireEvent.press(screen.getByTestId('session-editor-save'));
    await elapse(0);

    expect(mockSaveHandDay).toHaveBeenCalledWith(
      CONVERSATION_ID,
      AGENT_DAY.date,
      0,
      { rest: false, sport: 'run', workout: 'Easy run', duration_min: 50, intensity: 'easy', steps: [] },
      AGENT_EXPECT,
      'en',
    );
  });

  it('says what Save waits for while the session has no title, instead of a replacement', async () => {
    renderEditor(AGENT_DAY);
    await elapse(0);

    fireEvent.changeText(screen.getByTestId('session-editor-workout'), '  ');
    expect(screen.queryByTestId('session-editor-replacement')).toBeNull();
    expect(screen.getByTestId('session-editor-missing')).toHaveTextContent('Add a title to save.');
    const save = screen.getByTestId('session-editor-save');
    expect(save).toBeDisabled();
    expect(save).toHaveProp('accessibilityHint', 'Add a title to save.');
  });

  it('saves a rest day as { rest: true }, everything under the switch hidden', async () => {
    const { onClose } = renderEditor(AGENT_DAY);
    await elapse(0);

    fireEvent(screen.getByTestId('session-editor-rest'), 'valueChange', true);

    expect(screen.queryByTestId('session-editor-workout')).toBeNull();
    expect(screen.queryByTestId('session-editor-sport')).toBeNull();
    expect(screen.queryByTestId('session-editor-mode')).toBeNull();
    expect(screen.queryByTestId('session-editor-step-0')).toBeNull();
    expect(screen.queryByTestId('session-editor-unchecked')).toBeNull();
    expect(footer()).toHaveTextContent('Replaces Easy run with a rest day.');

    fireEvent.press(screen.getByTestId('session-editor-save'));
    await elapse(0);

    expect(mockSaveHandDay).toHaveBeenCalledWith(CONVERSATION_ID, '2026-11-26', 0, { rest: true }, AGENT_EXPECT, 'en');
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('holds Save until something changes, and while the session is not whole', async () => {
    renderEditor(AGENT_DAY);
    await elapse(0);
    expect(screen.getByTestId('session-editor-save')).toBeDisabled();

    fireEvent.press(screen.getByTestId('session-editor-sport-ride'));
    expect(screen.getByTestId('session-editor-sport-ride')).toHaveProp('accessibilityState', { selected: true });
    expect(screen.getByTestId('session-editor-sport-run')).toHaveProp('accessibilityState', { selected: false });
    // The row keeps its order under the thumb.
    const chips = () =>
      within(screen.getByTestId('session-editor-sport'))
        .getAllByRole('button')
        .map((chip) => chip.props.testID);
    expect(chips()).toEqual([
      'session-editor-sport-ride',
      'session-editor-sport-run',
      'session-editor-sport-swim',
      'session-editor-sport-strength',
      'session-editor-sport-mobility',
      'session-editor-sport-yoga',
      'session-editor-sport-crossfit',
      'session-editor-sport-other',
    ]);
    expect(screen.getByTestId('session-editor-save')).not.toBeDisabled();
    fireEvent.press(screen.getByTestId('session-editor-sport-run'));
    expect(screen.getByTestId('session-editor-save')).toBeDisabled();
    expect(footer()).toHaveTextContent('No changes yet.');

    // A session needs its title, and every step its name.
    fireEvent.changeText(screen.getByTestId('session-editor-workout'), '  ');
    expect(screen.getByTestId('session-editor-save')).toBeDisabled();
    fireEvent.changeText(screen.getByTestId('session-editor-workout'), 'Easy run, hills');
    expect(screen.getByTestId('session-editor-save')).not.toBeDisabled();
    fireEvent.press(screen.getByTestId('session-editor-add-step'));
    expect(screen.getByTestId('session-editor-save')).toBeDisabled();
  });

  it('turns a rest day into a session the coach writes', async () => {
    renderEditor(REST_DAY);
    await elapse(0);

    expect(screen.getByTestId('session-editor-title')).toHaveTextContent('Edit Wed, Nov 25');
    expect(screen.getByTestId('session-editor-rest')).toHaveProp('value', true);
    expect(screen.queryByTestId('session-editor-workout')).toBeNull();
    expect(screen.queryByTestId('session-editor-mode')).toBeNull();
    expect(footer()).toHaveTextContent('No changes yet.');

    fireEvent(screen.getByTestId('session-editor-rest'), 'valueChange', false);
    expect(screen.getByTestId('session-editor-no-steps')).toHaveTextContent(
      'No steps: the title and intensity describe the session.',
    );
    expect(screen.getByTestId('session-editor-sport-ride')).toHaveProp('accessibilityState', { selected: true });
    expect(screen.getByTestId('session-editor-save')).toBeDisabled();
    fireEvent.changeText(screen.getByTestId('session-editor-workout'), 'Tempo');
    expect(footer()).toHaveTextContent('Replaces the rest day with Tempo.');

    fireEvent.press(screen.getByTestId('session-editor-save'));
    await elapse(0);
    expect(mockSaveHandDay).toHaveBeenCalledWith(
      CONVERSATION_ID,
      '2026-11-25',
      0,
      { rest: false, sport: 'ride', workout: 'Tempo', intensity: '', steps: [] },
      { sport: 'rest', workout: 'Rest' },
      'en',
    );
  });

  it('keeps each mode as the coach left it', async () => {
    renderEditor(AGENT_DAY);
    await elapse(0);

    fireEvent.changeText(screen.getByTestId('session-editor-workout'), 'Easy 50 min');
    fireEvent.press(screen.getByTestId('session-editor-mode-family'));
    expect(screen.queryByTestId('session-editor-workout')).toBeNull();
    fireEvent.press(screen.getByTestId('session-editor-family-vo2max_short'));
    // The family mode's header is the format, which the coach does not edit.
    expect(screen.getByTestId('session-editor-preset-name')).toHaveTextContent('30/15 (Rønnestad)');
    expect(footer()).toHaveTextContent('Replaces Easy run with 30/15 (Rønnestad).');

    fireEvent.press(screen.getByTestId('session-editor-mode-session'));
    expect(screen.getByTestId('session-editor-workout')).toHaveProp('value', 'Easy 50 min');
    expect(footer()).toHaveTextContent('Replaces Easy run with Easy 50 min.');

    fireEvent.press(screen.getByTestId('session-editor-mode-family'));
    expect(screen.getByTestId('session-editor-family-vo2max_short')).toHaveProp('accessibilityState', { checked: true });
  });

  it('says a refused hand save in the sheet and stays open for another try', async () => {
    mockSaveHandDay.mockRejectedValueOnce(new Error('refused'));
    const { onClose } = renderEditor(AGENT_DAY);
    await elapse(0);

    fireEvent.changeText(screen.getByTestId('session-editor-workout'), 'Steady run');
    fireEvent.press(screen.getByTestId('session-editor-save'));
    await elapse(0);

    expect(screen.getByTestId('session-editor-save-failed')).toHaveTextContent(/^This session could not be saved\..+/);
    expect(onClose).not.toHaveBeenCalled();
    expect(Toast.show).not.toHaveBeenCalled();
  });

  it("words a target the calendar cannot read in the coach's language, naming the step and the target", async () => {
    mockSaveHandDay.mockRejectedValueOnce(
      Object.assign(new Error('Request failed with status code 400'), {
        isAxiosError: true,
        response: {
          status: 400,
          data: {
            error: { code: 'invalid_input', message: "step 2's target 'seuil' is not one the calendar can resolve" },
            details: { reason: 'unresolvable_target', step: 2, target: 'seuil' },
          },
        },
      }),
    );
    renderEditor(AGENT_DAY);
    await elapse(0);

    fireEvent.changeText(screen.getByTestId('session-editor-workout'), 'Steady run');
    fireEvent.press(screen.getByTestId('session-editor-save'));
    await elapse(0);

    expect(screen.getByTestId('session-editor-save-failed')).toHaveTextContent(
      'This session could not be saved.The target “seuil” of step 2 isn’t one the calendar reads: pick a zone, a % of a threshold or an RPE.',
    );
  });
});

describe('the sport chips', () => {
  it('closes the family mode for a sport no family is written for, with no note in its place', async () => {
    renderEditor(AGENT_DAY);
    await elapse(0);

    fireEvent.press(screen.getByTestId('session-editor-mode-family'));
    expect(screen.getByTestId('session-editor-form')).toBeTruthy();

    fireEvent.press(screen.getByTestId('session-editor-sport-strength'));
    expect(screen.getByTestId('session-editor-sport-strength')).toHaveProp('accessibilityState', { selected: true });
    expect(screen.queryByTestId('session-editor-mode')).toBeNull();
    expect(screen.queryByTestId('session-editor-form')).toBeNull();
    expect(screen.getByTestId('session-editor-hand')).toBeTruthy();
    expect(screen.queryByText(/No workout family/)).toBeNull();
    expect(screen.getByTestId('session-editor-summary')).toHaveTextContent('Strength training · 45 min · Z1–Z2');

    // Back on a family sport, the mode is offered again.
    fireEvent.press(screen.getByTestId('session-editor-sport-ride'));
    expect(screen.getByTestId('session-editor-mode')).toBeTruthy();
  });

  it('hides the mode switch while the catalogue is read', async () => {
    mockGetFamilies.mockReturnValue(new Promise(() => undefined));
    renderEditor(AGENT_DAY);
    await elapse(0);

    expect(screen.queryByTestId('session-editor-mode')).toBeNull();
    expect(screen.getByTestId('session-editor-hand')).toBeTruthy();
  });

  it('saves the sport the coach names under "Other…", and waits until there is one', async () => {
    renderEditor(AGENT_DAY);
    await elapse(0);

    fireEvent.press(screen.getByTestId('session-editor-sport-other'));
    expect(screen.getByTestId('session-editor-sport-other')).toHaveProp('accessibilityState', { selected: true });
    expect(screen.getByTestId('session-editor-sport-run')).toHaveProp('accessibilityState', { selected: false });
    const name = screen.getByTestId('session-editor-sport-other-name');
    expect(name).toHaveProp('value', '');
    expect(name).toHaveProp('accessibilityLabel', 'Name the sport');
    expect(screen.getByTestId('session-editor-missing')).toHaveTextContent('Name the sport to save.');
    expect(screen.getByTestId('session-editor-save')).toBeDisabled();
    expect(screen.queryByTestId('session-editor-mode')).toBeNull();

    fireEvent.changeText(name, 'Ski touring');
    expect(screen.getByTestId('session-editor-summary')).toHaveTextContent(/^Ski touring · 45 min/);
    fireEvent.press(screen.getByTestId('session-editor-save'));
    await elapse(0);

    expect(mockSaveHandDay).toHaveBeenCalledWith(
      CONVERSATION_ID,
      AGENT_DAY.date,
      0,
      expect.objectContaining({ sport: 'Ski touring', workout: 'Easy run' }),
      AGENT_EXPECT,
      'en',
    );
  });

  it("keeps the day's own sport word as a chip of its own", async () => {
    renderEditor({ ...AGENT_DAY, sport: 'hiking', family_sport: undefined });
    await elapse(0);

    expect(screen.getByTestId('session-editor-sport-hiking')).toHaveProp('accessibilityState', { selected: true });
    expect(screen.getByTestId('session-editor-sport-hiking')).toHaveTextContent('hiking');
    // No family is written for it: no mode switch.
    expect(screen.queryByTestId('session-editor-mode')).toBeNull();
  });
});

describe('steps, sets and targets', () => {
  it('adds a set of an effort and a recovery done five times, and saves it as one numbered set', async () => {
    renderEditor(AGENT_DAY);
    await elapse(0);

    fireEvent.press(screen.getByTestId('session-editor-add-series'));
    // The set's first step opens, named in the coach's words, the caret in its name.
    expect(screen.getByTestId('session-editor-step-3-label')).toHaveProp('value', 'Effort');
    expect(screen.getByTestId('session-editor-step-3-label')).toHaveProp('autoFocus', true);
    expect(screen.getByTestId('session-editor-step-3-target-z4')).toHaveProp('accessibilityState', { checked: true });
    expect(screen.getByTestId('session-editor-step-4')).toHaveTextContent('5Recovery2:00Z1');
    // The set is ruled under its × N stepper; the profile brackets it and draws every round.
    const repeat = screen.getByTestId('session-editor-set-3-repeat');
    expect(repeat).toHaveTextContent('× 5');
    expect(repeat).toHaveProp('accessibilityRole', 'adjustable');
    expect(repeat).toHaveProp('accessibilityLabel', 'Repetitions of the set');
    expect(screen.getByTestId('session-editor-set-3-fewer')).toHaveProp('accessibilityLabel', 'One fewer repetition');
    expect(screen.getByTestId('session-editor-set-3-more')).toHaveProp('accessibilityLabel', 'One more repetition');
    expect(screen.getByTestId('session-editor-profile-set-3')).toHaveTextContent('× 5');
    expect(screen.getByTestId('session-editor-profile-step-3')).toHaveProp(
      'accessibilityLabel',
      'Step 4: Effort, 3:00, Z4',
    );
    expect(screen.getByTestId('session-editor-summary')).toHaveTextContent('Run · 70 min · Z1–Z4');

    fireEvent.press(screen.getByTestId('session-editor-set-3-more'));
    expect(screen.getByTestId('session-editor-set-3-repeat')).toHaveTextContent('× 6');
    fireEvent.press(screen.getByTestId('session-editor-set-3-fewer'));
    expect(screen.getByTestId('session-editor-set-3-repeat')).toHaveTextContent('× 5');
    // Only the steps changed: the session keeps its name.
    expect(footer()).toHaveTextContent('Changes Easy run.');

    fireEvent.press(screen.getByTestId('session-editor-save'));
    await elapse(0);
    expect(mockSaveHandDay).toHaveBeenCalledWith(
      CONVERSATION_ID,
      AGENT_DAY.date,
      0,
      {
        rest: false,
        sport: 'run',
        workout: 'Easy run',
        intensity: 'Z1–Z4',
        steps: [
          { label: 'Warm-up', duration_seconds: 600, target_zone: 'Z1' },
          { label: 'Steady', duration_seconds: 1800, target_zone: 'Z2' },
          { label: 'Cool-down', duration_seconds: 300, target_zone: 'Z1' },
          { label: 'Effort', duration_seconds: 180, target_zone: 'Z4', repeat: 5, repeat_group: 1 },
          { label: 'Recovery', duration_seconds: 120, target_zone: 'Z1', repeat: 5, repeat_group: 1 },
        ],
      },
      AGENT_EXPECT,
      'en',
    );
  });

  it('moves a lone step past a whole set, the card open on it, and dissolves a set at one round', async () => {
    renderEditor(AGENT_DAY);
    await elapse(0);

    fireEvent.press(screen.getByTestId('session-editor-add-series'));
    openStep(2);
    fireEvent.press(screen.getByTestId('session-editor-step-2-down'));
    expect(screen.getByTestId('session-editor-step-4-label')).toHaveProp('value', 'Cool-down');
    expect(screen.getByTestId('session-editor-set-2')).toBeTruthy();
    expect(screen.getByTestId('session-editor-step-4-down')).toHaveProp('accessibilityState', { disabled: true });

    // Inside a set, a step moves among the set's own steps only.
    openStep(2);
    expect(screen.getByTestId('session-editor-step-2-up')).toHaveProp('accessibilityState', { disabled: true });
    fireEvent.press(screen.getByTestId('session-editor-step-2-down'));
    expect(screen.getByTestId('session-editor-step-3-label')).toHaveProp('value', 'Effort');

    for (let round = 5; round > 1; round -= 1) fireEvent.press(screen.getByTestId('session-editor-set-2-fewer'));
    expect(screen.queryByTestId('session-editor-set-2')).toBeNull();
    expect(screen.getByTestId('session-editor-step-2')).toHaveTextContent('3Recovery2:00Z1');
  });

  it("keeps a screen reader's focus on the move pressed, and on the step that takes a removed one's place", async () => {
    const focus = jest.spyOn(AccessibilityInfo, 'sendAccessibilityEvent');
    renderEditor(AGENT_DAY);
    await elapse(0);

    /** The control the last focus event landed on, by its test id. */
    const focused = () => {
      const [node, event] = focus.mock.calls[focus.mock.calls.length - 1];
      expect(event).toBe('focus');
      // Under jest the host instance is the mocked View, which carries its props.
      return (node as unknown as { props: { testID?: string } }).props.testID;
    };

    openStep(0);
    fireEvent.press(screen.getByTestId('session-editor-step-0-down'));
    expect(focused()).toBe('session-editor-step-1-down');
    // Moved to the end, its down move is spent: focus goes to its up move.
    fireEvent.press(screen.getByTestId('session-editor-step-1-down'));
    expect(focused()).toBe('session-editor-step-2-up');
    expect(screen.getByTestId('session-editor-step-2-label')).toHaveProp('value', 'Warm-up');

    // A removed step hands focus to the row that takes its place, else the one before.
    openStep(0);
    fireEvent.press(screen.getByTestId('session-editor-step-0-remove'));
    expect(focused()).toBe('session-editor-step-0');
    openStep(1);
    fireEvent.press(screen.getByTestId('session-editor-step-1-remove'));
    expect(focused()).toBe('session-editor-step-0');
    expect(screen.getByTestId('session-editor-step-0')).toHaveProp('accessibilityLabel', 'Edit step 1: Cool-down');
    focus.mockRestore();
  });

  it('adds a step to a set, repeating with it', async () => {
    renderEditor(AGENT_DAY);
    await elapse(0);

    fireEvent.press(screen.getByTestId('session-editor-add-series'));
    fireEvent.press(screen.getByTestId('session-editor-set-3-add-step'));
    expect(screen.getByTestId('session-editor-step-5-label')).toHaveProp('autoFocus', true);
    expect(within(screen.getByTestId('session-editor-set-3')).getByTestId('session-editor-step-5-open')).toBeTruthy();
    fireEvent.changeText(screen.getByTestId('session-editor-step-5-label'), 'Float');
    fireEvent.press(screen.getByTestId('session-editor-step-5-target-z2'));
    fireEvent.press(screen.getByTestId('session-editor-save'));
    await elapse(0);

    const sent = mockSaveHandDay.mock.calls[0][3] as { steps: object[] };
    expect(sent.steps[5]).toEqual({
      label: 'Float',
      duration_seconds: 300,
      target_zone: 'Z2',
      repeat: 5,
      repeat_group: 1,
    });
  });

  it('measures a step by distance, and saves the distance with its estimated time', async () => {
    renderEditor(AGENT_DAY);
    await elapse(0);

    openStep(1);
    expect(screen.getByTestId('session-editor-step-1-measure-time')).toHaveProp('accessibilityState', {
      checked: true,
    });
    fireEvent.press(screen.getByTestId('session-editor-step-1-measure-distance'));
    expect(screen.getByTestId('session-editor-step-1-distance')).toHaveProp('value', '');
    expect(screen.getByTestId('session-editor-step-1-distance')).toHaveProp('keyboardType', 'number-pad');
    expect(screen.queryByTestId('session-editor-step-1-duration-more')).toBeNull();
    expect(screen.getByTestId('session-editor-missing')).toHaveTextContent('Give step 2 a distance to save.');
    expect(screen.getByTestId('session-editor-save')).toBeDisabled();

    fireEvent.changeText(screen.getByTestId('session-editor-step-1-distance'), '5000');
    expect(screen.getByTestId('session-editor-step-1-duration')).toHaveProp('accessibilityLabel', 'Estimated time (m:ss)');
    typeStepDuration(1, '25:00');
    openStep(0);
    expect(screen.getByTestId('session-editor-step-1')).toHaveTextContent('2Steady5,000 m25:00Z2');
    fireEvent.press(screen.getByTestId('session-editor-save'));
    await elapse(0);

    const sent = mockSaveHandDay.mock.calls[0][3] as { steps: object[] };
    expect(sent.steps[1]).toEqual({ label: 'Steady', duration_seconds: 1500, target_zone: 'Z2', distance_meters: 5000 });
  });

  it('writes a zone, a percent of a threshold and an RPE from the target chips', async () => {
    renderEditor({ ...AGENT_DAY, sport: 'ride', family_sport: 'ride' });
    await elapse(0);

    openStep(0);
    const zones = within(screen.getByTestId('session-editor-step-0-target'));
    expect(zones.getAllByRole('radio').map((chip) => chip.props.accessibilityLabel)).toEqual([
      'Z1',
      'Z2',
      'Z3',
      'Z4',
      'Z5',
      'Z6',
      'Z7',
      '% of a threshold',
      'RPE',
      'Other',
    ]);
    expect(screen.getByTestId('session-editor-step-0-target')).toHaveProp('accessibilityRole', 'radiogroup');
    expect(screen.getByTestId('session-editor-step-0-target-z1')).toHaveProp('accessibilityState', { checked: true });
    fireEvent.press(screen.getByTestId('session-editor-step-0-target-z4'));
    expect(screen.getByTestId('session-editor-step-0-target-z4')).toHaveProp('accessibilityState', { checked: true });

    openStep(1);
    fireEvent.press(screen.getByTestId('session-editor-step-1-target-percent'));
    // A picked band with no value yet leaves the step without a target.
    expect(screen.getByTestId('session-editor-missing')).toHaveTextContent('Pick a target for step 2 to save.');
    // On the bike a percent starts on FTP.
    expect(screen.getByTestId('session-editor-step-1-target-anchor-ftp')).toHaveProp('accessibilityState', {
      checked: true,
    });
    fireEvent.changeText(screen.getByTestId('session-editor-step-1-target-from'), '88');
    fireEvent.changeText(screen.getByTestId('session-editor-step-1-target-to'), '93');

    openStep(2);
    fireEvent.press(screen.getByTestId('session-editor-step-2-target-rpe'));
    fireEvent.changeText(screen.getByTestId('session-editor-step-2-target-rpe-value'), '7');
    openStep(1);
    expect(screen.getByTestId('session-editor-step-1-target-percent')).toHaveProp('accessibilityState', {
      checked: true,
    });
    expect(screen.getByTestId('session-editor-step-1-target-from')).toHaveProp('value', '88');

    fireEvent.press(screen.getByTestId('session-editor-save'));
    await elapse(0);
    const sent = mockSaveHandDay.mock.calls[0][3] as { steps: { target_zone: string }[]; intensity: string };
    expect(sent.steps.map((step) => step.target_zone)).toEqual(['Z4', '88-93% FTP', 'RPE 7']);
    // Targets of mixed kinds have no span: the intensity stays as written.
    expect(sent.intensity).toBe('easy');
  });

  it('opens a target the chips do not write under "Other", as written', async () => {
    renderEditor({
      ...AGENT_DAY,
      steps: [{ label: 'Block', duration_seconds: 1200, target_zone: 'sweet spot' }],
    });
    await elapse(0);

    openStep(0);
    expect(screen.getByTestId('session-editor-step-0-target-typed')).toHaveProp('accessibilityState', { checked: true });
    expect(screen.getByTestId('session-editor-step-0-target-text')).toHaveProp('value', 'sweet spot');
    fireEvent.changeText(screen.getByTestId('session-editor-step-0-target-text'), 'tempo');
    fireEvent.press(screen.getByTestId('session-editor-save'));
    await elapse(0);
    const sent = mockSaveHandDay.mock.calls[0][3] as { steps: { target_zone: string }[] };
    expect(sent.steps[0].target_zone).toBe('tempo');
  });

  it("holds Save on the first step's gap, saying which, and points Save at the sentence", async () => {
    renderEditor(AGENT_DAY);
    await elapse(0);

    fireEvent.press(screen.getByTestId('session-editor-add-step'));
    expect(screen.getByTestId('session-editor-missing')).toHaveTextContent('Name step 4 to save.');
    expect(screen.queryByTestId('session-editor-replacement')).toBeNull();
    expect(screen.getByTestId('session-editor-save')).toBeDisabled();
    expect(screen.getByTestId('session-editor-save')).toHaveProp('accessibilityHint', 'Name step 4 to save.');

    fireEvent.changeText(screen.getByTestId('session-editor-step-3-label'), 'Strides');
    expect(screen.getByTestId('session-editor-missing')).toHaveTextContent('Pick a target for step 4 to save.');
    fireEvent.press(screen.getByTestId('session-editor-step-3-target-z5'));
    expect(screen.queryByTestId('session-editor-missing')).toBeNull();
    expect(footer()).toHaveTextContent('Changes Easy run.');
    expect(screen.getByTestId('session-editor-save')).not.toBeDisabled();
    expect(screen.getByTestId('session-editor-save').props.accessibilityHint).toBeUndefined();
  });

  it('says "Changes X." when only the steps change, and saves them', async () => {
    renderEditor(AGENT_DAY);
    await elapse(0);

    openStep(0);
    fireEvent.press(screen.getByTestId('session-editor-step-0-duration-more'));
    expect(footer()).toHaveTextContent('Changes Easy run.');
    fireEvent.press(screen.getByTestId('session-editor-step-0-duplicate'));
    expect(screen.getByTestId('session-editor-step-1-label')).toHaveProp('value', 'Warm-up');
    fireEvent.press(screen.getByTestId('session-editor-step-1-remove'));
    fireEvent.press(screen.getByTestId('session-editor-save'));
    await elapse(0);

    const sent = mockSaveHandDay.mock.calls[0][3] as { steps: { duration_seconds: number }[] };
    expect(sent.steps.map((step) => step.duration_seconds)).toEqual([615, 1800, 300]);
  });
});

describe('closing a sheet that holds changes', () => {
  /** The confirm the last close asked for: its title, body and two buttons. */
  function lastConfirm(alert: jest.SpyInstance) {
    const [title, body, buttons] = alert.mock.calls[alert.mock.calls.length - 1];
    return { title, body, buttons: buttons as { text: string; style?: string; onPress?: () => void }[] };
  }

  it('closes a clean sheet at once, by Cancel, the scrim and the back gesture alike', async () => {
    const alert = jest.spyOn(Alert, 'alert');
    const { onClose } = renderEditor(AGENT_DAY);
    await elapse(0);

    fireEvent.press(screen.getByTestId('session-editor-cancel'));
    fireEvent.press(screen.getByTestId('session-editor-scrim'));
    fireEvent(screen.UNSAFE_getByType(Modal), 'requestClose');

    expect(onClose).toHaveBeenCalledTimes(3);
    expect(alert).not.toHaveBeenCalled();
    alert.mockRestore();
  });

  it('asks before Cancel, the scrim or the back gesture discard a change, and keeps editing on request', async () => {
    const alert = jest.spyOn(Alert, 'alert');
    const { onClose } = renderEditor(AGENT_DAY);
    await elapse(0);
    fireEvent.changeText(screen.getByTestId('session-editor-workout'), 'Easy 50 min');

    fireEvent.press(screen.getByTestId('session-editor-cancel'));
    expect(alert).toHaveBeenCalledTimes(1);
    const { title, body, buttons } = lastConfirm(alert);
    expect(title).toBe('Discard your changes?');
    expect(body).toBe('Your changes to Thu, Nov 26 are not saved.');
    expect(buttons.map((b) => [b.text, b.style])).toEqual([
      ['Keep editing', 'cancel'],
      ['Discard changes', 'destructive'],
    ]);
    expect(onClose).not.toHaveBeenCalled();

    fireEvent.press(screen.getByTestId('session-editor-scrim'));
    fireEvent(screen.UNSAFE_getByType(Modal), 'requestClose');
    expect(alert).toHaveBeenCalledTimes(3);
    expect(onClose).not.toHaveBeenCalled();

    act(() => {
      lastConfirm(alert).buttons.find((b) => b.style === 'destructive')?.onPress?.();
    });
    expect(onClose).toHaveBeenCalledTimes(1);
    alert.mockRestore();
  });

  it('asks for a family build the coach changed, and not for one never touched', async () => {
    const alert = jest.spyOn(Alert, 'alert');
    const { onClose } = renderEditor(AGENT_DAY);
    await elapse(0);

    // Opened, read, left alone: the family mode holds no change of the coach's.
    fireEvent.press(screen.getByTestId('session-editor-mode-family'));
    await elapse(0);
    fireEvent.press(screen.getByTestId('session-editor-cancel'));
    expect(alert).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalledTimes(1);

    fireEvent.press(screen.getByTestId('session-editor-family-vo2max_short'));
    fireEvent.press(screen.getByTestId('session-editor-cancel'));
    expect(alert).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
    alert.mockRestore();
  });

  it('closes without asking once a change is saved', async () => {
    const alert = jest.spyOn(Alert, 'alert');
    const { onClose } = renderEditor(AGENT_DAY);
    await elapse(0);
    fireEvent.changeText(screen.getByTestId('session-editor-workout'), 'Easy 50 min');

    fireEvent.press(screen.getByTestId('session-editor-save'));
    await elapse(0);

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(alert).not.toHaveBeenCalled();
    alert.mockRestore();
  });
});

describe('an edited day', () => {
  it('says the coach edited it and what "Back to the original" puts back', async () => {
    renderEditor(EDITED_DAY);
    await elapse(0);

    expect(screen.getByTestId('session-editor-edited')).toHaveTextContent(/You edited this day in your draft\./);
    // The glyph ahead of the words is a font character; the control is named by its words alone.
    expect(screen.getByTestId('session-editor-restore')).toHaveTextContent(/Back to the original$/);
    expect(screen.getByTestId('session-editor-restore')).toHaveProp('accessibilityLabel', 'Back to the original');
    expect(screen.getByTestId('session-editor-restore-note')).toHaveTextContent(
      'Puts back Easy run, as the day stood before your first edit.',
    );
    expect(screen.getByTestId('session-editor-restore')).toHaveProp(
      'accessibilityHint',
      'Puts back Easy run, as the day stood before your first edit.',
    );
  });

  it('names the rest day a restore puts back', async () => {
    renderEditor({ ...AGENT_DAY, edited: { workout: 'Rest', rest: true } });
    await elapse(0);

    expect(screen.getByTestId('session-editor-restore-note')).toHaveTextContent(
      'Puts back the rest day, as the day stood before your first edit.',
    );
  });

  it('asks first, then restores through the restore mutation, closes, and says so', async () => {
    const alert = jest.spyOn(Alert, 'alert');
    const { onClose } = renderEditor(EDITED_DAY);
    await elapse(0);

    fireEvent.press(screen.getByTestId('session-editor-restore'));
    // Going back throws every edit away: nothing is restored until the coach says so.
    expect(mockRestore).not.toHaveBeenCalled();
    const [title, body, buttons] = alert.mock.calls[alert.mock.calls.length - 1] as [
      string,
      string,
      { text: string; style?: string; onPress?: () => void }[],
    ];
    expect(title).toBe('Discard your edits to this session?');
    expect(body).toBe('Puts back Easy run, as the day stood before your first edit.');
    expect(buttons.map((b) => [b.text, b.style])).toEqual([
      ['Keep editing', 'cancel'],
      ['Discard changes', 'destructive'],
    ]);
    await act(async () => {
      buttons.find((b) => b.style === 'destructive')?.onPress?.();
      await jest.advanceTimersByTimeAsync(0);
    });
    alert.mockRestore();

    expect(mockRestore).toHaveBeenCalledWith(
      CONVERSATION_ID,
      '2026-11-26',
      0,
      { sport: 'run', workout: 'Easy 50 min' },
      'en',
    );
    expect(mockSaveHandDay).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(Toast.show).toHaveBeenCalledWith({ type: 'success', text1: 'Thu, Nov 26 is back as it was in your draft.' });
  });

  it('restores nothing when the coach keeps editing', async () => {
    const alert = jest.spyOn(Alert, 'alert');
    const { onClose } = renderEditor(EDITED_DAY);
    await elapse(0);

    fireEvent.press(screen.getByTestId('session-editor-restore'));
    const [, , buttons] = alert.mock.calls[alert.mock.calls.length - 1] as [
      string,
      string,
      { style?: string; onPress?: () => void }[],
    ];
    act(() => {
      buttons.find((b) => b.style === 'cancel')?.onPress?.();
    });
    await elapse(0);

    expect(mockRestore).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    alert.mockRestore();
  });

  it('says a refused restore in the sheet and stays open', async () => {
    mockRestore.mockRejectedValueOnce(new Error('refused'));
    const alert = jest.spyOn(Alert, 'alert');
    const { onClose } = renderEditor(EDITED_DAY);
    await elapse(0);

    fireEvent.press(screen.getByTestId('session-editor-restore'));
    await confirmLast(alert);
    alert.mockRestore();

    expect(screen.getByTestId('session-editor-restore-failed')).toHaveTextContent(/^This day could not be restored\..+/);
    expect(onClose).not.toHaveBeenCalled();
    expect(Toast.show).not.toHaveBeenCalled();
  });
});

describe('the family mode', () => {
  it("opens a family day on its family, format, sport and values, under the day's date", async () => {
    renderEditor();
    await elapse(0);

    expect(mockGetFamilies).toHaveBeenCalledWith(CONVERSATION_ID, 'en');
    expect(screen.getByTestId('session-editor-title')).toHaveTextContent('Edit Tue, Nov 24');
    expect(screen.getByTestId('session-editor-mode-family')).toHaveProp('accessibilityState', { checked: true });
    expect(screen.getByTestId('session-editor-family-vo2max_short')).toHaveProp('accessibilityState', { checked: true });
    expect(screen.getByTestId('session-editor-family-vo2max_short')).toHaveProp('accessibilityRole', 'radio');
    expect(screen.getByTestId('session-editor-format-ronnestad_30_15')).toHaveProp('accessibilityState', {
      checked: true,
    });
    // The sport is the sheet's chips: no second sport row in the family form.
    expect(screen.getByTestId('session-editor-sport-run')).toHaveProp('accessibilityState', { selected: true });
    expect(within(screen.getByTestId('session-editor-form')).queryByText('Sport')).toBeNull();
    // The header is the format, which the coach does not edit.
    expect(screen.queryByTestId('session-editor-workout')).toBeNull();
    expect(screen.getByTestId('session-editor-preset-name')).toHaveTextContent('30/15 (Rønnestad)');
    // The two values the day states, and the format's defaults for the rest.
    expect(screen.getByTestId('session-editor-param-sets')).toHaveProp('value', '3');
    expect(screen.getByTestId('session-editor-param-reps')).toHaveProp('value', '12');
    expect(screen.getByTestId('session-editor-param-set_rest_seconds')).toHaveProp('value', '180');
    expect(screen.getByTestId('session-editor-range-set_rest_seconds')).toHaveTextContent('Tested range 120–300');
    expect(screen.queryByTestId('session-editor-param-duration_minutes')).toBeNull();
    expect(screen.getByText('Short work, shorter rest, three sets.')).toBeTruthy();
    expect(screen.getByText('VO2max, short intervals')).toBeTruthy();
    expect(screen.getByText('Format')).toBeTruthy();
    // The day already holds this build: nothing to save until it changes.
    expect(footer()).toHaveTextContent('No changes yet.');
    expect(screen.getByTestId('session-editor-save')).toBeDisabled();
  });

  it("filters the families by the day's sport, and follows the sport the coach picks", async () => {
    mockGetFamilies.mockResolvedValue(SPORT_CATALOGUE);
    renderEditor(AGENT_DAY);
    await elapse(0);

    fireEvent.press(screen.getByTestId('session-editor-mode-family'));
    await elapse(0);

    expect(screen.getByTestId('session-editor-sport-run')).toHaveProp('accessibilityState', { selected: true });
    expect(screen.getByTestId('session-editor-family-endurance_steady')).toHaveProp('accessibilityState', {
      checked: true,
    });
    expect(screen.getByTestId('session-editor-family-vo2max_short')).toBeTruthy();
    expect(screen.queryByTestId('session-editor-family-sweet_spot')).toBeNull();
    expect(screen.queryByTestId('session-editor-family-swim_css')).toBeNull();

    fireEvent.press(screen.getByTestId('session-editor-sport-swim'));
    expect(screen.getByTestId('session-editor-family-swim_css')).toHaveProp('accessibilityState', { checked: true });
    expect(screen.queryByTestId('session-editor-family-endurance_steady')).toBeNull();
    expect(screen.getByTestId('session-editor-param-reps')).toHaveProp('value', '10');

    fireEvent.press(screen.getByTestId('session-editor-sport-ride'));
    expect(screen.getByTestId('session-editor-family-sweet_spot')).toBeTruthy();
    expect(screen.queryByTestId('session-editor-family-swim_css')).toBeNull();
    // The 30/30 has no ride form, so the ride formats leave it out.
    fireEvent.press(screen.getByTestId('session-editor-family-vo2max_short'));
    expect(screen.getByTestId('session-editor-format-ronnestad_30_15')).toBeTruthy();
    expect(screen.queryByTestId('session-editor-format-billat_30_30')).toBeNull();
  });

  it("saves a family build over the agent's session through the family mutation, naming what it replaces", async () => {
    const { onClose } = renderEditor(AGENT_DAY);
    await elapse(0);

    fireEvent.press(screen.getByTestId('session-editor-mode-family'));
    await elapse(0);
    // A day no family built is replaced by any build, the one it opens on included.
    expect(footer()).toHaveTextContent('Replaces Easy run with Steady.');
    fireEvent.press(screen.getByTestId('session-editor-save'));
    await elapse(0);

    expect(mockSaveDay).toHaveBeenCalledWith(
      CONVERSATION_ID,
      '2026-11-26',
      0,
      { family: 'endurance_steady', preset: 'steady', sport: 'run', params: { duration_minutes: 90 } },
      AGENT_EXPECT,
      'en',
    );
    expect(mockSaveHandDay).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('opens a rest day on its session, and on the first family at its defaults in the family mode', async () => {
    renderEditor(REST_DAY);
    await elapse(0);

    // A rest day is a session the coach writes; the family mode is under the switch.
    expect(screen.queryByTestId('session-editor-mode')).toBeNull();
    fireEvent(screen.getByTestId('session-editor-rest'), 'valueChange', false);
    fireEvent.press(screen.getByTestId('session-editor-mode-family'));
    await elapse(0);
    expect(screen.getByTestId('session-editor-family-endurance_steady')).toHaveProp('accessibilityState', {
      checked: true,
    });
    expect(screen.getByTestId('session-editor-param-duration_minutes')).toHaveProp('value', '90');
    expect(footer()).toHaveTextContent('Replaces the rest day with Steady.');
  });

  it('previews the build once typing settles, and keeps the last preview while the next is measured', async () => {
    renderEditor();
    await elapse(0);
    expect(mockPreview).not.toHaveBeenCalled();

    await elapse(300);
    expect(mockPreview).toHaveBeenCalledTimes(1);
    expect(mockPreview).toHaveBeenCalledWith(
      CONVERSATION_ID,
      {
        date: '2026-11-24',
        family: 'vo2max_short',
        preset: 'ronnestad_30_15',
        sport: 'run',
        params: { sets: 3, reps: 12, work_seconds: 30, rest_seconds: 15, set_rest_seconds: 180 },
      },
      'en',
    );
    expect(screen.getByTestId('session-editor-anchor')).toHaveTextContent('Anchor: vVO2max 4:05/km, your own.');
    expect(screen.getByTestId('session-editor-derived-on_time')).toHaveTextContent('Time at work18 min');
    expect(screen.getByTestId('session-editor-derived-work_pct')).toHaveTextContent('Work, % of vVO2max100 %');
    expect(within(screen.getByTestId('session-editor-steps')).getByText(/30\/15/)).toBeTruthy();
    expect(screen.getByTestId('session-editor-findings-none')).toHaveTextContent('Inside the tested range.');

    // The next answer is held back, so the measuring state is on screen.
    let answer: (preview: SessionPreview) => void = () => undefined;
    mockPreview.mockImplementationOnce(
      () =>
        new Promise<SessionPreview>((resolve) => {
          answer = resolve;
        }),
    );
    fireEvent.changeText(screen.getByTestId('session-editor-param-sets'), '4');
    await elapse(299);
    expect(mockPreview).toHaveBeenCalledTimes(1);

    await elapse(1);
    expect(mockPreview).toHaveBeenCalledTimes(2);
    expect(lastPreviewed()).toMatchObject({ params: { sets: 4, reps: 12 } });
    expect(screen.getByTestId('session-editor-measuring')).toHaveTextContent('Measuring…');
    expect(screen.getByTestId('session-editor-derived-on_time')).toHaveTextContent('Time at work18 min');

    await act(async () => {
      answer(previewOf(4));
      await jest.advanceTimersByTimeAsync(0);
    });
    expect(screen.queryByTestId('session-editor-measuring')).toBeNull();
    expect(screen.getByTestId('session-editor-derived-on_time')).toHaveTextContent('Time at work24 min');
  });

  it('keeps a value outside its range as typed, shows the findings, and still saves it', async () => {
    mockPreview.mockImplementation((_id: string, request: { params?: { sets?: number } }) => {
      const sets = request.params?.sets ?? 3;
      return Promise.resolve(previewOf(sets, sets > 4 ? [ABOVE_RANGE] : []));
    });
    const { onClose } = renderEditor();
    await elapse(300);
    expect(screen.queryByTestId('session-editor-outside-sets')).toBeNull();

    fireEvent.changeText(screen.getByTestId('session-editor-param-sets'), '6');
    await elapse(300);

    // Never clamped: the field holds six, and the range line warns.
    expect(screen.getByTestId('session-editor-param-sets')).toHaveProp('value', '6');
    expect(screen.getByTestId('session-editor-outside-sets')).toBeTruthy();
    expect(lastPreviewed()).toMatchObject({ params: { sets: 6 } });
    expect(screen.getByTestId('session-editor-findings')).toHaveTextContent(ABOVE_RANGE.text);
    expect(screen.getByTestId('session-editor-findings-note')).toHaveTextContent(
      'Findings are warnings: the session saves as you built it.',
    );
    expect(screen.getByTestId('session-editor-outside-evidence')).toHaveTextContent('Outside the evidence');
    expect(footer()).toHaveTextContent('Replaces 3 × 12 × 30/15 with 30/15 (Rønnestad).');

    // A finding never holds Save back (D3).
    const save = screen.getByTestId('session-editor-save');
    expect(save).toHaveTextContent('Save to draft');
    expect(save).not.toBeDisabled();
    fireEvent.press(save);
    await elapse(0);

    expect(mockSaveDay).toHaveBeenCalledWith(
      CONVERSATION_ID,
      '2026-11-24',
      0,
      {
        family: 'vo2max_short',
        preset: 'ronnestad_30_15',
        sport: 'run',
        params: { sets: 6, reps: 12, work_seconds: 30, rest_seconds: 15, set_rest_seconds: 180 },
      },
      FAMILY_EXPECT,
      'en',
    );
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('saves the picked format into the draft, closes, and says so', async () => {
    const { onClose } = renderEditor();
    await elapse(0);

    fireEvent.press(screen.getByTestId('session-editor-format-billat_30_30'));
    // The new format starts at its defaults in the day's sport.
    expect(screen.getByTestId('session-editor-param-reps')).toHaveProp('value', '15');
    expect(screen.queryByTestId('session-editor-param-sets')).toBeNull();
    expect(footer()).toHaveTextContent('Replaces 3 × 12 × 30/15 with 30/30 (Billat).');

    fireEvent.press(screen.getByTestId('session-editor-save'));
    expect(screen.getByTestId('session-editor-save')).toHaveTextContent('Saving…');
    await elapse(0);

    expect(mockSaveDay).toHaveBeenCalledWith(
      CONVERSATION_ID,
      '2026-11-24',
      0,
      { family: 'vo2max_short', preset: 'billat_30_30', sport: 'run', params: { reps: 15 } },
      FAMILY_EXPECT,
      'en',
    );
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(Toast.show).toHaveBeenCalledWith({
      type: 'success',
      text1: 'Saved to your draft. Chloé Tremblay sees it only once you publish.',
    });
  });

  it('says a refused save in the sheet and stays open for another try', async () => {
    mockSaveDay.mockRejectedValueOnce(new Error('refused'));
    const { onClose } = renderEditor();
    await elapse(0);

    fireEvent.changeText(screen.getByTestId('session-editor-param-sets'), '4');
    fireEvent.press(screen.getByTestId('session-editor-save'));
    await elapse(0);

    expect(screen.getByTestId('session-editor-save-failed')).toHaveTextContent(/^This session could not be saved\..+/);
    expect(onClose).not.toHaveBeenCalled();
    expect(Toast.show).not.toHaveBeenCalled();
    expect(screen.getByTestId('session-editor-save')).not.toBeDisabled();
  });

  it('says a preview the server could not measure, and still lets the coach save', async () => {
    mockPreview.mockRejectedValue(new Error('unmeasurable'));
    renderEditor();
    await elapse(300);

    expect(screen.getByTestId('session-editor-preview-failed')).toHaveTextContent('This session could not be measured.');
    expect(screen.queryByTestId('session-editor-measuring')).toBeNull();
    fireEvent.changeText(screen.getByTestId('session-editor-param-sets'), '4');
    expect(screen.getByTestId('session-editor-save')).not.toBeDisabled();
  });

  it("offers no family mode when the athlete's agent has no family, and the day edits by hand", async () => {
    mockGetFamilies.mockResolvedValue({ families: [] });
    renderEditor();
    await elapse(0);

    expect(screen.queryByTestId('session-editor-mode')).toBeNull();
    expect(screen.queryByTestId('session-editor-no-families')).toBeNull();
    expect(screen.getByTestId('session-editor-workout')).toHaveProp('value', '3 × 12 × 30/15');
    expect(screen.getByTestId('session-editor-save')).toBeDisabled();
    expect(screen.getByTestId('session-editor-cancel')).toHaveTextContent('Cancel');
  });

  it('says when the families could not be read, and the session mode still edits', async () => {
    mockGetFamilies.mockRejectedValue(new Error('down'));
    renderEditor();
    await elapse(0);

    expect(screen.getByTestId('session-editor-load-failed')).toHaveTextContent(
      'The workout families could not be loaded.',
    );
    expect(mockPreview).not.toHaveBeenCalled();

    fireEvent.press(screen.getByTestId('session-editor-mode-session'));
    expect(screen.queryByTestId('session-editor-load-failed')).toBeNull();
    expect(screen.getByTestId('session-editor-workout')).toHaveProp('value', '3 × 12 × 30/15');
  });
});

/** A fortnight's first days: a family session, a rest day, the agent's session and the coach's edit of one. */
const PLAN: WorkoutPlan = {
  goal_race: { name: 'Harricana 65K', date: '2027-09-11', discipline: 'trail', priority: 'A' },
  phases: [],
  weeks: [
    {
      week_start: '2026-11-23',
      focus: 'build',
      current: true,
      days: [
        { ...FAMILY_DAY, edited: { workout: 'Easy run', rest: false } },
        REST_DAY,
        EDITED_DAY,
        { ...AGENT_DAY, date: '2026-11-27' },
      ],
    },
  ],
  weeks_deferred: 0,
};

describe('"Edit day" on the plan card', () => {
  it('ends every day, the rest day included, when the host hands the editor', () => {
    const onBuildDay = jest.fn();
    render(<WorkoutPlanCard plan={PLAN} onBuildDay={onBuildDay} />);

    const rest = screen.getByTestId('workout-plan-day-edit-2026-11-25-0');
    expect(rest).toHaveTextContent('Edit day');
    expect(rest).toHaveProp('accessibilityLabel', 'Edit day');
    expect(rest).toHaveProp('accessibilityHint', 'Edit Wed, Nov 25');
    fireEvent.press(screen.getByTestId('workout-plan-day-edit-2026-11-26-0'));
    expect(onBuildDay).toHaveBeenCalledWith({ day: EDITED_DAY, entries: [EDITED_DAY], mode: 'edit' });
  });

  it('is on no day of a card whose host hands no editor', () => {
    render(<WorkoutPlanCard plan={PLAN} />);
    expect(screen.queryByText('Edit day')).toBeNull();
  });

  it("marks the days the coach edited, and says a hand-written one is not checked against a family's evidence", () => {
    render(<WorkoutPlanCard plan={PLAN} />);

    expect(screen.getByTestId('workout-plan-day-edited-2026-11-26-0')).toHaveTextContent('Edited');
    expect(screen.getByTestId('workout-plan-day-unchecked-2026-11-26-0')).toHaveTextContent(
      "Not checked against a family's evidence.",
    );
    // A day the coach rebuilt from a family is checked against it.
    expect(screen.getByTestId('workout-plan-day-edited-2026-11-24-0')).toHaveTextContent('Edited');
    expect(screen.queryByTestId('workout-plan-day-unchecked-2026-11-24-0')).toBeNull();
    // The agent's own days carry neither.
    expect(screen.queryByTestId('workout-plan-day-edited-2026-11-27-0')).toBeNull();
    expect(screen.queryByTestId('workout-plan-day-unchecked-2026-11-27-0')).toBeNull();
    expect(screen.queryByTestId('workout-plan-day-edited-2026-11-25-0')).toBeNull();
  });

  // carnet#1814: a day the coach kept as their call says so for every reader,
  // the athlete's card included; a day they did not call carries no mark.
  it("marks a day the coach called as the coach's call, and no other", () => {
    const called: WorkoutPlan = {
      ...PLAN,
      weeks: [{ ...PLAN.weeks[0], days: [REST_DAY, { ...AGENT_DAY, coach_call: true }] }],
    };
    render(<WorkoutPlanCard plan={called} />);

    const row = within(screen.getByTestId('workout-plan-day-2026-11-26-0'));
    expect(row.getByTestId('plan-day-coach-call')).toHaveTextContent(i18n.t('humanCoach.coachCall.badge'));
    expect(screen.getAllByTestId('plan-day-coach-call')).toHaveLength(1);
  });

  it('reaches the card a transcript row draws, through the list', () => {
    const row: Message = {
      id: 'msg-plan-1',
      role: 'assistant',
      content: 'Here is her fortnight.',
      created_at: '2026-11-21T10:00:00Z',
      scene_blocks: JSON.stringify([{ kind: 'workout_plan', source_tool: 'save_training_plan', plan: PLAN }]),
    } as Message;
    const onBuildPlanDay = jest.fn();
    render(
      <QueryClientProvider client={newClient()}>
        <MessageList
          conversationId={CONVERSATION_ID}
          messages={[row]}
          planSubjectName="Chloé Tremblay"
          isLoading={false}
          isSending={false}
          messageFeedback={{}}
          messageFeedbackComment={{}}
          messageBlocks={{}}
          flatListRef={React.createRef()}
          onScrollToBottom={jest.fn()}
          onThumbsUp={jest.fn()}
          onThumbsDown={jest.fn()}
          onSubmitFeedbackReason={jest.fn()}
          onRetryMessage={jest.fn()}
          onOpenUrl={jest.fn()}
          onReconnectProvider={jest.fn()}
          onBuildPlanDay={onBuildPlanDay}
        />
      </QueryClientProvider>,
    );

    fireEvent.press(screen.getByTestId('workout-plan-day-edit-2026-11-25-0'));
    expect(onBuildPlanDay).toHaveBeenCalledWith({ day: REST_DAY, entries: [REST_DAY], mode: 'edit' });
  });
});

/** The agent's run and the coach's added core circuit, on one date. */
const TWO_SESSIONS: PlanDay[] = [{ ...AGENT_DAY, position: 0 }, ADDED_DAY];

describe('a date that holds two sessions, on the plan card', () => {
  const plan: WorkoutPlan = {
    ...PLAN,
    weeks: [{ ...PLAN.weeks[0], days: [REST_DAY, ...TWO_SESSIONS] }],
  };

  it('draws each session as its own row, the date on the first only', () => {
    render(<WorkoutPlanCard plan={plan} />);

    expect(screen.getByTestId('workout-plan-day-2026-11-26-0')).toHaveTextContent(/^Thu, Nov 26Easy run/);
    expect(screen.getByTestId('workout-plan-day-2026-11-26-1')).toHaveTextContent(/^Core circuit/);
    expect(screen.getAllByText('Thu, Nov 26')).toHaveLength(1);
    // The session the coach added says so, and that no family checked it.
    expect(screen.getByTestId('workout-plan-day-added-2026-11-26-1')).toHaveTextContent('Added');
    expect(screen.getByTestId('workout-plan-day-unchecked-2026-11-26-1')).toHaveTextContent(
      "Not checked against a family's evidence.",
    );
    expect(screen.queryByTestId('workout-plan-day-added-2026-11-26-0')).toBeNull();
    expect(screen.queryByTestId('workout-plan-day-unchecked-2026-11-26-0')).toBeNull();
  });

  it('names each edit "Edit session", and offers "Add a session" on the date\'s last entry only', () => {
    const onBuildDay = jest.fn();
    render(<WorkoutPlanCard plan={plan} onBuildDay={onBuildDay} />);

    const first = screen.getByTestId('workout-plan-day-edit-2026-11-26-0');
    expect(first).toHaveTextContent('Edit session');
    expect(first).toHaveProp('accessibilityLabel', 'Edit session');
    expect(first).toHaveProp('accessibilityHint', 'Edit Thu, Nov 26 · Easy run');
    expect(screen.getByTestId('workout-plan-day-edit-2026-11-26-1')).toHaveProp(
      'accessibilityHint',
      'Edit Thu, Nov 26 · Core circuit',
    );
    // A date with one entry keeps "Edit day".
    expect(screen.getByTestId('workout-plan-day-edit-2026-11-25-0')).toHaveTextContent('Edit day');

    // One "Add a session" per date, under its last entry.
    const add = screen.getByTestId('workout-plan-day-add-2026-11-26');
    expect(within(screen.getByTestId('workout-plan-day-2026-11-26-1')).getByTestId('workout-plan-day-add-2026-11-26')).toBe(
      add,
    );
    expect(add).toHaveTextContent('Add a session');
    expect(add).toHaveProp('accessibilityHint', 'Add a session on Thu, Nov 26');
    expect(screen.getAllByText('Add a session')).toHaveLength(2);

    fireEvent.press(screen.getByTestId('workout-plan-day-edit-2026-11-26-1'));
    expect(onBuildDay).toHaveBeenLastCalledWith({ day: ADDED_DAY, entries: TWO_SESSIONS, mode: 'edit' });
    fireEvent.press(add);
    expect(onBuildDay).toHaveBeenLastCalledWith({ day: ADDED_DAY, entries: TWO_SESSIONS, mode: 'add' });
  });

  it('offers no "Add a session" on a date already holding five entries', () => {
    const full = Array.from({ length: 5 }, (_, position) => ({
      ...AGENT_DAY,
      position,
      workout: `Session ${position + 1}`,
    }));
    render(<WorkoutPlanCard plan={{ ...PLAN, weeks: [{ ...PLAN.weeks[0], days: full }] }} onBuildDay={jest.fn()} />);

    expect(screen.getByTestId('workout-plan-day-edit-2026-11-26-4')).toHaveTextContent('Edit session');
    expect(screen.queryByTestId('workout-plan-day-add-2026-11-26')).toBeNull();
  });
});

describe('adding a session to a date', () => {
  it('opens on a blank session in the date\'s sport, with no Rest switch and no footer sentence', async () => {
    renderEditor(AGENT_DAY, { mode: 'add' });
    await elapse(0);

    expect(screen.getByTestId('session-editor-title')).toHaveTextContent('Add a session on Thu, Nov 26');
    expect(screen.getByTestId('session-editor-mode-session')).toHaveProp('accessibilityState', { checked: true });
    expect(screen.queryByTestId('session-editor-rest')).toBeNull();
    expect(screen.getByTestId('session-editor-workout')).toHaveProp('value', '');
    expect(screen.getByTestId('session-editor-sport-run')).toHaveProp('accessibilityState', { selected: true });
    expect(screen.getByTestId('session-editor-intensity')).toHaveProp('value', '');
    expect(screen.getByTestId('session-editor-no-steps')).toBeTruthy();
    // A new session replaces nothing, and is neither edited nor removable yet.
    expect(screen.queryByTestId('session-editor-replacement')).toBeNull();
    expect(screen.queryByTestId('session-editor-edited')).toBeNull();
    expect(screen.queryByTestId('session-editor-remove')).toBeNull();
    // Save waits for the session to be whole, and says so: a title, at least.
    expect(screen.getByTestId('session-editor-missing')).toHaveTextContent('Add a title to save.');
    expect(screen.getByTestId('session-editor-save')).toBeDisabled();
  });

  it('saves a written session through the add, closes, and says so', async () => {
    const { onClose } = renderEditor(AGENT_DAY, { mode: 'add' });
    await elapse(0);

    fireEvent.changeText(screen.getByTestId('session-editor-workout'), 'Core circuit');
    fireEvent.press(screen.getByTestId('session-editor-sport-ride'));
    fireEvent.changeText(screen.getByTestId('session-editor-duration'), '20');
    expect(screen.queryByTestId('session-editor-replacement')).toBeNull();
    fireEvent.press(screen.getByTestId('session-editor-save'));
    await elapse(0);

    expect(mockAdd).toHaveBeenCalledWith(
      CONVERSATION_ID,
      '2026-11-26',
      { day: { rest: false, sport: 'ride', workout: 'Core circuit', duration_min: 20, intensity: '', steps: [] } },
      'en',
    );
    expect(mockSaveHandDay).not.toHaveBeenCalled();
    expect(mockSaveDay).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(Toast.show).toHaveBeenCalledWith({
      type: 'success',
      text1: 'Saved to your draft. Chloé Tremblay sees it only once you publish.',
    });
  });

  it("saves a family build through the add once the build exists, in the date's sport", async () => {
    const { onClose } = renderEditor(AGENT_DAY, { mode: 'add' });
    await elapse(0);

    fireEvent.press(screen.getByTestId('session-editor-mode-family'));
    await elapse(0);
    expect(screen.queryByTestId('session-editor-replacement')).toBeNull();
    const save = screen.getByTestId('session-editor-save');
    expect(save).not.toBeDisabled();
    fireEvent.press(save);
    await elapse(0);

    expect(mockAdd).toHaveBeenCalledWith(
      CONVERSATION_ID,
      '2026-11-26',
      { build: { family: 'endurance_steady', preset: 'steady', sport: 'run', params: { duration_minutes: 90 } } },
      'en',
    );
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('says the date is full when another write filled it meanwhile, and stays open', async () => {
    mockAdd.mockRejectedValueOnce(
      Object.assign(new Error('Request failed with status code 422'), {
        isAxiosError: true,
        response: {
          status: 422,
          data: { error: { code: 'unprocessable', message: 'date full' }, details: { reason: 'date_full' } },
        },
      }),
    );
    const { onClose } = renderEditor(AGENT_DAY, { mode: 'add' });
    await elapse(0);

    fireEvent.changeText(screen.getByTestId('session-editor-workout'), 'Core circuit');
    fireEvent.press(screen.getByTestId('session-editor-save'));
    await elapse(0);

    expect(screen.getByTestId('session-editor-save-failed')).toHaveTextContent(
      'This session could not be saved.Thu, Nov 26 already holds 5 sessions.',
    );
    expect(onClose).not.toHaveBeenCalled();
    expect(Toast.show).not.toHaveBeenCalled();
  });

  it('asks before closing once the coach wrote something, and not before', async () => {
    const alert = jest.spyOn(Alert, 'alert');
    const { onClose } = renderEditor(AGENT_DAY, { mode: 'add' });
    await elapse(0);

    fireEvent.press(screen.getByTestId('session-editor-cancel'));
    expect(alert).not.toHaveBeenCalled();
    expect(onClose).toHaveBeenCalledTimes(1);

    fireEvent.changeText(screen.getByTestId('session-editor-workout'), 'Core');
    fireEvent.press(screen.getByTestId('session-editor-cancel'));
    expect(alert).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
    alert.mockRestore();
  });
});

describe('removing a session from a date', () => {
  /** The confirm the last remove asked for: its title, body and two buttons. */
  function lastConfirm(alert: jest.SpyInstance) {
    const [title, body, buttons] = alert.mock.calls[alert.mock.calls.length - 1];
    return { title, body, buttons: buttons as { text: string; style?: string; onPress?: () => void }[] };
  }

  it("is offered only on a date that holds more than one entry: the last one changes with the Rest switch", async () => {
    renderEditor(AGENT_DAY);
    await elapse(0);
    expect(screen.queryByTestId('session-editor-remove')).toBeNull();
  });

  it('asks first, then removes that session by position and as the card showed it, closes, and says so', async () => {
    const alert = jest.spyOn(Alert, 'alert');
    const { onClose } = renderEditor(ADDED_DAY, { entries: TWO_SESSIONS });
    await elapse(0);

    const remove = screen.getByTestId('session-editor-remove');
    expect(remove).toHaveProp('accessibilityLabel', 'Remove this session');
    fireEvent.press(remove);
    expect(mockRemove).not.toHaveBeenCalled();
    const { title, body, buttons } = lastConfirm(alert);
    expect(title).toBe('Remove this session?');
    expect(body).toBe('Core circuit leaves Thu, Nov 26 in your draft.');
    expect(buttons.map((b) => [b.text, b.style])).toEqual([
      ['Cancel', 'cancel'],
      ['Remove session', 'destructive'],
    ]);

    await act(async () => {
      buttons.find((b) => b.style === 'destructive')?.onPress?.();
      await jest.advanceTimersByTimeAsync(0);
    });

    expect(mockRemove).toHaveBeenCalledWith(
      CONVERSATION_ID,
      '2026-11-26',
      1,
      { sport: 'strength', workout: 'Core circuit' },
      'en',
    );
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(Toast.show).toHaveBeenCalledWith({ type: 'success', text1: 'Core circuit is off Thu, Nov 26 in your draft.' });
    alert.mockRestore();
  });

  it('removes nothing when the coach cancels', async () => {
    const alert = jest.spyOn(Alert, 'alert');
    const { onClose } = renderEditor(TWO_SESSIONS[0], { entries: TWO_SESSIONS });
    await elapse(0);

    fireEvent.press(screen.getByTestId('session-editor-remove'));
    act(() => {
      lastConfirm(alert).buttons.find((b) => b.style === 'cancel')?.onPress?.();
    });
    await elapse(0);

    expect(mockRemove).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    alert.mockRestore();
  });
});

describe('a session the coach added', () => {
  it('opens with "Back to the original" saying it removes the session, asks as a remove does, and removes it through the restore', async () => {
    const alert = jest.spyOn(Alert, 'alert');
    const { onClose } = renderEditor(ADDED_DAY, { entries: TWO_SESSIONS });
    await elapse(0);

    expect(screen.getByTestId('session-editor-edited')).not.toHaveTextContent(/You edited this day/);
    expect(screen.getByTestId('session-editor-restore-note')).toHaveTextContent('Removes this session: you added it.');
    fireEvent.press(screen.getByTestId('session-editor-restore'));
    expect(mockRestore).not.toHaveBeenCalled();
    const [title, body, buttons] = alert.mock.calls[alert.mock.calls.length - 1] as [
      string,
      string,
      { text: string; style?: string }[],
    ];
    expect(title).toBe('Remove this session?');
    expect(body).toBe('Core circuit leaves Thu, Nov 26 in your draft.');
    expect(buttons.map((b) => [b.text, b.style])).toEqual([
      ['Cancel', 'cancel'],
      ['Remove session', 'destructive'],
    ]);
    await confirmLast(alert);
    alert.mockRestore();

    expect(mockRestore).toHaveBeenCalledWith(
      CONVERSATION_ID,
      '2026-11-26',
      1,
      { sport: 'strength', workout: 'Core circuit' },
      'en',
    );
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(Toast.show).toHaveBeenCalledWith({ type: 'success', text1: 'Core circuit is off Thu, Nov 26 in your draft.' });
  });
});

describe('a write whose session moved since the card showed it', () => {
  it('reads the plan again, closes, says so, and never retries onto another session', async () => {
    mockSaveHandDay.mockRejectedValueOnce(SESSION_MOVED);
    const { onClose, client } = renderEditor(ADDED_DAY, { entries: TWO_SESSIONS });
    const invalidate = jest.spyOn(client, 'invalidateQueries');
    await elapse(0);

    fireEvent.changeText(screen.getByTestId('session-editor-workout'), 'Core and hips');
    fireEvent.press(screen.getByTestId('session-editor-save'));
    await elapse(0);

    expect(mockSaveHandDay).toHaveBeenCalledTimes(1);
    expect(invalidate).toHaveBeenCalledWith({ queryKey: QUERY_KEYS.chat.subjectPlan(CONVERSATION_ID) });
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(Toast.show).toHaveBeenCalledWith({
      type: 'info',
      text1: 'This day changed since you opened it. Here it is as it stands now.',
    });
    expect(screen.queryByTestId('session-editor-save-failed')).toBeNull();
  });

  it('does the same for a remove', async () => {
    mockRemove.mockRejectedValueOnce(SESSION_MOVED);
    const alert = jest.spyOn(Alert, 'alert');
    const { onClose, client } = renderEditor(ADDED_DAY, { entries: TWO_SESSIONS });
    const invalidate = jest.spyOn(client, 'invalidateQueries');
    await elapse(0);

    fireEvent.press(screen.getByTestId('session-editor-remove'));
    const [, , buttons] = alert.mock.calls[alert.mock.calls.length - 1];
    await act(async () => {
      (buttons as { style?: string; onPress?: () => void }[]).find((b) => b.style === 'destructive')?.onPress?.();
      await jest.advanceTimersByTimeAsync(0);
    });

    expect(mockRemove).toHaveBeenCalledTimes(1);
    expect(invalidate).toHaveBeenCalledWith({ queryKey: QUERY_KEYS.chat.subjectPlan(CONVERSATION_ID) });
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(screen.queryByTestId('session-editor-remove-failed')).toBeNull();
    alert.mockRestore();
  });

  it('keeps any other refused remove in the sheet, which stays open', async () => {
    mockRemove.mockRejectedValueOnce(new Error('refused'));
    const alert = jest.spyOn(Alert, 'alert');
    const { onClose } = renderEditor(ADDED_DAY, { entries: TWO_SESSIONS });
    await elapse(0);

    fireEvent.press(screen.getByTestId('session-editor-remove'));
    const [, , buttons] = alert.mock.calls[alert.mock.calls.length - 1];
    await act(async () => {
      (buttons as { style?: string; onPress?: () => void }[]).find((b) => b.style === 'destructive')?.onPress?.();
      await jest.advanceTimersByTimeAsync(0);
    });

    expect(screen.getByTestId('session-editor-remove-failed')).toHaveTextContent(
      /^This session could not be removed\..+/,
    );
    expect(onClose).not.toHaveBeenCalled();
    expect(Toast.show).not.toHaveBeenCalled();
    alert.mockRestore();
  });
});
