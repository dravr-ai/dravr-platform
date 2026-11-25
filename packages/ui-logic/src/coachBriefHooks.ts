// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hooks for the coach's morning brief, bound to each client's coach roster API — the read, a change, and the open ping (carnet#1791)
// ABOUTME: A change shows at once and is re-read once settled; a refused one rolls back; the ping never waits and never fails a screen

import { useEffect, useRef } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { CoachRosterApi } from '@pierre/api-client';
import type { CoachBrief, CoachBriefUpdate } from '@pierre/shared-types';
import { apiErrorStatus } from './apiError';
import { applyBriefUpdate, isCoachBrief } from './coachBrief';

/** The brief changes once a morning or when the coach changes it: a minute of staleness is plenty. */
const BRIEF_STALE_MS = 60_000;

/** A read the server failed (5xx) is tried again this many times. */
const BRIEF_READ_RETRIES = 2;

/** What `useCoachBrief` hands a surface. */
export interface UseCoachBriefResult {
  /** The brief; undefined until it is read, and for good when the caller coaches no one (403). */
  brief: CoachBrief | undefined;
  isLoading: boolean;
}

/** The hooks a client binds over its coach roster API. */
export interface CoachBriefHooks {
  useCoachBrief: (enabled: boolean) => UseCoachBriefResult;
  /** Change the brief: the change shows before the server answers and rolls back when it is refused. */
  useUpdateCoachBrief: () => { update: (update: CoachBriefUpdate) => Promise<void>; isPending: boolean };
  /** Tell the server, once per open, that the coach opened this athlete; `null` while none is open. */
  useBriefOpenedPing: (athleteId: string | null) => void;
}

/**
 * Retry a read only when the server answered that it failed. A 4xx answer,
 * 403 above all, stands; a read that never landed is left to the next
 * refetch (focus, reconnect, a minute's staleness) — the brief is a note on
 * the queue, not worth holding a spinner over.
 */
function retryRead(failureCount: number, error: unknown): boolean {
  const status = apiErrorStatus(error);
  return status !== undefined && status >= 500 && failureCount < BRIEF_READ_RETRIES;
}

/**
 * Build the brief hooks over one client's roster API.
 *
 * Several changes can be in flight at once (a coach flicking two reason
 * switches): each is applied to the cached brief as it is made, and the
 * server's answer is written back and the brief re-read only once the last
 * of them settles, so an early answer never undoes a later change on screen.
 */
export function createCoachBriefHooks(
  api: Pick<CoachRosterApi, 'getBrief' | 'updateBrief' | 'markBriefOpened'>,
): CoachBriefHooks {
  const key = QUERY_KEYS.coachRoster.brief();

  function useCoachBrief(enabled: boolean): UseCoachBriefResult {
    const query = useQuery({
      queryKey: key,
      enabled,
      staleTime: BRIEF_STALE_MS,
      retry: retryRead,
      queryFn: async () => {
        const answer: unknown = await api.getBrief();
        // Refused as a failed read: the control does not draw, the screen stands.
        if (!isCoachBrief(answer)) throw new Error('GET /api/coach/brief answered something that is not a brief');
        return answer;
      },
    });
    return { brief: query.data, isLoading: query.isLoading };
  }

  function useUpdateCoachBrief() {
    const queryClient = useQueryClient();
    const lastInFlight = () => queryClient.isMutating({ mutationKey: key }) === 1;
    const mutation = useMutation({
      mutationKey: key,
      mutationFn: (update: CoachBriefUpdate) => api.updateBrief(update),
      onMutate: async (update) => {
        await queryClient.cancelQueries({ queryKey: key });
        const previous = queryClient.getQueryData<CoachBrief>(key);
        if (previous !== undefined) queryClient.setQueryData<CoachBrief>(key, applyBriefUpdate(previous, update));
        return { previous };
      },
      onError: (_error, _update, context) => {
        if (context?.previous !== undefined) queryClient.setQueryData(key, context.previous);
      },
      onSuccess: (answer) => {
        if (lastInFlight() && isCoachBrief(answer)) queryClient.setQueryData(key, answer);
      },
      onSettled: () => {
        if (lastInFlight()) void queryClient.invalidateQueries({ queryKey: key });
      },
    });
    return {
      update: async (update: CoachBriefUpdate) => void (await mutation.mutateAsync(update)),
      isPending: mutation.isPending,
    };
  }

  function useBriefOpenedPing(athleteId: string | null): void {
    // The athlete last told of, so a re-render, or the double effect of a
    // development build, is not a second open; cleared when the detail
    // closes, so opening the same athlete again is.
    const told = useRef<string | null>(null);
    useEffect(() => {
      if (athleteId === null) {
        told.current = null;
        return;
      }
      if (told.current === athleteId) return;
      told.current = athleteId;
      // Fire and forget: a lost ping costs a count, never the screen.
      Promise.resolve()
        .then(() => api.markBriefOpened(athleteId))
        .catch(() => undefined);
    }, [athleteId]);
  }

  return { useCoachBrief, useUpdateCoachBrief, useBriefOpenedPing };
}
