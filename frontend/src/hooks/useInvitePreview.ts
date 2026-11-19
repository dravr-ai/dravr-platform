// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The invite card's read of a link — fetched once per page load, never refetched (carnet#1649)
// ABOUTME: Every fetch counts as an "opened" in the coach's funnel, so focus, remounts and 404/429 answers never trigger another

import { useQuery } from '@tanstack/react-query';
import { invitePreviewApi } from '../services/api';
import { QUERY_KEYS } from '@pierre/shared-constants';

/**
 * The public preview of an invite code. The card reads it with `enabled`; the
 * journey line reads what the card already cached, without a fetch of its
 * own (`enabled: false` still returns cached data).
 */
export function useInvitePreview(code: string | null, enabled: boolean) {
  return useQuery({
    queryKey: QUERY_KEYS.groupInvite.preview(code ?? ''),
    queryFn: () => invitePreviewApi.previewInvite(code ?? ''),
    enabled: enabled && code !== null,
    // Never automatic: every read counts as an "opened", a 404 or a 429 is an
    // answer, and the card's retry button is the person asking again.
    retry: false,
    staleTime: Infinity,
    gcTime: Infinity,
    refetchOnWindowFocus: false,
    refetchOnReconnect: false,
    refetchOnMount: false,
  });
}
