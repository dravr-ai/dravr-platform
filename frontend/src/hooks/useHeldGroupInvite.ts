// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Hands a carried group invite to the server once a session exists, then joins what it holds (carnet#1648)
// ABOUTME: Runs at the app root, apart from onboarding: the hold for a pending or active account, the join for an active one

import { useCallback, useEffect, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import type { GroupInviteKind } from '@pierre/shared-types';
import { apiErrorStatus } from '@pierre/ui-logic';
import { groupInviteApi } from '../services/api';
import { useToast } from '../components/ui';
import { QUERY_KEYS } from '../constants/queryKeys';
import { dropCarriedGroupInvite, readCarriedGroupInvite } from '../utils/groupInvite';
import { useAuth } from './useAuth';

/** The group room a join just opened, for the dashboard to show. */
export interface JoinedGroupRoom {
  /**
   * The account's conversation in the room, holding the join notice; `null`
   * when the server could not file it, and the Groups tab opens on its list.
   */
  conversationId: string | null;
}

export interface HeldGroupInvite {
  /**
   * This page came through a member's invite — a coach inviting an athlete —
   * as the server named it: the hold's answer, or the join's when no hold
   * answered. A coach-seat invite, or one whose kind is not known yet, is not.
   * Onboarding records the athlete role from it.
   */
  invitedAsAthlete: boolean;
  /** The room the join opened, until the dashboard has shown it. */
  joinedRoom: JoinedGroupRoom | null;
  /** The dashboard has shown the room above. */
  onJoinedRoomOpened: () => void;
}

/**
 * The server's word that the code is dead — unknown, deactivated, expired,
 * used up, or its group archived — so there is nothing to retry.
 */
function isDeadInvite(error: unknown): boolean {
  const status = apiErrorStatus(error);
  return status === 400 || status === 404;
}

/** What this page load has sent, for one account. */
interface SentForAccount {
  userId: string;
  /** The carried code's hold, once sent; the join waits on it. */
  hold: Promise<void> | null;
  joinSent: boolean;
}

/**
 * Settle a group invite from wherever the session came from: the hosted
 * sign-in, a Google popup or redirect, a registration that signed straight in,
 * or a tab that was already signed in.
 *
 * A carried code goes to the server for a pending or active account, and stays
 * carried until the server holds it or calls it dead: an aborted request, a
 * 5xx or the reload a service worker update forces all leave it for the next
 * page load. An active account then joins whatever the server holds, once per
 * page load — a member's invite as a member, a coach seat as the group's
 * human coach. Never while impersonating: that session is the target's, and
 * the operator's tab must not join their groups or spend their invite.
 *
 * Call it once, from the app root inside `AuthProvider` and `ToastProvider`.
 */
export function useHeldGroupInvite(): HeldGroupInvite {
  const { user, isAuthenticated, isLoading, impersonation } = useAuth();
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  // The invite's kind as the hold answered it, then as the join answered it.
  const [heldKind, setHeldKind] = useState<GroupInviteKind | null>(null);
  const [joinedKind, setJoinedKind] = useState<GroupInviteKind | null>(null);
  const [joinedRoom, setJoinedRoom] = useState<JoinedGroupRoom | null>(null);
  // A ref, so StrictMode's second effect run does not send either call twice.
  const sent = useRef<SentForAccount | null>(null);
  // The toast provider hands back a new `addToast` every render.
  const toast = useRef(addToast);
  toast.current = addToast;

  const userId = user?.id;
  const status = user?.user_status;
  const ready =
    !isLoading &&
    isAuthenticated &&
    (status === 'pending' || status === 'active') &&
    !impersonation.isImpersonating;

  useEffect(() => {
    if (!ready || userId === undefined) return;
    if (sent.current?.userId !== userId) {
      sent.current = { userId, hold: null, joinSent: false };
    }
    const progress = sent.current;

    const code = readCarriedGroupInvite();
    if (code !== null && progress.hold === null) {
      progress.hold = groupInviteApi.holdGroupInvite(code).then(
        ({ kind }) => {
          dropCarriedGroupInvite(code);
          setHeldKind(kind);
        },
        (error: unknown) => {
          if (isDeadInvite(error)) dropCarriedGroupInvite(code);
        },
      );
    }

    if (status !== 'active' || progress.joinSent) return;
    progress.joinSent = true;
    const hold = progress.hold;
    void (async () => {
      await hold;
      try {
        const result = await groupInviteApi.joinHeldInvite();
        switch (result.outcome) {
          case 'joined':
            setJoinedKind(result.kind);
            setJoinedRoom({ conversationId: result.conversation_id });
            void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.chat.conversations() });
            void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groups.all });
            break;
          case 'already_member':
            setJoinedKind(result.kind);
            break;
          case 'refused':
            toast.current({ type: 'error', title: result.message });
            break;
          case 'none':
            break;
        }
      } catch {
        // The server keeps the hold; the next page load asks again.
      }
    })();
  }, [ready, userId, status, queryClient]);

  const onJoinedRoomOpened = useCallback(() => setJoinedRoom(null), []);

  return {
    invitedAsAthlete: (heldKind ?? joinedKind) === 'member',
    joinedRoom,
    onJoinedRoomOpened,
  };
}
