// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The invite the server holds for the signed-in account, and the join the athlete confirms from Home (carnet#1650)
// ABOUTME: The phone never joins on its own: the Home strip's button is the consent moment, since a phone user never saw the invite card

import { useCallback } from 'react';
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { QUERY_KEYS } from '@pierre/shared-constants';
import type { HeldGroupInvite, JoinHeldInviteResponse } from '@pierre/shared-types';
import { groupsApi } from '../services/api';

export interface HeldGroupInviteState {
  /** The held invite, `null` once the server holds none or while it loads. */
  held: HeldGroupInvite | null;
  /** Read it again, on screen focus. */
  refetch: () => void;
  /** Ask the server to join; the answer is the strip's to read. */
  join: () => Promise<JoinHeldInviteResponse>;
  isJoining: boolean;
}

/**
 * The invite held for this account, read for an active athlete. A decided
 * join (joined, already a member, refused for good) drops the server's hold,
 * so the read is refreshed and the conversations list with it.
 */
export function useHeldGroupInvite(enabled: boolean): HeldGroupInviteState {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: QUERY_KEYS.groupInvite.held(),
    queryFn: () => groupsApi.heldInvite(),
    enabled,
  });
  const mutation = useMutation({
    mutationFn: () => groupsApi.joinHeldInvite(),
    onSuccess: (response) => {
      if (response.outcome === 'refused' && response.reason === 'group_full') return;
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groupInvite.held() });
      if (response.outcome === 'joined' || response.outcome === 'already_member') {
        void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
        void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.all });
      }
    },
  });
  const { refetch } = query;
  const { mutateAsync } = mutation;
  return {
    held: query.data?.held ?? null,
    refetch: useCallback(() => void refetch(), [refetch]),
    join: useCallback(() => mutateAsync(), [mutateAsync]),
    isJoining: mutation.isPending,
  };
}
