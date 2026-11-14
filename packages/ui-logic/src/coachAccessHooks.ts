// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach-access hooks both clients bind — a grant picked up without signing out, and the Groups list rows' coach-seat line
// ABOUTME: One definition for web and phone: each client hands in its APIs, its signed-in session and its translation

import { useEffect, useMemo, useRef } from 'react';
import { useQueries, useQuery, useQueryClient } from '@tanstack/react-query';
import type { ConversationRowModel } from '@pierre/chat-utils';
import type { AuthApi, GroupsApi, UserApi } from '@pierre/api-client';
import {
  COACH_ACCESS_WAIT_POLL_MS,
  COACH_SEAT_KEYS,
  QUERY_KEYS,
  coachSeatListLine,
  coachSeatMoment,
  coachSeatState,
} from '@pierre/shared-constants';

/** Who reads: the signed-in user, as the client holds them. */
export interface CoachAccessViewer {
  id: string;
  manages_roster?: boolean;
}

/** The signed-in session a client hands the grant refresh: the user, and how it patches them. */
export interface CoachAccessSession {
  user: CoachAccessViewer | null;
  updateUser: (patch: { manages_roster: boolean }) => unknown;
}

/** A client's translation, as the seat line needs it. */
export interface CoachAccessWords {
  t: (key: string, params?: Record<string, string>) => string;
  language: string;
}

/** What a client binds the hooks over. */
export interface CoachAccessHooksDeps {
  userApi: Pick<UserApi, 'getCoachAccessRequest'>;
  authApi: Pick<AuthApi, 'getSession'>;
  groupsApi: Pick<GroupsApi, 'getGroup'>;
  /** The client's own signed-in session hook. */
  useSession: () => CoachAccessSession;
  /** The client's own translation hook. */
  useWords: () => CoachAccessWords;
  /**
   * Whether the request is read again when the window regains focus. The web
   * says so; the phone's foreground read comes from its focus manager.
   */
  refetchOnWindowFocus?: boolean;
}

/** The hooks a client binds. */
export interface CoachAccessHooks {
  /** Mounted once by the signed-in shell: re-reads the session once the request reads granted. */
  useCoachAccessGrantRefresh: () => void;
  /** The coach-seat line each listed room shows in place of its last words, keyed by row id. */
  useGroupSeatLines: (
    rows: readonly ConversationRowModel[],
    viewer: CoachAccessViewer | null | undefined,
  ) => Map<string, string>;
}

/** How long after a grant the list still reads the rooms, so the line can say it. */
const GRANT_WATCH_MS = 24 * 60 * 60 * 1000;

/** How fresh a room read by the list stays. */
const GROUP_STALE_MS = 30_000;

/**
 * The day a request was sent as a list row has room for: day and month only
 * (« 12 nov. », "Nov 12"), the row's own timestamp carrying the rest. An
 * unparseable stamp comes back as it was.
 */
export function seatLineDay(iso: string, language: string): string {
  const date = new Date(iso);
  if (Number.isNaN(date.getTime())) return iso;
  return new Intl.DateTimeFormat(language, { day: 'numeric', month: 'short' }).format(date);
}

/**
 * Build the coach-access hooks over one client's APIs, session and words.
 *
 * - `useCoachAccessGrantRefresh`: coach access (`manages_roster`) rides the
 *   user the client keeps from sign-in, so without this a grant a super-admin
 *   gives while the coach is in the app would show only after signing out and
 *   in (carnet#1521). It reads the coach's request — on the client's focus, and
 *   once a minute while it waits — and the moment it reads `granted` for a
 *   user still held without coach access, re-reads the session once and
 *   patches the user: coach Home appears on the next navigation, and every
 *   group read is refreshed. A user who holds coach access reads nothing; an
 *   access an operator later took back does not loop.
 * - `useGroupSeatLines`: while the viewer's seat moves, a room they own reads
 *   « Accès coach en attente · demandé le 12 nov. » while the request waits,
 *   and « Tu es le coach · depuis 11:10 » once a grant attached it, until the
 *   room talks again. Rooms are read only while there is something to say —
 *   a request waiting, or one granted in the last day — so an athlete's list
 *   costs no read; without a viewer nothing is read.
 */
export function createCoachAccessHooks(deps: CoachAccessHooksDeps): CoachAccessHooks {
  function useCoachAccessGrantRefresh(): void {
    const { user, updateUser } = deps.useSession();
    const queryClient = useQueryClient();
    const managesRoster = user?.manages_roster === true;
    const latest = useQuery({
      queryKey: QUERY_KEYS.user.coachAccessRequest(),
      queryFn: () => deps.userApi.getCoachAccessRequest(),
      enabled: !!user && !managesRoster,
      ...(deps.refetchOnWindowFocus ? { refetchOnWindowFocus: true } : {}),
      refetchInterval: (query) =>
        coachSeatState(query.state.data?.request) === 'pending' ? COACH_ACCESS_WAIT_POLL_MS : false,
    });
    const request = latest.data?.request ?? null;
    const refreshedFor = useRef<string | null>(null);
    // The session read outlives a re-render (each refetch hands a new
    // request object), so only leaving the shell drops its answer.
    const mounted = useRef(true);
    useEffect(() => {
      mounted.current = true;
      return () => {
        mounted.current = false;
      };
    }, []);

    useEffect(() => {
      if (managesRoster || !request || request.status !== 'granted') return;
      if (refreshedFor.current === request.id) return;
      refreshedFor.current = request.id;
      deps.authApi
        .getSession()
        .then(async ({ user: fresh }) => {
          if (!mounted.current || fresh.manages_roster !== true) return;
          await updateUser({ manages_roster: true });
          void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.all });
          void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
        })
        // The next focus reads the request again; a failed read changes nothing.
        .catch(() => {
          refreshedFor.current = null;
        });
    }, [managesRoster, request, updateUser, queryClient]);
  }

  function useGroupSeatLines(
    rows: readonly ConversationRowModel[],
    viewer: CoachAccessViewer | null | undefined,
  ): Map<string, string> {
    const { t, language } = deps.useWords();
    const user = viewer ?? null;
    const managesRoster = user?.manages_roster === true;
    const latest = useQuery({
      queryKey: QUERY_KEYS.user.coachAccessRequest(),
      queryFn: () => deps.userApi.getCoachAccessRequest(),
      enabled: !!user,
    });
    const request = latest.data?.request ?? null;
    const moving =
      !!user &&
      ((request?.status === 'pending' && !managesRoster) ||
        (request?.status === 'granted' &&
          !!request.decided_at &&
          Date.now() - Date.parse(request.decided_at) < GRANT_WATCH_MS));

    const groupRows = useMemo(() => rows.filter((row) => row.kind === 'group' && row.groupId), [rows]);
    const groups = useQueries({
      queries: groupRows.map((row) => ({
        queryKey: QUERY_KEYS.groups.detail(row.groupId ?? ''),
        queryFn: () => deps.groupsApi.getGroup(row.groupId ?? ''),
        enabled: moving,
        staleTime: GROUP_STALE_MS,
      })),
    });

    return useMemo(() => {
      const lines = new Map<string, string>();
      if (!moving) return lines;
      groupRows.forEach((row, index) => {
        const line = coachSeatListLine({
          group: groups[index]?.data,
          viewerId: user?.id,
          managesRoster,
          request,
          lastActivityAt: row.lastActivityAt,
        });
        if (line?.kind === 'pending') {
          lines.set(row.id, t(COACH_SEAT_KEYS.listPending, { date: seatLineDay(line.sentAt, language) }));
        } else if (line?.kind === 'coach') {
          lines.set(row.id, t(COACH_SEAT_KEYS.listCoach, { time: coachSeatMoment(line.since, language).time }));
        }
      });
      return lines;
    }, [moving, groupRows, groups, user?.id, managesRoster, request, t, language]);
  }

  return { useCoachAccessGrantRefresh, useGroupSeatLines };
}
