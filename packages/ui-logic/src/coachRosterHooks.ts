// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hooks for Coach Home, bound to each client's coach roster API — the roster read page by page, "Traité" and its undo
// ABOUTME: A dismissal shows at once and settles on the server's answer; a refused write rolls back to what the server holds

import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { CoachRosterApi } from '@pierre/api-client';
import type {
  CoachAttentionDismissRequest,
  CoachAttentionRestoreRequest,
  CoachRosterResponse,
} from '@pierre/shared-types';
import { mergeRosterPages, type MergedRoster } from './coachHome';

/** Rows one request asks for: the server's page ceiling. */
const ROSTER_PAGE_SIZE = 100;

/** A runaway `next_offset` must not read forever: this many pages cover 5,000 athletes. */
const ROSTER_MAX_PAGES = 50;

/** The roster is read when the coach opens it, then kept fresh a minute at a time. */
const ROSTER_STALE_MS = 60_000;

/** What `useCoachRoster` hands a surface. */
export interface UseCoachRosterResult {
  /** The whole roster, every page merged; undefined until the first answer. */
  roster: MergedRoster | undefined;
  isLoading: boolean;
  isError: boolean;
  /** When the roster was last read, in epoch milliseconds; 0 before the first answer. */
  readAt: number;
  /** Reads the roster again; settles when the read does, so a pull-to-refresh spinner can wait on it. */
  refetch: () => Promise<void>;
}

/** The hooks a client binds over its coach roster API. */
export interface CoachRosterHooks {
  useCoachRoster: (enabled: boolean) => UseCoachRosterResult;
  /** Mark a reason handled for the signal it was raised on. */
  useDismissAttention: () => { dismiss: (request: CoachAttentionDismissRequest) => Promise<void>; isPending: boolean };
  /** Undo a dismissal. */
  useRestoreAttention: () => { restore: (request: CoachAttentionRestoreRequest) => Promise<void>; isPending: boolean };
}

/** The roster with one reason's `dismissed_at` set, or cleared with `null`. */
function withDismissal(
  roster: MergedRoster | undefined,
  athleteUserId: string,
  reasonId: string,
  dismissedAt: string | null,
): MergedRoster | undefined {
  if (roster === undefined) return roster;
  return {
    ...roster,
    athletes: roster.athletes.map((athlete) =>
      athlete.user_id !== athleteUserId
        ? athlete
        : {
            ...athlete,
            attention: athlete.attention.map((reason) =>
              reason.id === reasonId ? { ...reason, dismissed_at: dismissedAt } : reason,
            ),
          },
    ),
  };
}

/**
 * Build the Coach Home hooks over one client's roster API.
 *
 * The roster is read by following `next_offset` until it is null, so the
 * queue, the table and the season all work from the whole roster: a reason's
 * `rank` is comparable across pages, which is what lets the client merge
 * them. "Traité" and its undo patch the cached roster before the request
 * lands and roll back when it is refused.
 */
export function createCoachRosterHooks(
  coachRosterApi: Pick<CoachRosterApi, 'getRoster' | 'dismissAttention' | 'restoreAttention'>,
): CoachRosterHooks {
  function useCoachRoster(enabled: boolean): UseCoachRosterResult {
    const query = useQuery({
      queryKey: QUERY_KEYS.coachRoster.list(),
      enabled,
      staleTime: ROSTER_STALE_MS,
      queryFn: async (): Promise<MergedRoster> => {
        const pages: CoachRosterResponse[] = [];
        let offset: number | null = 0;
        while (offset !== null && pages.length < ROSTER_MAX_PAGES) {
          const page: CoachRosterResponse = await coachRosterApi.getRoster({ offset, limit: ROSTER_PAGE_SIZE });
          pages.push(page);
          offset = page.next_offset;
        }
        return mergeRosterPages(pages);
      },
    });
    return {
      roster: query.data,
      isLoading: query.isLoading,
      isError: query.isError && query.data === undefined,
      readAt: query.dataUpdatedAt,
      refetch: async () => {
        await query.refetch();
      },
    };
  }

  function useDismissAttention() {
    const queryClient = useQueryClient();
    const key = QUERY_KEYS.coachRoster.list();
    const mutation = useMutation({
      mutationFn: (request: CoachAttentionDismissRequest) => coachRosterApi.dismissAttention(request),
      onMutate: async (request) => {
        await queryClient.cancelQueries({ queryKey: key });
        const previous = queryClient.getQueryData<MergedRoster>(key);
        queryClient.setQueryData<MergedRoster | undefined>(
          key,
          withDismissal(previous, request.athlete_user_id, request.reason_id, new Date().toISOString()),
        );
        return { previous };
      },
      onError: (_error, _request, context) => {
        if (context?.previous !== undefined) queryClient.setQueryData(key, context.previous);
      },
      onSuccess: (answer, request) => {
        queryClient.setQueryData<MergedRoster | undefined>(
          key,
          withDismissal(queryClient.getQueryData<MergedRoster>(key), request.athlete_user_id, answer.reason_id, answer.dismissed_at),
        );
      },
    });
    return { dismiss: async (request: CoachAttentionDismissRequest) => void (await mutation.mutateAsync(request)), isPending: mutation.isPending };
  }

  function useRestoreAttention() {
    const queryClient = useQueryClient();
    const key = QUERY_KEYS.coachRoster.list();
    const mutation = useMutation({
      mutationFn: (request: CoachAttentionRestoreRequest) => coachRosterApi.restoreAttention(request),
      onMutate: async (request) => {
        await queryClient.cancelQueries({ queryKey: key });
        const previous = queryClient.getQueryData<MergedRoster>(key);
        queryClient.setQueryData<MergedRoster | undefined>(
          key,
          withDismissal(previous, request.athlete_user_id, request.reason_id, null),
        );
        return { previous };
      },
      onError: (_error, _request, context) => {
        if (context?.previous !== undefined) queryClient.setQueryData(key, context.previous);
      },
    });
    return { restore: async (request: CoachAttentionRestoreRequest) => void (await mutation.mutateAsync(request)), isPending: mutation.isPending };
  }

  return { useCoachRoster, useDismissAttention, useRestoreAttention };
}
