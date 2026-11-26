// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The athlete workspace both clients draw — its two threads found or opened, the coach's answers to the draft, and which state its plan and tabs are in
// ABOUTME: One definition for web and phone: each client hands in its groups API and freshness, its list, its plan and day-editor hooks, its words and its error report (carnet#1816, #1818)

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useQueries } from '@tanstack/react-query';
import type { GroupsApi } from '@pierre/api-client';
import type { Translate } from '@pierre/chat-utils';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type {
  CoachingGroup,
  CoachRosterAthlete,
  Conversation,
  PlanDay,
  SubjectPlanResponse,
  WorkoutPlan,
} from '@pierre/shared-types';
import { API_ERROR_KEYS, describeApiError } from './apiError';
import type { SubjectPlanRead, UseOpenAthleteThreadResult, UseOpenGroupRoomResult } from './coachPlanHooks';
import {
  athleteRoomGroup,
  PLAN_WORKSPACE_KEYS,
  roomConversationOf,
  shownPlan,
  usablePlanThread,
  type PlanCalendarDay,
} from './planWorkspace';
import { isSessionMoved, SESSION_EDITOR_KEYS, sessionExpect, type SessionAt } from './sessionEditor';

/** The athlete's own room with the coach. */
export interface WorkspaceRoom {
  group: CoachingGroup;
  /** The coach's conversation in it; `null` until the coach holds one. */
  conversationId: string | null;
}

/** What the workspace knows of its two threads. */
export interface WorkspaceThreads {
  /** The coach's plan thread on the athlete; `null` while it is found or opened. */
  planThreadId: string | null;
  /** The group the plan thread acts under; `null` for an athlete in no group of the coach's. */
  planGroup: CoachingGroup | null;
  /** Why the plan thread could not be opened, until a retry. */
  planError: unknown;
  retryPlanThread: () => void;
  /** The athlete's own room, once the groups are read; `null` when every group of theirs holds other athletes. */
  room: WorkspaceRoom | null;
  /** The groups have been read, so `room` is known. */
  roomKnown: boolean;
  /** A group's read failed: which group is the athlete's room is not known, so no plan thread is opened. */
  groupsFailed: boolean;
  /** Read the failed groups again. */
  retryGroups: () => void;
  /** Open the coach's conversation in the room, which the server finds or opens. */
  openRoom: () => void;
  roomOpening: boolean;
  /** The room could not be opened. */
  roomError: boolean;
}

/** What the Groupe tab holds: the athlete's room, why there is none, or the groups' read that failed. */
export type WorkspaceGroupTab =
  | { kind: 'room'; room: WorkspaceRoom }
  | { kind: 'noRoom' }
  | { kind: 'failed'; retry: () => void };

/**
 * The Groupe tab of `athlete`'s workspace, from what its threads know: the
 * athlete's own room once found, the failed groups' read to try again, or no
 * room of their own once every group was read. `null` — no Groupe tab — while
 * the groups are read, and for an athlete in no group of the coach's.
 */
export function workspaceGroupTab(
  threads: Pick<WorkspaceThreads, 'room' | 'roomKnown' | 'groupsFailed' | 'retryGroups'>,
  athlete: Pick<CoachRosterAthlete, 'groups'>,
): WorkspaceGroupTab | null {
  if (threads.room !== null) return { kind: 'room', room: threads.room };
  if (threads.groupsFailed) return { kind: 'failed', retry: threads.retryGroups };
  if (threads.roomKnown && athlete.groups.length > 0) return { kind: 'noRoom' };
  return null;
}

/**
 * The name the workspace's agent goes by: the plan thread's group's agent,
 * else `fallback` — the brand — while the group is read or names none.
 */
export function workspaceAgentName(planGroup: Pick<CoachingGroup, 'agent_title'> | null, fallback: string): string {
  return planGroup?.agent_title?.trim() || fallback;
}

/** The plan thread's subject-plan read, as the client's bound `useSubjectPlan` answers it. */
export interface WorkspacePlanRead {
  read: SubjectPlanRead | undefined;
  isLoading: boolean;
  isError: boolean;
  refetch: () => unknown;
}

/**
 * What the workspace's plan shows, and what each state needs to be drawn:
 * the retry it offers, the refusal it words, the subject plan and the plan
 * shown — the draft when one is open, else the live plan.
 */
export type WorkspacePlanState =
  /** The athlete is in no group of the coach's: there is no plan thread to work through. */
  | { kind: 'noGroup' }
  /** The plan thread could not be opened; `error` is the refusal the sentence words. */
  | { kind: 'threadFailed'; error: unknown; retry: () => void }
  /** A group's read failed before the plan thread was known. */
  | { kind: 'groupsFailed'; retry: () => void }
  /** The plan thread is still being found or opened, or its plan read. */
  | { kind: 'loading' }
  /** The athlete stopped sharing with the coach. */
  | { kind: 'consentRevoked' }
  /** The plan could not be read. */
  | { kind: 'loadFailed'; retry: () => void }
  /** The athlete has no plan yet, live or drafted. */
  | { kind: 'noPlan'; planThreadId: string; subject: SubjectPlanResponse }
  /** The plan to draw; `shown` is the draft when one is open, else the live plan. */
  | { kind: 'plan'; planThreadId: string; subject: SubjectPlanResponse; shown: WorkoutPlan };

/**
 * Which state `athlete`'s workspace plan is in, first match winning: no group,
 * the plan thread refused, the groups' read failed while no plan thread is
 * known, loading, sharing revoked, the plan's read failed, no plan, the plan.
 * A refused plan thread outranks a failed group read, which outranks the
 * wait; a read that answered `consentRevoked` is that, never a failure.
 */
export function workspacePlanState(
  athlete: Pick<CoachRosterAthlete, 'groups'>,
  threads: Pick<WorkspaceThreads, 'planThreadId' | 'planError' | 'retryPlanThread' | 'groupsFailed' | 'retryGroups'>,
  plan: WorkspacePlanRead,
): WorkspacePlanState {
  const { planThreadId } = threads;
  if (athlete.groups.length === 0) return { kind: 'noGroup' };
  if (threads.planError !== null) return { kind: 'threadFailed', error: threads.planError, retry: threads.retryPlanThread };
  if (planThreadId === null && threads.groupsFailed) return { kind: 'groupsFailed', retry: threads.retryGroups };
  if (planThreadId === null || plan.isLoading) return { kind: 'loading' };
  if (plan.read?.status === 'consentRevoked') return { kind: 'consentRevoked' };
  if (plan.isError || plan.read === undefined) {
    return {
      kind: 'loadFailed',
      retry: () => {
        void plan.refetch();
      },
    };
  }
  const subject = plan.read.plan;
  const shown = shownPlan(subject);
  return shown === null ? { kind: 'noPlan', planThreadId, subject } : { kind: 'plan', planThreadId, subject, shown };
}

/** What the coach can answer to the draft from the workspace. */
export interface WorkspaceAnswers {
  /** Whether the coach accepted the proposal on a date in this visit, as it stands now (decision W1). */
  isAccepted: (day: PlanCalendarDay) => boolean;
  /** « Accepter »: acknowledge a date's proposal for this visit; nothing is written. */
  accept: (day: PlanCalendarDay) => void;
  /** « Refuser »: put the live plan back on the date; resolves whether it was. */
  refuse: (date: string) => Promise<boolean>;
  /** The date being put back, while one is. */
  refusing: string | null;
  /**
   * « C'est mon choix » (`on`) or « Retirer mon choix »: keep a session as the
   * coach's call, or withdraw it; resolves whether it landed. `position` is
   * the session's place among its date's entries, the position a day write
   * names.
   */
  setCall: (day: PlanDay, position: number, on: boolean) => Promise<boolean>;
  /** A call is being saved: every call control waits. */
  callSaving: boolean;
}

/** A day-editor write as the workspace drives it: the bound mutation's own surface, no more. */
export interface WorkspaceWrite<V> {
  mutateAsync: (variables: V) => Promise<unknown>;
  isPending: boolean;
  variables?: V;
}

/** The client's words. */
export interface WorkspaceWords {
  t: Translate;
  language: string;
}

export interface WorkspaceHooksDeps {
  groupsApi: Pick<GroupsApi, 'getGroup'>;
  /** How long a group read stays fresh: the client's own `useGroup` freshness, which shares the cache entry. */
  groupStaleMs: number;
  /** The signed-in coach's id, `null` while it is not known. */
  useViewerId: () => string | null;
  /** The conversations the coach holds, from the client's one conversation list. */
  useConversations: () => { conversations: readonly Conversation[]; isLoading: boolean };
  /** The client's bound plan-thread hooks. */
  useOpenAthleteThread: () => UseOpenAthleteThreadResult;
  useOpenGroupRoom: () => UseOpenGroupRoomResult;
  /** The client's bound day-editor hooks: a date put back to the live plan, a session's call, the plan read again. */
  useRevertDraftDate: (conversationId: string | null, locale: string) => WorkspaceWrite<{ date: string }>;
  useSetCoachCall: (conversationId: string | null, locale: string) => WorkspaceWrite<SessionAt & { on: boolean }>;
  useRereadPlan: (conversationId: string | null) => () => void;
  /** The client's own translation hook. */
  useWords: () => WorkspaceWords;
  /** The client's way of saying a write was refused: a title, and why when there is a reason to give. */
  useReportError: () => (title: string, detail?: string) => void;
}

/** A date's entries as the acceptance of its proposal was given on: a changed date asks again. */
function entriesSignature(day: PlanCalendarDay): string {
  return JSON.stringify(day.entries);
}

/**
 * The athlete workspace's hooks over one client's APIs and hooks. Nothing is
 * reached until a hook runs, so binding the factory touches no client.
 */
export function createWorkspaceHooks(deps: WorkspaceHooksDeps) {
  /**
   * The plan thread and the room of `athlete`'s workspace.
   *
   * The plan thread is bound to the athlete through a group (carnet#1070): the
   * athlete's own room's group when they have one, so the thread is the one
   * that room's private lane writes to, else their first group. One found in
   * the coach's list is used as it is; otherwise it is opened once — the server
   * answers the existing thread, un-archived, or a new one — because the
   * workspace reads and writes the plan through it.
   *
   * The room is the group whose one athlete the server names to this coach as
   * them. The coach's conversation in it is found in the list and opened only
   * when the coach asks, so looking at the workspace never creates one.
   */
  function useWorkspaceThreads(athlete: CoachRosterAthlete): WorkspaceThreads {
    const viewerId = deps.useViewerId();
    const { conversations, isLoading: listLoading } = deps.useConversations();
    const { openThread } = deps.useOpenAthleteThread();
    const { openRoom: openGroupRoom, pendingRoomAthleteId } = deps.useOpenGroupRoom();

    const groupReads = useQueries({
      queries: athlete.groups.map((group) => ({
        queryKey: QUERY_KEYS.groups.detail(group.id),
        queryFn: () => deps.groupsApi.getGroup(group.id),
        staleTime: deps.groupStaleMs,
      })),
    });
    const groups = useMemo(
      () => groupReads.map((read) => read.data).filter((group): group is CoachingGroup => group !== undefined),
      [groupReads],
    );
    // Known only once every group answered: a failed read would otherwise read
    // as a group holding other athletes, and the plan thread would be opened
    // under a group that is not the athlete's room.
    const roomKnown = groupReads.every((read) => read.isSuccess);
    const groupsFailed = groupReads.some((read) => read.isError);
    const retryGroups = () => {
      for (const read of groupReads) if (read.isError) void read.refetch();
    };

    const roomGroup = useMemo(
      () => athleteRoomGroup(groups, athlete.user_id, viewerId),
      [groups, athlete.user_id, viewerId],
    );
    const planGroupId = roomGroup?.id ?? athlete.groups[0]?.id ?? null;
    const planGroup = groups.find((group) => group.id === planGroupId) ?? null;

    // A thread opened here, held with the athlete and group it was opened for,
    // so another athlete's workspace mounted in its place never inherits it.
    const [opened, setOpened] = useState<{ athleteId: string; groupId: string; id: string } | null>(null);
    const [planError, setPlanError] = useState<unknown>(null);
    // A retry asks again: it clears what was asked and moves the effect on.
    const [attempt, setAttempt] = useState(0);
    const asked = useRef<string | null>(null);

    const found = planGroupId === null ? null : usablePlanThread(conversations, athlete.user_id, planGroupId);
    const openedId =
      opened !== null && opened.athleteId === athlete.user_id && opened.groupId === planGroupId ? opened.id : null;
    const planThreadId = found?.id ?? openedId;

    // Opened once per athlete and group, after the list and the groups are read,
    // so a thread the coach already holds is never opened a second time.
    useEffect(() => {
      if (planThreadId !== null || planGroupId === null || listLoading || !roomKnown) return;
      const key = `${athlete.user_id}:${planGroupId}`;
      if (asked.current === key) return;
      asked.current = key;
      openThread(athlete.user_id, planGroupId).then(
        (thread) => setOpened({ athleteId: athlete.user_id, groupId: planGroupId, id: thread.id }),
        (error: unknown) => setPlanError(error),
      );
    }, [planThreadId, planGroupId, listLoading, roomKnown, athlete.user_id, openThread, attempt]);

    const retryPlanThread = useCallback(() => {
      asked.current = null;
      setPlanError(null);
      setAttempt((count) => count + 1);
    }, []);

    const [openedRoom, setOpenedRoom] = useState<{ groupId: string; id: string } | null>(null);
    const [roomError, setRoomError] = useState(false);
    const roomConversation = roomGroup === null ? null : roomConversationOf(conversations, roomGroup.id);
    const roomConversationId =
      roomConversation?.id ?? (openedRoom !== null && openedRoom.groupId === roomGroup?.id ? openedRoom.id : null);

    const openRoom = useCallback(() => {
      if (roomGroup === null) return;
      setRoomError(false);
      openGroupRoom(athlete.user_id, roomGroup.id)?.then(
        (conversation) => setOpenedRoom({ groupId: roomGroup.id, id: conversation.id }),
        () => setRoomError(true),
      );
    }, [athlete.user_id, openGroupRoom, roomGroup]);

    return {
      planThreadId,
      planGroup,
      planError: planThreadId === null ? planError : null,
      retryPlanThread,
      room: roomGroup === null ? null : { group: roomGroup, conversationId: roomConversationId },
      roomKnown,
      groupsFailed,
      retryGroups,
      openRoom,
      roomOpening: pendingRoomAthleteId === athlete.user_id,
      roomError,
    };
  }

  /**
   * The workspace's answers to the draft in the plan thread `planThreadId`.
   * « Accepter » is held for this visit only — publishing accepts every date
   * not refused — and a date whose entries change again asks again. A call
   * whose session moved reads the plan again, as the plan card's does, and
   * never lands on whichever session sits there now. A refused write says why
   * through the client's error report.
   */
  function useWorkspaceAnswers(planThreadId: string | null): WorkspaceAnswers {
    const { t, language } = deps.useWords();
    const reportError = deps.useReportError();

    const [accepted, setAccepted] = useState<ReadonlyMap<string, string>>(new Map());
    const isAccepted = useCallback(
      (day: PlanCalendarDay) => accepted.get(day.date) === entriesSignature(day),
      [accepted],
    );
    const accept = useCallback((day: PlanCalendarDay) => {
      setAccepted((held) => new Map(held).set(day.date, entriesSignature(day)));
    }, []);

    const revert = deps.useRevertDraftDate(planThreadId, language);
    const refuse = async (date: string): Promise<boolean> => {
      try {
        await revert.mutateAsync({ date });
        return true;
      } catch (error) {
        reportError(t(PLAN_WORKSPACE_KEYS.refuseFailed), describeApiError(error, { t, fallbackKey: API_ERROR_KEYS.unknown }));
        return false;
      }
    };

    const coachCall = deps.useSetCoachCall(planThreadId, language);
    const reread = deps.useRereadPlan(planThreadId);
    const setCall = async (day: PlanDay, position: number, on: boolean): Promise<boolean> => {
      try {
        await coachCall.mutateAsync({ date: day.date, position, expect: sessionExpect(day), on });
        return true;
      } catch (error) {
        if (isSessionMoved(error)) {
          reportError(t(SESSION_EDITOR_KEYS.sessionMoved));
          reread();
        } else {
          reportError(t(SESSION_EDITOR_KEYS.saveFailed), describeApiError(error, { t, fallbackKey: API_ERROR_KEYS.unknown }));
        }
        return false;
      }
    };

    return {
      isAccepted,
      accept,
      refuse,
      refusing: revert.isPending ? (revert.variables?.date ?? null) : null,
      setCall,
      callSaving: coachCall.isPending,
    };
  }

  return { useWorkspaceThreads, useWorkspaceAnswers };
}
