// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: A thread embedded in another view — an activity's, the athlete workspace's panel — with no list, header or info drawer of its own
// ABOUTME: Shows the host view's own way in until a thread exists, then that thread's transcript and composer alone

import type { ReactNode } from 'react';
import { chatApi } from '../../services/api';
import ChatThread from './ChatThread';
import { useChatThread, type OpenThread } from './useChatThread';
import type { PendingComposerAction, WorkspacePlanSurface } from './types';

interface EmbeddedChatProps {
  conversationId: string | null;
  onSelectConversation: (id: string | null) => void;
  /** Text the host wants sent or drafted in the thread — its own chips and question field. */
  pendingComposerAction?: PendingComposerAction | null;
  /** Called once the action above has been drafted or dispatched. */
  onPendingComposerActionConsumed?: () => void;
  /** What shows while no thread is open: the host view's own way in. */
  emptyState: ReactNode;
  /**
   * Whether the transcript opens on its latest message. Off by default: an
   * activity's thread sits in the page and leaves it where the athlete put
   * it. A host whose panel is the thread's own scroller turns it on.
   */
  followOnOpen?: boolean;
  /** The thread sits beside the athlete workspace's calendar: plan cards are one line, the open draft is not drawn. */
  planSurface?: WorkspacePlanSurface | null;
  /** Whether a one-athlete room weaves its coach's private lane in; on by default (see `useChatThread`). */
  privateLane?: boolean;
}

/** An embedded thread is created like any other; the host links it to what it is about. */
const createThread: OpenThread = (request) => chatApi.createConversation(request);

/**
 * The thread inside another view. The view owns the way into it, so the
 * composer does not take focus on open and, unless the host asks, the
 * transcript does not scroll to its end.
 */
export default function EmbeddedChat({
  conversationId,
  onSelectConversation,
  pendingComposerAction,
  onPendingComposerActionConsumed,
  emptyState,
  followOnOpen = false,
  planSurface = null,
  privateLane = true,
}: EmbeddedChatProps) {
  const thread = useChatThread({
    conversationId,
    onSelectConversation,
    openThread: createThread,
    pendingComposerAction,
    onPendingComposerActionConsumed,
    privateLane,
  });

  return (
    <div className="relative flex h-full flex-col" data-testid="embedded-chat">
      {conversationId ? (
        <ChatThread
          thread={thread}
          conversationTitle={thread.title}
          followOnOpen={followOnOpen}
          focusComposerOnOpen={false}
          planSurface={planSurface}
        />
      ) : (
        emptyState
      )}
    </div>
  );
}
