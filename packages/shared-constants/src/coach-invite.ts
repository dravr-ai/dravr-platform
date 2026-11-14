// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: The team-issued coach-seat invitation's mark on onboarding — which steps it settles, and the bar's state for them
// ABOUTME: Read by the progress bar on web and phone from server state: the status says an invitation settled the role and the seat

import type { OnboardingProgressItem, OnboardingStepId } from './onboarding';

/**
 * The onboarding steps a redeemed coach-seat invitation settles before the
 * coach signs in (carnet#1524): the role question, answered as a coach, and
 * the seat, granted by the team. The server records both done; the coach
 * starts on their group.
 */
const SETTLED_BY_INVITE_STEPS: readonly OnboardingStepId[] = ['profile_type', 'coach_seat'];

/** Catalogue key of the bar's line naming the steps the invitation settled. */
export const SETTLED_BY_INVITE_KEY = 'onboarding.settledByInvite';

/**
 * The bar's segments an invitation settled: those of
 * {@link SETTLED_BY_INVITE_STEPS} the bar shows done, when the server says an
 * invitation settled them (`settled_by_invite`). Empty otherwise, so a coach
 * who answered for themselves never reads "settled by your invitation".
 */
export function settledByInviteSegments(
  progress: readonly OnboardingProgressItem[],
  settledByInvite: boolean,
): OnboardingProgressItem[] {
  if (!settledByInvite) return [];
  return progress.filter((item) => item.status === 'done' && SETTLED_BY_INVITE_STEPS.includes(item.id));
}

/**
 * The bar's line for the segments an invitation settled — "Profil et Place de
 * coach réglés par ton invitation" — or `null` when it settled none. The
 * labels are joined by the platform's list formatter in the reader's
 * language; a comma list where it has none.
 */
export function settledByInviteLine(
  t: (key: string, params?: Record<string, unknown>) => string,
  progress: readonly OnboardingProgressItem[],
  settledByInvite: boolean,
  language: string,
): string | null {
  const labels = settledByInviteSegments(progress, settledByInvite).map((item) => t(item.labelKey));
  if (labels.length === 0) return null;
  const steps =
    typeof Intl !== 'undefined' && typeof Intl.ListFormat === 'function'
      ? new Intl.ListFormat(language, { style: 'long', type: 'conjunction' }).format(labels)
      : labels.join(', ');
  return t(SETTLED_BY_INVITE_KEY, { steps });
}
