// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests the coach's day editor on a plan card — who is offered it, the day's own session edited by hand, the family mode,
// ABOUTME: the footer's line, the guarded close, restore, add and remove per session, and a write that finds its session moved (carnet#1515, #1714, #1716)

import { describe, it, expect, vi, beforeAll, beforeEach, afterEach } from 'vitest';
import { act, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
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
  return { athlete: ALICE, today: '2026-11-21', live: plan, draft: plan, draft_state: null };
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

/** The fields of the `number`th step row. */
function step(editor: Scope, number: number): Scope {
  return within(editor.getByTestId(`session-editor-step-${number}`));
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

  describe('this session, by hand', () => {
    it("opens an agent's day on its own session, never waiting on the families", async () => {
      api.getSubjectFamilies.mockReturnValue(new Promise<FamilyCatalogueResponse>(() => {}));
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);

      expect(editor.getByRole('button', { name: 'This session' })).toHaveAttribute('aria-pressed', 'true');
      expect(editor.getByRole('button', { name: 'From a family' })).toHaveAttribute('aria-pressed', 'false');
      expect(editor.getByRole('switch', { name: 'Rest day' })).toHaveAttribute('aria-checked', 'false');
      expect(editor.getByLabelText('Title')).toHaveValue('Easy 45 min');
      expect(editor.getByLabelText('Sport')).toHaveValue('run');
      // Its steps total its duration, so the minutes are not the coach's to state.
      expect(editor.queryByLabelText('Duration (min)')).toBeNull();
      expect(editor.getByLabelText('Intensity')).toHaveValue('easy');
      expect(step(editor, 1).getByLabelText('Step')).toHaveValue('Warm-up');
      expect(step(editor, 1).getByLabelText('Duration (m:ss)')).toHaveValue('10:00');
      expect(step(editor, 1).getByLabelText('Target')).toHaveValue('easy');
      expect(step(editor, 2).getByLabelText('Step')).toHaveValue('Steady');
      expect(step(editor, 2).getByLabelText('Duration (m:ss)')).toHaveValue('35:00');
      expect(step(editor, 2).getByLabelText('Repeat')).toHaveValue('');
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
      // A session with no title yet names nothing, and does not save.
      expect(editor.queryByTestId('session-editor-replacement')).toBeNull();
      expect(save).toBeDisabled();
      expect(save).not.toHaveAttribute('aria-describedby');
      await user.type(title, 'Easy 50 min');

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
      // The new step's label takes the keyboard.
      expect(strides.getByLabelText('Step')).toHaveFocus();
      await user.type(strides.getByLabelText('Step'), 'Strides');
      await user.clear(strides.getByLabelText('Duration (m:ss)'));
      await user.type(strides.getByLabelText('Duration (m:ss)'), ':20');
      await user.type(strides.getByLabelText('Target'), 'fast');
      await user.type(strides.getByLabelText('Repeat'), '6');

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
              { label: 'Strides', duration_seconds: 20, target_zone: 'fast', repeat: 6 },
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
      const field = step(editor, 1).getByLabelText('Duration (m:ss)');
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

    it('saves a rest day as a rest day, the fields hidden', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      await user.click(editor.getByRole('switch', { name: 'Rest day' }));

      expect(editor.queryByLabelText('Title')).toBeNull();
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
      expect(editor.getByRole('button', { name: 'This session' })).toHaveAttribute('aria-pressed', 'true');
      const rest = editor.getByRole('switch', { name: 'Rest day' });
      expect(rest).toHaveAttribute('aria-checked', 'true');
      await user.click(rest);

      const save = editor.getByRole('button', { name: 'Save to draft' });
      expect(editor.getByLabelText('Title')).toHaveValue('');
      expect(editor.getByTestId('session-editor-no-steps')).toHaveTextContent(
        'No steps: the title and intensity describe the session.',
      );
      // A session needs its title before it saves.
      expect(save).toBeDisabled();
      await user.type(editor.getByLabelText('Title'), 'Recovery spin');
      expect(editor.getByTestId('session-editor-replacement')).toHaveTextContent(
        'Replaces the rest day with Recovery spin.',
      );
      // With no steps, the minutes are the coach's to state.
      await user.type(editor.getByLabelText('Duration (min)'), '40');
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
      expect(editor.getByRole('button', { name: 'Move step 1 up' })).toBeDisabled();
      expect(editor.getByRole('button', { name: 'Move step 2 down' })).toBeDisabled();

      await user.click(editor.getByRole('button', { name: 'Move step 2 up' }));
      expect(step(editor, 1).getByLabelText('Step')).toHaveValue('Steady');
      expect(step(editor, 2).getByLabelText('Step')).toHaveValue('Warm-up');
      // The moved step is first now, where up is spent: focus takes its down.
      expect(editor.getByRole('button', { name: 'Move step 1 down' })).toHaveFocus();

      await user.click(editor.getByRole('button', { name: 'Remove step 1' }));
      expect(step(editor, 1).getByLabelText('Step')).toHaveValue('Warm-up');
      expect(editor.getByRole('button', { name: 'Remove step 1' })).toHaveFocus();

      await user.click(editor.getByRole('button', { name: 'Remove step 1' }));
      expect(editor.getByTestId('session-editor-no-steps')).toBeInTheDocument();
      expect(editor.getByRole('button', { name: 'Add a step' })).toHaveFocus();
      // With the steps gone the minutes are the coach's again, as the day held them.
      expect(editor.getByLabelText('Duration (min)')).toHaveValue('45');
    });

    it('holds Save until each step has a name, a target and a repeat of at least one', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      await user.click(editor.getByRole('button', { name: 'Add a step' }));
      const save = editor.getByRole('button', { name: 'Save to draft' });

      expect(editor.getByTestId('session-editor-replacement')).toHaveTextContent(
        'Replaces Easy 45 min with Easy 45 min.',
      );
      expect(save).toBeDisabled();
      const added = step(editor, 3);
      await user.type(added.getByLabelText('Step'), 'Cool-down');
      expect(save).toBeDisabled();
      await user.type(added.getByLabelText('Target'), 'easy');
      expect(save).toBeEnabled();
      await user.type(added.getByLabelText('Repeat'), '0');
      expect(save).toBeDisabled();
      await user.clear(added.getByLabelText('Repeat'));
      expect(save).toBeEnabled();
    });

    it("says what the server refused in a hand save — the target words it takes — and keeps the sheet open", async () => {
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
      await user.type(step(editor, 3).getByLabelText('Target'), 'tempo-ish');
      await user.click(editor.getByRole('button', { name: 'Save to draft' }));

      expect(await screen.findByText('This session could not be saved.')).toBeInTheDocument();
      expect(
        screen.getByText('Step 3 target "tempo-ish" is not one the calendar knows: easy, steady, tempo, threshold, fast.'),
      ).toBeInTheDocument();
      expect(screen.getByRole('dialog', { name: 'Edit Mon, Nov 23' })).toBeInTheDocument();
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
      await user.click(editor.getByRole('button', { name: 'From a family' }));
      await user.click(await editor.findByLabelText('Ride'));
      await user.click(editor.getByRole('button', { name: 'This session' }));
      await user.click(editor.getByRole('button', { name: 'Cancel' }));

      expect(await screen.findByRole('dialog', { name: 'Discard your changes?' })).toBeInTheDocument();
    });

    it('closes without asking when only the family mode was opened', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      await user.click(editor.getByRole('button', { name: 'From a family' }));
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

      expect(editor.getByRole('button', { name: 'From a family' })).toHaveAttribute('aria-pressed', 'true');
      expect(await editor.findByLabelText('Family')).toHaveValue('vo2max-short');
      expect(editor.getByText('VO2max, short intervals')).toBeInTheDocument();
      expect(editor.getByLabelText('Format')).toHaveValue('ronnestad-30-15');
      expect(editor.getByLabelText('Ride')).toBeChecked();
      expect(editor.getByLabelText('Run')).not.toBeChecked();
      expect(editor.getByLabelText('Reps per set')).toHaveValue('11');
      expect(editor.getByLabelText('Sets')).toHaveValue('3');
      expect(editor.getByTestId('session-editor-range-reps')).toHaveTextContent('Tested range 9–15');
      expect(editor.getByTestId('session-editor-replacement')).toHaveTextContent('No changes yet.');
      expect(editor.getByRole('button', { name: 'Save to draft' })).toBeDisabled();

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
      expect(editor.getByText('Work · 30s · 120% FTP ×11')).toBeInTheDocument();
      expect(editor.getByTestId('plan-day-evidence')).toHaveTextContent('Cycling trials only');
      expect(editor.getByTestId('session-editor-counts-as')).toHaveTextContent('Counts as VO2max');
      expect(editor.getByTestId('session-editor-anchor')).toHaveTextContent('Anchor: FTP 280 W, their own.');
      expect(editor.getByTestId('session-editor-findings')).toHaveTextContent('Inside the tested range.');
      expect(editor.queryByTestId('session-editor-outside-evidence')).toBeNull();
    });

    it("puts the day's sport first and offers only the families written for it", async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      await user.click(editor.getByRole('button', { name: 'From a family' }));

      const family = await editor.findByLabelText('Family');
      expect(editor.getByLabelText('Run')).toBeChecked();
      expect(within(family).getAllByRole('option').map((option) => option.textContent)).toEqual([
        'Short VO2max intervals',
        'Endurance',
      ]);
      // A run day built from a family replaces its own session with the family's format.
      expect(editor.getByTestId('session-editor-replacement')).toHaveTextContent(
        'Replaces Easy 45 min with 30/15 (Rønnestad).',
      );
      expect(editor.getByRole('button', { name: 'Save to draft' })).toBeEnabled();

      await user.click(editor.getByLabelText('Ride'));
      expect(within(editor.getByLabelText('Family')).getAllByRole('option').map((option) => option.textContent)).toEqual([
        'Short VO2max intervals',
        'Sweet spot',
        'Endurance',
      ]);
      // Only the endurance family has a swim: picking it moves the build there.
      await user.click(editor.getByLabelText('Swim'));
      expect(editor.getByLabelText('Family')).toHaveValue('endurance');
      expect(editor.getByLabelText('Format')).toHaveValue('steady');
    });

    it("says when no family has the day's sport, above the build it opens on in another", async () => {
      api.getSubjectFamilies.mockResolvedValue({ families: CATALOGUE.families.filter((f) => f.slug === 'sweet-spot') });
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      await user.click(editor.getByRole('button', { name: 'From a family' }));

      expect(await editor.findByTestId('session-editor-no-family-for-sport')).toHaveTextContent(
        'No workout family has a form for this sport: Run.',
      );
      expect(editor.getByLabelText('Family')).toHaveValue('sweet-spot');
      expect(editor.getByLabelText('Ride')).toBeChecked();
    });

    it("keeps each mode's work when the coach switches between them", async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      const title = editor.getByLabelText('Title');
      await user.clear(title);
      await user.type(title, 'Easy 50 min');

      await user.click(editor.getByRole('button', { name: 'From a family' }));
      const reps = await editor.findByLabelText('Reps per set');
      await user.clear(reps);
      await user.type(reps, '12');

      await user.click(editor.getByRole('button', { name: 'This session' }));
      expect(editor.getByLabelText('Title')).toHaveValue('Easy 50 min');
      expect(editor.getByTestId('session-editor-replacement')).toHaveTextContent('Replaces Easy 45 min with Easy 50 min.');

      await user.click(editor.getByRole('button', { name: 'From a family' }));
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
      expect(await editor.findByText('Work · 30s · 120% FTP ×14')).toBeInTheDocument();
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
      await user.click(await editor.findByLabelText('Run'));
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
      await user.click(await editor.findByLabelText('Run'));
      await user.click(editor.getByRole('button', { name: 'Save to draft' }));

      expect(await screen.findByText('This session could not be saved.')).toBeInTheDocument();
      expect(screen.getByRole('dialog', { name: 'Edit Tue, Nov 24' })).toBeInTheDocument();
    });

    it('says when the families are still loading, when there are none, and when they failed — in this mode alone', async () => {
      const user = setupUser();
      let answer: (value: FamilyCatalogueResponse) => void = () => {};
      api.getSubjectFamilies.mockReturnValueOnce(new Promise<FamilyCatalogueResponse>((resolve) => (answer = resolve)));
      const { unmount } = renderCard();

      const editor = await openEasyRun(user);
      expect(editor.queryByTestId('session-editor-loading')).toBeNull();
      await user.click(editor.getByRole('button', { name: 'From a family' }));
      expect(editor.getByTestId('session-editor-loading')).toHaveTextContent('Loading families…');
      await act(async () => answer({ families: [] }));
      expect(await editor.findByTestId('session-editor-no-families')).toHaveTextContent(
        'No workout families are available for Alice Runner.',
      );
      unmount();

      api.getSubjectFamilies.mockRejectedValueOnce(new Error('down'));
      renderCard();
      const failed = await openEasyRun(user);
      // The session mode works on without them.
      expect(failed.getByLabelText('Title')).toHaveValue('Easy 45 min');
      await user.click(failed.getByRole('button', { name: 'From a family' }));
      expect(await failed.findByTestId('session-editor-load-failed')).toHaveTextContent(
        'The workout families could not be loaded.',
      );
    });
  });

  describe('undoing the edits', () => {
    it('says the day was edited, and puts it back as it stood before the first edit', async () => {
      const user = setupUser();
      renderCard({ blocks: planBlocks(EDITED_PLAN) });

      const editor = await openEasyRun(user);
      expect(editor.getByTestId('session-editor-edited')).toHaveTextContent('You edited this day in your draft.');
      const restore = editor.getByRole('button', { name: 'Undo your edits' });
      expect(restore).toHaveAccessibleDescription('Puts back Easy 45 min, as the day stood before your first edit.');

      await user.click(restore);

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
      expect(editor.getByRole('button', { name: 'Undo your edits' })).toHaveAccessibleDescription(
        'Puts back the rest day, as the day stood before your first edit.',
      );
    });

    it('says a refused restore and keeps the sheet open', async () => {
      api.restoreSubjectDraftDay.mockRejectedValue(new Error('boom'));
      const user = setupUser();
      renderCard({ blocks: planBlocks(EDITED_PLAN) });

      const editor = await openEasyRun(user);
      await user.click(editor.getByRole('button', { name: 'Undo your edits' }));

      expect(await screen.findByText('This day could not be restored.')).toBeInTheDocument();
      expect(screen.getByRole('dialog', { name: 'Edit Mon, Nov 23' })).toBeInTheDocument();
    });

    it('offers no undo on a day the coach has not edited', async () => {
      const user = setupUser();
      renderCard();

      const editor = await openEasyRun(user);
      expect(editor.queryByTestId('session-editor-edited')).toBeNull();
      expect(editor.queryByRole('button', { name: 'Undo your edits' })).toBeNull();
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
      expect(editor.getByRole('button', { name: 'This session' })).toHaveAttribute('aria-pressed', 'true');
      expect(editor.queryByRole('switch', { name: 'Rest day' })).toBeNull();
      expect(editor.getByLabelText('Title')).toHaveValue('');
      // In the sport of the date's first session.
      expect(editor.getByLabelText('Sport')).toHaveValue('ride');
      expect(editor.queryByTestId('session-editor-edited')).toBeNull();
      expect(editor.queryByTestId('session-editor-remove')).toBeNull();
      expect(editor.queryByTestId('session-editor-replacement')).toBeNull();
      const save = editor.getByRole('button', { name: 'Save to draft' });
      expect(save).toBeDisabled();

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
      await user.click(editor.getByRole('button', { name: 'From a family' }));
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
      await user.click(editor.getByRole('button', { name: 'From a family' }));
      expect(await editor.findByLabelText('Ride')).toBeChecked();
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
      const restore = added.getByRole('button', { name: 'Undo your edits' });
      expect(restore).toHaveAccessibleDescription('Removes this session: you added it.');
      await user.click(restore);
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
      expect(screen.getByText('Tue, Nov 24 already holds 5 sessions.')).toBeInTheDocument();
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
