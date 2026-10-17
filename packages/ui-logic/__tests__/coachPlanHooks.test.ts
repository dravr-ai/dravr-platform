// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tests for the shared plan-thread hooks — opening a thread from Group info, and the draft card's state, Publish and Discard
// ABOUTME: Both clients bind these, so a divergence here would show the coach different controls on web and on the phone

import { describe, it, expect, vi } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { act, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type {
  Conversation,
  ConversationParticipant,
  DraftState,
  PublishSubjectPlanResult,
  SubjectPlanResponse,
} from '@pierre/shared-types';
import type { Translate } from '@pierre/chat-utils';
import en from '../../i18n/src/locales/en/translation.json';
import {
  COACH_SHARING_REVOKED,
  createCoachPlanHooks,
  draftConflictOf,
  openThreadRefusalText,
  planDraftCardState,
  planRefusalCode,
  type CoachPlanApis,
  type SubjectPlanRead,
} from '../src/coachPlanHooks';

const CONVERSATION_ID = 'conv-plan-alice';
const COACH_ID = 'user-coach';
const ATHLETE_ID = 'user-alice';

const DRAFT: DraftState = {
  draft_id: 'draft-7',
  updated_at: '2026-10-16T14:05:00Z',
  conflicts: [],
  season_relaid: false,
};

/** The English catalogue, resolved the way a client's `t` resolves a key and its params. */
const t: Translate = (key, params = {}) => {
  const template = key
    .split('.')
    .reduce<unknown>((node, part) => (node as Record<string, unknown> | undefined)?.[part], en);
  if (typeof template !== 'string') throw new Error(`en has no ${key}`);
  return template.replace(/\{\{(\w+)\}\}/g, (_, name: string) => String(params[name]));
};

/** A refusal shaped as the shared axios instance rejects it. */
function refusal(status: number, data: Record<string, unknown>) {
  return Object.assign(new Error(`Request failed with status code ${status}`), { response: { status, data } });
}

function subjectPlan(draftState: DraftState | null): SubjectPlanResponse {
  return {
    athlete: { user_id: ATHLETE_ID, display_name: 'Alice Runner' },
    today: '2026-10-16',
    live: null,
    draft: null,
    draft_state: draftState,
  };
}

function participants(ownerId: string): ConversationParticipant[] {
  return [
    { user_id: ownerId, role: 'owner', added_by: ownerId, added_at: '2026-10-16T12:00:00Z' },
    { user_id: 'user-other', role: 'member', added_by: ownerId, added_at: '2026-10-16T12:01:00Z' },
  ];
}

function fakeChatApi() {
  return {
    openAthleteThread: vi.fn<(athleteUserId: string, groupId: string) => Promise<Conversation>>(),
    getSubjectPlan: vi.fn<(conversationId: string) => Promise<SubjectPlanResponse>>(),
    publishSubjectPlan: vi.fn<
      (conversationId: string, request: { overwrite_athlete_changes: boolean }) => Promise<PublishSubjectPlanResult>
    >(),
    discardSubjectPlanDraft: vi.fn<(conversationId: string) => Promise<void>>(),
    listParticipants: vi.fn<(conversationId: string) => Promise<ConversationParticipant[]>>(),
  };
}

function setup() {
  const api = fakeChatApi();
  api.listParticipants.mockResolvedValue(participants(COACH_ID));
  api.getSubjectPlan.mockResolvedValue(subjectPlan(DRAFT));
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const wrapper = ({ children }: { children: ReactNode }) =>
    createElement(QueryClientProvider, { client }, children);
  const hooks = createCoachPlanHooks({ chatApi: api as unknown as CoachPlanApis['chatApi'] });
  return { api, client, wrapper, hooks };
}

describe('planRefusalCode', () => {
  it('reads the error_code a subject-plan route answers with, else the details.reason', () => {
    expect(planRefusalCode(refusal(403, { error_code: COACH_SHARING_REVOKED }))).toBe('coach_sharing_revoked');
    expect(planRefusalCode(refusal(403, { code: 'PermissionDenied', details: { reason: 'not_group_coach' } }))).toBe(
      'not_group_coach',
    );
    expect(planRefusalCode(refusal(500, { message: 'boom' }))).toBeUndefined();
    expect(planRefusalCode(new Error('offline'))).toBeUndefined();
  });
});

describe('openThreadRefusalText', () => {
  it('names the athlete who stopped sharing, however the refusal carries the code', () => {
    const stopped = "Alice Runner stopped sharing with you, so their plan can't be changed.";
    expect(openThreadRefusalText(refusal(403, { error_code: COACH_SHARING_REVOKED }), 'Alice Runner', t)).toBe(stopped);
    expect(
      openThreadRefusalText(
        refusal(403, { code: 'PermissionDenied', details: { reason: COACH_SHARING_REVOKED } }),
        'Alice Runner',
        t,
      ),
    ).toBe(stopped);
  });

  it('asks the coach to try again on every other refusal', () => {
    const retry = "We couldn't open this plan thread. Please try again.";
    expect(
      openThreadRefusalText(refusal(404, { code: 'ResourceNotFound', details: { reason: 'not_a_member' } }), 'Alice Runner', t),
    ).toBe(retry);
    expect(openThreadRefusalText(refusal(500, { message: 'boom' }), 'Alice Runner', t)).toBe(retry);
    expect(openThreadRefusalText(new Error('offline'), 'Alice Runner', t)).toBe(retry);
  });
});

describe('draftConflictOf', () => {
  it('names each conflicting week once, in the order the server listed them', () => {
    expect(
      draftConflictOf({
        error_code: 'plan_publish_conflict',
        conflicts: [
          { week_start: '2026-10-26', reason: 'athlete_changed_week' },
          { week_start: '2026-10-19', reason: 'athlete_changed_week' },
          { week_start: '2026-10-26', reason: 'athlete_changed_week' },
        ],
        season_relaid: true,
      }),
    ).toEqual({ weeks: ['2026-10-26', '2026-10-19'], seasonRelaid: true });
  });
});

describe('planDraftCardState', () => {
  const ok = (draftState: DraftState | null): SubjectPlanRead => ({ status: 'ok', plan: subjectPlan(draftState) });
  const none = { kind: 'none' } as const;

  it('shows nothing to anyone but the thread owner — the athlete never sees the controls', () => {
    expect(planDraftCardState({ ownsThread: false, cardDraft: DRAFT, read: ok(DRAFT), outcome: none })).toEqual({
      kind: 'hidden',
    });
    expect(planDraftCardState({ ownsThread: undefined, cardDraft: DRAFT, read: ok(DRAFT), outcome: none })).toEqual({
      kind: 'hidden',
    });
  });

  it("shows the owner the badge and the buttons while the card's draft is the open one", () => {
    expect(planDraftCardState({ ownsThread: true, cardDraft: DRAFT, read: ok(DRAFT), outcome: none })).toEqual({
      kind: 'draft',
    });
  });

  it('hides them on a card from an earlier save, and once the draft is gone', () => {
    const later = { ...DRAFT, updated_at: '2026-10-16T15:00:00Z' };
    expect(planDraftCardState({ ownsThread: true, cardDraft: DRAFT, read: ok(later), outcome: none })).toEqual({
      kind: 'hidden',
    });
    expect(planDraftCardState({ ownsThread: true, cardDraft: DRAFT, read: ok(null), outcome: none })).toEqual({
      kind: 'hidden',
    });
    expect(
      planDraftCardState({ ownsThread: true, cardDraft: DRAFT, read: ok({ ...DRAFT, draft_id: 'draft-8' }), outcome: none }),
    ).toEqual({ kind: 'hidden' });
    expect(planDraftCardState({ ownsThread: true, cardDraft: DRAFT, read: undefined, outcome: none })).toEqual({
      kind: 'hidden',
    });
  });

  it('compares stamps written to the microsecond, and in an offset form, as instants', () => {
    // The server writes `updated_at` with chrono: six fractional digits from Postgres.
    const micro = { ...DRAFT, updated_at: '2026-10-16T14:05:00.482913Z' };
    const savedAfter = { ...DRAFT, updated_at: '2026-10-16T14:05:00.731004Z' };
    expect(planDraftCardState({ ownsThread: true, cardDraft: micro, read: ok(micro), outcome: none })).toEqual({
      kind: 'draft',
    });
    expect(planDraftCardState({ ownsThread: true, cardDraft: micro, read: ok(savedAfter), outcome: none })).toEqual({
      kind: 'hidden',
    });
    // The same instant, written with an offset, is the same save.
    const offset = { ...DRAFT, updated_at: '2026-10-16T10:05:00.482913-04:00' };
    expect(planDraftCardState({ ownsThread: true, cardDraft: offset, read: ok(micro), outcome: none })).toEqual({
      kind: 'draft',
    });
  });

  it('keeps a landed publish said after the draft is gone', () => {
    expect(
      planDraftCardState({ ownsThread: true, cardDraft: DRAFT, read: ok(null), outcome: { kind: 'published' } }),
    ).toEqual({ kind: 'published' });
  });

  it('shows the conflict a refused publish named while the draft is still open', () => {
    const conflict = { weeks: ['2026-10-19'], seasonRelaid: false };
    expect(
      planDraftCardState({ ownsThread: true, cardDraft: DRAFT, read: ok(DRAFT), outcome: { kind: 'conflict', conflict } }),
    ).toEqual({ kind: 'conflict', conflict });
  });

  it('says the athlete stopped sharing, from the read or from a refused action', () => {
    expect(
      planDraftCardState({ ownsThread: true, cardDraft: DRAFT, read: { status: 'consentRevoked' }, outcome: none }),
    ).toEqual({ kind: 'consentRevoked' });
    expect(
      planDraftCardState({ ownsThread: true, cardDraft: DRAFT, read: ok(DRAFT), outcome: { kind: 'consentRevoked' } }),
    ).toEqual({ kind: 'consentRevoked' });
  });
});

describe('useOpenAthleteThread', () => {
  it('opens the thread for the member in the group, and refreshes the list before it resolves', async () => {
    const { api, client, wrapper, hooks } = setup();
    const invalidate = vi.spyOn(client, 'invalidateQueries');
    let release: (conversation: Conversation) => void = () => undefined;
    api.openAthleteThread.mockReturnValue(
      new Promise<Conversation>((resolve) => {
        release = resolve;
      }),
    );
    const { result } = renderHook(() => hooks.useOpenAthleteThread(), { wrapper });

    let opened: Promise<Conversation> = Promise.resolve({} as Conversation);
    act(() => {
      opened = result.current.openThread(ATHLETE_ID, 'group-1');
    });
    await waitFor(() => expect(result.current.pendingAthleteId).toBe(ATHLETE_ID));

    release({ id: CONVERSATION_ID, title: 'Alice Runner', message_count: 0, created_at: '', updated_at: '' });
    await expect(opened).resolves.toMatchObject({ id: CONVERSATION_ID });

    expect(api.openAthleteThread).toHaveBeenCalledWith(ATHLETE_ID, 'group-1');
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['chat-conversations'] });
    await waitFor(() => expect(result.current.pendingAthleteId).toBeNull());
  });

  it('rejects with the refusal, so Group info can say the thread did not open', async () => {
    const { api, wrapper, hooks } = setup();
    api.openAthleteThread.mockRejectedValue(refusal(403, { error_code: 'not_group_coach' }));
    const { result } = renderHook(() => hooks.useOpenAthleteThread(), { wrapper });

    await expect(result.current.openThread(ATHLETE_ID, 'group-1')).rejects.toThrow('403');
  });
});

describe('usePlanDraftCard', () => {
  function renderCard(ctx: ReturnType<typeof setup>, viewerId = COACH_ID, draft: DraftState | null = DRAFT) {
    return renderHook(() => ctx.hooks.usePlanDraftCard(CONVERSATION_ID, viewerId, draft), { wrapper: ctx.wrapper });
  }

  it('reads nothing for a card that shows no draft', async () => {
    const ctx = setup();
    const { result } = renderCard(ctx, COACH_ID, null);

    expect(result.current.state).toEqual({ kind: 'hidden' });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(ctx.api.listParticipants).not.toHaveBeenCalled();
    expect(ctx.api.getSubjectPlan).not.toHaveBeenCalled();
  });

  it("never reads the coach's subject plan for a viewer who does not own the thread", async () => {
    const ctx = setup();
    const { result } = renderCard(ctx, ATHLETE_ID);

    await waitFor(() => expect(ctx.api.listParticipants).toHaveBeenCalledWith(CONVERSATION_ID));
    expect(result.current.state).toEqual({ kind: 'hidden' });
    expect(ctx.api.getSubjectPlan).not.toHaveBeenCalled();
  });

  it('shows the owner the draft controls once the read confirms the card is the open draft', async () => {
    const ctx = setup();
    const { result } = renderCard(ctx);

    await waitFor(() => expect(result.current.state).toEqual({ kind: 'draft' }));
    expect(ctx.api.getSubjectPlan).toHaveBeenCalledWith(CONVERSATION_ID);
  });

  it('asks again, once, when the card shows a draft newer than the read it was judged against', async () => {
    const ctx = setup();
    // The read predates this card's save: it still holds the previous version.
    const newer = { ...DRAFT, updated_at: new Date(Date.now() + 60_000).toISOString() };
    ctx.api.getSubjectPlan.mockResolvedValue(subjectPlan(DRAFT));
    const { result } = renderCard(ctx, COACH_ID, newer);

    await waitFor(() => expect(ctx.api.getSubjectPlan).toHaveBeenCalledTimes(2));
    // The card is newer than anything the server has said, so it is the open one.
    expect(result.current.state).toEqual({ kind: 'draft' });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(ctx.api.getSubjectPlan).toHaveBeenCalledTimes(2);
  });

  it('says the athlete stopped sharing when the read is refused for it, without failing the read', async () => {
    const ctx = setup();
    ctx.api.getSubjectPlan.mockRejectedValue(refusal(403, { error_code: COACH_SHARING_REVOKED }));
    const { result } = renderCard(ctx);

    await waitFor(() => expect(result.current.state).toEqual({ kind: 'consentRevoked' }));
  });

  it('publishes, says so, and refreshes the subject plan and the thread', async () => {
    const ctx = setup();
    ctx.api.publishSubjectPlan.mockResolvedValue({
      status: 'published',
      published: { plan_id: 'plan-2', superseded_plan_id: 'plan-1', athlete_notified: true },
    });
    const invalidate = vi.spyOn(ctx.client, 'invalidateQueries');
    const { result } = renderCard(ctx);
    await waitFor(() => expect(result.current.state).toEqual({ kind: 'draft' }));
    ctx.api.getSubjectPlan.mockResolvedValue(subjectPlan(null));

    let outcome: unknown;
    await act(async () => {
      outcome = await result.current.publish(false);
    });

    expect(outcome).toEqual({ kind: 'shown' });
    expect(ctx.api.publishSubjectPlan).toHaveBeenCalledWith(CONVERSATION_ID, { overwrite_athlete_changes: false });
    expect(result.current.state).toEqual({ kind: 'published' });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['chat', 'subject-plan', CONVERSATION_ID] });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: ['chat-messages', CONVERSATION_ID] });
  });

  it('shows a refused publish as a conflict, and publishes anyway with overwrite', async () => {
    const ctx = setup();
    ctx.api.publishSubjectPlan.mockResolvedValueOnce({
      status: 'conflict',
      conflict: {
        error_code: 'plan_publish_conflict',
        conflicts: [{ week_start: '2026-10-19', reason: 'athlete_changed_week' }],
        season_relaid: true,
      },
    });
    const { result } = renderCard(ctx);
    await waitFor(() => expect(result.current.state).toEqual({ kind: 'draft' }));

    await act(async () => {
      await result.current.publish(false);
    });
    expect(result.current.state).toEqual({
      kind: 'conflict',
      conflict: { weeks: ['2026-10-19'], seasonRelaid: true },
    });

    ctx.api.publishSubjectPlan.mockResolvedValueOnce({
      status: 'published',
      published: { plan_id: 'plan-2', superseded_plan_id: null, athlete_notified: true },
    });
    await act(async () => {
      await result.current.publish(true);
    });
    expect(ctx.api.publishSubjectPlan).toHaveBeenLastCalledWith(CONVERSATION_ID, { overwrite_athlete_changes: true });
    expect(result.current.state).toEqual({ kind: 'published' });
  });

  it('says the athlete stopped sharing when a publish is refused for it', async () => {
    const ctx = setup();
    ctx.api.publishSubjectPlan.mockRejectedValue(refusal(403, { error_code: COACH_SHARING_REVOKED }));
    const { result } = renderCard(ctx);
    await waitFor(() => expect(result.current.state).toEqual({ kind: 'draft' }));

    await act(async () => {
      await result.current.publish(false);
    });

    expect(result.current.state).toEqual({ kind: 'consentRevoked' });
  });

  it('hands any other refusal back for the client to word, and keeps the controls', async () => {
    const ctx = setup();
    const failure = refusal(500, { message: 'boom' });
    ctx.api.publishSubjectPlan.mockRejectedValue(failure);
    const { result } = renderCard(ctx);
    await waitFor(() => expect(result.current.state).toEqual({ kind: 'draft' }));

    let outcome: unknown;
    await act(async () => {
      outcome = await result.current.publish(false);
    });

    expect(outcome).toEqual({ kind: 'failed', error: failure });
    expect(result.current.state).toEqual({ kind: 'draft' });
  });

  it('discards the draft and refreshes, after which the card shows nothing', async () => {
    const ctx = setup();
    ctx.api.discardSubjectPlanDraft.mockResolvedValue(undefined);
    const { result } = renderCard(ctx);
    await waitFor(() => expect(result.current.state).toEqual({ kind: 'draft' }));
    ctx.api.getSubjectPlan.mockResolvedValue(subjectPlan(null));

    await act(async () => {
      await result.current.discard();
    });

    expect(ctx.api.discardSubjectPlanDraft).toHaveBeenCalledWith(CONVERSATION_ID);
    await waitFor(() => expect(result.current.state).toEqual({ kind: 'hidden' }));
  });

  it('treats a draft that is already gone as gone, not as a failure', async () => {
    const ctx = setup();
    // The body the route answers with: the code in `details.reason`.
    ctx.api.discardSubjectPlanDraft.mockRejectedValue(
      refusal(404, { code: 'ResourceNotFound', details: { reason: 'no_draft' } }),
    );
    const { result } = renderCard(ctx);
    await waitFor(() => expect(result.current.state).toEqual({ kind: 'draft' }));
    ctx.api.getSubjectPlan.mockResolvedValue(subjectPlan(null));

    let outcome: unknown;
    await act(async () => {
      outcome = await result.current.discard();
    });

    expect(outcome).toEqual({ kind: 'shown' });
    await waitFor(() => expect(result.current.state).toEqual({ kind: 'hidden' }));
  });

  it('hands a 404 that is not a missing draft back for the client to word', async () => {
    // The athlete left the group: the draft is not gone, the coach is refused.
    const ctx = setup();
    const failure = refusal(404, { code: 'ResourceNotFound', details: { reason: 'not_a_member' } });
    ctx.api.publishSubjectPlan.mockRejectedValue(failure);
    const { result } = renderCard(ctx);
    await waitFor(() => expect(result.current.state).toEqual({ kind: 'draft' }));

    let outcome: unknown;
    await act(async () => {
      outcome = await result.current.publish(false);
    });

    expect(outcome).toEqual({ kind: 'failed', error: failure });
    expect(result.current.state).toEqual({ kind: 'draft' });
  });
});
