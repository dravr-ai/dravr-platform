// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Copy helpers for the sciotte login modal's progress text
// ABOUTME: formatTimeout renders the server-configured login budget for humans

// Render a server-supplied timeout budget as human copy. Long interactive
// flows (number-match 2FA) run into minutes, so collapse anything >= 90s to
// whole minutes; shorter budgets stay in seconds.
export function formatTimeout(secs: number): string {
  if (secs >= 90) {
    const mins = Math.round(secs / 60);
    return `${mins} minute${mins === 1 ? '' : 's'}`;
  }
  return `${secs} seconds`;
}
