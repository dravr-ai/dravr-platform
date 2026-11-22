// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach's "build from family" editor on the phone — pre-filled from the day, previewed once typing settles, saved into the draft
// ABOUTME: A value outside its range earns findings and still saves; the plan card offers the editor on every day only when its host hands it

import React from 'react';
import { act, fireEvent, render, screen, within } from '@testing-library/react-native';
import { QueryClient, QueryClientProvider, defaultScheduler, notifyManager } from '@tanstack/react-query';
import Toast from 'react-native-toast-message';
import type {
  FamilyCatalogueResponse,
  PlanDay,
  SessionFinding,
  SessionPreview,
  SubjectPlanResponse,
  WorkoutPlan,
} from '@pierre/shared-types';

const CONVERSATION_ID = 'conv-plan-chloe';

// The chat API, in place of the server: the families, the preview and the
// draft-day write the editor's shared hooks call.
const mockGetFamilies = jest.fn();
const mockPreview = jest.fn();
const mockSaveDay = jest.fn();

jest.mock('../src/services/api', () => ({
  chatApi: {
    getSubjectFamilies: (...a: unknown[]) => mockGetFamilies(...a),
    previewSubjectSession: (...a: unknown[]) => mockPreview(...a),
    saveSubjectDraftDay: (...a: unknown[]) => mockSaveDay(...a),
  },
}));

jest.mock('../src/screens/chat/presentMessageMenu', () => ({ presentMessageMenu: jest.fn() }));

import SessionEditorSheet from '../src/screens/chat/SessionEditorSheet';
import WorkoutPlanCard from '../src/screens/chat/WorkoutPlanCard';
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

/** What the write answers: the subject plan, read after it. */
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

function renderEditor(day: PlanDay = FAMILY_DAY) {
  const onClose = jest.fn();
  render(
    <QueryClientProvider client={newClient()}>
      <SessionEditorSheet conversationId={CONVERSATION_ID} athleteName="Chloé Tremblay" day={day} onClose={onClose} />
    </QueryClientProvider>,
  );
  return { onClose };
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

describe('the family editor', () => {
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
  });

  afterEach(() => {
    notifyManager.setScheduler(defaultScheduler);
    jest.useRealTimers();
  });

  it("opens on the day's family, preset, sport and values, under the day's date", async () => {
    renderEditor();
    await elapse(0);

    expect(mockGetFamilies).toHaveBeenCalledWith(CONVERSATION_ID, 'en');
    expect(screen.getByTestId('session-editor-title')).toHaveTextContent('Build a session for Tuesday, November 24');
    expect(screen.getByTestId('session-editor-family-vo2max_short')).toHaveProp('accessibilityState', { selected: true });
    expect(screen.getByTestId('session-editor-preset-ronnestad_30_15')).toHaveProp('accessibilityState', {
      selected: true,
    });
    expect(screen.getByTestId('session-editor-sport-run')).toHaveProp('accessibilityState', { selected: true });
    // The sport row offers the preset's sports only, in the app's words.
    expect(screen.getByTestId('session-editor-sport-ride')).toHaveTextContent('Ride');
    expect(screen.queryByTestId('session-editor-sport-swim')).toBeNull();
    // The two values the day states, and the preset's defaults for the rest.
    expect(screen.getByTestId('session-editor-param-sets')).toHaveProp('value', '3');
    expect(screen.getByTestId('session-editor-param-reps')).toHaveProp('value', '12');
    expect(screen.getByTestId('session-editor-param-set_rest_seconds')).toHaveProp('value', '180');
    expect(screen.getByTestId('session-editor-range-set_rest_seconds')).toHaveTextContent('Evidence range 120–300');
    expect(screen.queryByTestId('session-editor-param-duration_minutes')).toBeNull();
    expect(screen.getByText('Short work, shorter rest, three sets.')).toBeTruthy();
    expect(screen.getByText('VO2max, short intervals')).toBeTruthy();
    // The day holds a session the save will replace.
    expect(screen.getByTestId('session-editor-replaces')).toHaveTextContent(
      "Replaces this day's session in your draft.",
    );
  });

  it('opens a rest day on the first family at its defaults, with nothing to replace', async () => {
    renderEditor(REST_DAY);
    await elapse(0);

    expect(screen.getByTestId('session-editor-family-endurance_steady')).toHaveProp('accessibilityState', {
      selected: true,
    });
    expect(screen.getByTestId('session-editor-param-duration_minutes')).toHaveProp('value', '90');
    expect(screen.queryByTestId('session-editor-replaces')).toBeNull();
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
    expect(screen.getByTestId('session-editor-findings-none')).toHaveTextContent("Inside the family's evidence.");

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

    const save = screen.getByTestId('session-editor-save');
    expect(save).toHaveTextContent('Save to draft');
    expect(save).not.toBeDisabled();
    fireEvent.press(save);
    await elapse(0);

    expect(mockSaveDay).toHaveBeenCalledWith(
      CONVERSATION_ID,
      '2026-11-24',
      {
        family: 'vo2max_short',
        preset: 'ronnestad_30_15',
        sport: 'run',
        params: { sets: 6, reps: 12, work_seconds: 30, rest_seconds: 15, set_rest_seconds: 180 },
      },
      'en',
    );
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it('saves the picked preset and sport into the draft, closes, and says so', async () => {
    const { onClose } = renderEditor();
    await elapse(0);

    fireEvent.press(screen.getByTestId('session-editor-preset-billat_30_30'));
    // The new preset starts at its defaults, in the one sport it has a form for.
    expect(screen.getByTestId('session-editor-param-reps')).toHaveProp('value', '15');
    expect(screen.queryByTestId('session-editor-param-sets')).toBeNull();
    expect(screen.queryByTestId('session-editor-sport-ride')).toBeNull();

    fireEvent.press(screen.getByTestId('session-editor-save'));
    expect(screen.getByTestId('session-editor-save')).toHaveTextContent('Saving…');
    await elapse(0);

    expect(mockSaveDay).toHaveBeenCalledWith(
      CONVERSATION_ID,
      '2026-11-24',
      { family: 'vo2max_short', preset: 'billat_30_30', sport: 'run', params: { reps: 15 } },
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
    expect(screen.getByTestId('session-editor-save')).not.toBeDisabled();
  });

  it("says when the athlete's agent offers no family, and offers no save", async () => {
    mockGetFamilies.mockResolvedValue({ families: [] });
    renderEditor();
    await elapse(0);

    expect(screen.getByTestId('session-editor-no-families')).toHaveTextContent(
      'No workout families are available for Chloé Tremblay.',
    );
    expect(screen.queryByTestId('session-editor-save')).toBeNull();
    expect(screen.getByTestId('session-editor-cancel')).toHaveTextContent('Cancel');
  });

  it('says when the families could not be read', async () => {
    mockGetFamilies.mockRejectedValue(new Error('down'));
    renderEditor();
    await elapse(0);

    expect(screen.getByTestId('session-editor-load-failed')).toHaveTextContent(
      'The workout families could not be loaded.',
    );
    expect(mockPreview).not.toHaveBeenCalled();
  });
});

/** A fortnight's first days: a family session and a rest day. */
const PLAN: WorkoutPlan = {
  goal_race: { name: 'Harricana 65K', date: '2027-09-11', discipline: 'trail', priority: 'A' },
  phases: [],
  weeks: [{ week_start: '2026-11-23', focus: 'build', current: true, days: [FAMILY_DAY, REST_DAY] }],
  weeks_deferred: 0,
};

describe('"Build from family" on the plan card', () => {
  it('ends every day, the rest day included, when the host hands the editor', () => {
    const onBuildDay = jest.fn();
    render(<WorkoutPlanCard plan={PLAN} onBuildDay={onBuildDay} />);

    const rest = screen.getByTestId('workout-plan-day-build-2026-11-25');
    expect(rest).toHaveTextContent('Build from family');
    expect(rest).toHaveProp('accessibilityHint', 'Build a session for Wednesday, November 25');
    fireEvent.press(screen.getByTestId('workout-plan-day-build-2026-11-24'));
    expect(onBuildDay).toHaveBeenCalledWith(FAMILY_DAY);
  });

  it('is on no day of a card whose host hands no editor', () => {
    render(<WorkoutPlanCard plan={PLAN} />);
    expect(screen.queryByText('Build from family')).toBeNull();
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

    fireEvent.press(screen.getByTestId('workout-plan-day-build-2026-11-25'));
    expect(onBuildPlanDay).toHaveBeenCalledWith(REST_DAY);
  });
});
