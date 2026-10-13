// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The rule where a new topic opens in the personal thread — "+" or /reset drew it, the time it opened beside it
// ABOUTME: A separator to assistive tech, a hairline with a quiet label on the canvas to everyone else

/**
 * Where a new topic opens in the athlete's personal thread. "+" and `/reset`
 * divide the one thread instead of starting another, and the coach reads the
 * thread from here on — so the athlete sees where it started over.
 */
export default function TopicDivider({ label, time }: { label: string; time: string }) {
  const text = time ? `${label} · ${time}` : label;
  return (
    <div
      role="separator"
      aria-label={text}
      data-testid="topic-divider"
      className="my-4 flex items-center gap-3"
    >
      <span aria-hidden="true" className="h-px flex-1 bg-outline-variant" />
      <span className="text-xs font-medium text-outline">{text}</span>
      <span aria-hidden="true" className="h-px flex-1 bg-outline-variant" />
    </div>
  );
}
