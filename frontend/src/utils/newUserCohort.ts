// ABOUTME: Pure formatting for the admin console's new-user view: time since sign-up and percentages
// ABOUTME: Kept out of the component file so it exports components only (fast refresh)

// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

import type { TFunction } from '@pierre/i18n';

/**
 * How long after sign-up `at` happened, in the coarsest unit that reads well:
 * minutes under an hour, hours under two days, days after that.
 */
export function sinceSignup(signedUpAt: string, at: string, t: TFunction): string {
  const minutes = Math.max(0, Math.round((Date.parse(at) - Date.parse(signedUpAt)) / 60_000));
  if (minutes < 60) {
    return t('newUsers.afterMinutes', { count: minutes });
  }
  const hours = Math.round(minutes / 60);
  if (hours < 48) {
    return t('newUsers.afterHours', { count: hours });
  }
  return t('newUsers.afterDays', { count: Math.round(hours / 24) });
}

/** `part` as a whole percentage of `whole`, or `null` when there is no whole. */
export function percentOf(part: number, whole: number): number | null {
  return whole > 0 ? Math.round((part / whole) * 100) : null;
}
