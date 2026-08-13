// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tracks whether the user has seen the one-time onboarding coach proposal (mobile)
// ABOUTME: Thin naming layer over useOnboardingFlag; the key mirrors the web `dravr.coach_proposal_done.{id}`

import { useOnboardingFlag } from './useOnboardingFlag';

/** Web-matching storage key prefix for the coach-proposal step. */
const STORAGE_PREFIX = 'dravr.coach_proposal_done.';

/**
 * Whether the coach-proposal step is already complete for `userId`.
 *
 * `seen` is `undefined` while the read is in flight; callers hold routing until
 * it resolves. See {@link useOnboardingFlag} for the fail-open rationale.
 */
export function useCoachProposalSeen(userId: string | undefined) {
  const { done, mark } = useOnboardingFlag(STORAGE_PREFIX, userId);
  return { seen: done, markSeen: mark };
}
