// SPDX-License-Identifier: MIT OR Apache-2.0
// Copyright (c) 2026 dravr.ai

// ABOUTME: React Query hook that resolves whether the current user needs the onboarding flow (mobile)
// ABOUTME: Mirrors the web App.tsx gate so chat/coach/messaging are reachable only with a connected provider

import { useQuery } from '@tanstack/react-query';
import { userApi } from '../services/api';

/**
 * Wraps `GET /api/me/onboarding-status`. `enabled` lets callers short-circuit
 * the request when the user isn't yet authenticated or is in a non-active
 * status (pending / suspended) — those states have their own screens upstream
 * of this gate, and firing the query early would just return a 401.
 *
 * Keep the result fresh enough that an OAuth round-trip flips the redirect
 * without a manual reload, but don't refetch in tight loops while the user
 * is parked on the onboarding screen.
 */
export function useOnboardingStatus(enabled: boolean) {
  return useQuery({
    queryKey: ['user-onboarding-status'] as const,
    queryFn: () => userApi.getOnboardingStatus(),
    enabled,
    staleTime: 5_000,
  });
}
