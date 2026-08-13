// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: Tracks whether the user has completed the athlete-vs-coach profile-type onboarding step (mobile)
// ABOUTME: Thin naming layer over useOnboardingFlag; the key mirrors the web `dravr.profile_type_chosen.{id}`

import { useOnboardingFlag } from './useOnboardingFlag';

/** Web-matching storage key prefix for the profile-type step. */
const STORAGE_PREFIX = 'dravr.profile_type_chosen.';

/**
 * Whether the profile-type step is already complete for `userId`.
 *
 * `chosen` is `undefined` while the read is in flight; callers hold routing until
 * it resolves. See {@link useOnboardingFlag} for the fail-open rationale.
 */
export function useProfileTypeChosen(userId: string | undefined) {
  const { done, mark } = useOnboardingFlag(STORAGE_PREFIX, userId);
  return { chosen: done, markChosen: mark };
}
