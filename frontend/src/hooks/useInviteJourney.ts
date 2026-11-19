// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The invite a signed-in account is on its way to joining, named for the screens that keep it in view (carnet#1649)
// ABOUTME: From what the server holds once it holds it, else from the card's cached read of the carried code; gone once joined

import { useQuery } from '@tanstack/react-query';
import type { GroupInviteKind } from '@pierre/shared-types';
import { groupsApi } from '../services/api';
import { readCarriedGroupInvite } from '../utils/groupInvite';
import { useAuth } from './useAuth';
import { useInvitePreview } from './useInvitePreview';
import { QUERY_KEYS } from '@pierre/shared-constants';

/** Who invited the account, to which group, and how it joins. */
export interface InviteJourney {
  /** Who sent the link; null when they have no display name */
  inviterName: string | null;
  groupName: string;
  kind: GroupInviteKind;
}

/** How long a read of the server's hold stays fresh: the hook that holds and joins invalidates it. */
const HELD_STALE_MS = 30_000;

/**
 * The invite the server holds for the signed-in account, or null. Read for a
 * pending or active account only, and never while impersonating: that session
 * is the target's.
 */
export function useServerHeldInvite() {
  const { user, isAuthenticated, impersonation } = useAuth();
  const status = user?.user_status;
  const enabled =
    isAuthenticated &&
    (status === 'pending' || status === 'active') &&
    !impersonation.isImpersonating;
  return useQuery({
    queryKey: QUERY_KEYS.groupInvite.held(),
    queryFn: () => groupsApi.heldInvite(),
    enabled,
    staleTime: HELD_STALE_MS,
    retry: false,
    refetchOnWindowFocus: false,
  });
}

/**
 * The invite to keep in view above the onboarding steps and on the waiting
 * screen: the server's hold when it has one, else the carried code's card as
 * the page already read it. Null once the account has joined (nothing is held
 * or carried then) and for a code the card never read.
 */
export function useInviteJourney(): InviteJourney | null {
  const held = useServerHeldInvite().data?.held ?? null;
  const code = readCarriedGroupInvite();
  const preview = useInvitePreview(code, false).data;
  if (held !== null) {
    return { inviterName: held.inviter_name, groupName: held.group_name, kind: held.kind };
  }
  if (code !== null && preview !== undefined && preview.status === 'live') {
    return { inviterName: preview.inviter_name, groupName: preview.group_name, kind: preview.kind };
  }
  return null;
}
