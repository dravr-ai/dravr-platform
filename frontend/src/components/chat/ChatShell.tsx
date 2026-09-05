// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The chat tab's two panes — the conversation list column and the thread — and which of them a viewport shows
// ABOUTME: Wide screens hold both side by side; narrower ones show the list until a thread is open, then the thread

import type { ReactNode } from 'react';
import { useIsDesktop } from '../../hooks/useBreakpoint';

interface ChatShellProps {
  /** The conversation list column. */
  list: ReactNode;
  /** The open thread, or the empty pane when nothing is selected. */
  thread: ReactNode;
  /** Whether a thread is open — what decides which pane a narrow viewport shows. */
  hasSelection: boolean;
}

/**
 * The messenger layout: a fixed-width list beside a thread that takes the rest.
 *
 * Below the desktop breakpoint the two panes are one, the way every phone
 * messenger works — the list first, the thread once a row is opened, and the
 * thread header's back button returns to the list.
 */
export default function ChatShell({ list, thread, hasSelection }: ChatShellProps) {
  const isDesktop = useIsDesktop();
  const showList = isDesktop || !hasSelection;
  const showThread = isDesktop || hasSelection;
  return (
    <div className="flex h-full min-h-0 bg-surface" data-testid="chat-shell">
      {showList && (
        <div
          data-testid="conversation-pane"
          className="flex min-h-0 w-full shrink-0 flex-col border-r ghost-border bg-surface lg:w-[360px] xl:w-[400px]"
        >
          {list}
        </div>
      )}
      {showThread && <div className="flex min-h-0 min-w-0 flex-1 flex-col">{thread}</div>}
    </div>
  );
}
