// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The coach's avatar beside a reply — the coach's initials on the primary tint, never the brand mark
// ABOUTME: Announces its author (role="img" + aria-label) so a turn is anchored on who wrote it, not on text the header repeats

import { initialsFor } from '@pierre/chat-utils';

interface CoachAvatarProps {
  /** The author label a reply carries — the coach's title, or the product name when no coach is attached. */
  label: string;
}

/**
 * Initials on the sage tint, like every other participant in the thread. The
 * brand mark used to sit here, which put a logo beside every bubble and made
 * the product read as the author of every reply; the coach is the author.
 */
export default function CoachAvatar({ label }: CoachAvatarProps) {
  return (
    <span
      role="img"
      aria-label={label}
      className="flex h-6 w-6 shrink-0 select-none items-center justify-center rounded-full bg-primary-container text-xs font-semibold leading-none text-on-primary-container"
    >
      {initialsFor(label)}
    </span>
  );
}
