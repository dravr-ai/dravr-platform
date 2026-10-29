// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The marks of a coach's private rows in a group room — the "Only you and <agent> see what follows" line and the "Only you" tag
// ABOUTME: Drawn in the private accent (the warning hue under its bound ink), so a private stretch never reads as the room's (carnet#1249)

import { Lock } from 'lucide-react';
import { PRIVATE_LANE_KEYS } from '@pierre/chat-utils';
import { useTranslation } from '@pierre/i18n';

/**
 * The line above a stretch of private rows, naming the room's agent: a
 * dashed rule each side of a lock and the sentence, a separator to assistive
 * tech the way the day pill is.
 */
export function PrivateLaneDivider({ agent }: { agent: string }) {
  const { t } = useTranslation();
  const label = t(PRIVATE_LANE_KEYS.divider, { agent });
  return (
    <div
      role="separator"
      aria-label={label}
      data-testid="private-lane-divider"
      className="mt-5 flex items-center gap-2.5"
    >
      <span className="flex-1 border-t border-dashed border-warning/60" aria-hidden="true" />
      <span className="inline-flex items-center gap-1 text-xs font-medium text-on-warning-container">
        <Lock className="h-3 w-3 shrink-0" aria-hidden="true" />
        {label}
      </span>
      <span className="flex-1 border-t border-dashed border-warning/60" aria-hidden="true" />
    </div>
  );
}

/** "Only you" — beside the time on the author line of the agent's private reply. */
export function PrivateReplyTag() {
  const { t } = useTranslation();
  return (
    <span data-testid="private-reply-tag" className="text-xs font-medium text-on-warning-container">
      {t(PRIVATE_LANE_KEYS.replyTag)}
    </span>
  );
}
