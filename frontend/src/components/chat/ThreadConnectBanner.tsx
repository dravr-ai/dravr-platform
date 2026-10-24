// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The connect-a-provider nudge over an open thread, for an athlete whose agent can read no data yet
// ABOUTME: Shown only where a router can take them to the connections pane — never as a button that does nothing

import { ConnectProviderBanner } from '../ConnectProviderBanner';
import type { ChatThreadState } from './useChatThread';

interface ThreadConnectBannerProps {
  thread: ChatThreadState;
  onNavigate?: (route: string) => void;
}

/**
 * The banner exists to send the athlete to the connections pane, so it is
 * shown only where there is a router to send them with, and only over an open
 * thread: the empty pane already carries the provider line and the link, so
 * the banner there said it twice.
 */
export default function ThreadConnectBanner({ thread, onNavigate }: ThreadConnectBannerProps) {
  if (!thread.showConnectBanner || !onNavigate || !thread.conversationId) return null;
  return (
    <div className="px-4 pt-3 md:px-6">
      <ConnectProviderBanner kind="connect" onNavigate={onNavigate} />
    </div>
  );
}
