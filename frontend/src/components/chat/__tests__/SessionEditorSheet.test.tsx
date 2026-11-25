// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the coach's day editor on a plan card — who is offered it, the session written (sport chips, steps, sets, targets), the family mode,
// ABOUTME: the footer's line, the guarded close, restore, add and remove per session, and a write that finds its session moved (carnet#1515, #1714, #1716, #1727)

import { describe, it, expect, vi, beforeAll, beforeEach, afterEach } from 'vitest';
import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { i18n } from '@pierre/i18n';
import type {
  FamilyCatalogueResponse,
  PlanDay,
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
// controls' test mocks it: the editor's calls beside the plan thread's.
const api = vi.hoisted(() => ({
  listParticipants: vi.fn(),
  getSubjectPlan: vi.fn(),
  publishSubjectPlan: vi.fn(),
  discardSubjectPlanDraft: vi.fn(),
  getSubjectFamilies: vi.fn(),
  previewSubjectSession: vi.fn(),
  saveSubjectDraftDay: vi.fn(),
  saveSubjectDraftHandDay: vi.fn(),
  restoreSubjectDraftDay: vi.fn(),
  setSubjectDraftDayCoachCall: vi.fn(),
  removeSubjectDraftDay: vi.fn(),
  addSubjectDraftSession: vi.fn(),
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
    saveSubjectDraftHandDay: (...a: unknown[]) => api.saveSubjectDraftHandDay(...a),
    restoreSubjectDraftDay: (...a: unknown[]) => api.restoreSubjectDraftDay(...a),
    setSubjectDraftDayCoachCall: (...a: unknown[]) => api.setSubjectDraftDayCoachCall(...a),
    removeSubjectDraftDay: (...a: unknown[]) => api.removeSubjectDraftDay(...a),
    addSubjectDraftSession: (...a: unknown[]) => api.addSubjectDraftSession(...a),
  },
}));

const viewer = vi.hoisted(() => ({ id: 'user-coach' }));
vi.mock('../../../hooks/useAuth', () => ({
  useAuth: () => ({ user: viewer }),
}));

/** The agent's easy run, two steps, no family: it opens on its own session. */
const EASY_RUN: PlanDay = {
  date: '2026-11-23',
  sport: 'run',
  workout: 'Easy 45 min',
  duration_min: 45,
  intensity: 'easy',
  rest: false,
  family_sport: 'run',
  steps: [
    { label: 'Warm-up', duration_seconds: 600, target_zone: 'easy' },
    { label: 'Steady', duration_seconds: 2100, target_zone: 'Z2' },
  ],
};

/** A 30/15 the agent built from a family: it opens on that family. */
const THIRTY_FIFTEEN: PlanDay = {
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
};

const REST: PlanDay = { date: '2026-11-25', sport: 'rest', workout: 'Rest', intensity: '', rest: true };

function planOf(days: PlanDay[]): WorkoutPlan {
  return {
    goal_race: { name: 'Harricana 65K', date: '2027-09-11', discipline: 'trail', priority: 'A' },
    phases: [],
    weeks: [{ week_start: '2026-11-23', focus: 'Build', current: true, days }],
    weeks_deferred: 0,
  };
}

const PLAN = planOf([EASY_RUN, THIRTY_FIFTEEN, REST]);

/** Tuesday's bike, the agent's, first of the date's two sessions (carnet#1716). */
const BRICK_BIKE: PlanDay = {
  date: '2026-11-24',
  position: 0,
  sport: 'ride',
  workout: 'Bike 90 min',
  duration_min: 90,
  intensity: 'Z2',
  rest: false,
};

/** Tuesday's run off the bike, which the coach added to the date. */
const BRICK_RUN: PlanDay = {
  date: '2026-11-24',
  position: 1,
  sport: 'run',
  workout: 'Run off the bike',
  duration_min: 20,
  intensity: 'tempo',
  rest: false,
  added: true,
};

/** A week whose Tuesday holds two sessions, between two dates of one entry each. */
const TWO_SESSIONS_PLAN = planOf([{ ...EASY_RUN, position: 0 }, BRICK_BIKE, BRICK_RUN, { ...REST, position: 0 }]);

/**
 * The same week after the coach's edits: the run rewritten by hand, the 30/15
 * turned into rest, and a rest day rebuilt from a family.
 */
const EDITED_PLAN = planOf([
  {
    ...EASY_RUN,
    workout: 'Easy 50 min',
    duration_min: 50,
    edited: { workout: 'Easy 45 min', rest: false },
  },
  {
    date: '2026-11-24',
    sport: 'rest',
    workout: 'Rest',
    intensity: '',
    rest: true,
    edited: { workout: '30/15 intervals', rest: false },
  },
  { ...THIRTY_FIFTEEN, date: '2026-11-26', edited: { workout: 'Rest', rest: true } },
]);

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
      slug: 'sweet-spot',
      name: 'Sweet spot',
      purpose: 'sweet_spot',
      sports: ['ride'],
      default_preset: 'two-by-twenty',
      presets: [
        {
          key: 'two-by-twenty',
          name: '2 × 20 min',
          sports: ['ride'],
          params: { ride: { sets: { min: 2, max: 3, default: 2 } } },
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
          params: {
            ride: { duration_minutes: { min: 30, max: 240, default: 90 } },
            run: { duration_minutes: { min: 30, max: 240, default: 90 } },
            swim: { duration_minutes: { min: 30, max: 240, default: 90 } },
          },
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
            text: `${reps} reps per set is above the tested range for 30/15 (9–15).`,
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

function subjectPlan(plan: WorkoutPlan = PLAN): SubjectPlanResponse {
  return { athlete: ALICE, today: '2026-11-21', live: plan, draft: plan, draft_state: null, proposed_dates: [] };
}

function reply(): Message {
  return { id: 'msg-1', role: 'assistant', content: 'Here is the plan.', created_at: '2026-11-21T09:00:01Z' };
}

function planBlocks(plan: WorkoutPlan = PLAN, withSubject = true): ReplyBlock[] {
  return [
    { type: 'prose', text: 'Here is the plan.' },
    withSubject ? { type: 'workout_plan', plan, subject: ALICE, draft: null } : { type: 'workout_plan', plan },
  ];
}

function renderCard({ editsPlanDays = true, blocks = planBlocks() }: { editsPlanDays?: boolean; blocks?: ReplyBlock[] } = {}) {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  const rendered = render(
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
  return { ...rendered, queryClient };
}

type User = ReturnType<typeof userEvent.setup>;

/**
 * A user who types without a timer between keys: the sheet re-renders on
 * every keystroke, and on a loaded machine the default per-key delay pushed
 * the longer journeys past the test timeout.
 */
function setupUser(): User {
  return userEvent.setup({ delay: null });
}
type Scope = ReturnType<typeof within>;

/** Open the editor from the `index`th day's "Edit day", and return the sheet named by its date. */
async function openDay(user: User, index: number, date: string): Promise<Scope> {
  const actions = await screen.findAllByRole('button', { name: 'Edit day' });
  await user.click(actions[index]);
  return within(await screen.findByRole('dialog', { name: `Edit ${date}` }));
}

const openEasyRun = (user: User) => openDay(user, 0, 'Mon, Nov 23');
const openThirtyFifteen = (user: User) => openDay(user, 1, 'Tue, Nov 24');

/** The `number`th step, folded to its row or open for editing. */
function step(editor: Scope, number: number): Scope {
  return within(editor.getByTestId(`session-editor-step-${number}`));
}

/** Open the `number`th step from its row, and return it. */
async function openStep(user: User, editor: Scope, number: number, label: string): Promise<Scope> {
  await user.click(editor.getByRole('button', { name: `Edit step ${number}: ${label}` }));
  return step(editor, number);
}

/** The sport chip named `name`. */
function sportChip(editor: Scope, name: string): HTMLElement {
  return within(editor.getByRole('group', { name: 'Sport' })).getByRole('button', { name });
}

describe('"Edit day" on a plan card', () => {
  // The card loads the sheet lazily on first open; loaded once here, the
  // first test to open it waits on the sheet, not on a cold module graph.
  beforeAll(async () => {
    await import('../SessionEditorSheet');
  });

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
    api.saveSubjectDraftHandDay.mockResolvedValue(subjectPlan(EDITED_PLAN));
    api.restoreSubjectDraftDay.mockResolvedValue(subjectPlan());
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  describe('who is offered it', () => {
    it("offers every day, rest days included, to the thread's owner", async () => {
      renderCard();

      const actions = await screen.findAllByRole('button', { name: 'Edit day' });
      expect(actions).toHaveLength(3);
      // Each action is described by its day's date, so a reader tells them apart.
      const description = actions.map((action) => document.getElementById(action.getAttribute('aria-describedby') ?? ''));
      expect(description.map((cell) => cell?.textContent)).toEqual(['Mon, Nov 23', 'Tue, Nov 24', 'Wed, Nov 25']);
    });

    it('offers it on no card outside a plan thread — a room, Home, the athlete’s own thread', () => {
      renderCard({ editsPlanDays: false });

      expect(screen.getByText('Harricana 65K')).toBeInTheDocument();
      expect(screen.queryByRole('button', { name: 'Edit day' })).toBeNull();
      expect(api.getSubjectFamilies).not.toHaveBeenCalled();
    });

    it('offers it on no card that names no athlete', () => {
      renderCard({ blocks: planBlocks(PLAN, false) });

      expect(screen.getByText('Harricana 65K')).toBeInTheDocument();
      expect(screen.queryByRole('button', { name: 'Edit day' })).toBeNull();
    });

    it("offers the editor on the coach's open draft drawn on its own", async () => {
      const user = setupUser();
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

      expect(await screen.findAllByRole('button', { name: 'Edit day' })).toHaveLength(3);
      await openEasyRun(user);
    });
  });

  describe('the card', () => {
    it('marks an edited day, and says a session written by hand was not checked against a family', () => {
      renderCard({ blocks: planBlocks(EDITED_PLAN) });

      const rows = screen.getAllByRole('row');
      expect(rows).toHaveLength(3);
      const [byHand, toRest, rebuilt] = rows.map((row) => within(row));
      expect(byHand.getByTestId('plan-day-edited')).toHaveTextContent('Edited');
      expect(byHand.getByTestId('plan-day-unchecked')).toHaveTextContent("Not checked against a family's evidence.");
      // A rest day has no session to check, and a family's build carries its own evidence.
      expect(toRest.getByTestId('plan-day-edited')).toHaveTextContent('Edited');
      expect(toRest.queryByTestId('plan-day-unchecked')).toBeNull();
      expect(rebuilt.getByTestId('plan-day-edited')).toBeInTheDocument();
      expect(rebuilt.queryByTestId('plan-day-unchecked')).toBeNull();
    });

    it('marks nothing on a day the coach has not edited, the agent’s own included', () => {
      renderCard();

      expect(screen.queryByTestId('plan-day-edited')).toBeNull();
      expect(screen.queryByTestId('plan-day-unchecked')).toBeNull();
    });
  });

  describe('the session, written', () => {
    it("opens an agent's day on its own session, never waiting on the families", async () => {
      api.getSubjectFamilies.mockReturnValue(new Promise<FamilyCatalogueResponse>(() => {}));
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);

      // No family mode until the catalogue has a family for the sport.
      expect(editor.queryByRole('group', { name: 'How to build the session' })).toBeNull();
      expect(editor.getByRole('switch', { name: 'Rest day' })).toHaveAttribute('aria-checked', 'false');
      expect(editor.getByLabelText('Title')).toHaveValue('Easy 45 min');
      expect(editor.getByLabelText('Title')).toHaveAttribute('placeholder', 'Session title');
      // Its steps total its minutes; their targets mix kinds, so no span.
      expect(editor.getByTestId('session-editor-summary')).toHaveTextContent('Run · 45 min');
      expect(sportChip(editor, 'Run')).toHaveAttribute('aria-pressed', 'true');
      expect(sportChip(editor, 'Ride')).toHaveAttribute('aria-pressed', 'false');
      expect(
        within(editor.getByRole('group', { name: 'Sport' }))
          .getAllByRole('button')
          .map((chip) => chip.textContent),
      ).toEqual(['Ride', 'Run', 'Swim', 'Strength training', 'Mobility', 'Yoga', 'CrossFit', 'Other…']);
      // Its steps total its duration and their targets its intensity: neither is the coach's to state.
      expect(editor.queryByLabelText('Duration (min)')).toBeNull();
      expect(editor.queryByLabelText('Intensity')).toBeNull();
      // Each step folded to its row, named by the button that opens it.
      expect(editor.getByRole('button', { name: 'Edit step 1: Warm-up' })).toHaveTextContent('Warm-up10:00easy');
      expect(editor.getByRole('button', { name: 'Edit step 2: Steady' })).toHaveTextContent('Steady35:00Z2');
      expect(editor.queryByLabelText('Step')).toBeNull();
      // The profile names every bar it draws.
      const profile = editor.getByRole('group', { name: 'Session profile' });
      expect(within(profile).getAllByRole('button').map((bar) => bar.getAttribute('aria-label'))).toEqual([
        'Step 1: Warm-up, 10:00, easy',
        'Step 2: Steady, 35:00, Z2',
      ]);
      expect(editor.getByTestId('session-editor-unchecked')).toHaveTextContent("Not checked against a family's evidence.");
      // Nothing about families in this mode, and nothing measured.
      expect(editor.queryByTestId('session-editor-loading')).toBeNull();
      expect(api.previewSubjectSession).not.toHaveBeenCalled();
    });

    it('holds Save until something changes, and names what a save would replace', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      const save = editor.getByRole('button', { name: 'Save to draft' });
      const footer = editor.getByTestId('session-editor-replacement');
      expect(footer).toHaveTextContent('No changes yet.');
      expect(save).toBeDisabled();

      const title = editor.getByLabelText('Title');
      await user.clear(title);
      // A session with no title names nothing: Save says what it waits for.
      expect(editor.queryByTestId('session-editor-replacement')).toBeNull();
      expect(editor.getByTestId('session-editor-missing')).toHaveTextContent('Add a title to save.');
      expect(save).toBeDisabled();
      expect(save).toHaveAccessibleDescription('Add a title to save.');
      await user.type(title, 'Easy 50 min');

      expect(editor.queryByTestId('session-editor-missing')).toBeNull();
      expect(editor.getByTestId('session-editor-replacement')).toHaveTextContent('Replaces Easy 45 min with Easy 50 min.');
      expect(save).toBeEnabled();
      // Save reads the sentence as its description.
      expect(save).toHaveAccessibleDescription('Replaces Easy 45 min with Easy 50 min.');

      // Back to what the day holds: nothing to save again.
      await user.clear(title);
      await user.type(title, 'Easy 45 min');
      expect(editor.getByTestId('session-editor-replacement')).toHaveTextContent('No changes yet.');
      expect(save).toBeDisabled();
    });

    it('saves a hand edit through the hand save with the body the coach wrote, says so, and closes', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      const title = editor.getByLabelText('Title');
      await user.clear(title);
      await user.type(title, 'Easy 50 min');

      await user.click(editor.getByRole('button', { name: 'Add a step' }));
      const strides = step(editor, 3);
      // The new step opens, and its name takes the keyboard.
      expect(strides.getByLabelText('Step')).toHaveFocus();
      await user.type(strides.getByLabelText('Step'), 'Strides');
      await user.clear(strides.getByLabelText('Duration (m:ss)'));
      await user.type(strides.getByLabelText('Duration (m:ss)'), ':20');
      await user.click(strides.getByRole('button', { name: 'Other' }));
      await user.type(strides.getByLabelText('Target as written'), 'fast');

      await user.click(editor.getByRole('button', { name: 'Save to draft' }));

      await waitFor(() =>
        expect(api.saveSubjectDraftHandDay).toHaveBeenCalledWith(
          CONVERSATION_ID,
          '2026-11-23',
          0,
          // No minutes: the server writes the duration the steps total.
          {
            rest: false,
            sport: 'run',
            workout: 'Easy 50 min',
            intensity: 'easy',
            steps: [
              { label: 'Warm-up', duration_seconds: 600, target_zone: 'easy' },
              { label: 'Steady', duration_seconds: 2100, target_zone: 'Z2' },
              { label: 'Strides', duration_seconds: 20, target_zone: 'fast' },
            ],
          },
          // The session as the card showed it: a write never lands on another.
          { sport: 'run', workout: 'Easy 45 min' },
          'en',
        ),
      );
      expect(
        await screen.findByText('Saved to your draft. Alice Runner sees it only once you publish.'),
      ).toBeInTheDocument();
      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
      expect(api.saveSubjectDraftDay).not.toHaveBeenCalled();
    });

    it('keeps a typed step duration while the field has focus, and writes it as m:ss once it leaves', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      const field = (await openStep(user, editor, 1, 'Warm-up')).getByLabelText('Duration (m:ss)');
      await user.clear(field);
      await user.type(field, '12');
      expect(field).toHaveValue('12');
      await user.tab();
      expect(field).toHaveValue('12:00');

      await user.clear(field);
      await user.type(field, '1:3');
      await user.tab();
      expect(field).toHaveValue('1:03');

      // A reading that does not parse is kept while typed, and leaves the last one that did.
      await user.clear(field);
      await user.type(field, '1:75');
      expect(field).toHaveValue('1:75');
      await user.tab();
      expect(field).toHaveValue('1:07');
    });

    it('saves a rest day as a rest day, everything under the switch hidden', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      await user.click(editor.getByRole('switch', { name: 'Rest day' }));

      expect(editor.queryByLabelText('Title')).toBeNull();
      expect(editor.queryByRole('group', { name: 'Sport' })).toBeNull();
      expect(editor.queryByRole('group', { name: 'How to build the session' })).toBeNull();
      expect(editor.queryByTestId('session-editor-steps')).toBeNull();
      expect(editor.queryByTestId('session-editor-unchecked')).toBeNull();
      expect(editor.getByTestId('session-editor-replacement')).toHaveTextContent('Replaces Easy 45 min with a rest day.');
      await user.click(editor.getByRole('button', { name: 'Save to draft' }));

      await waitFor(() =>
        expect(api.saveSubjectDraftHandDay).toHaveBeenCalledWith(
          CONVERSATION_ID,
          '2026-11-23',
          0,
          { rest: true },
          { sport: 'run', workout: 'Easy 45 min' },
          'en',
        ),
      );
    });

    it('turns a rest day into a session once it has a title', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openDay(user, 2, 'Wed, Nov 25');
      const rest = editor.getByRole('switch', { name: 'Rest day' });
      expect(rest).toHaveAttribute('aria-checked', 'true');
      expect(editor.queryByLabelText('Title')).toBeNull();
      await user.click(rest);

      const save = editor.getByRole('button', { name: 'Save to draft' });
      expect(editor.getByLabelText('Title')).toHaveValue('');
      expect(editor.getByRole('button', { name: 'Write' })).toHaveAttribute('aria-pressed', 'true');
      expect(sportChip(editor, 'Ride')).toHaveAttribute('aria-pressed', 'true');
      // A session needs its title before it saves.
      expect(save).toBeDisabled();
      expect(editor.getByTestId('session-editor-missing')).toHaveTextContent('Add a title to save.');
      await user.type(editor.getByLabelText('Title'), 'Recovery spin');
      expect(editor.getByTestId('session-editor-replacement')).toHaveTextContent(
        'Replaces the rest day with Recovery spin.',
      );
      // With no steps, the minutes are the coach's to state.
      await user.type(editor.getByLabelText('Duration (min)'), '40');
      expect(editor.getByTestId('session-editor-summary')).toHaveTextContent('Ride · 40 min');
      await user.click(save);

      await waitFor(() =>
        expect(api.saveSubjectDraftHandDay).toHaveBeenCalledWith(
          CONVERSATION_ID,
          '2026-11-25',
          0,
          { rest: false, sport: 'ride', workout: 'Recovery spin', duration_min: 40, intensity: '', steps: [] },
          { sport: 'rest', workout: 'Rest' },
          'en',
        ),
      );
    });

    it('moves and removes steps, the keyboard staying where the coach works', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      await openStep(user, editor, 2, 'Steady');
      // The last step goes no further down.
      expect(editor.getByRole('button', { name: 'Move step 2 down' })).toBeDisabled();

      await user.click(editor.getByRole('button', { name: 'Move step 2 up' }));
      expect(step(editor, 1).getByLabelText('Step')).toHaveValue('Steady');
      expect(editor.getByRole('button', { name: 'Edit step 2: Warm-up' })).toBeInTheDocument();
      // The moved step is first now, where up is spent: focus takes its down.
      expect(editor.getByRole('button', { name: 'Move step 1 up' })).toBeDisabled();
      expect(editor.getByRole('button', { name: 'Move step 1 down' })).toHaveFocus();

      await user.click(editor.getByRole('button', { name: 'Remove step 1' }));
      // The row now in its place takes the keyboard.
      expect(editor.getByRole('button', { name: 'Edit step 1: Warm-up' })).toHaveFocus();

      await openStep(user, editor, 1, 'Warm-up');
      await user.click(editor.getByRole('button', { name: 'Remove step 1' }));
      expect(editor.queryByTestId('session-editor-profile')).toBeNull();
      expect(editor.getByRole('button', { name: 'Add a step' })).toHaveFocus();
      // With the steps gone the minutes are the coach's again, as the day held them.
      expect(editor.getByLabelText('Duration (min)')).toHaveValue('45');
    });

    it('opens a step from its bar on the profile, and copies it in place', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      await user.click(editor.getByRole('button', { name: 'Step 2: Steady, 35:00, Z2' }));
      expect(step(editor, 2).getByLabelText('Step')).toHaveFocus();
      expect(editor.getByRole('button', { name: 'Step 2: Steady, 35:00, Z2' })).toHaveAttribute('aria-pressed', 'true');

      await user.click(editor.getByRole('button', { name: 'Duplicate step 2' }));
      // The copy opens after its original, its name taking the keyboard; one step is open at a time.
      expect(step(editor, 3).getByLabelText('Step')).toHaveValue('Steady');
      expect(step(editor, 3).getByLabelText('Step')).toHaveFocus();
      expect(editor.getByRole('button', { name: 'Edit step 2: Steady' })).toBeInTheDocument();
      expect(editor.getAllByLabelText('Step')).toHaveLength(1);
    });

    it('says what Save waits for, step by step, then "Changes X." when only the steps changed', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      await user.click(editor.getByRole('button', { name: 'Add a step' }));
      const save = editor.getByRole('button', { name: 'Save to draft' });
      const added = step(editor, 3);

      expect(save).toBeDisabled();
      expect(editor.getByTestId('session-editor-missing')).toHaveTextContent('Name step 3 to save.');
      expect(save).toHaveAccessibleDescription('Name step 3 to save.');
      await user.type(added.getByLabelText('Step'), 'Cool-down');
      expect(editor.getByTestId('session-editor-missing')).toHaveTextContent('Pick a target for step 3 to save.');
      await user.click(added.getByRole('button', { name: 'Z1' }));
      // The title kept its name: one session, named once.
      expect(editor.queryByTestId('session-editor-missing')).toBeNull();
      expect(editor.getByTestId('session-editor-replacement')).toHaveTextContent('Changes Easy 45 min.');
      expect(save).toBeEnabled();

      await user.clear(added.getByLabelText('Duration (m:ss)'));
      await user.type(added.getByLabelText('Duration (m:ss)'), '0');
      expect(editor.getByTestId('session-editor-missing')).toHaveTextContent('Give step 3 a time to save.');
      expect(save).toBeDisabled();
    });

    it("says what the server refused in a hand save — the target words it takes — in the footer too, and keeps the sheet open", async () => {
      api.saveSubjectDraftHandDay.mockRejectedValue(
        Object.assign(new Error('Request failed with status code 400'), {
          response: {
            status: 400,
            data: {
              error: 'invalid_input',
              message: 'Step 3 target "tempo-ish" is not one the calendar knows: easy, steady, tempo, threshold, fast.',
              details: { reason: 'unknown_target_zone' },
            },
          },
        }),
      );
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      await user.click(editor.getByRole('button', { name: 'Add a step' }));
      await user.type(step(editor, 3).getByLabelText('Step'), 'Cruise');
      await user.click(step(editor, 3).getByRole('button', { name: 'Other' }));
      await user.type(step(editor, 3).getByLabelText('Target as written'), 'tempo-ish');
      await user.click(editor.getByRole('button', { name: 'Save to draft' }));

      const sentence = 'Step 3 target "tempo-ish" is not one the calendar knows: easy, steady, tempo, threshold, fast.';
      expect(await screen.findByText('This session could not be saved.')).toBeInTheDocument();
      // The toast says it, and the footer keeps it while the coach fixes the step.
      const refusal = editor.getByTestId('session-editor-refusal');
      expect(refusal).toHaveAttribute('role', 'alert');
      expect(refusal).toHaveTextContent(sentence);
      expect(screen.getAllByText(sentence)).toHaveLength(2);
      expect(editor.getByRole('button', { name: 'Save to draft' })).toHaveAccessibleDescription(
        `${sentence} Changes Easy 45 min.`,
      );
      expect(screen.getByRole('dialog', { name: 'Edit Mon, Nov 23' })).toBeInTheDocument();

      // The coach changes what was refused: the footer lets it go.
      await user.clear(step(editor, 3).getByLabelText('Target as written'));
      await user.type(step(editor, 3).getByLabelText('Target as written'), 'tempo');
      expect(editor.queryByTestId('session-editor-refusal')).toBeNull();
    });

    it("words a target the calendar cannot read in the coach's language, naming the step and the target", async () => {
      api.saveSubjectDraftHandDay.mockRejectedValue(
        Object.assign(new Error('Request failed with status code 400'), {
          response: {
            status: 400,
            data: {
              error: 'invalid_input',
              message: "step 3's target 'seuil' is not one the calendar can resolve",
              details: { reason: 'unresolvable_target', step: 3, target: 'seuil' },
            },
          },
        }),
      );
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      await user.click(editor.getByRole('button', { name: 'Add a step' }));
      await user.type(step(editor, 3).getByLabelText('Step'), 'Cruise');
      await user.click(step(editor, 3).getByRole('button', { name: 'Other' }));
      await user.type(step(editor, 3).getByLabelText('Target as written'), 'seuil');
      await user.click(editor.getByRole('button', { name: 'Save to draft' }));

      expect(await screen.findByText('This session could not be saved.')).toBeInTheDocument();
      expect(editor.getByTestId('session-editor-refusal')).toHaveTextContent(
        'The target “seuil” of step 3 isn’t one the calendar reads: pick a zone, a % of a threshold or an RPE.',
      );
    });

    it('saves with Cmd or Ctrl+Enter once Save would, and not before', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      const title = editor.getByLabelText('Title');
      expect(editor.getByRole('button', { name: 'Save to draft' })).toHaveAttribute(
        'aria-keyshortcuts',
        'Meta+Enter Control+Enter',
      );
      // Nothing changed: the shortcut does nothing, as Save would.
      await user.click(title);
      await user.keyboard('{Control>}{Enter}{/Control}');
      expect(api.saveSubjectDraftHandDay).not.toHaveBeenCalled();

      await user.clear(title);
      await user.type(title, 'Easy 50 min');
      await user.keyboard('{Meta>}{Enter}{/Meta}');

      await waitFor(() => expect(api.saveSubjectDraftHandDay).toHaveBeenCalledTimes(1));
      expect(api.saveSubjectDraftHandDay).toHaveBeenCalledWith(
        CONVERSATION_ID,
        '2026-11-23',
        0,
        expect.objectContaining({ workout: 'Easy 50 min' }),
        { sport: 'run', workout: 'Easy 45 min' },
        'en',
      );
      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    });
  });

  describe('the sport', () => {
    it('hides the family mode for a sport no family is written for, with nothing in its place', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      expect(await editor.findByRole('group', { name: 'How to build the session' })).toBeInTheDocument();

      await user.click(sportChip(editor, 'Strength training'));
      expect(sportChip(editor, 'Strength training')).toHaveAttribute('aria-pressed', 'true');
      expect(sportChip(editor, 'Run')).toHaveAttribute('aria-pressed', 'false');
      expect(editor.queryByRole('group', { name: 'How to build the session' })).toBeNull();
      expect(editor.getByTestId('session-editor-summary')).toHaveTextContent('Strength training · 45 min');

      // From the family mode, the pick brings the written session back on screen.
      await user.click(sportChip(editor, 'Run'));
      await user.click(editor.getByRole('button', { name: 'From an evidence-backed family' }));
      expect(await editor.findByLabelText('Family')).toBeInTheDocument();
      await user.click(sportChip(editor, 'Strength training'));
      expect(editor.queryByLabelText('Family')).toBeNull();
      expect(editor.queryByRole('group', { name: 'How to build the session' })).toBeNull();
      expect(editor.getByTestId('session-editor-steps')).toBeInTheDocument();
    });

    it('saves the sport the coach names under Other…', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      await user.click(sportChip(editor, 'Other…'));
      const name = editor.getByLabelText('Name the sport');
      // The field opens empty and takes the keyboard; a sport with no name does not save.
      expect(name).toHaveValue('');
      expect(name).toHaveFocus();
      expect(sportChip(editor, 'Other…')).toHaveAttribute('aria-pressed', 'true');
      expect(sportChip(editor, 'Run')).toHaveAttribute('aria-pressed', 'false');
      expect(editor.getByTestId('session-editor-missing')).toHaveTextContent('Name the sport to save.');
      expect(editor.getByRole('button', { name: 'Save to draft' })).toBeDisabled();

      await user.type(name, 'Escalade');
      expect(editor.queryByRole('group', { name: 'How to build the session' })).toBeNull();
      expect(editor.getByTestId('session-editor-replacement')).toHaveTextContent('Changes Easy 45 min.');
      await user.click(editor.getByRole('button', { name: 'Save to draft' }));

      await waitFor(() =>
        expect(api.saveSubjectDraftHandDay).toHaveBeenCalledWith(
          CONVERSATION_ID,
          '2026-11-23',
          0,
          {
            rest: false,
            sport: 'Escalade',
            workout: 'Easy 45 min',
            intensity: 'easy',
            steps: EASY_RUN.steps,
          },
          { sport: 'run', workout: 'Easy 45 min' },
          'en',
        ),
      );
    });

    it("offers the day's own sport word as a chip of its own", async () => {
      const user = setupUser();
      renderCard({ blocks: planBlocks(planOf([{ ...EASY_RUN, sport: 'Escalade', family_sport: undefined, steps: [] }])) });

      const editor = await openDay(user, 0, 'Mon, Nov 23');
      expect(sportChip(editor, 'Escalade')).toHaveAttribute('aria-pressed', 'true');
      expect(editor.queryByLabelText('Name the sport')).toBeNull();
    });
  });

  describe('sets and targets', () => {
    it('adds a set of an effort and a recovery, five times, and saves them as one numbered set', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      await user.click(editor.getByRole('button', { name: 'Add a set' }));

      // The effort opens, its name taking the keyboard; the set is drawn under one × 5.
      expect(step(editor, 3).getByLabelText('Step')).toHaveValue('Effort');
      expect(step(editor, 3).getByLabelText('Step')).toHaveFocus();
      expect(editor.getByRole('button', { name: 'Edit step 4: Recovery' })).toHaveTextContent('Recovery2:00Z1');
      const set = within(editor.getByTestId('session-editor-series-3'));
      expect(set.getByText('Set')).toBeInTheDocument();
      expect(set.getByRole('group', { name: 'Repetitions of the set' })).toHaveTextContent('× 5');
      // Every round drawn; only the first is a stop for the keyboard.
      const profile = editor.getByRole('group', { name: 'Session profile' });
      expect(within(profile).getAllByRole('button')).toHaveLength(4);
      expect(editor.getByTestId('session-editor-summary')).toHaveTextContent('Run · 70 min');

      await user.click(editor.getByRole('button', { name: 'Save to draft' }));

      await waitFor(() =>
        expect(api.saveSubjectDraftHandDay).toHaveBeenCalledWith(
          CONVERSATION_ID,
          '2026-11-23',
          0,
          {
            rest: false,
            sport: 'run',
            workout: 'Easy 45 min',
            intensity: 'easy',
            steps: [
              { label: 'Warm-up', duration_seconds: 600, target_zone: 'easy' },
              { label: 'Steady', duration_seconds: 2100, target_zone: 'Z2' },
              { label: 'Effort', duration_seconds: 180, target_zone: 'Z4', repeat: 5, repeat_group: 1 },
              { label: 'Recovery', duration_seconds: 120, target_zone: 'Z1', repeat: 5, repeat_group: 1 },
            ],
          },
          { sport: 'run', workout: 'Easy 45 min' },
          'en',
        ),
      );
    });

    it('counts a set up and down with its × N stepper, and one round dissolves it into lone steps', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      await user.click(editor.getByRole('button', { name: 'Add a set' }));
      const stepper = () => within(editor.getByRole('group', { name: 'Repetitions of the set' }));

      await user.click(stepper().getByRole('button', { name: 'One more repetition' }));
      expect(stepper().getByText('× 6')).toBeInTheDocument();
      await user.click(stepper().getByRole('button', { name: 'One fewer repetition' }));
      await user.click(stepper().getByRole('button', { name: 'One fewer repetition' }));
      expect(stepper().getByText('× 4')).toBeInTheDocument();
      // A step added to the set repeats with it.
      await user.click(editor.getByRole('button', { name: 'Add a step to the set' }));
      expect(step(editor, 5).getByLabelText('Step')).toHaveFocus();
      await user.type(step(editor, 5).getByLabelText('Step'), 'Strides');
      await user.click(step(editor, 5).getByRole('button', { name: 'Z5' }));

      await user.click(editor.getByRole('button', { name: 'Save to draft' }));
      await waitFor(() => expect(api.saveSubjectDraftHandDay).toHaveBeenCalledTimes(1));
      const [, , , body] = api.saveSubjectDraftHandDay.mock.calls[0] as [string, string, number, { steps: unknown[] }];
      expect(body.steps.slice(2)).toEqual([
        { label: 'Effort', duration_seconds: 180, target_zone: 'Z4', repeat: 4, repeat_group: 1 },
        { label: 'Recovery', duration_seconds: 120, target_zone: 'Z1', repeat: 4, repeat_group: 1 },
        { label: 'Strides', duration_seconds: 300, target_zone: 'Z5', repeat: 4, repeat_group: 1 },
      ]);
    });

    it('dissolves a set taken down to one round, the keyboard going to its first row', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      await user.click(editor.getByRole('button', { name: 'Add a set' }));
      for (let round = 5; round > 1; round -= 1) {
        await user.click(editor.getByRole('button', { name: 'One fewer repetition' }));
      }
      expect(editor.queryByRole('group', { name: 'Repetitions of the set' })).toBeNull();
      expect(editor.queryByTestId('session-editor-series-3')).toBeNull();
      expect(editor.getByRole('button', { name: 'Edit step 3: Effort' })).toHaveFocus();
    });

    it('moves a step measured by time 15 seconds either way, never below zero', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      const warmUp = await openStep(user, editor, 1, 'Warm-up');
      const field = warmUp.getByLabelText('Duration (m:ss)');
      await user.click(warmUp.getByRole('button', { name: '15 seconds more' }));
      expect(field).toHaveValue('10:15');
      await user.click(warmUp.getByRole('button', { name: '15 seconds less' }));
      await user.click(warmUp.getByRole('button', { name: '15 seconds less' }));
      expect(field).toHaveValue('9:45');
      // The pressed button keeps the keyboard.
      expect(warmUp.getByRole('button', { name: '15 seconds less' })).toHaveFocus();

      await user.clear(field);
      await user.type(field, ':10');
      await user.tab();
      await user.click(warmUp.getByRole('button', { name: '15 seconds less' }));
      expect(field).toHaveValue('0:00');
      expect(warmUp.getByRole('button', { name: '15 seconds less' })).toBeDisabled();
      expect(editor.getByTestId('session-editor-missing')).toHaveTextContent('Give step 1 a time to save.');

      // A distance's estimated time is typed, not nudged.
      await user.click(within(warmUp.getByRole('group', { name: 'Measure the step by' })).getByRole('button', { name: 'Distance' }));
      expect(warmUp.queryByRole('button', { name: '15 seconds more' })).toBeNull();
    });

    it('measures a step by distance, saved with the time it is estimated to take', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      const steady = await openStep(user, editor, 2, 'Steady');
      const measure = within(steady.getByRole('group', { name: 'Measure the step by' }));
      expect(measure.getByRole('button', { name: 'Time' })).toHaveAttribute('aria-pressed', 'true');
      await user.click(measure.getByRole('button', { name: 'Distance' }));

      // A distance not written yet holds Save back.
      expect(steady.getByLabelText('Distance (m)')).toHaveValue('');
      expect(editor.getByTestId('session-editor-missing')).toHaveTextContent('Give step 2 a distance to save.');
      await user.type(steady.getByLabelText('Distance (m)'), '400');
      const estimate = steady.getByLabelText('Estimated time (m:ss)');
      expect(estimate).toHaveValue('35:00');
      await user.clear(estimate);
      await user.type(estimate, '1:30');

      await user.click(editor.getByRole('button', { name: 'Save to draft' }));
      await waitFor(() => expect(api.saveSubjectDraftHandDay).toHaveBeenCalledTimes(1));
      const [, , , body] = api.saveSubjectDraftHandDay.mock.calls[0] as [string, string, number, { steps: unknown[] }];
      expect(body.steps[1]).toEqual({ label: 'Steady', duration_seconds: 90, distance_meters: 400, target_zone: 'Z2' });
    });

    it('writes a zone chip as Z4, a percent band as 88-93% FTP, and an RPE as RPE 7', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      // The agent's "easy" is no chip's: it opens as written under Other.
      const warmUp = await openStep(user, editor, 1, 'Warm-up');
      expect(warmUp.getByRole('button', { name: 'Other' })).toHaveAttribute('aria-pressed', 'true');
      expect(warmUp.getByLabelText('Target as written')).toHaveValue('easy');
      await user.click(warmUp.getByRole('button', { name: 'Z4' }));
      expect(warmUp.getByRole('button', { name: 'Z4' })).toHaveAttribute('aria-pressed', 'true');
      expect(warmUp.queryByLabelText('Target as written')).toBeNull();

      const steady = await openStep(user, editor, 2, 'Steady');
      expect(steady.getByRole('button', { name: 'Z2' })).toHaveAttribute('aria-pressed', 'true');
      await user.click(steady.getByRole('button', { name: '% of a threshold' }));
      // A run's percent starts on its threshold pace; the band waits for its figures.
      expect(steady.getByLabelText('Threshold')).toHaveValue('threshold_pace');
      expect(editor.getByTestId('session-editor-missing')).toHaveTextContent('Pick a target for step 2 to save.');
      await user.type(steady.getByLabelText('From'), '88');
      await user.type(steady.getByLabelText('To'), '93');
      await user.selectOptions(steady.getByLabelText('Threshold'), 'ftp');

      await user.click(editor.getByRole('button', { name: 'Add a step' }));
      const strides = step(editor, 3);
      await user.type(strides.getByLabelText('Step'), 'Strides');
      await user.click(strides.getByRole('button', { name: 'RPE' }));
      await user.type(strides.getByLabelText('RPE (1–10)'), '7');
      expect(editor.getByRole('button', { name: 'Edit step 2: Steady' })).toHaveTextContent('88-93% FTP');

      await user.click(editor.getByRole('button', { name: 'Save to draft' }));
      await waitFor(() => expect(api.saveSubjectDraftHandDay).toHaveBeenCalledTimes(1));
      const [, , , body] = api.saveSubjectDraftHandDay.mock.calls[0] as [string, string, number, { steps: unknown[] }];
      expect(body.steps).toEqual([
        { label: 'Warm-up', duration_seconds: 600, target_zone: 'Z4' },
        { label: 'Steady', duration_seconds: 2100, target_zone: '88-93% FTP' },
        { label: 'Strides', duration_seconds: 300, target_zone: 'RPE 7' },
      ]);
    });

    it('writes the span the targets cover as the intensity when they share a kind', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      const warmUp = await openStep(user, editor, 1, 'Warm-up');
      await user.click(warmUp.getByRole('button', { name: 'Z1' }));
      expect(editor.getByTestId('session-editor-summary')).toHaveTextContent('Run · 45 min · Z1–Z2');

      await user.click(editor.getByRole('button', { name: 'Save to draft' }));
      await waitFor(() =>
        expect(api.saveSubjectDraftHandDay).toHaveBeenCalledWith(
          CONVERSATION_ID,
          '2026-11-23',
          0,
          expect.objectContaining({ intensity: 'Z1–Z2' }),
          { sport: 'run', workout: 'Easy 45 min' },
          'en',
        ),
      );
    });
  });

  describe('closing', () => {
    it('closes straight away when nothing changed', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      await user.click(editor.getByRole('button', { name: 'Cancel' }));

      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
      expect(api.saveSubjectDraftHandDay).not.toHaveBeenCalled();
    });

    it('asks before Cancel, Escape or the close button lose a change, and keeps editing on request', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      const title = editor.getByLabelText('Title');
      await user.clear(title);
      await user.type(title, 'Easy 50 min');

      await user.click(editor.getByRole('button', { name: 'Cancel' }));
      const confirm = within(await screen.findByRole('dialog', { name: 'Discard your changes?' }));
      expect(confirm.getByText('Your changes to Mon, Nov 23 are not saved.')).toBeInTheDocument();
      await user.click(confirm.getByRole('button', { name: 'Keep editing' }));
      await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Discard your changes?' })).toBeNull());
      expect(screen.getByRole('dialog', { name: 'Edit Mon, Nov 23' })).toBeInTheDocument();
      expect(editor.getByLabelText('Title')).toHaveValue('Easy 50 min');

      // Escape asks too; a second Escape answers the question alone, never the sheet under it.
      await user.keyboard('{Escape}');
      expect(await screen.findByRole('dialog', { name: 'Discard your changes?' })).toBeInTheDocument();
      await user.keyboard('{Escape}');
      await waitFor(() => expect(screen.queryByRole('dialog', { name: 'Discard your changes?' })).toBeNull());
      expect(screen.getByRole('dialog', { name: 'Edit Mon, Nov 23' })).toBeInTheDocument();

      await user.click(editor.getByRole('button', { name: 'Close' }));
      const again = within(await screen.findByRole('dialog', { name: 'Discard your changes?' }));
      await user.click(again.getByRole('button', { name: 'Discard changes' }));

      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
      expect(api.saveSubjectDraftHandDay).not.toHaveBeenCalled();
    });

    it('asks for a change made in the other mode too', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      await user.click(await editor.findByRole('button', { name: 'From an evidence-backed family' }));
      const reps = await editor.findByLabelText('Reps per set');
      await user.clear(reps);
      await user.type(reps, '12');
      await user.click(editor.getByRole('button', { name: 'Write' }));
      await user.click(editor.getByRole('button', { name: 'Cancel' }));

      expect(await screen.findByRole('dialog', { name: 'Discard your changes?' })).toBeInTheDocument();
    });

    it('closes without asking when only the family mode was opened', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      await user.click(await editor.findByRole('button', { name: 'From an evidence-backed family' }));
      await editor.findByLabelText('Family');
      await user.click(editor.getByRole('button', { name: 'Cancel' }));

      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    });
  });

  describe('from a family', () => {
    it("opens a family's day on its family, format, sport and values, and measures that build", async () => {
      const user = setupUser();
      renderCard();

      const editor = await openThirtyFifteen(user);

      expect(editor.getByRole('button', { name: 'From an evidence-backed family' })).toHaveAttribute('aria-pressed', 'true');
      expect(editor.getByRole('group', { name: 'How to build the session' })).toBeInTheDocument();
      expect(await editor.findByLabelText('Family')).toHaveValue('vo2max-short');
      expect(editor.getByText('VO2max, short intervals')).toBeInTheDocument();
      expect(editor.getByLabelText('Format')).toHaveValue('ronnestad-30-15');
      // The sport is the chips': no second sport question in this mode.
      expect(sportChip(editor, 'Ride')).toHaveAttribute('aria-pressed', 'true');
      expect(editor.queryByRole('radio')).toBeNull();
      expect(editor.getByLabelText('Reps per set')).toHaveValue('11');
      expect(editor.getByLabelText('Sets')).toHaveValue('3');
      expect(editor.getByTestId('session-editor-range-reps')).toHaveTextContent('Tested range 9–15');
      expect(editor.getByTestId('session-editor-replacement')).toHaveTextContent('No changes yet.');
      expect(editor.getByRole('button', { name: 'Save to draft' })).toBeDisabled();
      // The head reads the format's name, not editable, and the minutes the kernel measured.
      expect(editor.queryByLabelText('Title')).toBeNull();
      const head = editor.getByTestId('session-editor-family-title');
      expect(head).toHaveTextContent('30/15 (Rønnestad)');
      await waitFor(() => expect(within(head).getByTestId('session-editor-summary')).toHaveTextContent('Ride · 42 min'));

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
      const user = setupUser();
      renderCard();

      const editor = await openThirtyFifteen(user);

      const derived = await editor.findByTestId('session-editor-derived');
      expect(within(derived).getByText('Work:rest ratio')).toBeInTheDocument();
      expect(within(derived).getByText('2:1')).toBeInTheDocument();
      expect(within(derived).getByText('Work, % of MAP')).toBeInTheDocument();
      expect(within(derived).getByText('118 %')).toBeInTheDocument();
      expect(within(derived).getByText('19.5 min')).toBeInTheDocument();
      expect(editor.getByText('Work · 30 s · 120% FTP ×11')).toBeInTheDocument();
      expect(editor.getByTestId('plan-day-evidence')).toHaveTextContent('Cycling trials only');
      expect(editor.getByTestId('session-editor-counts-as')).toHaveTextContent('Counts as VO2max');
      expect(editor.getByTestId('session-editor-anchor')).toHaveTextContent('Anchor: FTP 280 W, their own.');
      expect(editor.getByTestId('session-editor-findings')).toHaveTextContent('Inside the tested range.');
      expect(editor.queryByTestId('session-editor-outside-evidence')).toBeNull();
    });

    it("builds in the chips' sport and offers only the families written for it", async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      await user.click(await editor.findByRole('button', { name: 'From an evidence-backed family' }));

      const family = await editor.findByLabelText('Family');
      expect(within(family).getAllByRole('option').map((option) => option.textContent)).toEqual([
        'Short VO2max intervals',
        'Endurance',
      ]);
      // A run day built from a family replaces its own session with the family's format.
      expect(editor.getByTestId('session-editor-replacement')).toHaveTextContent(
        'Replaces Easy 45 min with 30/15 (Rønnestad).',
      );
      expect(editor.getByRole('button', { name: 'Save to draft' })).toBeEnabled();

      await user.click(sportChip(editor, 'Ride'));
      expect(within(editor.getByLabelText('Family')).getAllByRole('option').map((option) => option.textContent)).toEqual([
        'Short VO2max intervals',
        'Sweet spot',
        'Endurance',
      ]);
      // Only the endurance family has a swim: picking it moves the build there.
      await user.click(sportChip(editor, 'Swim'));
      expect(editor.getByLabelText('Family')).toHaveValue('endurance');
      expect(editor.getByLabelText('Format')).toHaveValue('steady');
      expect(editor.getByTestId('session-editor-family-title')).toHaveTextContent('Swim');
    });

    it("offers no family mode for a sport no family is written for, and says nothing in its place", async () => {
      api.getSubjectFamilies.mockResolvedValue({ families: CATALOGUE.families.filter((f) => f.slug === 'sweet-spot') });
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      await waitFor(() => expect(api.getSubjectFamilies).toHaveBeenCalled());
      expect(editor.queryByRole('group', { name: 'How to build the session' })).toBeNull();
      expect(editor.queryByTestId('session-editor-no-family-for-sport')).toBeNull();

      // A sport the catalogue has a family for brings the mode back.
      await user.click(sportChip(editor, 'Ride'));
      expect(await editor.findByRole('group', { name: 'How to build the session' })).toBeInTheDocument();
      await user.click(editor.getByRole('button', { name: 'From an evidence-backed family' }));
      expect(await editor.findByLabelText('Family')).toHaveValue('sweet-spot');
    });

    it("keeps each mode's work when the coach switches between them", async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      const title = editor.getByLabelText('Title');
      await user.clear(title);
      await user.type(title, 'Easy 50 min');

      await user.click(await editor.findByRole('button', { name: 'From an evidence-backed family' }));
      const reps = await editor.findByLabelText('Reps per set');
      await user.clear(reps);
      await user.type(reps, '12');

      await user.click(editor.getByRole('button', { name: 'Write' }));
      expect(editor.getByLabelText('Title')).toHaveValue('Easy 50 min');
      expect(editor.getByTestId('session-editor-replacement')).toHaveTextContent('Replaces Easy 45 min with Easy 50 min.');

      await user.click(editor.getByRole('button', { name: 'From an evidence-backed family' }));
      expect(await editor.findByLabelText('Reps per set')).toHaveValue('12');
    });

    it('measures a changed value once the typing settles, never the keystrokes in between', async () => {
      vi.useFakeTimers({ shouldAdvanceTime: true });
      const user = userEvent.setup({ advanceTimers: vi.advanceTimersByTime });
      renderCard();

      const editor = await openThirtyFifteen(user);
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
      expect(await editor.findByText('Work · 30 s · 120% FTP ×14')).toBeInTheDocument();
    });

    it('keeps a value outside the range, shows what the check found, and saves it as built (D3)', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openThirtyFifteen(user);
      const reps = await editor.findByLabelText('Reps per set');
      await user.clear(reps);
      await user.type(reps, '30');

      expect(reps).toHaveValue('30');
      const findings = editor.getByTestId('session-editor-findings');
      await waitFor(() => expect(findings).toHaveTextContent('30 reps per set is above the tested range for 30/15 (9–15).'));
      // The field is marked from the kernel's finding, never from a client-side compare.
      expect(editor.getByTestId('session-editor-range-reps')).toHaveAttribute('data-outside', 'true');
      expect(findings).toHaveTextContent('Findings are warnings: the session saves as you built it.');
      expect(editor.getByTestId('session-editor-outside-evidence')).toHaveTextContent('Outside the evidence');
      expect(editor.getByTestId('session-editor-replacement')).toHaveTextContent(
        'Replaces 30/15 intervals with 30/15 (Rønnestad).',
      );

      const save = editor.getByRole('button', { name: 'Save to draft' });
      expect(save).toBeEnabled();
      await user.click(save);

      await waitFor(() =>
        expect(api.saveSubjectDraftDay).toHaveBeenCalledWith(
          CONVERSATION_ID,
          '2026-11-24',
          0,
          {
            family: 'vo2max-short',
            preset: 'ronnestad-30-15',
            sport: 'ride',
            params: { sets: 3, reps: 30, work_seconds: 30, rest_seconds: 15, set_rest_seconds: 180 },
          },
          { sport: 'ride', workout: '30/15 intervals' },
          'en',
        ),
      );
      expect(api.saveSubjectDraftHandDay).not.toHaveBeenCalled();
    });

    it('saves the build into the draft, says so, and closes', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openThirtyFifteen(user);
      // A new sport starts from the format's defaults in that sport: its run form has its own ranges.
      await editor.findByLabelText('Family');
      await user.click(sportChip(editor, 'Run'));
      await user.click(editor.getByRole('button', { name: 'Save to draft' }));

      expect(
        await screen.findByText('Saved to your draft. Alice Runner sees it only once you publish.'),
      ).toBeInTheDocument();
      expect(api.saveSubjectDraftDay).toHaveBeenCalledWith(
        CONVERSATION_ID,
        '2026-11-24',
        0,
        {
          family: 'vo2max-short',
          preset: 'ronnestad-30-15',
          sport: 'run',
          params: { sets: 2, reps: 13, work_seconds: 30, rest_seconds: 15, set_rest_seconds: 180 },
        },
        { sport: 'ride', workout: '30/15 intervals' },
        'en',
      );
      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    });

    it('says a refused save and keeps the sheet open', async () => {
      api.saveSubjectDraftDay.mockRejectedValue(new Error('boom'));
      const user = setupUser();
      renderCard();

      const editor = await openThirtyFifteen(user);
      await editor.findByLabelText('Family');
      await user.click(sportChip(editor, 'Run'));
      await user.click(editor.getByRole('button', { name: 'Save to draft' }));

      expect(await screen.findByText('This session could not be saved.')).toBeInTheDocument();
      expect(editor.getByTestId('session-editor-refusal')).toHaveAttribute('role', 'alert');
      expect(screen.getByRole('dialog', { name: 'Edit Tue, Nov 24' })).toBeInTheDocument();
    });

    it('offers the Rest day switch in this mode too, and a rest day goes back to Write', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openThirtyFifteen(user);
      await editor.findByLabelText('Family');
      await user.click(editor.getByRole('switch', { name: 'Rest day' }));

      // A rest day is no family build: everything under the switch goes, and the save is the hand save.
      expect(editor.queryByLabelText('Family')).toBeNull();
      expect(editor.queryByRole('group', { name: 'How to build the session' })).toBeNull();
      expect(editor.queryByTestId('session-editor-family-title')).toBeNull();
      expect(editor.getByTestId('session-editor-replacement')).toHaveTextContent('Replaces 30/15 intervals with a rest day.');

      await user.click(editor.getByRole('switch', { name: 'Rest day' }));
      expect(editor.getByRole('button', { name: 'Write' })).toHaveAttribute('aria-pressed', 'true');
      expect(editor.getByLabelText('Title')).toHaveValue('30/15 intervals');

      await user.click(editor.getByRole('switch', { name: 'Rest day' }));
      await user.click(editor.getByRole('button', { name: 'Save to draft' }));
      await waitFor(() =>
        expect(api.saveSubjectDraftHandDay).toHaveBeenCalledWith(
          CONVERSATION_ID,
          '2026-11-24',
          0,
          { rest: true },
          { sport: 'ride', workout: '30/15 intervals' },
          'en',
        ),
      );
      expect(api.saveSubjectDraftDay).not.toHaveBeenCalled();
    });

    it('says when the families are still loading or failed, and falls back to Write when there are none', async () => {
      const user = setupUser();
      let answer: (value: FamilyCatalogueResponse) => void = () => {};
      api.getSubjectFamilies.mockReturnValueOnce(new Promise<FamilyCatalogueResponse>((resolve) => (answer = resolve)));
      const { unmount } = renderCard();

      // A day a family built opens in that mode, which waits on the catalogue; the mode control waits with it.
      const editor = await openThirtyFifteen(user);
      expect(editor.getByTestId('session-editor-loading')).toHaveTextContent('Loading families…');
      expect(editor.queryByRole('group', { name: 'How to build the session' })).toBeNull();
      // A catalogue that answers with no family for the sport leaves the day on "Write", never stranded.
      await act(async () => answer({ families: [] }));
      expect(await editor.findByLabelText('Title')).toHaveValue('30/15 intervals');
      expect(editor.queryByTestId('session-editor-no-families')).toBeNull();
      expect(editor.queryByRole('group', { name: 'How to build the session' })).toBeNull();
      unmount();

      api.getSubjectFamilies.mockRejectedValue(new Error('down'));
      const failed = renderCard();
      const written = await openEasyRun(user);
      // The written mode works on without them.
      expect(written.getByLabelText('Title')).toHaveValue('Easy 45 min');
      // A family sport keeps the mode control past the failure, which the family mode says.
      await user.click(await written.findByRole('button', { name: 'From an evidence-backed family' }));
      expect(await written.findByTestId('session-editor-load-failed')).toHaveTextContent(
        'The workout families could not be loaded.',
      );
      await user.click(written.getByRole('button', { name: 'Write' }));
      // A sport no family is written for offers no family mode at all.
      await user.click(sportChip(written, 'Yoga'));
      expect(written.queryByRole('group', { name: 'How to build the session' })).toBeNull();
      failed.unmount();

      // A day that opened on its family is never stuck on the failure: Write stays one tap away.
      renderCard();
      const built = await openThirtyFifteen(user);
      expect(await built.findByTestId('session-editor-load-failed')).toHaveTextContent(
        'The workout families could not be loaded.',
      );
      await user.click(await built.findByRole('button', { name: 'Write' }));
      expect(built.getByLabelText('Title')).toHaveValue('30/15 intervals');
    });
  });

  describe('going back to the original', () => {
    it('says the day was edited, asks, and puts it back as it stood before the first edit', async () => {
      const user = setupUser();
      renderCard({ blocks: planBlocks(EDITED_PLAN) });

      const editor = await openEasyRun(user);
      expect(editor.getByTestId('session-editor-edited')).toHaveTextContent('You edited this day in your draft.');
      const restore = editor.getByRole('button', { name: 'Back to the original' });
      expect(restore).toHaveAccessibleDescription('Puts back Easy 45 min, as the day stood before your first edit.');

      // It asks first; keeping the edits writes nothing.
      await user.click(restore);
      let confirm = within(await screen.findByRole('dialog', { name: 'Discard your edits to this session?' }));
      expect(confirm.getByText('Puts back Easy 45 min, as the day stood before your first edit.')).toBeInTheDocument();
      await user.click(confirm.getByRole('button', { name: 'Keep editing' }));
      await waitFor(() =>
        expect(screen.queryByRole('dialog', { name: 'Discard your edits to this session?' })).toBeNull(),
      );
      expect(api.restoreSubjectDraftDay).not.toHaveBeenCalled();

      await user.click(restore);
      confirm = within(await screen.findByRole('dialog', { name: 'Discard your edits to this session?' }));
      await user.click(confirm.getByRole('button', { name: 'Discard changes' }));

      await waitFor(() =>
        expect(api.restoreSubjectDraftDay).toHaveBeenCalledWith(
          CONVERSATION_ID,
          '2026-11-23',
          0,
          { sport: 'run', workout: 'Easy 50 min' },
          'en',
        ),
      );
      expect(await screen.findByText('Mon, Nov 23 is back as it was in your draft.')).toBeInTheDocument();
      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
      expect(api.saveSubjectDraftHandDay).not.toHaveBeenCalled();
    });

    it('names a rest day it would put back', async () => {
      const user = setupUser();
      renderCard({ blocks: planBlocks(EDITED_PLAN) });

      const editor = await openDay(user, 2, 'Thu, Nov 26');
      expect(editor.getByRole('button', { name: 'Back to the original' })).toHaveAccessibleDescription(
        'Puts back the rest day, as the day stood before your first edit.',
      );
    });

    it('says a refused restore and keeps the sheet open', async () => {
      api.restoreSubjectDraftDay.mockRejectedValue(new Error('boom'));
      const user = setupUser();
      renderCard({ blocks: planBlocks(EDITED_PLAN) });

      const editor = await openEasyRun(user);
      await user.click(editor.getByRole('button', { name: 'Back to the original' }));
      await user.click(
        within(await screen.findByRole('dialog', { name: 'Discard your edits to this session?' })).getByRole('button', {
          name: 'Discard changes',
        }),
      );

      expect(await screen.findByText('This day could not be restored.')).toBeInTheDocument();
      expect(screen.queryByRole('dialog', { name: 'Discard your edits to this session?' })).toBeNull();
      expect(screen.getByRole('dialog', { name: 'Edit Mon, Nov 23' })).toBeInTheDocument();
    });

    it('offers no way back on a day the coach has not edited', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      expect(editor.queryByTestId('session-editor-edited')).toBeNull();
      expect(editor.queryByRole('button', { name: 'Back to the original' })).toBeNull();
    });
  });
  describe('several sessions on a date (carnet#1716)', () => {
    /** The sheet "Add a session" opens under Tuesday's last entry. */
    async function openAdd(user: User): Promise<Scope> {
      await user.click(
        within(await screen.findByTestId('plan-day-2026-11-24-1')).getByRole('button', { name: 'Add a session' }),
      );
      return within(await screen.findByRole('dialog', { name: 'Add a session on Tue, Nov 24' }));
    }

    /** The sheet one of Tuesday's sessions opens on. */
    async function openTuesday(user: User, position: number): Promise<Scope> {
      await user.click(
        within(await screen.findByTestId(`plan-day-2026-11-24-${position}`)).getByRole('button', { name: 'Edit session' }),
      );
      return within(await screen.findByRole('dialog', { name: 'Edit Tue, Nov 24' }));
    }

    it("draws the date once, tells the date's sessions apart, and offers Add a session under its last", async () => {
      renderCard({ blocks: planBlocks(TWO_SESSIONS_PLAN) });

      const rows = await screen.findAllByRole('row');
      expect(rows.map((row) => row.getAttribute('data-testid'))).toEqual([
        'plan-day-2026-11-23-0',
        'plan-day-2026-11-24-0',
        'plan-day-2026-11-24-1',
        'plan-day-2026-11-25-0',
      ]);
      // The date heads the date's first row and spans the second.
      expect(screen.getAllByText('Tue, Nov 24')).toHaveLength(1);
      expect(within(rows[1]).getByText('Tue, Nov 24')).toHaveAttribute('rowspan', '2');
      expect(within(rows[2]).getAllByRole('cell')).toHaveLength(2);

      // A date of one entry edits the day; a date of two edits each session, named by date and title.
      expect(screen.getAllByRole('button', { name: 'Edit day' })).toHaveLength(2);
      const sessions = screen.getAllByRole('button', { name: 'Edit session' });
      expect(sessions.map((button) => button.getAttribute('data-testid'))).toEqual([
        'plan-day-edit-2026-11-24-0',
        'plan-day-edit-2026-11-24-1',
      ]);
      expect(sessions[0]).toHaveAccessibleDescription('Tue, Nov 24 Bike 90 min');
      expect(sessions[1]).toHaveAccessibleDescription('Tue, Nov 24 Run off the bike');

      // Add a session under each date's last entry only, described by its date.
      expect(within(rows[1]).queryByRole('button', { name: 'Add a session' })).toBeNull();
      const add = within(rows[2]).getByRole('button', { name: 'Add a session' });
      expect(add).toHaveAccessibleDescription('Tue, Nov 24');
      expect(screen.getAllByRole('button', { name: 'Add a session' })).toHaveLength(3);

      // The coach's own session is marked Added, and as unchecked; the agent's carries nothing.
      expect(within(rows[2]).getByTestId('plan-day-added')).toHaveTextContent('Added');
      expect(within(rows[2]).getByTestId('plan-day-unchecked')).toBeInTheDocument();
      expect(within(rows[2]).queryByTestId('plan-day-edited')).toBeNull();
      expect(within(rows[1]).queryByTestId('plan-day-added')).toBeNull();
    });

    it('offers no Add a session on a date already holding five entries', async () => {
      const full = Array.from({ length: 5 }, (_, position) => ({
        ...BRICK_BIKE,
        position,
        workout: `Session ${position + 1}`,
      }));
      renderCard({ blocks: planBlocks(planOf([{ ...EASY_RUN, position: 0 }, ...full])) });

      expect(await screen.findAllByRole('button', { name: 'Edit session' })).toHaveLength(5);
      expect(within(screen.getByTestId('plan-day-2026-11-24-4')).queryByRole('button', { name: 'Add a session' })).toBeNull();
      expect(screen.getAllByRole('button', { name: 'Add a session' })).toHaveLength(1);
    });

    it('adds a blank session by hand — no Rest switch, no footer sentence — through the add, once it has a title', async () => {
      api.addSubjectDraftSession.mockResolvedValue(subjectPlan(TWO_SESSIONS_PLAN));
      const user = setupUser();
      renderCard({ blocks: planBlocks(TWO_SESSIONS_PLAN) });

      const editor = await openAdd(user);
      expect(editor.getByRole('button', { name: 'Write' })).toHaveAttribute('aria-pressed', 'true');
      expect(editor.queryByRole('switch', { name: 'Rest day' })).toBeNull();
      expect(editor.getByLabelText('Title')).toHaveValue('');
      // In the sport of the date's first session.
      expect(sportChip(editor, 'Ride')).toHaveAttribute('aria-pressed', 'true');
      expect(editor.queryByTestId('session-editor-edited')).toBeNull();
      expect(editor.queryByTestId('session-editor-remove')).toBeNull();
      expect(editor.queryByTestId('session-editor-replacement')).toBeNull();
      const save = editor.getByRole('button', { name: 'Save to draft' });
      expect(save).toBeDisabled();
      expect(editor.getByTestId('session-editor-missing')).toHaveTextContent('Add a title to save.');

      await user.type(editor.getByLabelText('Title'), 'Core 20 min');
      await user.type(editor.getByLabelText('Duration (min)'), '20');
      expect(editor.queryByTestId('session-editor-replacement')).toBeNull();
      expect(save).toBeEnabled();
      await user.click(save);

      await waitFor(() =>
        expect(api.addSubjectDraftSession).toHaveBeenCalledWith(
          CONVERSATION_ID,
          '2026-11-24',
          { day: { rest: false, sport: 'ride', workout: 'Core 20 min', duration_min: 20, intensity: '', steps: [] } },
          'en',
        ),
      );
      expect(
        await screen.findByText('Saved to your draft. Alice Runner sees it only once you publish.'),
      ).toBeInTheDocument();
      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
      expect(api.saveSubjectDraftHandDay).not.toHaveBeenCalled();
    });

    it('asks before closing an add with a title typed, and closes an untouched one straight away', async () => {
      const user = setupUser();
      renderCard({ blocks: planBlocks(TWO_SESSIONS_PLAN) });

      let editor = await openAdd(user);
      await user.click(await editor.findByRole('button', { name: 'From an evidence-backed family' }));
      await user.click(editor.getByRole('button', { name: 'Cancel' }));
      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());

      editor = await openAdd(user);
      await user.type(editor.getByLabelText('Title'), 'Core');
      await user.click(editor.getByRole('button', { name: 'Cancel' }));
      expect(await screen.findByRole('dialog', { name: 'Discard your changes?' })).toBeInTheDocument();
    });

    it('adds a session from a family through the add once a build stands', async () => {
      api.addSubjectDraftSession.mockResolvedValue(subjectPlan(TWO_SESSIONS_PLAN));
      const user = setupUser();
      renderCard({ blocks: planBlocks(TWO_SESSIONS_PLAN) });

      const editor = await openAdd(user);
      await user.click(await editor.findByRole('button', { name: 'From an evidence-backed family' }));
      await editor.findByLabelText('Family');
      expect(sportChip(editor, 'Ride')).toHaveAttribute('aria-pressed', 'true');
      const save = editor.getByRole('button', { name: 'Save to draft' });
      expect(save).toBeEnabled();
      await user.click(save);

      await waitFor(() =>
        expect(api.addSubjectDraftSession).toHaveBeenCalledWith(
          CONVERSATION_ID,
          '2026-11-24',
          {
            build: {
              family: 'vo2max-short',
              preset: 'ronnestad-30-15',
              sport: 'ride',
              params: { sets: 3, reps: 13, work_seconds: 30, rest_seconds: 15, set_rest_seconds: 180 },
            },
          },
          'en',
        ),
      );
      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
      expect(api.saveSubjectDraftDay).not.toHaveBeenCalled();
    });

    it('offers Remove only on a date holding more than one entry, confirms, and removes that session as the card showed it', async () => {
      api.removeSubjectDraftDay.mockResolvedValue(subjectPlan(PLAN));
      const user = setupUser();
      renderCard({ blocks: planBlocks(TWO_SESSIONS_PLAN) });

      // The date's only entry is changed with the Rest switch, never removed.
      const single = await openDay(user, 0, 'Mon, Nov 23');
      expect(single.queryByRole('button', { name: 'Remove this session' })).toBeNull();
      await user.click(single.getByRole('button', { name: 'Cancel' }));
      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());

      const editor = await openTuesday(user, 1);
      await user.click(editor.getByRole('button', { name: 'Remove this session' }));
      const confirm = within(await screen.findByRole('dialog', { name: 'Remove this session?' }));
      expect(confirm.getByText('Run off the bike leaves Tue, Nov 24 in your draft.')).toBeInTheDocument();
      // Cancel keeps the sheet and removes nothing.
      await user.click(confirm.getByRole('button', { name: 'Cancel' }));
      expect(api.removeSubjectDraftDay).not.toHaveBeenCalled();
      expect(screen.getByRole('dialog', { name: 'Edit Tue, Nov 24' })).toBeInTheDocument();

      await user.click(editor.getByRole('button', { name: 'Remove this session' }));
      await user.click(
        within(await screen.findByRole('dialog', { name: 'Remove this session?' })).getByRole('button', {
          name: 'Remove session',
        }),
      );

      await waitFor(() =>
        expect(api.removeSubjectDraftDay).toHaveBeenCalledWith(
          CONVERSATION_ID,
          '2026-11-24',
          1,
          { sport: 'run', workout: 'Run off the bike' },
          'en',
        ),
      );
      expect(await screen.findByText('Run off the bike is off Tue, Nov 24 in your draft.')).toBeInTheDocument();
      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
    });

    it('writes an edit over the session at its position, and undoing an added session takes it off', async () => {
      api.saveSubjectDraftHandDay.mockResolvedValue(subjectPlan(TWO_SESSIONS_PLAN));
      const user = setupUser();
      renderCard({ blocks: planBlocks(TWO_SESSIONS_PLAN) });

      const added = await openTuesday(user, 1);
      const restore = added.getByRole('button', { name: 'Back to the original' });
      expect(restore).toHaveAccessibleDescription('Removes this session: you added it.');
      // Going back on a session the coach added asks as a remove does.
      await user.click(restore);
      const confirm = within(await screen.findByRole('dialog', { name: 'Remove this session?' }));
      expect(confirm.getByText('Run off the bike leaves Tue, Nov 24 in your draft.')).toBeInTheDocument();
      await user.click(confirm.getByRole('button', { name: 'Remove session' }));
      await waitFor(() =>
        expect(api.restoreSubjectDraftDay).toHaveBeenCalledWith(
          CONVERSATION_ID,
          '2026-11-24',
          1,
          { sport: 'run', workout: 'Run off the bike' },
          'en',
        ),
      );
      expect(await screen.findByText('Run off the bike is off Tue, Nov 24 in your draft.')).toBeInTheDocument();
      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());

      const bike = await openTuesday(user, 0);
      const title = bike.getByLabelText('Title');
      await user.clear(title);
      await user.type(title, 'Bike 80 min');
      await user.click(bike.getByRole('button', { name: 'Save to draft' }));
      await waitFor(() =>
        expect(api.saveSubjectDraftHandDay).toHaveBeenCalledWith(
          CONVERSATION_ID,
          '2026-11-24',
          0,
          expect.objectContaining({ workout: 'Bike 80 min', sport: 'ride' }),
          { sport: 'ride', workout: 'Bike 90 min' },
          'en',
        ),
      );
    });

    it('says a session moved since the card showed it, reads the plan again and closes — never a retry onto another', async () => {
      api.saveSubjectDraftHandDay.mockRejectedValue(
        Object.assign(new Error('Request failed with status code 409'), {
          response: {
            status: 409,
            data: {
              error: 'conflict',
              message: 'Session 1 on 2026-11-24 is no longer Bike 90 min.',
              details: { reason: 'session_moved' },
            },
          },
        }),
      );
      const user = setupUser();
      const { queryClient } = renderCard({ blocks: planBlocks(TWO_SESSIONS_PLAN) });
      const invalidate = vi.spyOn(queryClient, 'invalidateQueries');

      const editor = await openTuesday(user, 0);
      const title = editor.getByLabelText('Title');
      await user.clear(title);
      await user.type(title, 'Bike 80 min');
      await user.click(editor.getByRole('button', { name: 'Save to draft' }));

      expect(
        await screen.findByText('This day changed since you opened it. Here it is as it stands now.'),
      ).toBeInTheDocument();
      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
      expect(invalidate).toHaveBeenCalledWith({ queryKey: ['chat', 'subject-plan', CONVERSATION_ID] });
      expect(api.saveSubjectDraftHandDay).toHaveBeenCalledTimes(1);
      // No discard question on the way out: the sheet closed on the server's word.
      expect(screen.queryByText('Discard your changes?')).toBeNull();
    });

    it('says a date filled meanwhile in the coach’s words when an add is refused, and keeps the sheet open', async () => {
      api.addSubjectDraftSession.mockRejectedValue(
        Object.assign(new Error('Request failed with status code 422'), {
          response: {
            status: 422,
            data: { error: 'unprocessable', message: '2026-11-24 holds 5 entries.', details: { reason: 'date_full' } },
          },
        }),
      );
      const user = setupUser();
      renderCard({ blocks: planBlocks(TWO_SESSIONS_PLAN) });

      const editor = await openAdd(user);
      await user.type(editor.getByLabelText('Title'), 'Core 20 min');
      await user.click(editor.getByRole('button', { name: 'Save to draft' }));

      expect(await screen.findByText('This session could not be saved.')).toBeInTheDocument();
      expect(editor.getByTestId('session-editor-refusal')).toHaveTextContent('Tue, Nov 24 already holds 5 sessions.');
      expect(screen.getAllByText('Tue, Nov 24 already holds 5 sessions.')).toHaveLength(2);
      expect(screen.queryByText('2026-11-24 holds 5 entries.')).toBeNull();
      expect(screen.getByRole('dialog', { name: 'Add a session on Tue, Nov 24' })).toBeInTheDocument();
    });

    it('says a refused remove, with the server’s sentence, and keeps the sheet open', async () => {
      api.removeSubjectDraftDay.mockRejectedValue(
        Object.assign(new Error('Request failed with status code 422'), {
          response: {
            status: 422,
            data: {
              error: 'unprocessable',
              message: 'The last entry on a date is changed, never removed.',
              details: { reason: 'last_session' },
            },
          },
        }),
      );
      const user = setupUser();
      renderCard({ blocks: planBlocks(TWO_SESSIONS_PLAN) });

      const editor = await openTuesday(user, 1);
      await user.click(editor.getByRole('button', { name: 'Remove this session' }));
      await user.click(
        within(await screen.findByRole('dialog', { name: 'Remove this session?' })).getByRole('button', {
          name: 'Remove session',
        }),
      );

      expect(await screen.findByText('This session could not be removed.')).toBeInTheDocument();
      expect(screen.getByText('The last entry on a date is changed, never removed.')).toBeInTheDocument();
      expect(screen.queryByRole('dialog', { name: 'Remove this session?' })).toBeNull();
      expect(screen.getByRole('dialog', { name: 'Edit Tue, Nov 24' })).toBeInTheDocument();
    });

    it('closes the same way when a remove finds its session moved', async () => {
      api.removeSubjectDraftDay.mockRejectedValue(
        Object.assign(new Error('Request failed with status code 409'), {
          response: { status: 409, data: { error: 'conflict', message: 'moved', details: { reason: 'session_moved' } } },
        }),
      );
      const user = setupUser();
      renderCard({ blocks: planBlocks(TWO_SESSIONS_PLAN) });

      const editor = await openTuesday(user, 1);
      await user.click(editor.getByRole('button', { name: 'Remove this session' }));
      await user.click(
        within(await screen.findByRole('dialog', { name: 'Remove this session?' })).getByRole('button', {
          name: 'Remove session',
        }),
      );

      expect(
        await screen.findByText('This day changed since you opened it. Here it is as it stands now.'),
      ).toBeInTheDocument();
      await waitFor(() => expect(screen.queryByRole('dialog')).toBeNull());
      expect(api.removeSubjectDraftDay).toHaveBeenCalledTimes(1);
    });
  });
});

// carnet#1814: a session resting on convention, or outside its family's
// evidence, can be kept as the coach's own call, saved on the day.
describe('« C\'est mon choix » on a plan card', () => {
  const CONVENTION_RUN: PlanDay = {
    ...EASY_RUN,
    evidence: { kind: 'convention', label: 'Coaching convention' },
  };
  const CALLED_RUN: PlanDay = { ...CONVENTION_RUN, coach_call: true };

  beforeEach(() => {
    vi.clearAllMocks();
    viewer.id = COACH_ID;
    api.listParticipants.mockResolvedValue([
      { user_id: COACH_ID, role: 'owner', added_by: COACH_ID, added_at: '2026-11-21T08:00:00Z' },
      { user_id: ATHLETE_ID, role: 'member', added_by: COACH_ID, added_at: '2026-11-21T08:01:00Z' },
    ]);
    api.getSubjectPlan.mockResolvedValue(subjectPlan());
    api.setSubjectDraftDayCoachCall.mockResolvedValue(subjectPlan(planOf([CALLED_RUN, THIRTY_FIFTEEN, REST])));
  });

  it('offers the call only on a session the evidence check names, and saves it on the day', async () => {
    const user = setupUser();
    renderCard({ blocks: planBlocks(planOf([CONVENTION_RUN, THIRTY_FIFTEEN, REST])) });

    const call = await screen.findByTestId('plan-day-coach-call-2026-11-23-0');
    // The words name what a press does, and the hint says what the call
    // means to a screen reader too, beside the date it is for.
    expect(call).toHaveTextContent(i18n.t('humanCoach.coachCall.action'));
    expect(call).not.toHaveAttribute('aria-pressed');
    expect(call).toHaveAccessibleDescription(expect.stringContaining(i18n.t('humanCoach.coachCall.hint')));
    expect(screen.queryByTestId('plan-day-coach-call-2026-11-24-0')).toBeNull();
    expect(screen.queryByTestId('plan-day-coach-call-2026-11-25-0')).toBeNull();

    await user.click(call);
    await waitFor(() =>
      expect(api.setSubjectDraftDayCoachCall).toHaveBeenCalledWith(
        CONVERSATION_ID,
        '2026-11-23',
        0,
        true,
        { sport: 'run', workout: 'Easy 45 min' },
        'en',
      ),
    );
  });

  it('marks a called day for every reader and lets the coach take the call back', async () => {
    const user = setupUser();
    renderCard({ blocks: planBlocks(planOf([CALLED_RUN, THIRTY_FIFTEEN, REST])) });

    expect(await screen.findByTestId('plan-day-coach-call')).toBeInTheDocument();
    const undo = screen.getByTestId('plan-day-coach-call-2026-11-23-0');
    expect(undo).toHaveTextContent(i18n.t('humanCoach.coachCall.undo'));
    expect(undo).not.toHaveAccessibleDescription(expect.stringContaining(i18n.t('humanCoach.coachCall.hint')));
    await user.click(undo);
    await waitFor(() =>
      expect(api.setSubjectDraftDayCoachCall).toHaveBeenCalledWith(
        CONVERSATION_ID,
        '2026-11-23',
        0,
        false,
        { sport: 'run', workout: 'Easy 45 min' },
        'en',
      ),
    );
  });

  it('reads the plan again when the session moved, never calling whichever sits there now', async () => {
    const user = setupUser();
    api.setSubjectDraftDayCoachCall.mockRejectedValue({
      response: { status: 409, data: { error: 'conflict', message: 'moved', details: { reason: 'session_moved' } } },
    });
    const { queryClient } = renderCard({ blocks: planBlocks(planOf([CONVENTION_RUN, THIRTY_FIFTEEN, REST])) });
    const invalidate = vi.spyOn(queryClient, 'invalidateQueries');

    await user.click(await screen.findByTestId('plan-day-coach-call-2026-11-23-0'));

    expect(
      await screen.findByText('This day changed since you opened it. Here it is as it stands now.'),
    ).toBeInTheDocument();
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['chat', 'subject-plan', CONVERSATION_ID] });
    expect(api.setSubjectDraftDayCoachCall).toHaveBeenCalledTimes(1);
  });
});
