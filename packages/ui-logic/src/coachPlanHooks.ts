// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hooks behind a coach's plan thread on both clients — open it from Group info, read the live plan, publish or discard the draft
// ABOUTME: What a plan card's draft controls show is decided here once, so web and the phone draw the same state (carnet#1069/#1070)

import { useCallback, useEffect, useRef, useState } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { ChatApi } from '@pierre/api-client';
import { PLAN_DRAFT_KEYS, PLAN_THREAD_KEYS, type PlanDraftConflict, type Translate } from '@pierre/chat-utils';
import {
  instantMs,
  type AthletePlanResponse,
  type Conversation,
  type DraftState,
  type PlanPublishConflict,
  type SubjectPlanResponse,
} from '@pierre/shared-types';
import { refusalReason } from './apiError';

/** The chat API calls a plan thread makes, bound by each client. */
export interface CoachPlanApis {
  chatApi: Pick<
    ChatApi,
    | 'openAthleteThread'
    | 'getAthletePlan'
    | 'getSubjectPlan'
    | 'publishSubjectPlan'
    | 'discardSubjectPlanDraft'
    | 'listParticipants'
  >;
}

/** The refusal code of a coaching link the athlete stopped sharing on. */
export const COACH_SHARING_REVOKED = 'coach_sharing_revoked';

/** The refusal code of a publish or discard that found no draft to act on. */
const NO_DRAFT = 'no_draft';

/** How long the subject-plan read stays fresh; a newer draft on a card asks again sooner. */
const SUBJECT_PLAN_STALE_MS = 30_000;

/**
 * The machine code a plan-thread refusal names: the body's `error_code`,
 * which the subject-plan routes answer with, else the `details.reason` every
 * other platform refusal carries. `undefined` when it names neither.
 */
export function planRefusalCode(error: unknown): string | undefined {
  const data = (error as { response?: { data?: { error_code?: unknown } } } | null | undefined)?.response
    ?.data;
  if (typeof data?.error_code === 'string') {
    return data.error_code;
  }
  return refusalReason(error);
}

/**
 * The sentence Group info shows when `openThread` is refused for `name`. An
 * athlete who stopped sharing with the coach is said as such, since trying
 * again cannot help; every other refusal asks the coach to try again.
 */
export function openThreadRefusalText(error: unknown, name: string, t: Translate): string {
  return planRefusalCode(error) === COACH_SHARING_REVOKED
    ? t(PLAN_DRAFT_KEYS.consentRevoked, { name })
    : t(PLAN_THREAD_KEYS.openError);
}

/**
 * What the subject-plan read came to. An athlete who stopped sharing with the
 * coach is an answer the card states, not a failure, so it is a value here.
 */
export type SubjectPlanRead =
  | { status: 'ok'; plan: SubjectPlanResponse }
  | { status: 'consentRevoked' };

/**
 * What the read-only athlete plan came to (carnet#1244). An athlete who
 * stopped sharing with the coach is an answer the sheet states, as the
 * subject-plan read does.
 */
export type AthletePlanRead =
  | { status: 'ok'; plan: AthletePlanResponse }
  | { status: 'consentRevoked' };

/** What `useAthletePlan` hands the read-only plan sheet. */
export interface UseAthletePlanResult {
  /** The read; `undefined` while it loads, and after it failed. */
  read: AthletePlanRead | undefined;
  isLoading: boolean;
  /** The read failed for a reason other than revoked consent. */
  isError: boolean;
  refetch: () => void;
}

/** What the last Publish or Discard on a card came to, as far as the card shows it. */
export type PlanDraftOutcome =
  | { kind: 'none' }
  | { kind: 'published' }
  | { kind: 'conflict'; conflict: PlanDraftConflict }
  | { kind: 'consentRevoked' };

/**
 * What a plan card's draft controls show.
 *
 * - `hidden`: nothing — the viewer does not own the thread, the card shows a
 *   draft that is no longer the open one, or the read has not answered yet.
 * - `draft`: the badge, Publish and Discard.
 * - `conflict`: a publish was refused over what the athlete changed — the
 *   badge, the conflict, Publish anyway and Discard.
 * - `published`: the publish landed and the athlete was told.
 * - `consentRevoked`: the athlete stopped sharing; nothing can be published.
 */
export type PlanDraftCardState =
  | { kind: 'hidden' }
  | { kind: 'draft' }
  | { kind: 'conflict'; conflict: PlanDraftConflict }
  | { kind: 'published' }
  | { kind: 'consentRevoked' };

/** What decides a card's draft controls. */
export interface PlanDraftCardInput {
  /** Whether the viewer owns the thread; `undefined` while that is being read. */
  ownsThread: boolean | undefined;
  /** The draft the card's block shows. */
  cardDraft: DraftState;
  /** The subject-plan read; `undefined` until it answers. */
  read: SubjectPlanRead | undefined;
  outcome: PlanDraftOutcome;
}

/**
 * Whether the card's draft is the coach's open draft as it stands: the same
 * draft, and no later save over it. A card from an earlier save in the
 * thread shows a version the coach has since changed, and publishing from
 * it would publish something the card does not show.
 */
function isOpenDraft(open: DraftState, card: DraftState): boolean {
  return open.draft_id === card.draft_id && instantMs(card.updated_at) >= instantMs(open.updated_at);
}

/**
 * The state a plan card's draft controls are in.
 *
 * Only the thread's owner — the coach — ever sees them: the athlete reads
 * neither the badge nor the buttons. A publish that landed stays said after
 * the draft is gone; an athlete who stopped sharing is said on every draft
 * card, since nothing in the thread can be published any more.
 */
export function planDraftCardState(input: PlanDraftCardInput): PlanDraftCardState {
  const { ownsThread, cardDraft, read, outcome } = input;
  if (ownsThread !== true) return { kind: 'hidden' };
  if (outcome.kind === 'published') return { kind: 'published' };
  if (outcome.kind === 'consentRevoked' || read?.status === 'consentRevoked') {
    return { kind: 'consentRevoked' };
  }
  const open = read?.plan.draft_state ?? null;
  if (open === null || !isOpenDraft(open, cardDraft)) return { kind: 'hidden' };
  if (outcome.kind === 'conflict') return { kind: 'conflict', conflict: outcome.conflict };
  return { kind: 'draft' };
}

/** A refused publish's conflict as the card names it: each week once, in the order the server listed them. */
export function draftConflictOf(conflict: PlanPublishConflict): PlanDraftConflict {
  return {
    weeks: [...new Set(conflict.conflicts.map((entry) => entry.week_start))],
    seasonRelaid: conflict.season_relaid,
  };
}

/**
 * What a Publish or Discard press came to for the client: `shown` when the
 * card states the result itself, `failed` with the refusal when it cannot and
 * the client has to say it. Never a rejection.
 */
export type PlanDraftActionResult = { kind: 'shown' } | { kind: 'failed'; error: unknown };

/** What `usePlanDraftCard` hands a plan card. */
export interface UsePlanDraftCardResult {
  state: PlanDraftCardState;
  /** Publish the draft; `overwrite` publishes over the athlete's changes. */
  publish: (overwrite: boolean) => Promise<PlanDraftActionResult>;
  /** Throw the draft away; the athlete's live plan stays as it is. */
  discard: () => Promise<PlanDraftActionResult>;
  isPublishing: boolean;
  isDiscarding: boolean;
}

/** What `useOpenAthleteThread` hands Group info. */
export interface UseOpenAthleteThreadResult {
  /**
   * Open the caller's plan thread for `athleteUserId` in `groupId` and
   * resolve with it once the conversation list holds it. Rejects with the
   * server's refusal.
   */
  openThread: (athleteUserId: string, groupId: string) => Promise<Conversation>;
  /** The athlete whose thread is being opened, while one is. */
  pendingAthleteId: string | null;
}

/** A publish or discard the server answered with something the card states. */
type DraftActionAnswer =
  | { kind: 'published' }
  | { kind: 'conflict'; conflict: PlanDraftConflict }
  | { kind: 'consentRevoked' }
  | { kind: 'discarded' }
  | { kind: 'gone' };

/**
 * The answer a refused draft action carries for the card, or `undefined` for
 * a refusal the client has to word: the athlete stopped sharing, or the draft
 * is gone (`no_draft`) — published or discarded from elsewhere — and a
 * re-read shows it. Every other 404 — the athlete left the group, the thread
 * is not the caller's — is a refusal the client words, never a draft gone.
 */
function refusedDraftAction(error: unknown): DraftActionAnswer | undefined {
  const code = planRefusalCode(error);
  if (code === COACH_SHARING_REVOKED) return { kind: 'consentRevoked' };
  if (code === NO_DRAFT) return { kind: 'gone' };
  return undefined;
}

/**
 * Build the plan-thread hooks over one client's chat API.
 *
 * Each client binds this once, in its own `hooks/useCoachPlan`, with calls
 * that reach its API when they run — so importing the binding touches no
 * client. Everything else — the query keys, what a publish refreshes, which
 * controls a card shows — is this one definition.
 */
export function createCoachPlanHooks({ chatApi }: CoachPlanApis) {
  /**
   * Open a coach's plan thread from Group info. The list is refetched before
   * the promise resolves, so the thread the caller navigates to is already a
   * row of it, with its header and its athlete.
   */
  function useOpenAthleteThread(): UseOpenAthleteThreadResult {
    const queryClient = useQueryClient();
    const mutation = useMutation({
      mutationFn: ({ athleteUserId, groupId }: { athleteUserId: string; groupId: string }) =>
        chatApi.openAthleteThread(athleteUserId, groupId),
      onSuccess: () => queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() }),
    });
    return {
      openThread: (athleteUserId, groupId) => mutation.mutateAsync({ athleteUserId, groupId }),
      pendingAthleteId: mutation.isPending ? (mutation.variables?.athleteUserId ?? null) : null,
    };
  }

  /**
   * A coached athlete's live plan, read-only, for the sheet Group info opens
   * from a member row. Nothing is read until the sheet names an athlete.
   */
  function useAthletePlan(athleteUserId: string | null, groupId: string | null): UseAthletePlanResult {
    const query = useQuery({
      queryKey: QUERY_KEYS.chat.athletePlan(athleteUserId, groupId),
      queryFn: async (): Promise<AthletePlanRead> => {
        try {
          return {
            status: 'ok',
            plan: await chatApi.getAthletePlan(athleteUserId as string, groupId as string),
          };
        } catch (error) {
          if (planRefusalCode(error) === COACH_SHARING_REVOKED) return { status: 'consentRevoked' };
          throw error;
        }
      },
      enabled: athleteUserId !== null && groupId !== null,
      staleTime: SUBJECT_PLAN_STALE_MS,
    });
    return {
      read: query.data,
      isLoading: query.isLoading,
      isError: query.isError,
      refetch: () => void query.refetch(),
    };
  }

  /**
   * Whether `viewerId` owns the conversation, from its participant list —
   * the same read the participants control keeps. `undefined` until it
   * answers, and while either id is missing.
   */
  function useOwnsConversation(conversationId: string | null, viewerId: string | null): boolean | undefined {
    const query = useQuery({
      queryKey: QUERY_KEYS.chat.participants(conversationId),
      queryFn: () => chatApi.listParticipants(conversationId as string),
      enabled: conversationId !== null && viewerId !== null,
    });
    if (query.data === undefined || viewerId === null) return undefined;
    return query.data.some((participant) => participant.role === 'owner' && participant.user_id === viewerId);
  }

  /**
   * The plan thread's athlete as the coach reads them. An athlete who
   * stopped sharing reads as `consentRevoked`; any other refusal fails the
   * read as usual.
   */
  function useSubjectPlan(conversationId: string | null, enabled: boolean) {
    const query = useQuery({
      queryKey: QUERY_KEYS.chat.subjectPlan(conversationId),
      queryFn: async (): Promise<SubjectPlanRead> => {
        try {
          return { status: 'ok', plan: await chatApi.getSubjectPlan(conversationId as string) };
        } catch (error) {
          if (planRefusalCode(error) === COACH_SHARING_REVOKED) return { status: 'consentRevoked' };
          throw error;
        }
      },
      enabled: enabled && conversationId !== null,
      staleTime: SUBJECT_PLAN_STALE_MS,
    });
    return { read: query.data, dataUpdatedAt: query.dataUpdatedAt, refetch: query.refetch };
  }

  /**
   * Everything a plan card needs for the draft its block shows: the state its
   * controls are in, and Publish and Discard.
   *
   * Nothing is read unless the block shows a draft: the participant list
   * says whether the viewer owns the thread, and only then is the subject
   * plan read, to know whether the card's draft is still the open one. A card
   * whose draft is newer than that read asks again once, so the card a turn
   * just added is not judged against the read the previous card made.
   */
  function usePlanDraftCard(
    conversationId: string | null,
    viewerId: string | null,
    cardDraft: DraftState | null,
  ): UsePlanDraftCardResult {
    const queryClient = useQueryClient();
    const showsDraft = cardDraft !== null && conversationId !== null;
    const ownsThread = useOwnsConversation(showsDraft ? conversationId : null, viewerId);
    const { read, dataUpdatedAt, refetch } = useSubjectPlan(conversationId, showsDraft && ownsThread === true);
    const [outcome, setOutcome] = useState<PlanDraftOutcome>({ kind: 'none' });

    // One re-read per draft version, so a clock that runs ahead of this one
    // cannot turn the comparison into a loop.
    const askedFor = useRef<string | null>(null);
    const cardVersion = cardDraft ? `${cardDraft.draft_id}@${cardDraft.updated_at}` : null;
    useEffect(() => {
      if (cardDraft === null || cardVersion === null || read === undefined || askedFor.current === cardVersion) return;
      if (instantMs(cardDraft.updated_at) > dataUpdatedAt) {
        askedFor.current = cardVersion;
        void refetch();
      }
    }, [cardDraft, cardVersion, read, dataUpdatedAt, refetch]);

    const settle = useCallback(async () => {
      if (conversationId === null) return;
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.subjectPlan(conversationId) }),
        queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.messages(conversationId) }),
      ]);
    }, [conversationId, queryClient]);

    const publishMutation = useMutation({
      mutationFn: async (overwrite: boolean): Promise<DraftActionAnswer> => {
        try {
          const result = await chatApi.publishSubjectPlan(conversationId as string, {
            overwrite_athlete_changes: overwrite,
          });
          return result.status === 'conflict'
            ? { kind: 'conflict', conflict: draftConflictOf(result.conflict) }
            : { kind: 'published' };
        } catch (error) {
          const answer = refusedDraftAction(error);
          if (answer === undefined) throw error;
          return answer;
        }
      },
    });

    const discardMutation = useMutation({
      mutationFn: async (): Promise<DraftActionAnswer> => {
        try {
          await chatApi.discardSubjectPlanDraft(conversationId as string);
          return { kind: 'discarded' };
        } catch (error) {
          const answer = refusedDraftAction(error);
          if (answer === undefined) throw error;
          return answer;
        }
      },
    });

    const apply = useCallback(
      async (answer: DraftActionAnswer): Promise<PlanDraftActionResult> => {
        switch (answer.kind) {
          case 'conflict':
            setOutcome({ kind: 'conflict', conflict: answer.conflict });
            return { kind: 'shown' };
          case 'consentRevoked':
            setOutcome({ kind: 'consentRevoked' });
            return { kind: 'shown' };
          case 'published':
            setOutcome({ kind: 'published' });
            await settle();
            return { kind: 'shown' };
          case 'discarded':
          case 'gone':
            setOutcome({ kind: 'none' });
            await settle();
            return { kind: 'shown' };
        }
      },
      [settle],
    );

    // `mutateAsync` is stable across renders; the mutation result object is not.
    const { mutateAsync: runPublish } = publishMutation;
    const { mutateAsync: runDiscard } = discardMutation;

    const publish = useCallback(
      async (overwrite: boolean): Promise<PlanDraftActionResult> => {
        if (conversationId === null) return { kind: 'shown' };
        try {
          return await apply(await runPublish(overwrite));
        } catch (error) {
          return { kind: 'failed', error };
        }
      },
      [apply, conversationId, runPublish],
    );

    const discard = useCallback(async (): Promise<PlanDraftActionResult> => {
      if (conversationId === null) return { kind: 'shown' };
      try {
        return await apply(await runDiscard());
      } catch (error) {
        return { kind: 'failed', error };
      }
    }, [apply, conversationId, runDiscard]);

    const state: PlanDraftCardState =
      cardDraft === null
        ? { kind: 'hidden' }
        : planDraftCardState({ ownsThread, cardDraft, read, outcome });

    return {
      state,
      publish,
      discard,
      isPublishing: publishMutation.isPending,
      isDiscarding: discardMutation.isPending,
    };
  }

  return { useOpenAthleteThread, useAthletePlan, usePlanDraftCard };
}
