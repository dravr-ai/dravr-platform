// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Proves the shared athlete workspace hooks — the plan thread found or opened once, the athlete's room found or opened on asking, and the coach's answers to the draft
// ABOUTME: Each client binds the same factory; what a client hands in (its list, plan hooks, day-editor writes, words and error report) is faked at that seam

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { createElement, type ReactNode } from 'react';
import { act, cleanup, renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider, useMutation } from '@tanstack/react-query';
import type { CoachingGroup, CoachRosterAthlete, Conversation, PlanDay } from '@pierre/shared-types';
import { API_ERROR_KEYS } from '../src/apiError';
import { PLAN_WORKSPACE_KEYS, type PlanCalendarDay } from '../src/planWorkspace';
import { SESSION_EDITOR_KEYS, type SessionAt } from '../src/sessionEditor';
import { createWorkspaceHooks, workspaceGroupTab, type WorkspaceRoom } from '../src/workspaceHooks';

const COACH_ID = 'user-coach';
const LEA = { user_id: 'u-lea', display_name: 'Léa Gagnon' };
const PLAN_ID = 'conv-plan-lea';
const ROOM_ID = 'conv-room-lea';

const getGroup = vi.fn();
const openThread = vi.fn();
const openRoom = vi.fn();
const revertDate = vi.fn();
const setCoachCall = vi.fn();
const reread = vi.fn();
const report = vi.fn();

/** What the faked client holds: its list, and whose room is opening. */
const client: { conversations: Conversation[]; listLoading: boolean; pendingRoomAthleteId: string | null } = {
  conversations: [],
  listLoading: false,
  pendingRoomAthleteId: null,
};

const { useWorkspaceThreads, useWorkspaceAnswers } = createWorkspaceHooks({
  groupsApi: { getGroup },
  groupStaleMs: 30_000,
  useViewerId: () => COACH_ID,
  useConversations: () => ({ conversations: client.conversations, isLoading: client.listLoading }),
  useOpenAthleteThread: () => ({ openThread, pendingAthleteId: null }),
  useOpenGroupRoom: () => ({ openRoom, pendingRoomAthleteId: client.pendingRoomAthleteId }),
  // The day-editor writes as each client binds them: React Query mutations over its chat API.
  useRevertDraftDate: (conversationId, locale) =>
    useMutation({ mutationFn: ({ date }: { date: string }) => revertDate(conversationId, date, locale) }),
  useSetCoachCall: (conversationId, locale) =>
    useMutation({
      mutationFn: ({ date, position, expect: session, on }: SessionAt & { on: boolean }) =>
        setCoachCall(conversationId, date, position, on, session, locale),
    }),
  useRereadPlan: () => reread,
  useWords: () => ({ t: (key) => key, language: 'fr' }),
  useReportError: () => report,
});

function group(id: string, solo: boolean): CoachingGroup {
  return {
    id,
    tenant_id: 'tenant-a',
    name: id === 'g-lea' ? 'TRAKA' : 'Squad',
    description: null,
    agent_id: 'agent-endurance',
    agent_title: 'Coach Max',
    agent_handle: 'max',
    owner_id: COACH_ID,
    coach_user_id: COACH_ID,
    coach_display_name: 'Phil',
    peer_data_sharing: false,
    respond_mode: 'all',
    digest_mode: 'off',
    max_members: 10,
    is_active: true,
    created_at: '2026-10-01T00:00:00Z',
    updated_at: '2026-10-01T00:00:00Z',
    ...(solo ? { solo_athlete: LEA } : {}),
  };
}

const planThread = {
  id: PLAN_ID,
  title: LEA.display_name,
  agent_id: null,
  group_id: null,
  subject_user_id: LEA.user_id,
  subject_group_id: 'g-lea',
  message_count: 2,
  created_at: '2026-11-14T00:00:00Z',
  updated_at: '2026-11-25T10:05:00Z',
} as Conversation;
const room = {
  id: ROOM_ID,
  title: 'TRAKA',
  agent_id: null,
  group_id: 'g-lea',
  message_count: 3,
  created_at: '2026-10-01T00:00:00Z',
  updated_at: '2026-11-25T10:04:00Z',
} as Conversation;

/** Léa in a squad first and her own room second: the room is found by its one athlete, not by its place. */
const ATHLETE: CoachRosterAthlete = {
  user_id: LEA.user_id,
  name: LEA.display_name,
  groups: [
    { id: 'g-squad', name: 'Squad' },
    { id: 'g-lea', name: 'TRAKA' },
  ],
  status: 'shared',
  attention: [],
};

function wrapper() {
  const queryClient = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 }, mutations: { retry: false, gcTime: 0 } },
  });
  return ({ children }: { children: ReactNode }) => createElement(QueryClientProvider, { client: queryClient }, children);
}

function entry(date: string, workout: string, extra: Partial<PlanDay> = {}): PlanDay {
  return { date, position: 0, sport: 'ride', workout, duration_min: 60, intensity: 'Z2', rest: false, ...extra };
}

function calendarDay(date: string, entries: PlanDay[]): PlanCalendarDay {
  return { date, entries, proposed: true, timing: 'ahead' };
}

/** A refusal the server answered with `status`, as the axios client rejects. */
function refusal(status: number, details?: Record<string, string>) {
  return { response: { status, data: details ? { details } : {} } };
}

beforeEach(() => {
  vi.clearAllMocks();
  client.conversations = [planThread, room];
  client.listLoading = false;
  client.pendingRoomAthleteId = null;
  getGroup.mockImplementation(async (id: string) => group(id, id === 'g-lea'));
  openThread.mockResolvedValue(planThread);
  openRoom.mockResolvedValue(room);
});

afterEach(cleanup);

describe('useWorkspaceThreads', () => {
  it('uses the plan thread and the room the coach already holds, opening nothing', async () => {
    const { result } = renderHook(() => useWorkspaceThreads(ATHLETE), { wrapper: wrapper() });

    await waitFor(() => expect(result.current.roomKnown).toBe(true));
    expect(result.current.planThreadId).toBe(PLAN_ID);
    expect(result.current.planGroup?.id).toBe('g-lea');
    expect(result.current.room).toEqual({ group: group('g-lea', true), conversationId: ROOM_ID });
    expect(result.current.groupsFailed).toBe(false);
    expect(getGroup).toHaveBeenCalledTimes(2);
    expect(openThread).not.toHaveBeenCalled();
    expect(openRoom).not.toHaveBeenCalled();
  });

  it("opens the plan thread once, under the athlete's room, after the list and the groups are read", async () => {
    client.conversations = [room];
    client.listLoading = true;
    const { result, rerender } = renderHook(() => useWorkspaceThreads(ATHLETE), { wrapper: wrapper() });
    await waitFor(() => expect(result.current.roomKnown).toBe(true));
    expect(openThread).not.toHaveBeenCalled();

    client.listLoading = false;
    rerender();

    await waitFor(() => expect(result.current.planThreadId).toBe(PLAN_ID));
    expect(openThread).toHaveBeenCalledTimes(1);
    expect(openThread).toHaveBeenCalledWith(LEA.user_id, 'g-lea');
    rerender();
    expect(openThread).toHaveBeenCalledTimes(1);
  });

  it('opens it under their first group, and names no room, when every group of theirs holds others', async () => {
    getGroup.mockImplementation(async (id: string) => group(id, false));
    client.conversations = [];
    const { result } = renderHook(() => useWorkspaceThreads(ATHLETE), { wrapper: wrapper() });

    await waitFor(() => expect(result.current.planThreadId).toBe(PLAN_ID));
    expect(openThread).toHaveBeenCalledWith(LEA.user_id, 'g-squad');
    expect(result.current.room).toBeNull();
    expect(result.current.roomKnown).toBe(true);
  });

  it('opens no plan thread while a group could not be read, and finds the room once it is', async () => {
    client.conversations = [room];
    getGroup.mockImplementation(async (id: string) => {
      if (id === 'g-lea') throw refusal(500);
      return group(id, false);
    });
    const { result } = renderHook(() => useWorkspaceThreads(ATHLETE), { wrapper: wrapper() });

    await waitFor(() => expect(result.current.groupsFailed).toBe(true));
    expect(result.current.roomKnown).toBe(false);
    expect(openThread).not.toHaveBeenCalled();

    getGroup.mockImplementation(async (id: string) => group(id, id === 'g-lea'));
    act(() => result.current.retryGroups());

    await waitFor(() => expect(result.current.planThreadId).toBe(PLAN_ID));
    expect(openThread).toHaveBeenCalledWith(LEA.user_id, 'g-lea');
    expect(result.current.room?.conversationId).toBe(ROOM_ID);
  });

  it('says why the plan thread was refused until a retry asks again', async () => {
    client.conversations = [room];
    const refused = refusal(403);
    openThread.mockRejectedValueOnce(refused);
    const { result } = renderHook(() => useWorkspaceThreads(ATHLETE), { wrapper: wrapper() });

    await waitFor(() => expect(result.current.planError).toBe(refused));
    expect(result.current.planThreadId).toBeNull();

    act(() => result.current.retryPlanThread());

    await waitFor(() => expect(result.current.planThreadId).toBe(PLAN_ID));
    expect(result.current.planError).toBeNull();
    expect(openThread).toHaveBeenCalledTimes(2);
  });

  it("opens the coach's conversation in the room only when asked, and says when it could not", async () => {
    client.conversations = [planThread];
    const { result } = renderHook(() => useWorkspaceThreads(ATHLETE), { wrapper: wrapper() });
    await waitFor(() => expect(result.current.room).not.toBeNull());
    expect(result.current.room?.conversationId).toBeNull();
    expect(openRoom).not.toHaveBeenCalled();

    openRoom.mockRejectedValueOnce(refusal(500));
    act(() => result.current.openRoom());
    await waitFor(() => expect(result.current.roomError).toBe(true));

    act(() => result.current.openRoom());
    await waitFor(() => expect(result.current.room?.conversationId).toBe(ROOM_ID));
    expect(result.current.roomError).toBe(false);
    expect(openRoom).toHaveBeenLastCalledWith(LEA.user_id, 'g-lea');
  });

  it("says the room is opening while this athlete's is", async () => {
    client.pendingRoomAthleteId = LEA.user_id;
    const { result } = renderHook(() => useWorkspaceThreads(ATHLETE), { wrapper: wrapper() });
    expect(result.current.roomOpening).toBe(true);

    client.pendingRoomAthleteId = 'u-other';
    const other = renderHook(() => useWorkspaceThreads(ATHLETE), { wrapper: wrapper() });
    expect(other.result.current.roomOpening).toBe(false);
  });
});

describe('workspaceGroupTab', () => {
  const ROOM: WorkspaceRoom = { group: group('g-lea', true), conversationId: ROOM_ID };
  const retryGroups = vi.fn();
  const known = { room: null, roomKnown: true, groupsFailed: false, retryGroups };

  it("holds the athlete's own room once it is found", () => {
    expect(workspaceGroupTab({ ...known, room: ROOM }, ATHLETE)).toEqual({ kind: 'room', room: ROOM });
  });

  it("offers the failed groups' read again while which group is the room is not known", () => {
    const tab = workspaceGroupTab({ ...known, roomKnown: false, groupsFailed: true }, ATHLETE);
    expect(tab?.kind).toBe('failed');
    if (tab?.kind === 'failed') tab.retry();
    expect(retryGroups).toHaveBeenCalledTimes(1);
  });

  it('says there is no room of their own once every group was read and none is theirs alone', () => {
    expect(workspaceGroupTab(known, ATHLETE)).toEqual({ kind: 'noRoom' });
  });

  it('has no tab while the groups are read, nor for an athlete in no group of the coach\'s', () => {
    expect(workspaceGroupTab({ ...known, roomKnown: false }, ATHLETE)).toBeNull();
    expect(workspaceGroupTab(known, { groups: [] })).toBeNull();
  });
});

describe('useWorkspaceAnswers', () => {
  it('holds an acceptance for the entries it was given on, writes nothing, and asks again once they change', () => {
    const { result } = renderHook(() => useWorkspaceAnswers(PLAN_ID), { wrapper: wrapper() });
    const proposed = calendarDay('2026-11-25', [entry('2026-11-25', 'CrossFit moved')]);

    expect(result.current.isAccepted(proposed)).toBe(false);
    act(() => result.current.accept(proposed));

    expect(result.current.isAccepted(proposed)).toBe(true);
    expect(result.current.isAccepted(calendarDay('2026-11-25', [entry('2026-11-25', 'Track tempo')]))).toBe(false);
    expect(revertDate).not.toHaveBeenCalled();
    expect(setCoachCall).not.toHaveBeenCalled();
  });

  it('puts the live plan back on a refused date, naming the date while it is written', async () => {
    let finish: () => void = () => undefined;
    revertDate.mockReturnValue(new Promise<void>((resolve) => (finish = resolve)));
    const { result } = renderHook(() => useWorkspaceAnswers(PLAN_ID), { wrapper: wrapper() });

    let refused: Promise<boolean> = Promise.resolve(false);
    act(() => {
      refused = result.current.refuse('2026-11-25');
    });
    await waitFor(() => expect(result.current.refusing).toBe('2026-11-25'));
    expect(revertDate).toHaveBeenCalledWith(PLAN_ID, '2026-11-25', 'fr');

    await act(async () => finish());
    await expect(refused).resolves.toBe(true);
    await waitFor(() => expect(result.current.refusing).toBeNull());
    expect(report).not.toHaveBeenCalled();
  });

  it('says why a refusal was not written, and resolves that it was not', async () => {
    revertDate.mockRejectedValue(refusal(500));
    const { result } = renderHook(() => useWorkspaceAnswers(PLAN_ID), { wrapper: wrapper() });

    let refused = true;
    await act(async () => {
      refused = await result.current.refuse('2026-11-25');
    });

    expect(refused).toBe(false);
    expect(report).toHaveBeenCalledWith(PLAN_WORKSPACE_KEYS.refuseFailed, API_ERROR_KEYS.server);
  });

  it("keeps a session as the coach's call, or withdraws it, for the session as it is shown", async () => {
    setCoachCall.mockResolvedValue(undefined);
    const { result } = renderHook(() => useWorkspaceAnswers(PLAN_ID), { wrapper: wrapper() });
    const kept = entry('2026-12-05', 'Kept hike', { position: 1, sport: 'hike' });

    let landed = false;
    await act(async () => {
      landed = await result.current.setCall(kept, 1, true);
    });
    expect(landed).toBe(true);
    expect(setCoachCall).toHaveBeenLastCalledWith(PLAN_ID, '2026-12-05', 1, true, { sport: 'hike', workout: 'Kept hike' }, 'fr');

    await act(async () => {
      landed = await result.current.setCall(kept, 1, false);
    });
    expect(landed).toBe(true);
    expect(setCoachCall).toHaveBeenLastCalledWith(PLAN_ID, '2026-12-05', 1, false, { sport: 'hike', workout: 'Kept hike' }, 'fr');
    expect(report).not.toHaveBeenCalled();
  });

  it('holds every call control while a call is saved', async () => {
    let finish: () => void = () => undefined;
    setCoachCall.mockReturnValue(new Promise<void>((resolve) => (finish = resolve)));
    const { result } = renderHook(() => useWorkspaceAnswers(PLAN_ID), { wrapper: wrapper() });

    act(() => {
      void result.current.setCall(entry('2026-11-29', 'Long gravel ride'), 0, true);
    });
    await waitFor(() => expect(result.current.callSaving).toBe(true));

    await act(async () => finish());
    await waitFor(() => expect(result.current.callSaving).toBe(false));
  });

  it('reads the plan again when the session moved, and never lands the call on whatever sits there now', async () => {
    setCoachCall.mockRejectedValue(refusal(409, { reason: 'session_moved' }));
    const { result } = renderHook(() => useWorkspaceAnswers(PLAN_ID), { wrapper: wrapper() });

    let landed = true;
    await act(async () => {
      landed = await result.current.setCall(entry('2026-11-29', 'Long gravel ride'), 0, true);
    });

    expect(landed).toBe(false);
    expect(reread).toHaveBeenCalledTimes(1);
    expect(report).toHaveBeenCalledWith(SESSION_EDITOR_KEYS.sessionMoved);
    expect(setCoachCall).toHaveBeenCalledTimes(1);
  });

  it('says why any other refused call was not saved', async () => {
    setCoachCall.mockRejectedValue(refusal(500));
    const { result } = renderHook(() => useWorkspaceAnswers(PLAN_ID), { wrapper: wrapper() });

    await act(async () => {
      await result.current.setCall(entry('2026-11-29', 'Long gravel ride'), 0, true);
    });

    expect(report).toHaveBeenCalledWith(SESSION_EDITOR_KEYS.saveFailed, API_ERROR_KEYS.server);
    expect(reread).not.toHaveBeenCalled();
  });
});
