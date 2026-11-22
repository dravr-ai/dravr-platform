// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the coach's "build from family" editor on a plan card — who is offered it, the day it opens on, the live preview, the save
// ABOUTME: Drives a workout_plan block through MessageItem as the plan thread draws it; the chat API is mocked at the services barrel

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type {
  FamilyCatalogueResponse,
  ReplyBlock,
  SessionPreview,
  SessionPreviewRequest,
  SubjectPlanResponse,
  WorkoutPlan,
} from '@pierre/shared-types';
import MessageItem from '../MessageItem';
import PlanThreadOpenDraft from '../PlanThreadOpenDraft';
import { ToastProvider } from '../../ui';
import type { Message } from '../types';

const CONVERSATION_ID = 'conv-plan-alice';
const COACH_ID = 'user-coach';
const ATHLETE_ID = 'user-alice';
const ALICE = { user_id: ATHLETE_ID, display_name: 'Alice Runner' };

// The chat API at the services barrel the web hooks bind, as the draft
// controls' test mocks it: the editor's three calls beside the plan thread's.
const api = vi.hoisted(() => ({
  listParticipants: vi.fn(),
  getSubjectPlan: vi.fn(),
  publishSubjectPlan: vi.fn(),
  discardSubjectPlanDraft: vi.fn(),
  getSubjectFamilies: vi.fn(),
  previewSubjectSession: vi.fn(),
  saveSubjectDraftDay: vi.fn(),
}));

vi.mock('../../../services/api', () => ({
  chatApi: {
    listParticipants: (...a: unknown[]) => api.listParticipants(...a),
    getSubjectPlan: (...a: unknown[]) => api.getSubjectPlan(...a),
    publishSubjectPlan: (...a: unknown[]) => api.publishSubjectPlan(...a),
    discardSubjectPlanDraft: (...a: unknown[]) => api.discardSubjectPlanDraft(...a),
    getSubjectFamilies: (...a: unknown[]) => api.getSubjectFamilies(...a),
    previewSubjectSession: (...a: unknown[]) => api.previewSubjectSession(...a),
    saveSubjectDraftDay: (...a: unknown[]) => api.saveSubjectDraftDay(...a),
  },
}));

const viewer = vi.hoisted(() => ({ id: 'user-coach' }));
vi.mock('../../../hooks/useAuth', () => ({
  useAuth: () => ({ user: viewer }),
}));

/** A week with a 30/15 day the agent saved and a rest day after it. */
const PLAN: WorkoutPlan = {
  goal_race: { name: 'Harricana 65K', date: '2027-09-11', discipline: 'trail', priority: 'A' },
  phases: [],
  weeks: [
    {
      week_start: '2026-11-23',
      focus: 'Build',
      current: true,
      days: [
        {
          date: '2026-11-24',
          sport: 'ride',
          workout: '30/15 intervals',
          duration_min: 50,
          intensity: 'hard',
          rest: false,
          template_slug: 'vo2max-short',
          template_source: 'catalogue',
          template_preset: 'ronnestad-30-15',
          template_params: { reps: 11 },
          family_sport: 'ride',
        },
        { date: '2026-11-25', sport: 'rest', workout: 'Rest', intensity: '', rest: true },
      ],
    },
  ],
  weeks_deferred: 0,
};

const CATALOGUE: FamilyCatalogueResponse = {
  families: [
    {
      slug: 'vo2max-short',
      name: 'Short VO2max intervals',
      purpose: 'vo2max_short',
      purpose_label: 'VO2max, short intervals',
      sports: ['ride', 'run'],
      default_preset: 'ronnestad-30-15',
      presets: [
        {
          key: 'ronnestad-30-15',
          name: '30/15 (Rønnestad)',
          summary: 'Three sets of 30 s on, 15 s off.',
          sports: ['ride', 'run'],
          params: {
            ride: {
              sets: { min: 2, max: 4, default: 3 },
              reps: { min: 9, max: 15, default: 13 },
              work_seconds: { min: 30, max: 30, default: 30 },
              rest_seconds: { min: 15, max: 15, default: 15 },
              set_rest_seconds: { min: 120, max: 300, default: 180 },
            },
            run: {
              sets: { min: 2, max: 3, default: 2 },
              reps: { min: 9, max: 15, default: 13 },
              work_seconds: { min: 30, max: 30, default: 30 },
              rest_seconds: { min: 15, max: 15, default: 15 },
              set_rest_seconds: { min: 120, max: 300, default: 180 },
            },
          },
        },
      ],
    },
    {
      slug: 'endurance',
      name: 'Endurance',
      purpose: 'endurance',
      sports: ['ride', 'run', 'swim'],
      default_preset: 'steady',
      presets: [
        {
          key: 'steady',
          name: 'Steady',
          sports: ['ride', 'run', 'swim'],
          params: { ride: { duration_minutes: { min: 30, max: 240, default: 90 } }, run: { duration_minutes: { min: 30, max: 240, default: 90 } }, swim: { duration_minutes: { min: 30, max: 240, default: 90 } } },
        },
      ],
    },
  ],
};

/** What the kernel answers for a build: findings only once the reps leave the range. */
function previewFor(request: SessionPreviewRequest): SessionPreview {
  const reps = request.params?.reps ?? 13;
  const outside = reps > 15;
  return {
    family: request.family,
    preset: request.preset ?? 'ronnestad-30-15',
    sport: request.sport,
    duration_min: 42,
    steps: [
      { label: 'Warm-up', duration_seconds: 900, target_zone: 'Z2' },
      { label: 'Work', duration_seconds: 30, target_zone: '120% FTP', repeat: reps },
    ],
    derived: {
      sets: 3,
      reps,
      work_seconds: 30,
      rest_seconds: 15,
      ratio: 2,
      work_pct: 118,
      on_time_minutes: 19.5,
      total_minutes: 42,
      aerobic_max: 'MAP',
    },
    findings: outside
      ? [
          {
            code: 'param_out_of_range',
            text: `${reps} reps per set is above the evidence for 30/15 (9–15).`,
            param: 'reps' as const,
          },
        ]
      : [],
    ...(outside ? { outside_evidence: 'Outside the evidence' } : {}),
    evidence: { label: 'Cycling trials only', basis: 'Rønnestad et al. 2020' },
    counts_as: 'Counts as VO2max',
    anchor: 'Anchor: FTP 280 W, their own.',
  };
}

function subjectPlan(): SubjectPlanResponse {
  return { athlete: ALICE, today: '2026-11-21', live: PLAN, draft: PLAN, draft_state: null };
}

function reply(): Message {
  return { id: 'msg-1', role: 'assistant', content: 'Here is the plan.', created_at: '2026-11-21T09:00:01Z' };
}

function planBlocks(withSubject = true): ReplyBlock[] {
  return [
    { type: 'prose', text: 'Here is the plan.' },
    withSubject ? { type: 'workout_plan', plan: PLAN, subject: ALICE, draft: null } : { type: 'workout_plan', plan: PLAN },
  ];
}

function renderCard({ editsPlanDays = true, blocks = planBlocks() }: { editsPlanDays?: boolean; blocks?: ReplyBlock[] } = {}) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return render(
    <QueryClientProvider client={queryClient}>
      <ToastProvider>
        <MessageItem
          conversationId={CONVERSATION_ID}
          editsPlanDays={editsPlanDays}
          message={reply()}
          blocks={blocks}
        />
      </ToastProvider>
    </QueryClientProvider>,
  );
}

/** Open the editor from the first day's (the 30/15's) action. */
async function openFirstDay(user: ReturnType<typeof userEvent.setup>) {
  const [first] = await screen.findAllByRole('button', { name: 'Build from family' });
  await user.click(first);
  return screen.findByRole('dialog', { name: 'Build a session for Tue, Nov 24' });
}

describe('"Build from family" on a plan card', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    viewer.id = COACH_ID;
    api.listParticipants.mockResolvedValue([
      { user_id: COACH_ID, role: 'owner', added_by: COACH_ID, added_at: '2026-11-21T08:00:00Z' },
      { user_id: ATHLETE_ID, role: 'member', added_by: COACH_ID, added_at: '2026-11-21T08:01:00Z' },
    ]);
    api.getSubjectPlan.mockResolvedValue(subjectPlan());
    api.getSubjectFamilies.mockResolvedValue(CATALOGUE);
    api.previewSubjectSession.mockImplementation((_id: string, request: SessionPreviewRequest) =>
      Promise.resolve(previewFor(request)),
    );
    api.saveSubjectDraftDay.mockResolvedValue(subjectPlan());
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("offers every day, rest days included, to the thread's owner", async () => {
    renderCard();

    const actions = await screen.findAllByRole('button', { name: 'Build from family' });
    expect(actions).toHaveLength(2);
    // Each action is described by its day's date, so a reader tells them apart.
    const description = actions.map((action) => document.getElementById(action.getAttribute('aria-describedby') ?? ''));
    expect(description.map((cell) => cell?.textContent)).toEqual(['Tue, Nov 24', 'Wed, Nov 25']);
  });

  it('offers it on no card outside a plan thread — a room, Home, the athlete’s own thread', () => {
    renderCard({ editsPlanDays: false });

    expect(screen.getByText('Harricana 65K')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Build from family' })).toBeNull();
    expect(api.getSubjectFamilies).not.toHaveBeenCalled();
  });

  it('offers it on no card that names no athlete', () => {
    renderCard({ blocks: planBlocks(false) });

    expect(screen.getByText('Harricana 65K')).toBeInTheDocument();
    expect(screen.queryByRole('button', { name: 'Build from family' })).toBeNull();
  });

  it("opens on the day's own family, preset, sport and values, and measures that build", async () => {
    const user = userEvent.setup();
    renderCard();

    const sheet = await openFirstDay(user);

    const editor = within(sheet);
    expect(await editor.findByLabelText('Family')).toHaveValue('vo2max-short');
    expect(editor.getByText('VO2max, short intervals')).toBeInTheDocument();
    expect(editor.getByLabelText('Preset')).toHaveValue('ronnestad-30-15');
    expect(editor.getByLabelText('Ride')).toBeChecked();
    expect(editor.getByLabelText('Run')).not.toBeChecked();
    expect(editor.queryByLabelText('Swim')).toBeNull();
    expect(editor.getByLabelText('Reps per set')).toHaveValue('11');
    expect(editor.getByLabelText('Sets')).toHaveValue('3');
    expect(editor.getByTestId('session-editor-range-reps')).toHaveTextContent('Evidence range 9–15');
    expect(editor.getByTestId('session-editor-replaces')).toHaveTextContent(
      "Replaces this day's session in your draft.",
    );

    await waitFor(() =>
      expect(api.previewSubjectSession).toHaveBeenCalledWith(
        CONVERSATION_ID,
        {
          date: '2026-11-24',
          family: 'vo2max-short',
          preset: 'ronnestad-30-15',
          sport: 'ride',
          params: { sets: 3, reps: 11, work_seconds: 30, rest_seconds: 15, set_rest_seconds: 180 },
        },
        'en',
      ),
    );
    expect(api.getSubjectFamilies).toHaveBeenCalledWith(CONVERSATION_ID, 'en');
  });

  it("draws the kernel's preview: derived figures, steps, evidence chip, relabel and anchor lines", async () => {
    const user = userEvent.setup();
    renderCard();

    const editor = within(await openFirstDay(user));

    const derived = await editor.findByTestId('session-editor-derived');
    expect(within(derived).getByText('Work:rest ratio')).toBeInTheDocument();
    expect(within(derived).getByText('2:1')).toBeInTheDocument();
    expect(within(derived).getByText('Work, % of MAP')).toBeInTheDocument();
    expect(within(derived).getByText('118 %')).toBeInTheDocument();
    expect(within(derived).getByText('19.5 min')).toBeInTheDocument();
    expect(editor.getByText('Work · 30s · 120% FTP ×11')).toBeInTheDocument();
    expect(editor.getByTestId('plan-day-evidence')).toHaveTextContent('Cycling trials only');
    expect(editor.getByTestId('session-editor-counts-as')).toHaveTextContent('Counts as VO2max');
    expect(editor.getByTestId('session-editor-anchor')).toHaveTextContent('Anchor: FTP 280 W, their own.');
    expect(editor.getByTestId('session-editor-findings')).toHaveTextContent("Inside the family's evidence.");
    expect(editor.queryByTestId('session-editor-outside-evidence')).toBeNull();
  });

  it('opens a rest day on the first family, saying nothing about replacing a session', async () => {
    const user = userEvent.setup();
    renderCard();

    const actions = await screen.findAllByRole('button', { name: 'Build from family' });
    await user.click(actions[1]);
    const editor = within(await screen.findByRole('dialog', { name: 'Build a session for Wed, Nov 25' }));

    expect(await editor.findByLabelText('Family')).toHaveValue('vo2max-short');
    expect(editor.getByLabelText('Reps per set')).toHaveValue('13');
    expect(editor.queryByTestId('session-editor-replaces')).toBeNull();
  });

  it('measures a changed value once the typing settles, never the keystrokes in between', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
    renderCard();

    const editor = within(await openFirstDay(user));
    await waitFor(() => expect(api.previewSubjectSession).toHaveBeenCalledTimes(1));

    const reps = await editor.findByLabelText('Reps per set');
    await user.clear(reps);
    await user.type(reps, '14');
    expect(reps).toHaveValue('14');
    const repsAsked = () =>
      api.previewSubjectSession.mock.calls.map(([, request]) => (request as SessionPreviewRequest).params?.reps);
    expect(repsAsked()).not.toContain(14);

    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });

    await waitFor(() => expect(repsAsked()).toContain(14));
    expect(repsAsked()).not.toContain(1);
    expect(await editor.findByText('Work · 30s · 120% FTP ×14')).toBeInTheDocument();
  });

  it('keeps a value outside the range, shows what the check found, and saves it as built (D3)', async () => {
    const user = userEvent.setup();
    renderCard();

    const editor = within(await openFirstDay(user));
    const reps = await editor.findByLabelText('Reps per set');
    await user.clear(reps);
    await user.type(reps, '30');

    expect(reps).toHaveValue('30');
    const findings = editor.getByTestId('session-editor-findings');
    await waitFor(() => expect(findings).toHaveTextContent('30 reps per set is above the evidence for 30/15 (9–15).'));
    // The field is marked from the kernel's finding, never from a client-side compare.
    expect(editor.getByTestId('session-editor-range-reps')).toHaveAttribute('data-outside', 'true');
    expect(findings).toHaveTextContent('Findings are warnings: the session saves as you built it.');
    expect(editor.getByTestId('session-editor-outside-evidence')).toHaveTextContent('Outside the evidence');

    const save = editor.getByRole('button', { name: 'Save to draft' });
    expect(save).toBeEnabled();
    await user.click(save);

    await waitFor(() =>
      expect(api.saveSubjectDraftDay).toHaveBeenCalledWith(
        CONVERSATION_ID,
        '2026-11-24',
        {
          family: 'vo2max-short',
          preset: 'ronnestad-30-15',
          sport: 'ride',
          params: { sets: 3, reps: 30, work_seconds: 30, rest_seconds: 15, set_rest_seconds: 180 },
        },
        'en',
      ),
    );
  });

  it('saves the build into the draft, says so, and closes', async () => {
    const user = userEvent.setup();
    renderCard();

    const editor = within(await openFirstDay(user));
    // A new sport starts from the preset's defaults in that sport: its run form has its own ranges.
    await user.click(await editor.findByLabelText('Run'));
    await user.click(editor.getByRole('button', { name: 'Save to draft' }));

    expect(
      await screen.findByText('Saved to your draft. Alice Runner sees it only once you publish.'),
    ).toBeInTheDocument();
    expect(api.saveSubjectDraftDay).toHaveBeenCalledWith(
      CONVERSATION_ID,
      '2026-11-24',
      {
        family: 'vo2max-short',
        preset: 'ronnestad-30-15',
        sport: 'run',
        params: { sets: 2, reps: 13, work_seconds: 30, rest_seconds: 15, set_rest_seconds: 180 },
      },
      'en',
    );
    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
  });

  it('says a refused save and keeps the build open', async () => {
    api.saveSubjectDraftDay.mockRejectedValue(new Error('boom'));
    const user = userEvent.setup();
    renderCard();

    const editor = within(await openFirstDay(user));
    await editor.findByLabelText('Family');
    await user.click(editor.getByRole('button', { name: 'Save to draft' }));

    expect(await screen.findByText('This session could not be saved.')).toBeInTheDocument();
    expect(screen.getByRole('dialog', { name: 'Build a session for Tue, Nov 24' })).toBeInTheDocument();
  });

  it('closes on Cancel without saving', async () => {
    const user = userEvent.setup();
    renderCard();

    const editor = within(await openFirstDay(user));
    await editor.findByLabelText('Family');
    await user.click(editor.getByRole('button', { name: 'Cancel' }));

    await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    expect(api.saveSubjectDraftDay).not.toHaveBeenCalled();
  });

  it('picks a new family at its defaults, in a sport it has a form for', async () => {
    const user = userEvent.setup();
    renderCard();

    const editor = within(await openFirstDay(user));
    await user.selectOptions(await editor.findByLabelText('Family'), 'endurance');

    expect(editor.getByLabelText('Preset')).toHaveValue('steady');
    expect(editor.getByLabelText('Duration (min)')).toHaveValue('90');
    expect(editor.queryByLabelText('Reps per set')).toBeNull();
    expect(editor.getByLabelText('Swim')).not.toBeChecked();
    expect(editor.getByLabelText('Ride')).toBeChecked();
  });

  it('says when the families are still loading, when there are none, and when they failed', async () => {
    const user = userEvent.setup();
    let answer: (value: FamilyCatalogueResponse) => void = () => {};
    api.getSubjectFamilies.mockReturnValueOnce(new Promise<FamilyCatalogueResponse>((resolve) => (answer = resolve)));
    const { unmount } = renderCard();

    const editor = within(await openFirstDay(user));
    expect(editor.getByTestId('session-editor-loading')).toHaveTextContent('Loading families…');
    await act(async () => answer({ families: [] }));
    expect(await editor.findByTestId('session-editor-no-families')).toHaveTextContent(
      'No workout families are available for Alice Runner.',
    );
    unmount();

    api.getSubjectFamilies.mockRejectedValueOnce(new Error('down'));
    renderCard();
    const failed = within(await openFirstDay(user));
    expect(await failed.findByTestId('session-editor-load-failed')).toHaveTextContent(
      'The workout families could not be loaded.',
    );
  });

  it("offers the editor on the coach's open draft drawn on its own", async () => {
    const user = userEvent.setup();
    const queryClient = new QueryClient({ defaultOptions: { queries: { retry: false } } });
    render(
      <QueryClientProvider client={queryClient}>
        <ToastProvider>
          <PlanThreadOpenDraft
            conversationId={CONVERSATION_ID}
            open={{
              subject: ALICE,
              plan: PLAN,
              draft: { draft_id: 'draft-7', updated_at: '2026-11-21T09:00:00Z', conflicts: [], season_relaid: false },
            }}
          />
        </ToastProvider>
      </QueryClientProvider>,
    );

    expect(await screen.findAllByRole('button', { name: 'Build from family' })).toHaveLength(2);
    await openFirstDay(user);
  });
});
