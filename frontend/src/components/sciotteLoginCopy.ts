// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Copy helpers for the sciotte login modal's progress text
// ABOUTME: formatTimeout renders the server-configured login budget in the reader's language

import { BUDGET_MINUTES_KEYS, BUDGET_SECONDS_KEYS, countKey } from '@pierre/shared-constants';

type Translate = (key: string, options?: { count: number }) => string;

// Render a server-supplied timeout budget as human copy. Long interactive
// flows (number-match 2FA) run into minutes, so collapse anything >= 90s to
// whole minutes; shorter budgets stay in seconds. An unknown budget reads as
// « quelques minutes ».
export function formatTimeout(secs: number | null, t: Translate, language: string | undefined): string {
  if (secs === null) {
    return t('frag.budgetFewMinutes');
  }
  if (secs >= 90) {
    const mins = Math.round(secs / 60);
    return t(countKey(mins, language, BUDGET_MINUTES_KEYS), { count: mins });
  }
  return t(countKey(secs, language, BUDGET_SECONDS_KEYS), { count: secs });
}
