// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Whether the open thread's info drawer is showing, and whether it opens on its participants control
// ABOUTME: A drawer left open belongs to the thread it was opened on, so a change of thread closes it

import { useCallback, useEffect, useState } from 'react';

/** Whether the drawer is open, and whether it opens on its participants control. */
export interface ThreadInfoState {
  open: boolean;
  opensParticipants: boolean;
  /** Open the drawer; `participants` expands the add-someone control, as the "+" menu's "Add someone" does. */
  show: (participants?: boolean) => void;
  close: () => void;
}

/**
 * The info drawer's open state for the thread on screen. A drawer left open
 * belongs to the previous thread, so a change of thread closes it.
 */
export function useThreadInfo(conversationId: string | null): ThreadInfoState {
  const [open, setOpen] = useState(false);
  const [opensParticipants, setOpensParticipants] = useState(false);

  useEffect(() => {
    setOpen(false);
    setOpensParticipants(false);
  }, [conversationId]);

  const show = useCallback((participants = false) => {
    setOpensParticipants(participants);
    setOpen(true);
  }, []);
  const close = useCallback(() => setOpen(false), []);

  return { open, opensParticipants, show, close };
}
