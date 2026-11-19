// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Hands a carried group invite to the server once a session exists, then joins what it holds (carnet#1648)
// ABOUTME: Runs at the app root, apart from onboarding: the hold for a pending or active account, the join for an active one
// ABOUTME: Waits while the invite card is up: the card's own button is the person's yes (carnet#1649), and a group_full refusal is Home's strip (carnet#1650)

import { useCallback, useEffect, useRef, useState } from 'react';
import { useQueryClient } from '@tanstack/react-query';
import type { GroupInviteKind, JoinHeldInviteResponse } from '@pierre/shared-types';
import { apiErrorStatus, isCoachInviteNotForAccount } from '@pierre/ui-logic';
import { groupsApi } from '../services/api';
import { useToast } from '../components/ui';
import { QUERY_KEYS } from '../constants/queryKeys';
import {
  dropCarriedGroupInvite,
  readCarriedGroupInvite,
  trackInviteSettling,
  useInviteCardOpen,
} from '../utils/groupInvite';
import { useAuth } from './useAuth';

/** The group room a join just opened, for the dashboard to show. */
export interface JoinedGroupRoom {
  /**
   * The account's conversation in the room, holding the join notice; `null`
   * when the server could not file it, and the Groups tab opens on its list.
   */
  conversationId: string | null;
}

/** What the invite card's button came to. */
export type CardJoinOutcome =
  /** Joined (or already in): the page carries on, into the room. */
  | 'joined'
  /** The server holds the invite for an account that is not active yet: the page carries on to the waiting screen. */
  | 'held'
  /** The group is full: the hold is kept and Home offers the join once a seat opens. */
  | 'full'
  /** The server called the link dead. */
  | 'dead'
  /** A coach link this account may not redeem: it has no coach access (carnet#1650). */
  | 'not_for_account'
  /** Nothing was decided: the card stays and the button tries again. */
  | 'failed';

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
  /**
   * The invite card's yes: hold the carried code, then join what the server
   * holds (an active account), as the automatic path does once the card has
   * been answered. Never both: this one marks the page's join as sent.
   */
  joinFromCard: () => Promise<CardJoinOutcome>;
}

/**
 * The server's word that the code is dead — unknown, deactivated, expired,
 * used up, or its group archived — so there is nothing to retry.
 */
function isDeadInvite(error: unknown): boolean {
  const status = apiErrorStatus(error);
  return status === 400 || status === 404;
}

/**
 * How a hold came out: the server holds it, called it dead, refused a coach
 * link this account may not redeem (nothing to retry either), or did not
 * answer.
 */
type HoldResult = 'held' | 'dead' | 'not_for_account' | 'failed';

/** What this page load has sent, for one account. */
interface SentForAccount {
  userId: string;
  /** The carried code's hold, once sent; the join waits on it. */
  hold: Promise<HoldResult> | null;
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
 * A page load that opened on an invite link starts with the invite card, and
 * nothing is held or joined until the card is answered: its join button calls
 * `joinFromCard`, and a form or "continue" button just lets the page carry on.
 *
 * Call it once, from the app root inside `AuthProvider` and `ToastProvider`.
 */
export function useHeldGroupInvite(): HeldGroupInvite {
  const { user, isAuthenticated, isLoading, impersonation } = useAuth();
  const { addToast } = useToast();
  const queryClient = useQueryClient();
  const cardOpen = useInviteCardOpen();
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

  const progressFor = useCallback((id: string): SentForAccount => {
    if (sent.current?.userId !== id) {
      sent.current = { userId: id, hold: null, joinSent: false };
    }
    return sent.current;
  }, []);

  /** Send the carried code's hold once; the server's answer decides whether the tab keeps carrying it. */
  const startHold = useCallback(
    (progress: SentForAccount, code: string): Promise<HoldResult> => {
      if (progress.hold !== null) return progress.hold;
      progress.hold = trackInviteSettling(
        groupsApi.holdInvite(code).then(
          ({ kind }): HoldResult => {
            dropCarriedGroupInvite(code);
            setHeldKind(kind);
            void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groupInvite.held() });
            return 'held';
          },
          (error: unknown): HoldResult => {
            if (isCoachInviteNotForAccount(error)) {
              dropCarriedGroupInvite(code);
              return 'not_for_account';
            }
            if (isDeadInvite(error)) {
              dropCarriedGroupInvite(code);
              return 'dead';
            }
            return 'failed';
          },
        ),
      );
      return progress.hold;
    },
    [queryClient],
  );

  /** Join what the server holds and show what came of it; `null` when the call did not answer. */
  const runJoin = useCallback(async (): Promise<JoinHeldInviteResponse | null> => {
    try {
      const result = await trackInviteSettling(groupsApi.joinHeldInvite());
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
          // A full group keeps the hold and Home's strip offers the retry; a
          // toast on every load would nag about what the strip already says.
          if (result.reason !== 'group_full') toast.current({ type: 'error', title: result.message });
          break;
        case 'none':
          break;
      }
      void queryClient.invalidateQueries({ queryKey: QUERY_KEYS.groupInvite.held() });
      return result;
    } catch {
      // The server keeps the hold; the next page load asks again.
      return null;
    }
  }, [queryClient]);

  useEffect(() => {
    if (!ready || cardOpen || userId === undefined) return;
    const progress = progressFor(userId);

    const code = readCarriedGroupInvite();
    if (code !== null && progress.hold === null) {
      void startHold(progress, code);
    }

    if (status !== 'active' || progress.joinSent) return;
    progress.joinSent = true;
    const hold = progress.hold;
    void (async () => {
      if (hold !== null) await hold;
      await runJoin();
    })();
  }, [ready, cardOpen, userId, status, progressFor, startHold, runJoin]);

  const joinFromCard = useCallback(async (): Promise<CardJoinOutcome> => {
    if (!ready || userId === undefined) return 'failed';
    const progress = progressFor(userId);
    const code = readCarriedGroupInvite();
    if (code !== null && progress.hold === null) {
      const held = await startHold(progress, code);
      if (held === 'dead' || held === 'not_for_account') return held;
      if (held === 'failed') {
        // Sent once and settled: a retry sends the hold again.
        progress.hold = null;
        return 'failed';
      }
    } else if (progress.hold !== null) {
      const held = await progress.hold;
      if (held === 'dead' || held === 'not_for_account') return held;
    }
    if (status !== 'active') return 'held';
    if (progress.joinSent) return 'joined';
    progress.joinSent = true;
    const result = await runJoin();
    if (result === null) {
      progress.joinSent = false;
      return 'failed';
    }
    if (result.outcome === 'refused') return result.reason === 'group_full' ? 'full' : 'dead';
    return 'joined';
  }, [ready, userId, status, progressFor, startHold, runJoin]);

  const onJoinedRoomOpened = useCallback(() => setJoinedRoom(null), []);

  return {
    invitedAsAthlete: (heldKind ?? joinedKind) === 'member',
    joinedRoom,
    onJoinedRoomOpened,
    joinFromCard,
  };
}
