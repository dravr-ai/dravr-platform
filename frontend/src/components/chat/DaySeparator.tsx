// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The pill between two days of a thread — today, yesterday, or the date spelled out
// ABOUTME: A separator to assistive tech, a quiet chip on the canvas to everyone else

export default function DaySeparator({ label }: { label: string }) {
  return (
    <div role="separator" aria-label={label} data-testid="day-separator" className="my-3 flex justify-center">
      <span className="rounded-full bg-surface-container-high px-3 py-1 text-xs text-on-surface-variant">
        {label}
      </span>
    </div>
  );
}
