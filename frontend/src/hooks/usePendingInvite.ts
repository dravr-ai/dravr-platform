// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The group invite this page still has to join: read from /groups/join/:code or from any tab's stored invite
// ABOUTME: Sent once per page, and forgotten only when its /group join turn is answered (registre#1648)

import { useCallback, useEffect, useState } from 'react';
import {
  clearPendingInvite,
  onPendingInviteClearedElsewhere,
  readPendingInvite,
  writePendingInvite,
} from '../utils/pendingInvite';

/** The invite code an invite link carries: `/groups/join/:code`, or null. */
function inviteCodeFromPath(): string | null {
  const match = window.location.pathname.match(/^\/groups\/join\/([^/]+)$/);
  return match ? decodeURIComponent(match[1]) : null;
}

export interface PendingInvite {
  /** The invite this page should send `/group join` for, or null. */
  pendingInviteCode: string | null;
  /**
   * The join turn was dispatched or queued on this page: it is not sent a
   * second time here. The stored invite stays, so a reload, a failed send or
   * another tab still joins.
   */
  onInviteCodeConsumed: () => void;
  /** The join turn was answered: the stored invite is forgotten, in every tab. */
  onInviteJoinAnswered: () => void;
}

/**
 * The group invite opened in this browser that has not been joined yet.
 *
 * An invite link stores its code, with an expiry, where every tab reads it,
 * and the link leaves the address bar. Each page load with an unexpired
 * stored invite hands it to the dashboard, which sends `/group join` once;
 * a repeat of an answered join is harmless on the server (an existing member
 * is told so, and no use of the code is counted), so a reply that never
 * reached this page is safe to ask for again.
 */
export function usePendingInvite(): PendingInvite {
  const [pendingInviteCode, setPendingInviteCode] = useState<string | null>(
    () => inviteCodeFromPath() ?? readPendingInvite(),
  );

  // An invite link is stored and its path cleaned, so a refresh reads the
  // stored invite rather than the link.
  useEffect(() => {
    const code = inviteCodeFromPath();
    if (!code) return;
    writePendingInvite(code);
    window.history.replaceState({}, document.title, '/');
  }, []);

  // The join was answered in another tab: this one has nothing left to send.
  useEffect(() => onPendingInviteClearedElsewhere(() => setPendingInviteCode(null)), []);

  const onInviteCodeConsumed = useCallback(() => setPendingInviteCode(null), []);
  const onInviteJoinAnswered = useCallback(() => {
    clearPendingInvite();
    setPendingInviteCode(null);
  }, []);

  return { pendingInviteCode, onInviteCodeConsumed, onInviteJoinAnswered };
}
